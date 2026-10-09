use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::ptr::{self, NonNull};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::library::{LiteRtLmError, LiteRtLmLibrary};

/// How often a running stream checks `should_cancel`.
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(20);
/// Upper bound on waiting for the library's final callback after a cancel.
const CANCEL_GRACE: Duration = Duration::from_secs(10);
const BACKEND: &[u8] = b"cpu\0";
const EMPTY_EXTRA_CONTEXT: &[u8] = b"{}\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEnd {
    Completed,
    Cancelled,
    MaxTokensReached,
}

/// A loaded model. Not `Send` (raw pointer): it stays on the thread that created it.
pub struct Engine {
    library: Arc<LiteRtLmLibrary>,
    pointer: NonNull<c_void>,
}

/// One conversation; drop it to free it. Borrowing the engine keeps the engine alive.
pub struct Conversation<'engine> {
    engine: &'engine Engine,
    pointer: NonNull<c_void>,
    /// Target of the latest stream callback's `user_data`, from `Box::into_raw`.
    /// Kept as a raw pointer because a litert thread may dereference it at
    /// any time; owning it through a `Box` would assert unique access.
    /// Freed only once its final callback has arrived. A state whose final
    /// callback never arrived (the cancel timed out, the send failed, a
    /// non-final error ended the wait, or `on_text`/`should_cancel`
    /// panicked) is leaked: the library may still call into it, even during
    /// or after `conversation_delete` (conversation.py keeps its ctypes
    /// callback alive for the same reason). It costs a few hundred bytes.
    stream_state: Option<*mut StreamState>,
    /// Whether the library delivered the final callback for `stream_state`.
    has_stream_ended: bool,
}

struct StreamState {
    events: Sender<StreamEvent>,
}

enum StreamEvent {
    Chunk(String),
    /// The library reported an error or its final callback (`is_final`).
    /// Only a final callback means the library is done with `user_data`.
    End {
        error: Option<String>,
        is_final: bool,
    },
}

fn c_string(text: &str) -> Result<CString, LiteRtLmError> {
    CString::new(text)
        .map_err(|_| LiteRtLmError::InvalidArgument(format!("{text:?} contains a NUL byte")))
}

fn path_c_string(path: &Path) -> Result<CString, LiteRtLmError> {
    c_string(path.to_str().ok_or_else(|| {
        LiteRtLmError::InvalidArgument(format!("{} is not UTF-8", path.display()))
    })?)
}

impl Engine {
    /// Loads `model_file` on the CPU backend; `cache_dir` must be writable
    /// (litert-lm stores its compiled-model cache there).
    pub fn new(
        library: &Arc<LiteRtLmLibrary>,
        model_file: &Path,
        cache_dir: &Path,
    ) -> Result<Engine, LiteRtLmError> {
        if !model_file.is_file() {
            return Err(LiteRtLmError::ModelMissing(model_file.to_path_buf()));
        }
        let model = path_c_string(model_file)?;
        let cache = path_c_string(cache_dir)?;
        // SAFETY: valid C strings; NULL vision/audio backends as in engine.py.
        let settings = unsafe {
            (library.engine_settings_create)(
                model.as_ptr(),
                BACKEND.as_ptr().cast(),
                ptr::null(),
                ptr::null(),
            )
        };
        if settings.is_null() {
            return Err(LiteRtLmError::Call("litert_lm_engine_settings_create"));
        }
        // SAFETY: `settings` is live until the delete below.
        let engine = unsafe {
            (library.engine_settings_set_cache_dir)(settings, cache.as_ptr());
            let engine = (library.engine_create)(settings);
            (library.engine_settings_delete)(settings);
            engine
        };
        let pointer = NonNull::new(engine).ok_or(LiteRtLmError::Call("litert_lm_engine_create"))?;
        Ok(Engine {
            library: Arc::clone(library),
            pointer,
        })
    }

    pub fn create_conversation(
        &self,
        system_message: &str,
    ) -> Result<Conversation<'_>, LiteRtLmError> {
        let system_message = c_string(system_message)?;
        let library = &self.library;
        // SAFETY: the config is created, used and deleted here; the engine is live.
        let conversation = unsafe {
            let config = (library.conversation_config_create)();
            if config.is_null() {
                return Err(LiteRtLmError::Call("litert_lm_conversation_config_create"));
            }
            (library.conversation_config_set_system_message)(config, system_message.as_ptr());
            let conversation = (library.conversation_create)(self.pointer.as_ptr(), config);
            (library.conversation_config_delete)(config);
            conversation
        };
        let pointer = NonNull::new(conversation)
            .ok_or(LiteRtLmError::Call("litert_lm_conversation_create"))?;
        Ok(Conversation {
            engine: self,
            pointer,
            stream_state: None,
            has_stream_ended: false,
        })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: conversations borrow the engine, so none is alive here.
        unsafe { (self.library.engine_delete)(self.pointer.as_ptr()) }
    }
}

impl Conversation<'_> {
    /// Sends `text` as a user message and blocks until the reply ends.
    /// `on_text` receives each text delta. `should_cancel` is polled every
    /// 20 ms; once it returns true the reply is cancelled, and the call still
    /// waits for the library's final callback before returning.
    pub fn send_message_stream(
        &mut self,
        text: &str,
        on_text: &mut dyn FnMut(&str),
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<StreamEnd, LiteRtLmError> {
        let message = c_string(
            &serde_json::json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
                .to_string(),
        )?;
        // A previous state may be freed only if its final callback arrived;
        // otherwise it is leaked (see `stream_state`).
        if let Some(previous) = self.stream_state.take() {
            if self.has_stream_ended {
                // SAFETY: from `Box::into_raw`, freed once; the library made
                // its final callback, so it no longer uses the pointer.
                drop(unsafe { Box::from_raw(previous) });
            }
        }
        self.has_stream_ended = false;
        let (events, receiver) = mpsc::channel();
        let state = Box::into_raw(Box::new(StreamState { events }));
        self.stream_state = Some(state);
        let user_data = state.cast::<c_void>();

        let library = &self.engine.library;
        // SAFETY: valid C strings; `user_data` is freed only after its final
        // callback and is otherwise leaked (see `stream_state`).
        let code = unsafe {
            (library.conversation_send_message_stream)(
                self.pointer.as_ptr(),
                message.as_ptr(),
                EMPTY_EXTRA_CONTEXT.as_ptr().cast(),
                ptr::null(),
                on_stream_event,
                user_data,
            )
        };
        if code != 0 {
            return Err(LiteRtLmError::Call(
                "litert_lm_conversation_send_message_stream",
            ));
        }

        let mut cancelled_at: Option<Instant> = None;
        loop {
            match receiver.recv_timeout(CANCEL_POLL_INTERVAL) {
                Ok(StreamEvent::Chunk(chunk)) => {
                    if cancelled_at.is_none() {
                        let text = chunk_text(&chunk);
                        if !text.is_empty() {
                            on_text(&text);
                        }
                    }
                }
                Ok(StreamEvent::End { error, is_final }) => {
                    self.has_stream_ended = is_final;
                    return stream_end(error);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    unreachable!("`stream_state` holds the sender until the next send")
                }
            }
            match cancelled_at {
                None if should_cancel() => {
                    // SAFETY: the conversation is live.
                    unsafe { (library.conversation_cancel_process)(self.pointer.as_ptr()) };
                    cancelled_at = Some(Instant::now());
                }
                Some(started) if started.elapsed() > CANCEL_GRACE => {
                    return Err(LiteRtLmError::CancelTimedOut(CANCEL_GRACE));
                }
                _ => {}
            }
        }
    }
}

impl Drop for Conversation<'_> {
    fn drop(&mut self) {
        // SAFETY: the conversation is live and deleted once.
        unsafe { (self.engine.library.conversation_delete)(self.pointer.as_ptr()) }
        if let Some(state) = self.stream_state.take() {
            if self.has_stream_ended {
                // SAFETY: from `Box::into_raw`, freed once; the library made
                // its final callback, so it no longer uses the pointer. An
                // un-finalised state is leaked (see `stream_state`).
                drop(unsafe { Box::from_raw(state) });
            }
        }
    }
}

/// Called on litert-lm's own threads.
unsafe extern "C" fn on_stream_event(
    user_data: *mut c_void,
    chunk: *const c_char,
    is_final: bool,
    error: *const c_char,
) {
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `user_data` is a `StreamState` the conversation frees only
        // after this stream's final callback, and otherwise never frees.
        let state = unsafe { &*(user_data as *const StreamState) };
        let read = |text: *const c_char| {
            // SAFETY: non-NULL strings from the library are NUL-terminated.
            (!text.is_null()).then(|| {
                unsafe { CStr::from_ptr(text) }
                    .to_string_lossy()
                    .into_owned()
            })
        };
        if let Some(chunk) = read(chunk) {
            let _ = state.events.send(StreamEvent::Chunk(chunk));
        }
        let error = read(error);
        if is_final || error.is_some() {
            let _ = state.events.send(StreamEvent::End { error, is_final });
        }
    }));
}

fn stream_end(error: Option<String>) -> Result<StreamEnd, LiteRtLmError> {
    match error {
        None => Ok(StreamEnd::Completed),
        Some(message) if message.contains("CANCELLED") => Ok(StreamEnd::Cancelled),
        Some(message) if message.contains("Max number of tokens reached") => {
            Ok(StreamEnd::MaxTokensReached)
        }
        Some(message) => Err(LiteRtLmError::Stream(message)),
    }
}

/// The spoken text of one streamed message: the `text` of every
/// `"type": "text"` content item. Thinking arrives in `channels` and is
/// dropped. A chunk that is not JSON is returned as is (conversation.py).
pub fn chunk_text(chunk: &str) -> String {
    let Ok(message) = serde_json::from_str::<serde_json::Value>(chunk) else {
        return chunk.to_string();
    };
    match message.get("content") {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter(|item| item.get("type").and_then(|kind| kind.as_str()) == Some("text"))
            .filter_map(|item| item.get("text").and_then(|text| text.as_str()))
            .collect(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_text_items_only() {
        assert_eq!(
            chunk_text(r#"{"role":"assistant","content":[{"type":"text","text":"早上"}]}"#),
            "早上"
        );
        assert_eq!(
            chunk_text(
                r#"{"role":"assistant","content":[{"type":"text","text":"a"},{"type":"tool_call","text":"x"},{"type":"text","text":"b"}]}"#
            ),
            "ab"
        );
    }

    #[test]
    fn ignores_thinking_channels() {
        assert_eq!(
            chunk_text(r#"{"role":"assistant","channels":{"thought":"hmm"},"content":[]}"#),
            ""
        );
    }

    #[test]
    fn accepts_string_content_and_falls_back_to_raw_text() {
        assert_eq!(chunk_text(r#"{"role":"assistant","content":"hi"}"#), "hi");
        assert_eq!(chunk_text("not json"), "not json");
    }

    #[test]
    fn only_the_final_callback_ends_the_stream() {
        let (events, receiver) = mpsc::channel();
        let mut state = StreamState { events };
        let user_data = (&mut state as *mut StreamState).cast::<c_void>();
        // SAFETY: `state` outlives both calls; the strings are NUL-terminated.
        unsafe {
            on_stream_event(user_data, ptr::null(), false, c"INTERNAL: boom".as_ptr());
            on_stream_event(user_data, ptr::null(), true, ptr::null());
        }
        assert!(matches!(
            receiver.try_recv(),
            Ok(StreamEvent::End { error: Some(message), is_final: false }) if message == "INTERNAL: boom"
        ));
        assert!(matches!(
            receiver.try_recv(),
            Ok(StreamEvent::End {
                error: None,
                is_final: true
            })
        ));
    }

    #[test]
    fn classifies_stream_ends() {
        assert_eq!(stream_end(None), Ok(StreamEnd::Completed));
        assert_eq!(
            stream_end(Some("CANCELLED: Task cancelled".into())),
            Ok(StreamEnd::Cancelled)
        );
        assert_eq!(
            stream_end(Some("Max number of tokens reached".into())),
            Ok(StreamEnd::MaxTokensReached)
        );
        assert!(matches!(
            stream_end(Some("INTERNAL: boom".into())),
            Err(LiteRtLmError::Stream(message)) if message == "INTERNAL: boom"
        ));
    }
}
