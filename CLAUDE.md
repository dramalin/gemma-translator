# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

Offline, on-device voice translator (target: Raspberry Pi 5 8GB with a 480x320 kiosk display). One Rust program per frontend (`translator` with a Slint UI, `translator-tui` in a terminal) that records audio, runs Moonshine STT, Gemma translation (LiteRT-LM) and Moonshine TTS in-process through the vendor native libraries, and plays the result. No server processes, no Python, no Node.

## Commands

```bash
make setup              # check system tools, fetch native libs and models (SETUP_ARGS=--yes)
make run                # Slint UI with in-process inference on MODEL_DIR (default ./models)
make run-tui            # terminal UI (ARGS=--mock for mock services)
make run-mock           # Slint UI with mock services
make test               # cargo test --workspace
make clippy             # clippy with -D warnings
make fmt                # cargo fmt --all
make native-libs        # fetch pinned libmoonshine / liblitert-lm into native/<arch>/lib
make models             # download all models into MODEL_DIR (default ./models)
make test-ffi           # #[ignore]d tests that need native libs + models
make test-soak          # long-running native memory tests
make package-pi         # build dist/gemma-translator-pi.tar (cross + Docker, x86_64 Linux)
sudo ./install.sh       # on the Pi, from the unpacked package: install binary, models and systemd service
cargo test -p translator-core controller   # one crate, tests matching "controller"
```

The Rust workspace (`litert-lm-sys`, `moonshine-sys`, `translator-app`, `translator-core`, `translator-io`, `translator-models`, `translator-tui`, `translator-ui`) is checked with `make test`, `make clippy` (`-D warnings`) and `cargo fmt --all -- --check`. `make run` and `make run-tui` pass `--model-dir $(MODEL_DIR) $(ARGS)`; `--model-dir` is a run-time override that is never saved to the config and is ignored with `--mock`.

## Architecture

- `translator-core`: pure logic (languages, key mapping, sentence splitting, config, the `Controller` state machine). It must not depend on cpal or slint. The controller returns `Effect`s instead of doing I/O; `translator-app/src/state.rs` executes them.
- `translator-io`: cpal audio, volume commands, `chinese_script.rs` (Traditional/Simplified conversion via zhconv), and `JobRunner`, which pipelines STT -> streaming translation -> per-sentence TTS on background threads and reports `JobEvent`s.
- `translator-ui`: Slint binary `translator`; `--mock` swaps in fake services. Job events reach the UI thread through an mpsc channel drained by a 30 ms timer.
- `moonshine-sys` / `litert-lm-sys`: run-time (`libloading`) bindings to the vendor libraries from the pinned wheels; signatures are copied from the wheels' ctypes code, so re-check them on any version bump. Handles are `!Send` and live on one worker thread each (`translator-io/src/native/`). The libraries load from `lib/` next to the executable.
- `translator-app`: UI-independent `AppState` (audio, job runner, volume, config saving), the `AppView` view model, mock services and CLI parsing, shared by both frontends. Frontends call `handle_key`, `tick` and `view`; `RecordStyle::Hold` (Slint) vs `Toggle` (TUI). `AppState::headless` runs without audio devices or volume commands, for tests.
- `translator-tui`: ratatui/crossterm terminal frontend (`make run-tui`). Filters auto-repeat on the recording keys (1 s window), ignores Ctrl/Alt combinations, exits cleanly on SIGTERM/SIGINT/SIGHUP, and redirects fd 2 to `~/.local/state/gemma-translator/translator-tui.log` while running; only a main-thread panic restores the terminal.
- `translator-models`: downloads Gemma, the Moonshine STT models and TTS assets into `model_dir` (layout in `translator-core/src/models.rs`).
- Native tests are `#[ignore]`d: `make native-libs models test-ffi`.
- `make native-libs` needs `patchelf` (it sets the RUNPATH on the fetched `.so` files). The native libraries print log lines into `make test-ffi` output, so count results from the `test result:` lines only.
- Pi packaging: `deploy/Dockerfile.pi` (Debian bookworm, arm64 dev packages) via `Cross.toml`; `scripts/check-symbol-versions.sh` caps GLIBC_2.36 / GLIBCXX_3.4.30; `deploy/install.sh` checks a fixed runtime package list before changing anything and enables linger so PipeWire runs without a desktop session.
- macOS (Apple Silicon, macOS 15+): the libraries are `.dylib` files that carry an `@loader_path` rpath, so no patchelf; volume goes through `osascript`.
- Agents cannot inject key presses into the Slint window on Wayland; check Slint key handling by hand. The TUI can be smoke-tested in a pty with `--mock`: closing the pty master must exit within ~30 ms with no stray process.
- TUI: don't use `ratatui::init()`/`restore()` (their panic hook fires on job threads; use `terminal::enter`/`leave`), and never `writeln!(...).unwrap()`/`println!` to the tty: a dead tty aborts the process, so use `let _ = writeln!`.

## Git

- Never add a `Co-Authored-By:` trailer (or any other AI attribution line) to commit messages or PR bodies.
