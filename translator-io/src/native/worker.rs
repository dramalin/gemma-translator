//! The thread a native service runs on, and the channel its requests go
//! through.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

/// How long dropping a `Worker` waits for its thread to free the native
/// handles: long enough for a request in progress to finish.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(3);

/// A named thread that serves requests of type `R`. Dropping it closes the
/// request channel and waits (up to `SHUTDOWN_WAIT`) until the thread has
/// returned and freed its handles. Process exit must not run while it does:
/// libmoonshine and ONNX Runtime tearing down on the worker as the C++ runtime
/// shut down threw a C++ exception into Rust, which aborts (6 of 30 exits
/// under full CPU load).
pub(crate) struct Worker<R> {
    /// `None` once dropping has begun.
    requests: Option<Sender<R>>,
    /// Disconnects when the thread has returned. (A `Mutex` only to make the
    /// worker `Sync`; only `drop` uses it.)
    finished: Mutex<Receiver<()>>,
}

impl<R: Send + 'static> Worker<R> {
    /// Starts the thread; `run` serves requests until the channel closes.
    pub(crate) fn spawn(name: &str, run: impl FnOnce(Receiver<R>) + Send + 'static) -> Worker<R> {
        let (requests, receiver) = mpsc::channel();
        let (finished_sender, finished) = mpsc::channel::<()>();
        thread::Builder::new()
            .name(name.to_string())
            .spawn(move || {
                run(receiver);
                drop(finished_sender);
            })
            .unwrap_or_else(|error| panic!("spawning the {name} thread: {error}"));
        Worker {
            requests: Some(requests),
            finished: Mutex::new(finished),
        }
    }

    /// Queues `request`; fails once the thread has stopped.
    pub(crate) fn send(&self, request: R) -> Result<(), R> {
        match &self.requests {
            Some(requests) => requests.send(request).map_err(|error| error.0),
            None => Err(request),
        }
    }
}

impl<R> Drop for Worker<R> {
    fn drop(&mut self) {
        self.requests = None;
        if let Ok(finished) = self.finished.get_mut() {
            let _ = finished.recv_timeout(SHUTDOWN_WAIT);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    use super::*;

    #[test]
    fn requests_reach_the_thread() {
        let (replies, received) = mpsc::channel();
        let worker = Worker::spawn("test", move |requests: Receiver<u32>| {
            for request in requests {
                replies.send(request * 2).unwrap();
            }
        });
        worker.send(21).unwrap();
        assert_eq!(received.recv().unwrap(), 42);
    }

    #[test]
    fn drop_waits_for_the_thread_to_finish() {
        let done = Arc::new(AtomicBool::new(false));
        let thread_done = Arc::clone(&done);
        let worker = Worker::spawn("test", move |requests: Receiver<()>| {
            for _ in requests {}
            thread::sleep(Duration::from_millis(50));
            thread_done.store(true, Ordering::SeqCst);
        });
        drop(worker);
        assert!(done.load(Ordering::SeqCst));
    }

    #[test]
    fn drop_gives_up_on_a_thread_that_does_not_finish() {
        let worker = Worker::spawn("test", |_requests: Receiver<()>| {
            thread::sleep(SHUTDOWN_WAIT * 2);
        });
        let started = Instant::now();
        drop(worker);
        let waited = started.elapsed();
        assert!(
            waited >= SHUTDOWN_WAIT && waited < SHUTDOWN_WAIT * 2,
            "{waited:?}"
        );
    }

    #[test]
    fn a_stopped_thread_refuses_requests() {
        let worker = Worker::spawn("test", |requests: Receiver<u32>| drop(requests));
        let started = Instant::now();
        while worker.send(1).is_ok() {
            assert!(started.elapsed() < Duration::from_secs(1));
            thread::sleep(Duration::from_millis(1));
        }
    }
}
