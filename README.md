<img width="960" height="540" src="https://storage.googleapis.com/experiments-uploads/gemma-translator/gemma-translator.gif" />

# Gemma Translator

This repo was built with the assistance of [Google Antigravity](https://antigravity.google/) and includes code to run an on-device, fully offline voice translator powered by [Gemma 4](https://ai.google.dev/gemma/docs/core) and [LiteRT-LM](https://github.com/google-ai-edge/LiteRT-lm). This project features a web frontend optimized for small handheld displays (e.g., 480x320) and a Python API server (`http.server`) that communicates with Gemma. Text-to-speech is powered by [Moonshine](https://github.com/moonshine-ai/moonshine).

https://github.com/user-attachments/assets/343072ce-dc78-44a7-a783-99312845cabe

## Features

- **On-Device Inference**: Uses LiteRT-LM to run the `gemma4-e2b` model entirely locally. No internet required after setup.
- **Voice Interface**: Captures microphone audio, processes it, and sends it to the local model.
- **Optimized UI**: Retro-terminal styling custom-built for small hardware screens (like Raspberry Pi displays).
- **Unified Startup**: One script to launch the LLM server, the Python API, and the React frontend.

## Prerequisites

- Python 3.10+
- Node.js 18+ (20 LTS recommended) & npm — installed automatically by `deploy-pi.sh` on Raspberry Pi OS / Debian
- Linux, or macOS 15+ on Apple Silicon (see [macOS](#macos-apple-silicon))

## Required Hardware

- **Compute**: Raspberry Pi 5 with 8GB RAM
- **Audio Input**: Microphone or USB audio capture interface
- **Audio Output**: Speaker or headphone output device
- **Display**: Display monitor or touchscreen (e.g., 480x320 kiosk display)

<img width="3024" height="1672" src="https://storage.googleapis.com/experiments-uploads/gemma-translator/gemma-translator-cad.gif" />

## Setup Instructions

`make setup` checks the system tools, then creates `venv/`, fetches the native
libraries and models, imports Gemma for `litert-lm serve` and installs the
frontend packages, skipping anything already there. It prints the install
command for any missing tool instead of installing it; `make setup
SETUP_ARGS=--yes` downloads the ~3.8 GB of models without asking. Models
downloaded before `make setup` existed have no completion stamp, so the
first run asks once; answering yes only checks them and fetches nothing.
The steps below do the same by hand.

1. **Make Scripts Executable**
   Ensure the setup, download, start, and deployment scripts have execute permissions:
   ```bash
   chmod +x setup.sh download_model.sh start.sh deploy-pi.sh
   ```

2. **Install Dependencies**
   Run the setup script to create a Python virtual environment (`venv`) and install all required packages:
   ```bash
   ./setup.sh
   ```

3. **Download the Model**
   Run the model downloader script to fetch the `gemma4-e2b` model from Hugging Face and import it into LiteRT-LM:
   ```bash
   ./download_model.sh
   ```

## Running the Application

Start all services (LiteRT-LM, the Python API server, and the Vite Web UI) in development mode:
```bash
./start.sh
```

To run in production mode (skipping Vite dev server and serving compiled UI assets from `frontend/dist/` via `backend/server.py` on port 3000):
```bash
./start.sh --prod
```

The application will be accessible at:
- **Web UI (Dev)**: `http://localhost:5173`
- **Web UI (Prod) / API server**: `http://localhost:3000`
- **LiteRT-LM**: `http://localhost:9379`

## Raspberry Pi Appliance Deployment

To deploy as a permanent systemd kiosk service on a Raspberry Pi 5 (8GB):
```bash
./deploy-pi.sh
```
This automated script installs Debian audio/venv packages, sets up the Python environment, builds production UI assets, downloads the LiteRT model, registers the systemd unit from `deploy/gemma-translator.service`, and configures LXDE GUI autostart (`~/.config/lxsession/rpd-x/autostart`) to launch Chromium in kiosk mode pointing to `http://localhost:3000`.

## Rust UI (experimental)

A native Rust + [Slint](https://slint.dev) replacement for the Chromium kiosk UI lives in the Cargo workspace at the repo root (`translator-core`, `translator-io`, `translator-ui`). It records and plays audio itself and calls the Python service's `/v2/stt` and `/v2/tts` endpoints and `litert-lm serve` directly. See `docs/superpowers/specs/2026-10-09-rust-frontend-design.md`.

Building needs Rust (stable) and the ALSA and fontconfig development headers (`sudo apt install libasound2-dev libfontconfig1-dev` on Debian/Raspberry Pi OS).

```bash
make test       # Rust + Python unit tests
make run-mock   # desktop UI with mock speech/translation (no models needed)
make run        # desktop UI against backend/server.py and litert-lm
make build-pi   # release build for the Pi (KMS backend, aarch64)
```

Settings are stored in `~/.config/gemma-translator/config.toml`. In the Rust UI, `Esc` opens the settings page (↑/↓ select, ←/→ change); all other keys match the tables above. Recordings shorter than 300 ms are ignored.

### Terminal UI

`translator-tui` is the same translator in a terminal, for a desktop terminal
or an SSH session (audio uses the machine it runs on):

```bash
make run-tui ARGS=--mock   # mock services
make run-tui               # uses inference_backend from the config
```

Terminals do not report key releases, so recording toggles: tap `Z` (or `Z`/`X`
in vertical mode) to start and tap again to stop. Holding the key, or tapping
it again within 1 s, counts as one tap. `Esc` opens settings and `Ctrl-C`
quits; Ctrl-Z does not suspend. A missing microphone or speaker is shown on
the status line. Native-library output goes to
`~/.local/state/gemma-translator/translator-tui.log`. Arabic is shown in
logical order; right-to-left display depends on the terminal (Konsole and
mlterm handle it). Without `COLORTERM=truecolor` the theme colour is
approximated.

### In-process inference (phase B, experimental)

The `translator` binary can run speech recognition, translation and speech
synthesis itself, without `server.py` or `litert-lm serve`:

```bash
sudo apt-get install -y patchelf unzip libvulkan1   # Linux; macOS needs none of these
make native-libs     # pinned libmoonshine / liblitert-lm into native/<arch>/lib
make models          # ~3.8 GB of models into ./models (MODEL_DIR=... to change)
make test-ffi        # FFI tests against those libraries and models
```

Then set in `~/.config/gemma-translator/config.toml`:

```toml
inference_backend = "native"
model_dir = "/path/to/models"
```

and start it with `make run-native`. The libraries are loaded from `lib/`
next to the executable. `make test-soak` runs the long memory test.

## macOS (Apple Silicon)

Needs macOS 15 or later on an M-series Mac (moonshine-voice publishes no
Intel macOS build) and the Xcode Command Line Tools
(`xcode-select --install`); Homebrew is the easiest way to get Python 3.12
and Node.js.

```bash
make setup           # lists anything missing, then fetches the rest
make run-native      # Slint window, in-process inference
make run-tui         # terminal UI (set inference_backend in the config)
./start.sh           # Python backend + React UI on http://localhost:5173
```

The native libraries are the `.dylib` files from the pinned macOS wheels in
`native/arm64/lib`. Volume keys change the system output volume through
`osascript`. The first recording asks whether your terminal app may use the
microphone; if it was denied, enable it under System Settings → Privacy &
Security → Microphone and restart the terminal.

## Project Structure

- `frontend/` - React (Vite) web frontend (`index.html`, `src/`, styles, and Vite configuration).
- `backend/` - Python API server (`server.py` and `requirements.txt`) for Moonshine STT, moonshine-voice TTS, and model proxying.
- `deploy/` - Parameterizable systemd service unit template (`gemma-translator.service`).
- `stl/` - STL files for 3D printing the hardware case.
- `setup.sh` - Automates Python virtual environment creation and dependency installation.
- `download_model.sh` - Fetches the required LiteRT model.
- `start.sh` - Multi-process launcher supporting `--prod` and development modes.
- `deploy-pi.sh` - One-command Raspberry Pi automated deployment script.

## Keyboard Shortcuts

The Gemma Translator supports **two keyboard modes**. Switch between them anytime from the **Settings panel → "Keyboard Mode"** dropdown. The choice is remembered across restarts (stored in the browser's `localStorage` under the key `keyboardMode`).

The app has two lanes (two people facing each other on the kiosk):
- **Lane 1 / Person 1** — the left/top lane.
- **Lane 2 / Person 2** — the right/bottom lane.

Each lane has a rotating language "revolver" and records speech, which is transcribed (Moonshine STT), translated (Gemma), and spoken back in the other lane's language (moonshine-voice TTS).

### Landscape Mode (default) — "active person"
One lane is the **active person** at a time. The active lane is framed with **corner brackets on all four corners**. You drive everything from a single set of keys and switch focus with Space.

| Key | Action | Description |
| :--- | :--- | :--- |
| **Spacebar** | Switch active person | Toggles the active lane (Person 1 ⇄ Person 2). Disabled while recording. |
| **Z** | Record (push-to-talk) | Hold to record the **active** person; release to transcribe & translate. |
| **← Left Arrow** | Previous language | Rotates the **active** person's language backward. |
| **→ Right Arrow** | Next language | Rotates the **active** person's language forward. |

Notes:
- The active lane shows four-corner brackets; while it is recording, the brackets invert to black along with the lane's color reversal.
- Best for one-handed / single-operator use.

### Vertical Mode — "two-hand" (original mapping)
Each lane has its **own dedicated keys** — there is no active-person concept and **no bracket highlight**. Both people can be controlled independently.

| Key | Action | Description |
| :--- | :--- | :--- |
| **Z** | Record — Person 1 (push-to-talk) | Hold to record Lane 1; release to transcribe & translate. |
| **X** | Record — Person 2 (push-to-talk) | Hold to record Lane 2; release to transcribe & translate. |
| **← Left Arrow** | Previous language — Person 1 | Rotates Lane 1's language backward. |
| **→ Right Arrow** | Next language — Person 1 | Rotates Lane 1's language forward. |
| **− Minus** (`_`) | Previous language — Person 2 | Rotates Lane 2's language backward. |
| **+ Plus** (`=`) | Next language — Person 2 | Rotates Lane 2's language forward. |

Notes:
- No corner-bracket selection highlight in this mode.
- Best for two operators, each handling their own side.

### Common behavior (both modes)
- **Input focus guard:** all shortcuts are ignored while focus is on a configuration field (`<input>`, `<textarea>`, or `<select>`) — e.g. when editing the API endpoint or settings.
- **Recording lock:** language rotation is blocked while a recording is in progress.
- **Keyboard-driven:** recording and language rotation are keyboard-only in the current build; on-screen touch controls are not enabled.

### Switching modes
Open **Settings (⚙)** → **Keyboard Mode** → choose **Landscape** or **Vertical**. The change takes effect immediately and persists on the device.

| Setting value | Mode |
| :--- | :--- |
| `landscape` | Active-person scheme (Space / Z / ← →) — default |
| `vertical` | Two-hand scheme (Z / X / ← → / − +) |

## Credits
Made by a small team at [Google Creative Lab](https://github.com/googlecreativelab):
- [Alan Yam](https://github.com/alanvww)
- [Shashwath Santosh](https://x.com/shashwth)
- [Dan Motzenbecker](https://github.com/dmotz)

## Disclaimer

This is not an officially supported Google product. This project is not
eligible for the [Google Open Source Software Vulnerability Rewards
Program](https://bughunters.google.com/open-source-security).
