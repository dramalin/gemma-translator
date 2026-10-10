//! `translator`: the Gemma Translator kiosk UI.

mod render;

slint::include_modules!();

use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use translator_app::cli::{load_config, parse_args};
use translator_app::{AppState, KeyAction, RecordStyle};
use translator_core::config::Config;
use translator_core::keys::Key;

const USAGE: &str = "usage: translator [--mock] [--config <path>]";
/// Job events, visualizer and rendering are updated at ~33 fps.
const FRAME_INTERVAL: Duration = Duration::from_millis(30);

fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let (config, config_path) = match options.config_path.or_else(Config::default_path) {
        Some(path) => load_config(path),
        None => (Config::default(), None),
    };
    let window = match AppWindow::new() {
        Ok(window) => window,
        Err(error) => {
            eprintln!("[ui] cannot create window: {error}");
            return ExitCode::FAILURE;
        }
    };

    let state = Rc::new(RefCell::new(AppState::new(
        config,
        config_path,
        options.is_mock,
        RecordStyle::Hold,
    )));
    let models = Rc::new(render::Models::bind(&window));
    render::render(&state.borrow().view(), &window, &models);

    let key_state = Rc::clone(&state);
    window.on_key_pressed(move |text| {
        Key::from_event_text(&text)
            .is_some_and(|key| key_state.borrow_mut().handle_key(key, KeyAction::Pressed))
    });
    let release_state = Rc::clone(&state);
    window.on_key_released(move |text| {
        Key::from_event_text(&text).is_some_and(|key| {
            release_state
                .borrow_mut()
                .handle_key(key, KeyAction::Released)
        })
    });
    let focus_state = Rc::clone(&state);
    window.on_keyboard_focus_lost(move || focus_state.borrow_mut().handle_focus_lost());

    let frame_timer = slint::Timer::default();
    let weak_window = window.as_weak();
    let frame_state = Rc::clone(&state);
    let frame_models = Rc::clone(&models);
    frame_timer.start(slint::TimerMode::Repeated, FRAME_INTERVAL, move || {
        let Some(window) = weak_window.upgrade() else {
            return;
        };
        let mut state = frame_state.borrow_mut();
        state.tick();
        render::render(&state.view(), &window, &frame_models);
    });

    match window.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[ui] {error}");
            ExitCode::FAILURE
        }
    }
}
