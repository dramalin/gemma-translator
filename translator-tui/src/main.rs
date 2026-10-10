//! `translator-tui`: the Gemma Translator in a terminal (desktop or SSH).
//! Recording toggles: tap Z to start, tap it again to stop.

mod keys;
mod render;
mod terminal;

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event};
use translator_app::cli::{load_config, parse_args};
use translator_app::{AppState, KeyAction, RecordStyle};
use translator_core::config::Config;

use crate::keys::{is_quit, key_from_event, RepeatFilter, REPEAT_WINDOW};
use crate::terminal::{
    enter, install_panic_hook, install_quit_signals, is_quit_requested, is_truecolor,
    is_usable_term, leave, log_path, StderrRedirect, Tui,
};

const USAGE: &str = "usage: translator-tui [--mock] [--config <path>] [--model-dir <path>]";
/// Job events, visualizer and drawing are updated at ~33 fps.
const FRAME_INTERVAL: Duration = Duration::from_millis(30);

fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let term = std::env::var("TERM").ok();
    if !io::stdout().is_terminal() || !is_usable_term(term.as_deref()) {
        eprintln!("translator-tui needs a terminal (stdout is not one, or TERM is unset or dumb)");
        return ExitCode::from(2);
    }
    let (config, config_path) = match options.config_path.or_else(Config::default_path) {
        Some(path) => load_config(path),
        None => (Config::default(), None),
    };

    let log = log_path(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"));
    let redirect = match &log {
        Some(path) => match StderrRedirect::to_file(path) {
            Ok(redirect) => Some(redirect),
            Err(error) => {
                eprintln!(
                    "warning: cannot log to {} ({error}); library output may garble the screen",
                    path.display()
                );
                None
            }
        },
        None => {
            eprintln!("warning: no XDG_STATE_HOME or HOME; library output may garble the screen");
            None
        }
    };
    install_panic_hook(log.clone());
    install_quit_signals();
    let is_truecolor = is_truecolor(std::env::var("COLORTERM").ok().as_deref());

    let mut state = AppState::new(
        config,
        config_path,
        options.is_mock,
        RecordStyle::Toggle,
        options.model_dir,
    );
    let mut terminal = match enter() {
        Ok(terminal) => terminal,
        Err(error) => {
            drop(redirect);
            let _ = writeln!(
                io::stderr(),
                "translator-tui: cannot set up the terminal: {error}"
            );
            return ExitCode::from(2);
        }
    };
    let result = run(&mut terminal, &mut state, is_truecolor);
    leave();
    drop(terminal);
    let is_logging = redirect.is_some();
    drop(redirect);

    if let (true, Some(path)) = (is_logging, &log) {
        let _ = writeln!(io::stdout(), "log: {}", path.display());
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "translator-tui: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Terminal events are read on their own thread: after a hangup crossterm's
/// read loop can spin without ever returning, and the main loop must still
/// see the SIGHUP quit flag. Returning from `main` ends the process even if
/// that thread is stuck. The thread stops after a read error (which it
/// forwards) or once the receiver is gone.
fn spawn_event_reader() -> io::Result<Receiver<io::Result<Event>>> {
    let (sender, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("terminal-events".into())
        .spawn(move || loop {
            let event = event::read();
            let is_error = event.is_err();
            if sender.send(event).is_err() || is_error {
                break;
            }
        })?;
    Ok(receiver)
}

fn run(terminal: &mut Tui, state: &mut AppState, is_truecolor: bool) -> io::Result<()> {
    let events = spawn_event_reader()?;
    let mut filter = RepeatFilter::new(REPEAT_WINDOW);
    while !is_quit_requested() {
        let view = state.view();
        terminal.draw(|frame| render::draw(frame, &view, is_truecolor))?;
        match events.recv_timeout(FRAME_INTERVAL) {
            Ok(Ok(Event::Key(key_event))) => {
                if is_quit(&key_event) {
                    return Ok(());
                }
                if let Some(key) = key_from_event(&key_event) {
                    if filter.accept(key, key_event.kind, Instant::now()) {
                        state.handle_key(key, KeyAction::Pressed);
                    }
                }
            }
            Ok(Ok(_)) | Err(RecvTimeoutError::Timeout) => {}
            Ok(Err(error)) => return Err(error),
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        state.tick();
    }
    Ok(())
}
