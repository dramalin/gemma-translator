//! UI-independent application state shared by the Slint (`translator`) and
//! terminal (`translator-tui`) frontends: each frontend turns its key events
//! into `handle_key` calls, calls `tick` every frame and draws `view()`.

pub mod cli;
pub mod mock;
pub mod state;
pub mod view;

pub use state::{AppState, KeyAction, RecordStyle};
pub use view::{AppView, LaneView, SettingsRowView};
