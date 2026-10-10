<img width="960" height="540" src="https://storage.googleapis.com/experiments-uploads/gemma-translator/gemma-translator.gif" />

# Gemma Translator

This repo was built with the assistance of [Google Antigravity](https://antigravity.google/) and includes code to run an on-device, fully offline voice translator powered by [Gemma 4](https://ai.google.dev/gemma/docs/core) and [LiteRT-LM](https://github.com/google-ai-edge/LiteRT-lm). It is a single Rust program with a [Slint](https://slint.dev) UI optimized for small handheld displays (e.g., 480x320), plus a terminal UI. Speech recognition and text-to-speech are powered by [Moonshine](https://github.com/moonshine-ai/moonshine). All inference runs inside the program; there is no server process.

## Features

- **On-Device Inference**: Runs the `gemma4-e2b` model through LiteRT-LM, and Moonshine for speech, entirely locally. No internet required after setup.
- **Voice Interface**: Captures microphone audio, transcribes it, translates it and speaks the result.
- **Optimized UI**: Retro-terminal styling custom-built for small hardware screens (like Raspberry Pi displays), with a terminal UI for desktops and SSH sessions.
- **One Program**: One binary, `translator` (or `translator-tui`), with the vendor libraries in `lib/` next to it and the models in one folder (`./models` on a desktop, `/var/lib/gemma-translator/models` on the Pi). No server process.

## Required Hardware

- **Compute**: Raspberry Pi 5 with 8GB RAM
- **Audio Input**: Microphone or USB audio capture interface
- **Audio Output**: Speaker or headphone output device
- **Display**: Display monitor or touchscreen (e.g., 480x320 kiosk display)

<img width="3024" height="1672" src="https://storage.googleapis.com/experiments-uploads/gemma-translator/gemma-translator-cad.gif" />

## Prerequisites

- Rust, installed with [rustup](https://rustup.rs)
- Linux: `make`, `patchelf`, `pkg-config`, `libasound2-dev`, `libfontconfig1-dev`, `libvulkan1`, `unzip`, `curl`
- macOS 15+ on Apple Silicon: the Xcode Command Line Tools (`xcode-select --install`)

`make setup` checks these tools and prints the install command for anything missing.

## Desktop (Linux or macOS)

```bash
make setup           # checks the tools, fetches the native libraries and the ~3.8 GB of models
make run             # Slint window, in-process inference
make run-tui         # terminal UI, in-process inference
make run-mock        # Slint window with mock speech and translation (no models needed)
make run-tui ARGS=--mock   # terminal UI with mock services (still fetches the native libraries first, unlike make run-mock)
```

`make setup SETUP_ARGS=--yes` downloads the models without asking. The models live in `./models`; `make run` and `make run-tui` pass `--model-dir <repo>/models` (an absolute path) to the program. Set `MODEL_DIR=/path` to use another folder, or run the binary yourself with `--model-dir <path>` (a run-time override that is never written to the config file; ignored with `--mock`).

### Terminal UI

`translator-tui` is the same translator in a terminal, for a desktop terminal
or an SSH session (audio uses the machine it runs on). Terminals do not report
key releases, so recording toggles: tap `Z` (or `Z`/`X` in vertical mode) to
start and tap again to stop. Holding the key, or tapping it again within 1 s,
counts as one tap. `Esc` opens settings and `Ctrl-C` quits; Ctrl-Z does not
suspend. A missing microphone or speaker is shown on the status line.
Native-library output goes to `~/.local/state/gemma-translator/translator-tui.log`.
Arabic is shown in logical order; right-to-left display depends on the terminal
(Konsole and mlterm handle it). Without `COLORTERM=truecolor` the theme colour
is approximated.

### Log output

LiteRT-LM and Moonshine print their own log lines to stderr. Lines such as
`Creating Gemma4DataProcessor` (one per translation, because every translation
starts a fresh conversation) and `mel_filterbank.cc ... Missing 10 bands`
(from Gemma's audio front end, which the translator does not use) are normal
and do not mean anything went wrong.

### macOS notes

Needs macOS 15 or later on an M-series Mac (moonshine-voice publishes no
Intel macOS build). The native libraries are the `.dylib` files from the
pinned macOS wheels in `native/arm64/lib`. Volume keys change the system
output volume through `osascript`. The first recording asks whether your
terminal app may use the microphone; if it was denied, enable it under System
Settings → Privacy & Security → Microphone and restart the terminal.

## Raspberry Pi 5

Supported system: Raspberry Pi OS Bookworm, 64-bit, desktop. The Pi gets a
self-contained package that you build on an x86_64 Linux machine.

On the build machine you need Docker, then:

```bash
cargo install cross --version 0.2.5 --locked
rustup target add aarch64-unknown-linux-gnu
make models          # skip when ./models is already complete
make package-pi      # writes dist/gemma-translator-pi.tar
```

The tar is about 4.0 GB, which is more than a FAT32 drive can hold in one
file, so copy it with `scp` or on an exFAT or ext4 drive. The install needs
about 12 GB free on the Pi (the tar, the unpacked package and the installed
models). Unpack it in your home directory, not in `/opt/gemma-translator` or
`/var/lib/gemma-translator`, then:

```bash
tar -xf gemma-translator-pi.tar && cd gemma-translator && sudo ./install.sh
```

No network is needed on the Pi. `install.sh` checks the system and the runtime
packages before it changes anything; if it lists missing packages, install
them with the printed apt command and run it again. It installs the program
into `/opt/gemma-translator`, the models into `/var/lib/gemma-translator/models`
and a systemd service. After a reboot the Pi starts straight into the
translator instead of the desktop; the first start is slower. Running
`install.sh` again from a newer package updates the program and skips models
that did not change. Delete the tar once it is unpacked. The service always
uses the installed models, whatever `model_dir` says in `config.toml`.

To see the translator's output, or to restart it after changing something:

```bash
journalctl -u gemma-translator -f
sudo systemctl restart gemma-translator
```

To go back to the desktop:

```bash
sudo systemctl disable gemma-translator && sudo systemctl set-default graphical.target
```

## Settings

Settings are stored in `~/.config/gemma-translator/config.toml` (or under `$XDG_CONFIG_HOME`; on the Pi, in the home directory of the user that ran `sudo ./install.sh`; `--config <path>` uses another file). Press `Esc` to open the settings panel (↑/↓ select, ←/→ change). Recordings shorter than 300 ms are ignored.

## Development

```bash
make             # list every target
make native-libs # fetch the pinned libmoonshine / liblitert-lm into native/<arch>/lib
make models      # download the models into MODEL_DIR (default ./models)
make test        # all Rust tests
make clippy      # lint, warnings are errors
make test-ffi    # tests that need the native libraries and models (after make models)
make test-soak   # long-running native memory test
make fmt         # format the Rust code
make clean       # remove target/ and dist/ (keeps native/ and models/)
```

The native libraries are loaded at run time from the pinned vendor wheels;
their function signatures are copied from the wheels' ctypes code, so re-check
`moonshine-sys` and `litert-lm-sys` whenever `scripts/fetch-native-libs.sh`
moves to a new version. `make test-ffi` output contains the libraries' log
lines; count results from the `test result:` lines only.

## Project Structure

- `translator-core/` - pure logic: languages, key mapping, sentence splitting, config, the controller state machine.
- `translator-io/` - audio, volume and the in-process inference jobs.
- `translator-app/` - UI-independent application state shared by both frontends.
- `translator-ui/` - the Slint program `translator`.
- `translator-tui/` - the terminal program `translator-tui`.
- `translator-models/` - downloads the models.
- `moonshine-sys/`, `litert-lm-sys/` - run-time bindings to the vendor libraries.
- `deploy/` - Pi cross-build image, `install.sh` and the systemd unit template.
- `scripts/` - setup, native-library fetch and Pi packaging scripts.
- `docs/` - design specs and implementation plans.
- `stl/` - STL files for 3D printing the hardware case.

## Keyboard Shortcuts

The Gemma Translator supports **two keyboard modes**. Switch between them anytime from the settings panel (Esc) or by editing `config.toml`; the choice is remembered across restarts.

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
- **Recording lock:** language rotation is blocked while a recording is in progress.
- **Keyboard-driven:** recording and language rotation are keyboard-only; there are no on-screen touch controls.

### Switching modes
Open the settings panel (Esc) and choose **Landscape** or **Vertical**, or edit `config.toml`. The change takes effect immediately and persists on the device.

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
