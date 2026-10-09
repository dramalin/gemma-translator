//! `translator`: the Gemma Translator kiosk UI.

mod app;
mod mock;
mod view;

slint::include_modules!();

use std::cell::RefCell;
use std::path::{Path, PathBuf};
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

/// Loads the config and returns it with the path settings are saved to. A file
/// that cannot be read or parsed is renamed to `<name>.bad` so the first saved
/// settings change does not overwrite it; if even that fails, settings are not
/// saved this session.
fn load_config(path: PathBuf) -> (Config, Option<PathBuf>) {
    let error = match Config::load(&path) {
        Ok(config) => return (config, Some(path)),
        Err(error) => error,
    };
    let bad_path = unused_backup_path(&path);
    match std::fs::rename(&path, &bad_path) {
        Ok(()) => {
            eprintln!(
                "[config] {error}; moved it to {} and using defaults",
                bad_path.display()
            );
            (Config::default(), Some(path))
        }
        Err(rename_error) => {
            eprintln!(
                "[config] {error}; cannot move it to {} ({rename_error}); \
                 using defaults and not saving settings",
                bad_path.display()
            );
            (Config::default(), None)
        }
    }
}

/// `<name>.bad`, or `<name>.bad.1`, `.bad.2`, … when earlier backups exist,
/// so a second broken config never overwrites the first one's backup.
fn unused_backup_path(path: &Path) -> PathBuf {
    let with_suffix = |suffix: String| {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        PathBuf::from(name)
    };
    std::iter::once(with_suffix(".bad".to_string()))
        .chain((1..).map(|index| with_suffix(format!(".bad.{index}"))))
        .find(|candidate| !candidate.exists())
        .expect("an unused backup name exists")
}

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
    let focus_state = Rc::clone(&state);
    window.on_keyboard_focus_lost(move || focus_state.borrow_mut().handle_focus_lost());

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
    use translator_core::keys::KeyboardMode;

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

    #[test]
    fn invalid_config_is_moved_aside_before_anything_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "theme_color = [broken").unwrap();
        let (config, save_path) = load_config(path.clone());
        assert_eq!(config, Config::default());
        assert_eq!(save_path.as_ref(), Some(&path));
        assert!(!path.exists());
        let bad_path = dir.path().join("config.toml.bad");
        assert_eq!(
            std::fs::read_to_string(&bad_path).unwrap(),
            "theme_color = [broken"
        );
        config.save(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&bad_path).unwrap(),
            "theme_color = [broken"
        );
    }

    #[test]
    fn missing_or_valid_config_is_loaded_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(
            load_config(path.clone()),
            (Config::default(), Some(path.clone()))
        );
        std::fs::write(&path, "keyboard_mode = \"vertical\"\n").unwrap();
        let (config, save_path) = load_config(path.clone());
        assert_eq!(config.keyboard_mode, KeyboardMode::Vertical);
        assert_eq!(save_path, Some(path));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_second_broken_config_keeps_the_first_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "first = [broken").unwrap();
        load_config(path.clone());
        std::fs::write(&path, "second = [broken").unwrap();
        load_config(path.clone());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("config.toml.bad")).unwrap(),
            "first = [broken"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("config.toml.bad.1")).unwrap(),
            "second = [broken"
        );
    }

    #[test]
    fn config_that_cannot_be_moved_aside_disables_saving() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        let path = locked.join("config.toml");
        std::fs::write(&path, "theme_color = [broken").unwrap();
        // No write permission on the directory: the rename to `.bad` fails.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = load_config(path);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(result, (Config::default(), None));
    }
}
