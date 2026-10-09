# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

Offline, on-device voice translator (target: Raspberry Pi 5 8GB with a 480x320 kiosk display). Three processes:

| Process | Port | Role |
| --- | --- | --- |
| `litert-lm serve` (from `venv/`) | 9379 | Runs the `gemma4-e2b` model behind an OpenAI-compatible `/v1` API |
| `backend/server.py` (stdlib `http.server`) | 3000 | Moonshine STT, moonshine-voice TTS, LLM proxy, system volume, and (prod) serves `frontend/dist/` |
| Vite dev server (dev only) | 5173 | React UI; proxies `/api` and `/proxy` to :3000 (`frontend/vite.config.js`) |

## Commands

```bash
./setup.sh              # create venv/ and install backend/requirements.txt
./download_model.sh     # fetch gemma4-e2b and import into LiteRT-LM
./start.sh              # dev: litert-lm + API server + Vite (http://localhost:5173)
./start.sh --prod       # prod: builds frontend/dist if missing, served on :3000
./deploy-pi.sh          # full Raspberry Pi kiosk install (systemd unit from deploy/, Chromium autostart)

cd frontend
npm run dev | npm run build | npm run format   # format uses oxfmt
make test               # Rust workspace tests + backend/test_*.py
make run-mock           # Rust/Slint UI with mock services
make native-libs        # fetch pinned libmoonshine / liblitert-lm into native/<arch>/lib
make models             # download all models into MODEL_DIR (default ./models)
make test-ffi           # #[ignore]d tests that need native libs + models
make run-native         # Rust UI with in-process inference (inference_backend = "native")
cargo test -p translator-core controller   # one crate, tests matching "controller"
```

The Rust workspace (`litert-lm-sys`, `moonshine-sys`, `translator-core`, `translator-io`, `translator-ui`, `translator-models`) is checked with `make test` (all Rust tests plus the Python backend unit tests), `make clippy` and `cargo fmt --all -- --check`; the React frontend has no test suite or linter beyond `oxfmt`. `start.sh` kills anything on ports 9379/3000/5173 when not under systemd (`$INVOCATION_ID` unset). In prod, a stale `frontend/dist/` is NOT rebuilt automatically — run `npm run build` after frontend changes.

## Architecture

Translation flow (orchestrated in `frontend/src/TranslatorApp.jsx`):
1. Hold a key → `hooks/useAudioRecorder.js` captures mic audio → base64 Float32 PCM, 16 kHz mono.
2. `POST /api/stt` → Moonshine transcription in the source language.
3. `translateText` (`utils/api.js`) sends an OpenAI chat-completions request through `/proxy?url=...` to litert-lm. The system prompt demands a strict `{"translation": ...}` JSON reply; `api.js` strips/parses it and falls back to raw text.
4. Result is split into ~180-char chunks and played sequentially via `GET /api/tts?text=...&lang=...`.

Backend details (`backend/server.py`):
- `/proxy` only allows targets on `localhost`/`127.0.0.1` port 9379.
- STT recognizers and TTS engines are lazily loaded per language and cached in an LRU (`MAX_MODELS = 2`) guarded by **RLocks** — handlers hold the lock and re-enter it inside `get_*`, so a plain `Lock` would deadlock.
- Supported languages are defined in two places that must stay in sync: `LANGUAGES` in `TranslatorApp.jsx` and `SUPPORTED_STT_LANGS` / `TTS_LANG_MAP` / `TTS_VOICE_MAP` in `server.py` (currently ar, en, es, ja, zh, ko).
- `/api/volume` shells out to `wpctl` (PipeWire), falling back to `amixer`.

Frontend:
- Input is keyboard-only (no touch controls). Two keyboard modes, `landscape` (default; Space switches active lane, Z records, ←/→ rotate language) and `vertical` (Z/X record per lane, ←/→ and −/+ per lane), persisted in `localStorage` key `keyboardMode`. Shortcuts are ignored while focus is in input/textarea/select, and language rotation is blocked during recording. See README for full tables.
- Settings (endpoint, proxy toggle, API key, model, keyboard mode) live in `components/SettingsOverlay.jsx`.
- Styling is a retro-terminal theme in `frontend/style.css`, sized for 480x320.

## Rust UI (phase A of docs/superpowers/specs/2026-10-09-rust-frontend-design.md)

- `translator-core`: pure logic (languages, key mapping, sentence splitting, config, the `Controller` state machine). It must not depend on cpal, ureq or slint. The controller returns `Effect`s instead of doing I/O; `translator-ui/src/app.rs` executes them.
- `translator-io`: cpal audio, HTTP clients (`/v2/stt`, `/v2/tts` on :3000, streaming chat completions on :9379), volume commands, and `JobRunner`, which pipelines STT → streaming translation → per-sentence TTS on background threads and reports `JobEvent`s.
- `translator-ui`: Slint binary `translator`; `--mock` swaps in fake services. Job events reach the UI thread through an mpsc channel drained by a 30 ms timer.
- Python `/v2` endpoints take and return raw 16-bit PCM (`backend/pcm.py`); the old `/api/*` endpoints still serve the React UI.
- `moonshine-sys` / `litert-lm-sys`: run-time (`libloading`) bindings to the vendor libraries from the pinned wheels; signatures are copied from the wheels' ctypes code, so re-check them on any version bump. Handles are `!Send` and live on one worker thread each (`translator-io/src/native/`).
- `translator-models`: downloads Gemma, the Moonshine STT models and TTS assets into `model_dir` (layout in `translator-core/src/models.rs`).
- `inference_backend = "native"` in the config switches the UI from the HTTP services to the native ones. Native tests are `#[ignore]`d: `make native-libs models test-ffi`.
