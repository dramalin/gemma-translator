//! Keeping the terminal usable: raw mode and the alternate screen, fd 2 sent
//! to a log file while the TUI owns the screen (native libraries write there
//! directly), clean exits on SIGTERM/SIGINT/SIGHUP, and a main-thread panic
//! hook.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Stdout, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::thread;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::Terminal;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

static IS_QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);
/// The original stderr saved by a live `StderrRedirect`, or -1 when none.
static SAVED_STDERR: AtomicI32 = AtomicI32::new(-1);

/// Raw mode and the alternate screen. Unlike `ratatui::init`, this installs
/// no panic hook (see `install_panic_hook`) and reports failure instead of
/// panicking.
pub fn enter() -> io::Result<Tui> {
    enable_raw_mode()?;
    if let Err(error) = execute!(io::stdout(), EnterAlternateScreen) {
        leave();
        return Err(error);
    }
    Terminal::new(CrosstermBackend::new(io::stdout())).inspect_err(|_| leave())
}

/// Cooked mode, main screen, visible cursor. Errors are ignored: after an
/// SSH drop the terminal may already be gone.
pub fn leave() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}

/// `$XDG_STATE_HOME/gemma-translator/translator-tui.log`, else
/// `$HOME/.local/state/...`; `None` when neither is set and non-empty, so the
/// log never lands in a relative path.
pub fn log_path(state_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = state_home
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            home.filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".local").join("state"))
        })?;
    Some(base.join("gemma-translator").join("translator-tui.log"))
}

/// While alive, fd 2 writes to the log file; dropping it puts the original
/// stderr back.
pub struct StderrRedirect {
    saved: RawFd,
}

impl StderrRedirect {
    pub fn to_file(path: &Path) -> io::Result<StderrRedirect> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        // SAFETY: duplicating the process's own stderr descriptor; close-on-exec
        // keeps it out of child processes (wpctl, amixer).
        let saved = unsafe { libc::fcntl(libc::STDERR_FILENO, libc::F_DUPFD_CLOEXEC, 3) };
        if saved < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: both descriptors are open; fd 2 now refers to the file,
        // which stays open after `file` is dropped because fd 2 holds it.
        if unsafe { libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) } < 0 {
            let error = io::Error::last_os_error();
            // SAFETY: `saved` was returned by fcntl above and is not used again.
            unsafe { libc::close(saved) };
            return Err(error);
        }
        SAVED_STDERR.store(saved, Ordering::SeqCst);
        Ok(StderrRedirect { saved })
    }
}

impl Drop for StderrRedirect {
    fn drop(&mut self) {
        // The panic hook must not use the number once it is closed.
        SAVED_STDERR.store(-1, Ordering::SeqCst);
        // SAFETY: `saved` is the duplicate made in `to_file`; it is closed once.
        unsafe {
            libc::dup2(self.saved, libc::STDERR_FILENO);
            libc::close(self.saved);
        }
    }
}

/// Starts logging fd 2 to `log`, warning on stderr (still the terminal) when
/// that is not possible.
pub fn redirect_stderr(log: Option<&Path>) -> Option<StderrRedirect> {
    let Some(path) = log else {
        eprintln!("warning: no XDG_STATE_HOME or HOME; library output may garble the screen");
        return None;
    };
    StderrRedirect::to_file(path)
        .inspect_err(|error| {
            eprintln!(
                "warning: cannot log to {} ({error}); library output may garble the screen",
                path.display()
            );
        })
        .ok()
}

/// Must be called on the main thread. A panic there restores the terminal
/// and, while a `StderrRedirect` is alive, the original stderr, then prints
/// the message and the log path. Panics on other threads (JobRunner catches
/// job panics and the TUI keeps running) only go to the default hook, which
/// writes to the log file.
pub fn install_panic_hook(log: Option<PathBuf>) {
    let main_thread = thread::current().id();
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let is_main_thread = thread::current().id() == main_thread;
        if is_main_thread {
            leave();
            let saved = SAVED_STDERR.load(Ordering::SeqCst);
            if saved >= 0 {
                // SAFETY: `saved` is the live StderrRedirect's duplicate.
                unsafe { libc::dup2(saved, libc::STDERR_FILENO) };
            }
        }
        default_hook(info);
        if is_main_thread {
            if let Some(log) = &log {
                let _ = writeln!(io::stderr(), "log: {}", log.display());
            }
        }
    }));
}

extern "C" fn request_quit(_signal: libc::c_int) {
    IS_QUIT_REQUESTED.store(true, Ordering::SeqCst);
}

/// SIGTERM (`kill`), SIGINT (`kill -INT`; Ctrl-C itself arrives as a key in
/// raw mode) and SIGHUP (the SSH session dropped) end the main loop like
/// Ctrl-C, so the terminal is restored.
pub fn install_quit_signals() {
    let handler = request_quit as extern "C" fn(libc::c_int) as libc::sighandler_t;
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        // SAFETY: the handler only stores to an atomic (async-signal-safe).
        unsafe { libc::signal(signal, handler) };
    }
}

pub fn is_quit_requested() -> bool {
    IS_QUIT_REQUESTED.load(Ordering::SeqCst)
}

/// `COLORTERM=truecolor` or `24bit`.
pub fn is_truecolor(colorterm: Option<&str>) -> bool {
    matches!(colorterm, Some("truecolor" | "24bit"))
}

/// A terminal type the TUI can draw on: set, and not `dumb`.
pub fn is_usable_term(term: Option<&str>) -> bool {
    matches!(term, Some(term) if !term.is_empty() && term != "dumb")
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::os::fd::AsFd;
    use std::os::unix::fs::MetadataExt;

    use super::*;

    #[test]
    fn log_path_prefers_xdg_state_home() {
        assert_eq!(
            log_path(
                Some(OsString::from("/state")),
                Some(OsString::from("/home/u"))
            ),
            Some(PathBuf::from("/state/gemma-translator/translator-tui.log"))
        );
    }

    #[test]
    fn log_path_falls_back_to_home_local_state() {
        let expected = Some(PathBuf::from(
            "/home/u/.local/state/gemma-translator/translator-tui.log",
        ));
        assert_eq!(log_path(None, Some(OsString::from("/home/u"))), expected);
        assert_eq!(
            log_path(Some(OsString::new()), Some(OsString::from("/home/u"))),
            expected
        );
        assert_eq!(log_path(None, None), None);
        assert_eq!(log_path(None, Some(OsString::new())), None);
        assert_eq!(log_path(Some(OsString::new()), Some(OsString::new())), None);
    }

    /// One test drives both checks, so no other test in this binary can see
    /// fd 2 redirected.
    #[test]
    fn stderr_redirect_sends_fd_2_to_the_file_and_restores_it_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("translator-tui.log");
        let message = b"from C\n";
        {
            let _redirect = StderrRedirect::to_file(&path).unwrap();
            assert!(SAVED_STDERR.load(Ordering::SeqCst) > 2);
            assert_eq!(stderr_identity(), file_identity(&path));
            // Write straight to fd 2, as a C library would.
            // SAFETY: valid buffer and length.
            let written = unsafe { libc::write(2, message.as_ptr().cast(), message.len()) };
            assert_eq!(written, message.len() as isize);
        }
        assert_eq!(std::fs::read(&path).unwrap(), message);
        assert_eq!(SAVED_STDERR.load(Ordering::SeqCst), -1);
        // Restored: fd 2 no longer refers to the log file.
        assert_ne!(stderr_identity(), file_identity(&path));
    }

    /// Device and inode of whatever fd 2 refers to.
    fn stderr_identity() -> (u64, u64) {
        // A duplicate of fd 2 refers to the same file; std's metadata gives
        // u64 dev/ino on every Unix (libc's `stat` fields differ by platform).
        let stderr = std::io::stderr().as_fd().try_clone_to_owned().unwrap();
        let metadata = std::fs::File::from(stderr).metadata().unwrap();
        (metadata.dev(), metadata.ino())
    }

    fn file_identity(path: &Path) -> (u64, u64) {
        let metadata = std::fs::metadata(path).unwrap();
        (metadata.dev(), metadata.ino())
    }

    #[test]
    fn stderr_redirect_to_an_unwritable_path_is_an_error() {
        assert!(StderrRedirect::to_file(Path::new("/proc/no/such/dir/log")).is_err());
    }

    #[test]
    fn sigterm_requests_a_clean_quit() {
        install_quit_signals();
        assert!(!is_quit_requested());
        // SAFETY: the handler installed above only sets an atomic flag.
        unsafe { libc::raise(libc::SIGTERM) };
        assert!(is_quit_requested());
    }

    #[test]
    fn truecolor_comes_from_colorterm() {
        assert!(is_truecolor(Some("truecolor")));
        assert!(is_truecolor(Some("24bit")));
        assert!(!is_truecolor(Some("")));
        assert!(!is_truecolor(None));
    }

    #[test]
    fn dumb_or_missing_term_is_unusable() {
        assert!(is_usable_term(Some("xterm-256color")));
        assert!(!is_usable_term(Some("dumb")));
        assert!(!is_usable_term(Some("")));
        assert!(!is_usable_term(None));
    }
}
