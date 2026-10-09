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
    /// Target of the latest stream callback's `user_data`.
    stream_state: Option<Box<StreamState>>,
    /// Whether the library delivered the final callback for `stream_state`.
    has_stream_ended: bool,
    /// States of streams whose final callback never arrived (the cancel timed
    /// out, the send failed, or `on_text`/`should_cancel` panicked). The
    /// library may still call into them, so they are freed only after
    /// `conversation_delete`, after which no callback follows. Boxed because
    /// the library holds their addresses, which a growing `Vec` would move.
    #[allow(clippy::vec_box)]
    abandoned_stream_states: Vec<Box<StreamState>>,
}

struct StreamState {
    events: Sender<StreamEvent>,
}

enum StreamEvent {
    Chunk(String),
    Final { error: Option<String> },
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
            abandoned_stream_states: Vec::new(),
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
        // A previous state may be freed only if its final callback arrived.
        if let Some(previous) = self.stream_state.take() {
            if !self.has_stream_ended {
                self.abandoned_stream_states.push(previous);
            }
        }
        self.has_stream_ended = false;
        let (events, receiver) = mpsc::channel();
        let state = self.stream_state.insert(Box::new(StreamState { events }));
        let user_data = &**state as *const StreamState as *mut c_void;

        let library = &self.engine.library;
        // SAFETY: valid C strings; `user_data` outlives every callback (see
        // `stream_state` and `abandoned_stream_states`).
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
                Ok(StreamEvent::Final { error }) => {
                    self.has_stream_ended = true;
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
        // SAFETY: deleted once; the stream states are dropped after this body
        // runs, and no callback follows `conversation_delete`.
        unsafe { (self.engine.library.conversation_delete)(self.pointer.as_ptr()) }
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
        // SAFETY: `user_data` is a `StreamState` kept alive by the conversation.
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
            let _ = state.events.send(StreamEvent::Final { error });
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
