//! `translator`: the Gemma Translator kiosk UI.

mod app;
mod mock;
mod view;

slint::include_modules!();

use std::cell::RefCell;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use translator_core::config::Config;

const USAGE: &str = "usage: translator [--mock] [--config <path>]";
/// Job events, visualizer and rendering are updated at ~33 fps.
const FRAME_INTERVAL: Duration = Duration::from_millis(30);

#[derive(Debug, PartialEq, Eq)]
struct CliOptions {
    is_mock: bool,
    config_path: Option<PathBuf>,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<CliOptions, String> {
    let mut options = CliOptions {
        is_mock: false,
        config_path: None,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mock" => options.is_mock = true,
            "--config" => {
                let path = args.next().ok_or("--config needs a path")?;
                options.config_path = Some(PathBuf::from(path));
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(options)
}

fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let config_path = options.config_path.or_else(Config::default_path);
    let config = match &config_path {
        Some(path) => Config::load(path).unwrap_or_else(|error| {
            eprintln!("[config] {error}; using defaults");
            Config::default()
        }),
        None => Config::default(),
    };
    let window = match AppWindow::new() {
        Ok(window) => window,
        Err(error) => {
            eprintln!("[ui] cannot create window: {error}");
            return ExitCode::FAILURE;
        }
    };

    let state = Rc::new(RefCell::new(app::AppState::new(
        config,
        config_path,
        options.is_mock,
    )));
    state.borrow().bind_models(&window);
    state.borrow().render(&window);

    let key_state = Rc::clone(&state);
    window.on_key_pressed(move |text| key_state.borrow_mut().handle_key_pressed(&text));
    let release_state = Rc::clone(&state);
    window.on_key_released(move |text| release_state.borrow_mut().handle_key_released(&text));

    let frame_timer = slint::Timer::default();
    let weak_window = window.as_weak();
    let frame_state = Rc::clone(&state);
    frame_timer.start(slint::TimerMode::Repeated, FRAME_INTERVAL, move || {
        let Some(window) = weak_window.upgrade() else {
            return;
        };
        let mut state = frame_state.borrow_mut();
        state.tick();
        state.render(&window);
    });

    match window.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[ui] {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn parses_mock_and_config() {
        assert_eq!(
            parse_args(args(&["--mock", "--config", "/tmp/c.toml"])),
            Ok(CliOptions {
                is_mock: true,
                config_path: Some(PathBuf::from("/tmp/c.toml"))
            })
        );
        assert_eq!(
            parse_args(args(&[])),
            Ok(CliOptions {
                is_mock: false,
                config_path: None
            })
        );
    }

    #[test]
    fn rejects_unknown_and_incomplete_arguments() {
        assert!(parse_args(args(&["--verbose"])).is_err());
        assert!(parse_args(args(&["--config"])).is_err());
    }
}
