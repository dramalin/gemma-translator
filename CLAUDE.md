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
make release            # optimized translator + translator-tui in target/release, native libs linked beside them
make test               # cargo test --workspace
make clippy             # clippy with -D warnings
make fmt                # cargo fmt --all
make clean              # cargo clean + rm -rf dist (keeps native/ and models/)
make native-libs        # fetch pinned libmoonshine / liblitert-lm into native/<arch>/lib
make models             # download all models into MODEL_DIR (default ./models), plus the config's [tts_voices] (CONFIG=<file>)
make voices             # list every TTS voice per language (* = downloaded)
make test-ffi           # #[ignore]d tests that need native libs + models
make test-soak          # long-running native memory tests
make eval-stt           # score STT on the recorded corpus in eval/ (see eval/README.md)
make package-pi         # build dist/gemma-translator-pi.tar (cross + Docker, x86_64 Linux)
sudo ./install.sh       # on the Pi, from the unpacked package: install binary, models and systemd service
cargo test -p translator-core controller   # one crate, tests matching "controller"
```

The Rust workspace (`litert-lm-sys`, `moonshine-sys`, `onnxruntime-sys`, `translator-app`, `translator-core`, `translator-io`, `translator-models`, `translator-tui`, `translator-ui`) is checked with `make test`, `make clippy` (`-D warnings`) and `cargo fmt --all -- --check`. `make run` and `make run-tui` pass `--model-dir $(MODEL_DIR) $(ARGS)`; `--model-dir` is a run-time override that is never saved to the config and is ignored with `--mock`.

## Architecture

- `translator-core`: pure logic (languages, key mapping, sentence splitting, config, the `Controller` state machine). It must not depend on cpal or slint. The controller returns `Effect`s instead of doing I/O; `translator-app/src/state.rs` executes them.
- `translator-io`: cpal audio, volume commands, `chinese_script.rs` (Traditional/Simplified conversion via zhconv), and `JobRunner`, which pipelines STT -> streaming translation -> per-sentence TTS on background threads and reports `JobEvent`s.
- `translator-ui`: Slint binary `translator`; `--mock` swaps in fake services. Job events reach the UI thread through an mpsc channel drained by a 30 ms timer.
- `moonshine-sys` / `litert-lm-sys`: run-time (`libloading`) bindings to the vendor libraries from the pinned wheels; signatures and struct layouts are copied from the wheels' ctypes code (`moonshine_api.py`), so re-check them on any version bump, along with `MOONSHINE_HEADER_VERSION` (30000 for 0.1.x). Buffers the C API returns are freed with `moonshine_free_buffer`, not libc `free`. Handles are `!Send` and live on one worker thread each (`translator-io/src/native/`). Dropping `NativeSpeech` or `NativeTranslator` waits (up to 3 s, `native/worker.rs`) for its thread to free them: Moonshine/ORT tearing down while the process exits threw a C++ exception into Rust and aborted (about 1 exit in 5 under CPU load). The libraries load from `lib/` next to the executable.
- `onnxruntime-sys`: minimal bindings to the ONNX Runtime that ships with libmoonshine (found by globbing `lib/libonnxruntime*`), used only for the Silero VAD (`<model_dir>/vad/silero_vad.onnx`). It indexes the `OrtApi` function table by slot, so re-check the slots in `ffi.rs` against `onnxruntime_c_api.h` when the wheel's ORT version changes. The ORT environment is shared with libmoonshine and never released.
- Speech trimming (`translator-io/src/speech_trim.rs`) runs on the speech thread before Moonshine: by level, or by the Silero VAD when it shows the level took noise for speech (`TrimMode::Adaptive`). Moonshine's own VAD stays off (`vad_threshold=0`): its state is process-global and never reset. Without the VAD model, trimming falls back to level only. Streaming models then get 250 ms of digital silence appended (`STREAMING_TAIL_PADDING_MS`, `pad_tail`): their encoder looks ahead, and without it the last word of a trimmed recording came out wrong under moonshine-voice 0.1.5. Judge changes with `make eval-stt` (`EVAL_ARGS="--trim energy"` to compare).
- Transcript cleanup (`translator-core/src/text.rs`, `clean_transcript`, called on the speech thread): Moonshine's zh model outputs Simplified text with spaces between characters, falls into repetition loops, and restarts the sentence at the end in noise; the cleanup removes CJK spaces, then `drop_trailing_repetition`, then `drop_restarted_tail`. Keep that order (the loop length is counted in characters, not spaces).
- STT evaluation: `eval/corpus/` is gitignored and must be recorded (`eval/record.sh`; set `EVAL_DEVICE`, since a Bluetooth default input records at 16 kHz). With 30 sentences per language one flipped clip moves CER by 3-10%, and the zh model's output flips on tiny audio changes (resampler, padding), so adopt a tuning only if it wins in aggregate on more than one voice or noise type, with `unstable` 0.
- `translator-app`: UI-independent `AppState` (audio, job runner, volume, config saving), the `AppView` view model, mock services and CLI parsing, shared by both frontends. Frontends call `handle_key`, `tick` and `view`; `RecordStyle::Hold` (Slint) vs `Toggle` (TUI). `AppState::headless` runs without audio devices or volume commands, for tests.
- `translator-tui`: ratatui/crossterm terminal frontend (`make run-tui`). Filters auto-repeat on the recording keys (1 s window), ignores Ctrl/Alt combinations, exits cleanly on SIGTERM/SIGINT/SIGHUP, and redirects fd 2 to `~/.local/state/gemma-translator/translator-tui.log` while running; only a main-thread panic restores the terminal.
- `translator-models`: downloads Gemma, the Moonshine STT models, TTS assets and the Silero VAD into `model_dir` (layout in `translator-core/src/models.rs`). `make models` always runs it. Moonshine's files and URLs come from its dependency manifests (`moonshine_get_stt_dependencies` for the arch `LanguageCode::stt_model` pins, `moonshine_get_tts_dependencies`); STT entries carry a size and crc32c, so a file on disk that no longer matches (a new build under the same name) is fetched again, and the speech thread refuses a model file of the wrong size. After a successful run it deletes what no current model lists under `stt/` and leftover `.onnx` files under `tts/` (Moonshine loads `.ort` only; Piper's `.onnx.json` stays), since `package-pi` ships the whole directory.
- TTS voices: `tts_speed` and `[tts_voices]` in `config.toml` become `TtsSettings` (`translator-core/src/models.rs`), the single place that resolves a language's voice for both `translator-models` and the speech thread. `translator-models` reads the config (`--config`, else the default path), rejects ids that `moonshine_get_tts_voices` does not list for the language, and fetches the built-in voices plus the configured ones. The speech thread checks the voice itself (Moonshine would silently pick another one) and falls back, logged, to the built-in voice: a TTS error ends the whole job in `JobRunner`, translation included.
- Native tests are `#[ignore]`d: `make native-libs models test-ffi`.
- `make native-libs` needs `patchelf` (it sets the RUNPATH on the fetched `.so` files). The native libraries print log lines into `make test-ffi` output, so count results from the `test result:` lines only.
- Pi packaging: `deploy/Dockerfile.pi` (Debian bookworm, arm64 dev packages) via `Cross.toml`; `scripts/check-symbol-versions.sh` caps GLIBC_2.36 / GLIBCXX_3.4.30; `deploy/install.sh` checks a fixed runtime package list before changing anything and enables linger so PipeWire runs without a desktop session.
- macOS (Apple Silicon, macOS 15+): the libraries are `.dylib` files that carry an `@loader_path` rpath, so no patchelf; volume goes through `osascript`.
- Agents cannot inject key presses into the Slint window on Wayland; check Slint key handling by hand. The TUI can be smoke-tested in a pty with `--mock`: closing the pty master must exit within ~30 ms with no stray process.
- TUI: don't use `ratatui::init()`/`restore()` (their panic hook fires on job threads; use `terminal::enter`/`leave`), and never `writeln!(...).unwrap()`/`println!` to the tty: a dead tty aborts the process, so use `let _ = writeln!`.

## Git

- Never add a `Co-Authored-By:` trailer (or any other AI attribution line) to commit messages or PR bodies.
