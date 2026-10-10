# macOS Support and Setup Script Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run the whole project (Rust Slint/TUI frontends with `http` and `native` backends, plus the Python backend) on Apple Silicon macOS 15+, and add `make setup`, which checks system tools and fetches everything the project downloads on Linux and macOS.

**Architecture:** Platform differences are selected at compile time in Rust (`cfg(target_os = "macos")`), by `sys.platform` in Python and by `uname -s` in shell. macOS wheels already ship `.dylib` files with an `@loader_path` rpath, so they only need unpacking. Volume control gains an `osascript` path in both Rust and Python. A new `scripts/setup-env.sh` checks tools, then fetches or skips each project item and prints a status table.

**Tech Stack:** Rust (libloading, thiserror), Python 3 stdlib (`subprocess`, `unittest`), bash 3.2-compatible shell, GNU make.

**Spec:** `docs/superpowers/specs/2026-10-10-macos-support-design.md`

## Global Constraints

- Supported macOS: Apple Silicon (`uname -m` = `arm64`), macOS 15 or later. Intel Macs and older macOS versions get a clear error and a non-zero exit.
- macOS wheels (exact URLs and sha256):
  - `https://files.pythonhosted.org/packages/7f/49/93c28833572682c434c60cea8bcf338b74b8f6b2dbe816186c6bb5242c16/moonshine_voice-0.0.65-py3-none-macosx_15_0_arm64.whl`, sha256 `3e92fa8497bf4fc40b6241e4dc29fd4ac77832f7f3d8addc4fd467a6bf1cf605`
  - `https://files.pythonhosted.org/packages/58/ae/5d1e2862d14ad1b08bdbc3392574dde37b98a06f4efb5181d946766ec81c/litert_lm_api-0.13.1-py3-none-macosx_12_0_arm64.whl`, sha256 `4a1bb68dbf5a60af9ac5fe03331968a7b4e91779e721cb531e6b41e71f2a2d77`
- macOS library files: `moonshine_voice/libmoonshine.dylib`, `moonshine_voice/libonnxruntime.1.23.2.dylib`, `litert_lm/liblitert-lm.dylib`. No `install_name_tool` or `patchelf` on macOS.
- Linux behaviour must not change: the same commands in the same order, the same wheels and the same files in `native/<arch>/lib`. The one accepted difference: `start.sh` notices an exited child up to ~1 s later, because it polls instead of using `wait -n`.
- Shell scripts must run under macOS's `/bin/bash` 3.2: no `wait -n`, no associative arrays, no `mapfile`, and no `set -u` combined with possibly empty arrays.
- `make setup` never installs system packages. It only lists the install commands.
- Volume steps are 5 % and are clamped to 0–100.
- Project rules: booleans are prefixed `is`/`has`/`should`/`can`; commit messages use the imperative mood with no `Co-Authored-By` trailer; no TODOs; no unused helpers.
- Checks before every commit that touches Rust: `cargo test --workspace`, `make clippy` and `cargo fmt --all -- --check`. Python changes run `python3 -m unittest discover -s backend -p 'test_*.py'`.

## Review Focus

1. A Mac with no output device makes `osascript` print `missing value`. The volume reads as unknown and nothing is set (Task 2 test `macos_unknown_volume_sets_nothing`; Task 3 test `test_macos_unknown_volume_sets_nothing`).
2. Pressing volume-up at 98 % ends at 100 %, not 103 %, on both Rust and Python (Task 2 `macos_adjust_up_clamps_at_one_hundred`; Task 3 `test_step_clamps`).
3. One `start.sh` child dies, for example litert-lm crashing. The script must notice within about 2 s under bash 3.2 and shut the rest down (Task 5 Step 2 timing check).
4. `make setup` runs non-interactively (stdin is not a TTY) without `--yes`. It must not start the 3.8 GB model download, and it must say it skipped it (Task 7 Step 3).
5. A model download is interrupted. The next `make setup` must not report models as `ok`; readiness is a stamp written only after `translator-models` succeeds (Task 7 Steps 1 and 4).

## Working in a worktree

A fresh worktree has none of the git-ignored downloads (`venv/`, `native/`, `models/`, `frontend/node_modules`). Before Task 4, link the main checkout's copies so the checks do not download gigabytes:

```bash
MAIN="$(git worktree list --porcelain | awk 'NR==1 { print $2 }')"
for item in venv native models frontend/node_modules; do
    [ -e "$item" ] || ln -s "$MAIN/$item" "$item"
done
```

Commands that rewrite these items, such as `make -B native-libs` or `make models`, then write into the main checkout's copies. That is expected and leaves the same files there. `fetch-native-libs.sh` deletes the target directory before unpacking. If it fails after that point, the main checkout's `native/<arch>/lib` is left without `.complete`; run `make native-libs` again before continuing.

## Refinements to the spec

- Spec §4.6 judges models ready by non-empty `stt/` and `tts/` folders. This plan uses a `$(MODEL_DIR)/.complete` stamp written by `make models` instead, so an interrupted download is never reported as `ok` (Review Focus 5).
- Spec §4.6 runs `./download_model.sh` without asking. When no local Gemma file exists, that would download about 2.6 GB, so this plan asks first, the same way as for the models.
- Linux tool checks add `libvulkan.so.1`, which the README already lists as a requirement for native mode.

---

## File Structure

| File | Change |
|---|---|
| `moonshine-sys/src/library.rs`, `moonshine-sys/src/lib.rs` | Per-OS `LIBRARY_FILE_NAME`; error text and docs use it |
| `litert-lm-sys/src/library.rs`, `litert-lm-sys/src/lib.rs`, `litert-lm-sys/tests/engine.rs` | Per-OS `LITERT_LM_LIBRARY_FILE_NAME`; same |
| `translator-io/src/native/speech.rs`, `translator-io/src/native/translator.rs` | Tests and doc comments use the constants |
| `translator-io/src/volume.rs` | `VolumePlatform`, macOS `osascript` branch, Linux-only `XDG_RUNTIME_DIR` |
| `backend/volume.py` (new), `backend/test_volume.py` (new), `backend/server.py` | Volume logic moved out of the handler, with a macOS branch |
| `scripts/fetch-native-libs.sh` | Darwin arm64 wheels; `shasum` fallback; `patchelf` on Linux only |
| `start.sh` | Per-OS volume and `XDG_RUNTIME_DIR`; bash 3.2 child-exit loop |
| `download_model.sh` | Imports `models/gemma/gemma-4-E2B-it.litertlm` when present |
| `scripts/setup-env.sh` (new), `Makefile` | `make setup`; `make models` writes `$(MODEL_DIR)/.complete` |
| `README.md`, `CLAUDE.md` | macOS and `make setup` docs |

---

### Task 1: Per-platform native library file names

**Files:**
- Modify: `moonshine-sys/src/library.rs:10-17`, `moonshine-sys/src/lib.rs:1`
- Modify: `moonshine-sys/tests/library.rs` (test `missing_library_names_the_path`, line 17)
- Modify: `litert-lm-sys/src/library.rs:7-14`, `litert-lm-sys/src/lib.rs:1`, `litert-lm-sys/tests/engine.rs:31-35`
- Modify: `translator-io/src/native/speech.rs` (doc comment near line 39; test near lines 250-262), `translator-io/src/native/translator.rs` (doc comment near line 42; test near lines 280-287)

**Interfaces:**
- Produces: `moonshine_sys::LIBRARY_FILE_NAME`, which is `"libmoonshine.dylib"` on macOS and `"libmoonshine.so"` elsewhere. `litert_lm_sys::LITERT_LM_LIBRARY_FILE_NAME` is `"liblitert-lm.dylib"` on macOS and `"liblitert-lm.so"` elsewhere. Both names and their exports stay the same.

- [ ] **Step 1: Write the failing tests**

Append to the existing `#[cfg(test)] mod tests` in `moonshine-sys/src/library.rs`. If the file has no test module, add this one at the end of the file:

```rust
#[cfg(test)]
mod file_name_tests {
    use super::*;

    #[test]
    fn library_file_name_matches_the_platform() {
        let expected = if cfg!(target_os = "macos") {
            "libmoonshine.dylib"
        } else {
            "libmoonshine.so"
        };
        assert_eq!(LIBRARY_FILE_NAME, expected);
    }

    #[test]
    fn symbol_error_names_the_platform_library() {
        let error = MoonshineError::Symbol {
            name: "moonshine_get_version".into(),
            message: "not found".into(),
        };
        assert_eq!(
            error.to_string(),
            format!("moonshine_get_version missing from {LIBRARY_FILE_NAME}: not found")
        );
    }
}
```

Add the matching module at the end of `litert-lm-sys/src/library.rs`:

```rust
#[cfg(test)]
mod file_name_tests {
    use super::*;

    #[test]
    fn library_file_name_matches_the_platform() {
        let expected = if cfg!(target_os = "macos") {
            "liblitert-lm.dylib"
        } else {
            "liblitert-lm.so"
        };
        assert_eq!(LITERT_LM_LIBRARY_FILE_NAME, expected);
    }

    #[test]
    fn symbol_error_names_the_platform_library() {
        let error = LiteRtLmError::Symbol {
            name: "litert_lm_engine_create".into(),
            message: "not found".into(),
        };
        assert_eq!(
            error.to_string(),
            format!("litert_lm_engine_create missing from {LITERT_LM_LIBRARY_FILE_NAME}: not found")
        );
    }
}
```

- [ ] **Step 2: Run the tests to check they compile and pass on Linux**

Run: `cargo test -p moonshine-sys -p litert-lm-sys file_name_tests`
Expected: 4 passed. On Linux they already pass; they exist to fail on macOS until Step 3, and to pin the error text.

- [ ] **Step 3: Make the names per-platform**

In `moonshine-sys/src/library.rs` replace `pub const LIBRARY_FILE_NAME: &str = "libmoonshine.so";` with:

```rust
/// The library's file name inside the native library directory.
#[cfg(target_os = "macos")]
pub const LIBRARY_FILE_NAME: &str = "libmoonshine.dylib";
/// The library's file name inside the native library directory.
#[cfg(not(target_os = "macos"))]
pub const LIBRARY_FILE_NAME: &str = "libmoonshine.so";
```

and the `Symbol` variant's attribute with:

```rust
    #[error("{name} missing from {library}: {message}", library = LIBRARY_FILE_NAME)]
```

In `moonshine-sys/src/lib.rs`, change line 1 to:

```rust
//! Run-time bindings to `libmoonshine` (`.so` on Linux, `.dylib` on macOS) from the moonshine-voice 0.0.65 wheel.
```

In `litert-lm-sys/src/library.rs`, replace the constant with:

```rust
/// The library's file name inside the native library directory.
#[cfg(target_os = "macos")]
pub const LITERT_LM_LIBRARY_FILE_NAME: &str = "liblitert-lm.dylib";
/// The library's file name inside the native library directory.
#[cfg(not(target_os = "macos"))]
pub const LITERT_LM_LIBRARY_FILE_NAME: &str = "liblitert-lm.so";
```

and its `Symbol` attribute with:

```rust
    #[error("{name} missing from {library}: {message}", library = LITERT_LM_LIBRARY_FILE_NAME)]
```

In `litert-lm-sys/src/lib.rs`, change line 1 to:

```rust
//! Run-time bindings to `liblitert-lm` (`.so` on Linux, `.dylib` on macOS) from the litert-lm-api 0.13.1 wheel.
```

In `litert-lm-sys/tests/engine.rs`, test `missing_library_names_the_path`, replace the assertion with:

```rust
    assert!(
        error.to_string().contains(litert_lm_sys::LITERT_LM_LIBRARY_FILE_NAME),
        "{error}"
    );
```

In `moonshine-sys/tests/library.rs`, test `missing_library_names_the_path`, replace `contains("libmoonshine.so")` with `contains(moonshine_sys::LIBRARY_FILE_NAME)`.

In `translator-io/src/native/speech.rs`, replace both `contains("libmoonshine.so")` assertions with `contains(moonshine_sys::LIBRARY_FILE_NAME)`. Change the doc comment `/// Starts the speech thread. It loads \`libmoonshine.so\` from \`lib_dir\`,` to say `` `libmoonshine` `` instead of `` `libmoonshine.so` ``.

In `translator-io/src/native/translator.rs`, replace `contains("liblitert-lm.so")` with `contains(litert_lm_sys::LITERT_LM_LIBRARY_FILE_NAME)`. Change the doc comment's `` `liblitert-lm.so` `` to `` `liblitert-lm` ``.

Then search for any other hard-coded name:

Run: `grep -rn 'libmoonshine\.so\|liblitert-lm\.so' --include=*.rs moonshine-sys litert-lm-sys translator-*`
Expected: only the two `#[cfg(not(target_os = "macos"))]` constants. The five test and doc sites above now use the constants.

- [ ] **Step 4: Run the full checks**

Run: `cargo test --workspace && make clippy && cargo fmt --all -- --check`
Expected: all pass, clippy shows no warnings, and fmt prints nothing.

- [ ] **Step 5: Commit**

```bash
git add moonshine-sys litert-lm-sys translator-io/src/native
git commit -m "Name the native libraries per platform (.dylib on macOS)"
```

---

### Task 2: macOS volume control in translator-io

**Files:**
- Modify: `translator-io/src/volume.rs` (whole file; tests live in its `mod tests`)

**Interfaces:**
- Consumes: the existing `CommandRunner` trait, `RotateDirection` from `translator_core::language`.
- Produces: no new public API. `VolumeControl::new(runner)` and `VolumeWorker::spawn(runner)` keep their signatures. A private `enum VolumePlatform { Linux, MacOs }` is added, plus a private field `platform` on `VolumeControl` and a private `VolumeWorker::spawn_control(control)`.

- [ ] **Step 1: Write the failing tests**

In `translator-io/src/volume.rs` `mod tests`, add these helpers after `impl CommandRunner for FakeRunner`:

```rust
    fn linux_control<R: CommandRunner>(runner: R) -> VolumeControl<R> {
        VolumeControl {
            runner,
            platform: VolumePlatform::Linux,
        }
    }

    fn macos_control<R: CommandRunner>(runner: R) -> VolumeControl<R> {
        VolumeControl {
            runner,
            platform: VolumePlatform::MacOs,
        }
    }

    const OSASCRIPT_GET: &str = "osascript -e output volume of (get volume settings)";
```

Then add these tests:

```rust
    #[test]
    fn parses_osascript_volume() {
        assert_eq!(parse_osascript_volume("75\n"), Some(75));
        assert_eq!(parse_osascript_volume("0"), Some(0));
        assert_eq!(parse_osascript_volume("100\n"), Some(100));
        assert_eq!(parse_osascript_volume("missing value\n"), None);
        assert_eq!(parse_osascript_volume(""), None);
        assert_eq!(parse_osascript_volume("101"), None);
    }

    #[test]
    fn macos_reads_osascript() {
        let control = macos_control(FakeRunner::with(&[(OSASCRIPT_GET, "75\n")]));
        assert_eq!(control.current_percent(), Some(75));
    }

    #[test]
    fn macos_adjust_up_clamps_at_one_hundred() {
        let control = macos_control(FakeRunner::with(&[
            (OSASCRIPT_GET, "98\n"),
            ("osascript -e set volume output volume 100", ""),
        ]));
        // The fake keeps answering 98; the set command proves the clamp.
        assert_eq!(control.adjust(RotateDirection::Next), Some(98));
        let calls = control.runner.calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            [
                OSASCRIPT_GET,
                "osascript -e set volume output volume 100",
                OSASCRIPT_GET
            ]
        );
    }

    #[test]
    fn macos_adjust_down_clamps_at_zero() {
        let control = macos_control(FakeRunner::with(&[
            (OSASCRIPT_GET, "3\n"),
            ("osascript -e set volume output volume 0", ""),
        ]));
        control.adjust(RotateDirection::Previous);
        let calls = control.runner.calls.lock().unwrap().clone();
        assert!(calls.contains(&"osascript -e set volume output volume 0".to_string()));
    }

    #[test]
    fn macos_unknown_volume_sets_nothing() {
        let control = macos_control(FakeRunner::with(&[(OSASCRIPT_GET, "missing value\n")]));
        assert_eq!(control.adjust(RotateDirection::Next), None);
        let calls = control.runner.calls.lock().unwrap().clone();
        assert_eq!(calls, [OSASCRIPT_GET]);
    }

    #[test]
    fn macos_failed_set_gives_none() {
        let control = macos_control(FakeRunner::with(&[(OSASCRIPT_GET, "50\n")]));
        assert_eq!(control.adjust(RotateDirection::Next), None);
    }

    #[test]
    fn new_picks_the_build_platform() {
        let control = VolumeControl::new(FakeRunner::default());
        let expected = if cfg!(target_os = "macos") {
            VolumePlatform::MacOs
        } else {
            VolumePlatform::Linux
        };
        assert_eq!(control.platform, expected);
    }
```

The Linux tests must keep passing when `cargo test` runs on a Mac. In the existing tests, replace every `VolumeControl::new(` with `linux_control(`. These are `reads_wpctl_first`, `falls_back_to_pactl_then_amixer`, `pactl_is_pulled_back_to_one_hundred_percent`, `missing_tools_give_none_without_panicking`, `adjust_uses_five_percent_steps_and_rereads` and `adjust_up_falls_back_to_pactl`. Also replace every `VolumeWorker::spawn(` in `worker_reads_at_start_and_after_each_adjustment` and `worker_requests_do_not_wait_for_the_commands` with `VolumeWorker::spawn_control(linux_control(`, and close the extra parenthesis. For example:

```rust
        let worker = VolumeWorker::spawn_control(linux_control(FakeRunner::with(&[
            ("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.75"),
            ("wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 5%+", ""),
        ])));
```

and

```rust
        let worker = VolumeWorker::spawn_control(linux_control(GatedRunner {
            inner: FakeRunner::with(&[
                ("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.40"),
                ("wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 5%-", ""),
            ]),
            release: Mutex::new(release_receiver),
        }));
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p translator-io volume`
Expected: compile errors, because `VolumePlatform`, `platform`, `parse_osascript_volume` and `spawn_control` are not defined.

- [ ] **Step 3: Implement**

In `SystemCommandRunner::run`, replace the `XDG_RUNTIME_DIR` block with:

```rust
        // PipeWire's wpctl finds its socket through XDG_RUNTIME_DIR, which systemd
        // services may lack.
        if cfg!(target_os = "linux") && std::env::var_os("XDG_RUNTIME_DIR").is_none() {
            if let Ok(metadata) = std::fs::metadata("/proc/self") {
                command.env("XDG_RUNTIME_DIR", format!("/run/user/{}", metadata.uid()));
            }
        }
```

Replace the `VolumeControl` struct, its `impl` and the start of the file's constants as follows. Keep `COMMAND_TIMEOUT` and `COMMAND_POLL_INTERVAL`, and add:

```rust
const VOLUME_STEP_PERCENT: u16 = 5;
const MACOS_GET_VOLUME_SCRIPT: &str = "output volume of (get volume settings)";

/// Which system tools control the output volume.
#[derive(Clone, Copy, Debug, PartialEq)]
enum VolumePlatform {
    /// wpctl (PipeWire), then pactl (PulseAudio), then amixer (ALSA).
    Linux,
    /// AppleScript's `output volume` through osascript.
    MacOs,
}

/// System output volume: osascript on macOS; wpctl, pactl, then amixer elsewhere.
pub struct VolumeControl<R: CommandRunner> {
    runner: R,
    platform: VolumePlatform,
}

impl<R: CommandRunner> VolumeControl<R> {
    pub fn new(runner: R) -> Self {
        let platform = if cfg!(target_os = "macos") {
            VolumePlatform::MacOs
        } else {
            VolumePlatform::Linux
        };
        VolumeControl { runner, platform }
    }

    pub fn current_percent(&self) -> Option<u16> {
        match self.platform {
            VolumePlatform::Linux => self.linux_percent(),
            VolumePlatform::MacOs => self.macos_percent(),
        }
    }

    /// Steps the volume by 5 % without going past 0–100 % and returns the new level.
    pub fn adjust(&self, direction: RotateDirection) -> Option<u16> {
        match self.platform {
            VolumePlatform::Linux => self.adjust_linux(direction),
            VolumePlatform::MacOs => self.adjust_macos(direction),
        }
    }

    fn macos_percent(&self) -> Option<u16> {
        self.runner
            .run("osascript", &["-e", MACOS_GET_VOLUME_SCRIPT])
            .and_then(|output| parse_osascript_volume(&output))
    }

    fn adjust_macos(&self, direction: RotateDirection) -> Option<u16> {
        let current = self.macos_percent()?;
        let target = if direction == RotateDirection::Next {
            (current + VOLUME_STEP_PERCENT).min(100)
        } else {
            current.saturating_sub(VOLUME_STEP_PERCENT)
        };
        let script = format!("set volume output volume {target}");
        self.runner.run("osascript", &["-e", &script])?;
        self.macos_percent()
    }
```

Then:
- Rename the old body of `current_percent` (the wpctl → pactl → amixer chain) to `fn linux_percent(&self) -> Option<u16>`.
- Rename the old body of `adjust` to `fn adjust_linux(&self, direction: RotateDirection) -> Option<u16>`, and change its calls to `self.current_percent()` into `self.linux_percent()`.
- Keep the Linux code unchanged otherwise. Close the `impl` block.

In `impl VolumeWorker`, split `spawn`:

```rust
    /// Starts the thread, which first reads the current volume.
    pub fn spawn<R: CommandRunner + Send + 'static>(runner: R) -> Self {
        Self::spawn_control(VolumeControl::new(runner))
    }

    fn spawn_control<R: CommandRunner + Send + 'static>(control: VolumeControl<R>) -> Self {
        let (adjustments, adjustment_receiver) = mpsc::channel();
        let (reading_sender, readings) = mpsc::channel();
        thread::spawn(move || {
            if reading_sender.send(control.current_percent()).is_err() {
                return;
            }
            for direction in adjustment_receiver {
                if reading_sender.send(control.adjust(direction)).is_err() {
                    return;
                }
            }
        });
        VolumeWorker {
            adjustments,
            readings,
        }
    }
```

Add the parser next to the other `parse_*` functions:

```rust
/// "75\n" → 75; "missing value" (no output device) → None
fn parse_osascript_volume(output: &str) -> Option<u16> {
    output
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|percent| *percent <= 100)
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p translator-io volume`
Expected: all the volume tests pass, both the new ones and the renamed ones.

Run: `cargo test --workspace && make clippy && cargo fmt --all -- --check`
Expected: all pass and clippy shows no warnings.

- [ ] **Step 5: Commit**

```bash
git add translator-io/src/volume.rs
git commit -m "Control the macOS output volume through osascript"
```

---

### Task 3: macOS volume in the Python backend

**Files:**
- Create: `backend/volume.py`
- Create: `backend/test_volume.py`
- Modify: `backend/server.py` (`handle_volume`, the block from `import subprocess` through the end of `set_vol`)

**Interfaces:**
- Produces: `volume.get_volume(platform=sys.platform, run=run_command) -> int | None` and `volume.adjust_volume(direction, platform=sys.platform, run=run_command) -> bool`. Here `direction` is `"up"` or `"down"`. `run(args, env)` returns stdout as `str`, or `None` on failure.

- [ ] **Step 1: Write the failing tests**

Create `backend/test_volume.py`. Copy the Apache license header from `backend/test_pcm.py` lines 1-13, then add:

```python
import unittest

from volume import adjust_volume, get_volume, parse_osascript_volume, step_percent

OSASCRIPT_GET = ("osascript", "-e", "output volume of (get volume settings)")


class FakeRun:
    """Answers commands by their argument tuple; anything unscripted fails."""

    def __init__(self, outputs):
        self.outputs = outputs
        self.calls = []

    def __call__(self, args, env=None):
        self.calls.append(tuple(args))
        return self.outputs.get(tuple(args))


class ParseTests(unittest.TestCase):
    def test_parses_osascript_volume(self):
        self.assertEqual(parse_osascript_volume("75\n"), 75)
        self.assertEqual(parse_osascript_volume("0"), 0)
        self.assertEqual(parse_osascript_volume("100"), 100)
        self.assertIsNone(parse_osascript_volume("missing value\n"))
        self.assertIsNone(parse_osascript_volume(""))
        self.assertIsNone(parse_osascript_volume("101"))
        self.assertIsNone(parse_osascript_volume("\u00b2"))

    def test_step_clamps(self):
        self.assertEqual(step_percent(98, "up"), 100)
        self.assertEqual(step_percent(50, "up"), 55)
        self.assertEqual(step_percent(3, "down"), 0)
        self.assertEqual(step_percent(50, "down"), 45)


class MacTests(unittest.TestCase):
    def test_reads_osascript(self):
        run = FakeRun({OSASCRIPT_GET: "75\n"})
        self.assertEqual(get_volume(platform="darwin", run=run), 75)

    def test_adjust_up_sets_clamped_value(self):
        run = FakeRun({
            OSASCRIPT_GET: "98\n",
            ("osascript", "-e", "set volume output volume 100"): "",
        })
        self.assertTrue(adjust_volume("up", platform="darwin", run=run))
        self.assertIn(("osascript", "-e", "set volume output volume 100"), run.calls)

    def test_macos_unknown_volume_sets_nothing(self):
        run = FakeRun({OSASCRIPT_GET: "missing value\n"})
        self.assertFalse(adjust_volume("up", platform="darwin", run=run))
        self.assertEqual(run.calls, [OSASCRIPT_GET])


class LinuxTests(unittest.TestCase):
    def test_reads_wpctl_first(self):
        run = FakeRun({("wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"): "Volume: 0.75\n"})
        self.assertEqual(get_volume(platform="linux", run=run), 75)

    def test_falls_back_to_pactl_then_amixer(self):
        pactl = FakeRun({("pactl", "get-sink-volume", "@DEFAULT_SINK@"): "front-left: 1 /  75% / 0 dB"})
        self.assertEqual(get_volume(platform="linux", run=pactl), 75)
        amixer = FakeRun({("amixer", "sget", "Master"): "  Mono: Playback 52 [81%] [on]"})
        self.assertEqual(get_volume(platform="linux", run=amixer), 81)

    def test_adjust_tries_wpctl_then_pactl_then_amixer(self):
        run = FakeRun({("amixer", "sset", "Master", "5%-"): ""})
        self.assertTrue(adjust_volume("down", platform="linux", run=run))
        self.assertEqual(run.calls, [
            ("wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"),
            ("pactl", "set-sink-volume", "@DEFAULT_SINK@", "-5%"),
            ("amixer", "sset", "Master", "5%-"),
        ])

    def test_missing_tools(self):
        run = FakeRun({})
        self.assertIsNone(get_volume(platform="linux", run=run))
        self.assertFalse(adjust_volume("up", platform="linux", run=run))


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `python3 -m unittest discover -s backend -p 'test_volume.py'`
Expected: `ModuleNotFoundError: No module named 'volume'`.

- [ ] **Step 3: Implement `backend/volume.py`**

Copy the license header from `backend/pcm.py`, then add:

```python
"""System output volume: osascript on macOS; wpctl, pactl, then amixer elsewhere."""

import os
import re
import subprocess
import sys

STEP_PERCENT = 5
COMMAND_TIMEOUT_SECONDS = 2
MACOS_GET_VOLUME_SCRIPT = "output volume of (get volume settings)"


def run_command(args, env=None):
    """Returns the program's stdout, or None when it is missing, fails or times out."""
    try:
        return subprocess.run(
            args, check=True, capture_output=True, text=True,
            timeout=COMMAND_TIMEOUT_SECONDS, env=env,
        ).stdout
    except (OSError, subprocess.SubprocessError):
        return None


def linux_env():
    """PipeWire's wpctl finds its socket through XDG_RUNTIME_DIR, which systemd services may lack."""
    env = os.environ.copy()
    env.setdefault("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}")
    return env


def parse_osascript_volume(output):
    """"75\\n" -> 75; "missing value" (no output device) -> None."""
    text = output.strip()
    if not (text.isascii() and text.isdigit()):
        return None
    percent = int(text)
    return percent if percent <= 100 else None


def step_percent(current, direction):
    """One 5 % step up or down, kept within 0-100."""
    delta = STEP_PERCENT if direction == "up" else -STEP_PERCENT
    return max(0, min(100, current + delta))


def first_match(output, pattern):
    if output is None:
        return None
    match = re.search(pattern, output)
    return match.group(1) if match else None


def get_volume(platform=sys.platform, run=run_command):
    """The output volume in percent, or None when it cannot be read."""
    if platform == "darwin":
        output = run(["osascript", "-e", MACOS_GET_VOLUME_SCRIPT])
        return None if output is None else parse_osascript_volume(output)
    env = linux_env()
    value = first_match(run(["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"], env), r"Volume:\s+([0-9.]+)")
    if value is not None:
        return round(float(value) * 100)
    value = first_match(run(["pactl", "get-sink-volume", "@DEFAULT_SINK@"], env), r"(\d+)%")
    if value is not None:
        return int(value)
    value = first_match(run(["amixer", "sget", "Master"], env), r"\[(\d+)%\]")
    if value is not None:
        return int(value)
    return None


def adjust_volume(direction, platform=sys.platform, run=run_command):
    """Steps the volume up or down by 5 %; True when a command succeeded."""
    if platform == "darwin":
        current = get_volume(platform, run)
        if current is None:
            return False
        script = f"set volume output volume {step_percent(current, direction)}"
        return run(["osascript", "-e", script]) is not None
    env = linux_env()
    wpctl_step = "5%+" if direction == "up" else "5%-"
    pactl_step = "+5%" if direction == "up" else "-5%"
    attempts = [
        ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", wpctl_step],
        ["pactl", "set-sink-volume", "@DEFAULT_SINK@", pactl_step],
        ["amixer", "sset", "Master", wpctl_step],
    ]
    return any(run(args, env) is not None for args in attempts)
```

`any()` stops at the first success, so the order matches the old handler.

- [ ] **Step 4: Use it from `server.py`**

At the top of `backend/server.py`, next to `from pcm import ...`, add `import volume`. In `handle_volume`, delete everything from `import subprocess` through the end of the nested `set_vol` function. That is the `env` setup, `get_vol` and `set_vol`. Then change the code after it to:

```python
            success = False
            if action in ("up", "down"):
                success = volume.adjust_volume(action)
            elif action == "get":
                success = True

            if success:
                current_vol = volume.get_volume()
```

Leave the rest of the handler as is.

Run: `grep -n "get_vol\|set_vol\|subprocess" backend/server.py`
Expected: no `get_vol(` or `set_vol(` lines remain. Any remaining `subprocess` use must be outside `handle_volume`.

- [ ] **Step 5: Run the tests**

Run: `python3 -m unittest discover -s backend -p 'test_*.py'`
Expected: the old 17 tests plus the 9 new ones pass ("Ran 26 tests … OK").

Run: `python3 -c "import ast,sys; ast.parse(open('backend/server.py').read())"`
Expected: no output.

- [ ] **Step 6: Commit**

```bash
git add backend/volume.py backend/test_volume.py backend/server.py
git commit -m "Move backend volume control into a module and support macOS"
```

---

### Task 4: Fetch the macOS native libraries

**Files:**
- Modify: `scripts/fetch-native-libs.sh` (whole file)

**Interfaces:**
- Produces: `scripts/fetch-native-libs.sh [x86_64|aarch64|arm64]` fills `native/<arch>/lib/` and writes `.complete` last. On Darwin arm64 the directory holds `libmoonshine.dylib`, `libonnxruntime.1.23.2.dylib` and `liblitert-lm.dylib`. On Linux it holds the same files as today.

- [ ] **Step 1: Record the current Linux output**

Run: `ls -A native/$(uname -m)/lib | sort > /tmp/native-before.txt; cat /tmp/native-before.txt`
Expected on x86_64: `.complete`, `liblitert-lm.so`, `libmoonshine.so`, `libonnxruntime-13ab8084.so.1`. In a worktree, set up the links from "Working in a worktree" first.

- [ ] **Step 2: Rewrite the script**

Replace `scripts/fetch-native-libs.sh` with:

```bash
#!/bin/bash
# Downloads the pinned moonshine-voice and litert-lm-api wheels, verifies their
# sha256, and unpacks the native libraries into native/<arch>/lib/.
# Usage: scripts/fetch-native-libs.sh [x86_64|aarch64|arm64]   (default: this machine)
# Linux: x86_64 or aarch64. macOS: arm64 (Apple Silicon, macOS 15 or later).
set -euo pipefail

OS="$(uname -s)"
ARCH="${1:-$(uname -m)}"
PYPI=https://files.pythonhosted.org/packages
case "$OS/$ARCH" in
    Linux/x86_64)
        MOONSHINE_URL="$PYPI/de/b9/f45749837ecb75a7de2ffed261c0ec12333f66e2e7696b15655717a0e28f/moonshine_voice-0.0.65-py3-none-manylinux_2_34_x86_64.whl"
        MOONSHINE_SHA256=946e0a056dd3a13ee8cc28dec84c0f01676743a841bae5e6c52a87483b10241b
        LITERT_URL="$PYPI/36/27/bb0c2e084d59938bc12f960db50e7e8e056337ed286f31468aaa39bc9d9a/litert_lm_api-0.13.1-py3-none-manylinux_2_27_x86_64.whl"
        LITERT_SHA256=b981d55e057be9664f30670d5c1faa6e738ad516a2617dbdac10dc6b5b409cca
        ;;
    Linux/aarch64)
        MOONSHINE_URL="$PYPI/c4/b7/fe75106ffb8001ef43da1c4b75968c82f0322d9880dd3efccca1f315b556/moonshine_voice-0.0.65-py3-none-manylinux_2_34_aarch64.whl"
        MOONSHINE_SHA256=e6200c95950f6385a346b220ad046e586221d9a38db30152fa0cd12a809e9e86
        LITERT_URL="$PYPI/12/a7/c23db30dc19ad07a188eeeab48fd8d676554e8647dac6e9dbf62be0f82f1/litert_lm_api-0.13.1-py3-none-manylinux_2_27_aarch64.whl"
        LITERT_SHA256=4092c4f7ad75fb23d83bc5b00a9aa6896a19d75f148515efeef0d079ba22ef74
        ;;
    Darwin/arm64)
        MOONSHINE_URL="$PYPI/7f/49/93c28833572682c434c60cea8bcf338b74b8f6b2dbe816186c6bb5242c16/moonshine_voice-0.0.65-py3-none-macosx_15_0_arm64.whl"
        MOONSHINE_SHA256=3e92fa8497bf4fc40b6241e4dc29fd4ac77832f7f3d8addc4fd467a6bf1cf605
        LITERT_URL="$PYPI/58/ae/5d1e2862d14ad1b08bdbc3392574dde37b98a06f4efb5181d946766ec81c/litert_lm_api-0.13.1-py3-none-macosx_12_0_arm64.whl"
        LITERT_SHA256=4a1bb68dbf5a60af9ac5fe03331968a7b4e91779e721cb531e6b41e71f2a2d77
        ;;
    Darwin/x86_64)
        if [ "$(sysctl -n hw.optional.arm64 2>/dev/null)" = 1 ]; then
            echo "error: this shell runs under Rosetta; use a native arm64 terminal (arch -arm64 bash)" >&2
        else
            echo "error: Intel Macs are not supported (moonshine-voice has no x86_64 macOS wheel)" >&2
        fi
        exit 2
        ;;
    *)
        echo "usage: $0 [x86_64|aarch64|arm64]  (arm64 on macOS, x86_64/aarch64 on Linux; got $OS/$ARCH)" >&2
        exit 2
        ;;
esac

install_hint() {
    if [ "$OS" = Darwin ]; then echo "brew install $1"; else echo "sudo apt-get install -y $1"; fi
}

REQUIRED_TOOLS="curl unzip"
[ "$OS" = Linux ] && REQUIRED_TOOLS="$REQUIRED_TOOLS patchelf"
for tool in $REQUIRED_TOOLS; do
    if ! command -v "$tool" >/dev/null; then
        echo "error: '$tool' is required ($(install_hint "$tool"))" >&2
        exit 1
    fi
done
if command -v sha256sum >/dev/null; then
    SHA256_CHECK=(sha256sum --status -c -)
elif command -v shasum >/dev/null; then
    SHA256_CHECK=(shasum -a 256 --status -c -)
else
    echo "error: 'sha256sum' or 'shasum' is required ($(install_hint coreutils))" >&2
    exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/native/$ARCH/lib"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fetch() {
    local url="$1" sha256="$2" out="$3"
    curl -fL --retry 3 -o "$out" "$url"
    if ! echo "$sha256  $out" | "${SHA256_CHECK[@]}"; then
        echo "error: sha256 mismatch for $url" >&2
        exit 1
    fi
}

fetch "$MOONSHINE_URL" "$MOONSHINE_SHA256" "$WORK/moonshine.whl"
fetch "$LITERT_URL" "$LITERT_SHA256" "$WORK/litert.whl"

rm -rf "$DEST"
mkdir -p "$DEST"
printf '*\n' > "$ROOT/native/.gitignore"
if [ "$OS" = Darwin ]; then
    # The dylibs carry an @loader_path rpath, so keeping them side by side is enough.
    unzip -q -j "$WORK/moonshine.whl" 'moonshine_voice/libmoonshine.dylib' \
        'moonshine_voice/libonnxruntime.1.23.2.dylib' -d "$DEST"
    unzip -q -j "$WORK/litert.whl" 'litert_lm/liblitert-lm.dylib' -d "$DEST"
else
    unzip -q -j "$WORK/moonshine.whl" 'moonshine_voice/libmoonshine.so' \
        'moonshine_voice.libs/libonnxruntime-*.so.1' -d "$DEST"
    unzip -q -j "$WORK/litert.whl" 'litert_lm/liblitert-lm.so' -d "$DEST"
    # libmoonshine.so looks for onnxruntime in ../moonshine_voice.libs; point it at its own directory.
    patchelf --set-rpath '$ORIGIN' "$DEST/libmoonshine.so"
fi
# Written last: make treats the libraries as fetched only once every step worked.
touch "$DEST/.complete"

echo "Native libraries for $OS/$ARCH in $DEST:"
ls -1 "$DEST"
```

Bash 3.2 supports the indexed array `SHA256_CHECK` and `local`.

- [ ] **Step 3: Verify**

Run: `bash -n scripts/fetch-native-libs.sh && echo syntax-ok`
Expected: `syntax-ok`.

Run: `scripts/fetch-native-libs.sh arm64; echo "exit=$?"`
Expected on Linux: the usage line mentioning `got Linux/arm64`, then `exit=2`.

Run: `make -B native-libs && ls -A native/$(uname -m)/lib | sort | diff /tmp/native-before.txt - && echo same-files`
Expected: both downloads pass the sha256 check, and the output ends with `same-files`.

Run: `for u in https://files.pythonhosted.org/packages/7f/49/93c28833572682c434c60cea8bcf338b74b8f6b2dbe816186c6bb5242c16/moonshine_voice-0.0.65-py3-none-macosx_15_0_arm64.whl https://files.pythonhosted.org/packages/58/ae/5d1e2862d14ad1b08bdbc3392574dde37b98a06f4efb5181d946766ec81c/litert_lm_api-0.13.1-py3-none-macosx_12_0_arm64.whl; do curl -sfIL -o /dev/null -w '%{http_code}\n' "$u"; done`
Expected: `200` twice.

Check the hashes: run `printf 'abc' > /tmp/x && echo "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  /tmp/x" | shasum -a 256 --status -c - && echo shasum-ok`. This proves the `shasum` fallback syntax works, if `shasum` is installed here.

- [ ] **Step 4: Commit**

```bash
git add scripts/fetch-native-libs.sh
git commit -m "Fetch the macOS arm64 native libraries"
```

---

### Task 5: Make `start.sh` work on macOS

**Files:**
- Modify: `start.sh` (the `XDG_RUNTIME_DIR` export, the "Set system volume to max" block, and the final `wait -n` block)

- [ ] **Step 1: Edit**

Replace

```bash
# Ensure PipeWire env is available for volume control (wpctl)
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
```

with

```bash
OS="$(uname -s)"

# Ensure PipeWire env is available for volume control (wpctl)
if [ "$OS" = "Linux" ]; then
    export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
fi
```

Replace

```bash
wpctl set-volume @DEFAULT_AUDIO_SINK@ 1.0 || amixer sset Master 100% 2>/dev/null || true
```

with

```bash
if [ "$OS" = "Darwin" ]; then
    osascript -e "set volume output volume 100" || true
else
    wpctl set-volume @DEFAULT_AUDIO_SINK@ 1.0 || amixer sset Master 100% 2>/dev/null || true
fi
```

Replace the final block

```bash
# Wait for any child to exit (then the trap will clean up the others)
wait -n
echo "[start.sh] A child process exited. Shutting down."
exit 1
```

with

```bash
# Wait for any child to exit (then the trap will clean up the others).
# macOS ships bash 3.2, which has no `wait -n`, so poll the children instead.
while true; do
    for pid in $LITERT_PID $API_PID $WEB_PID; do
        if ! kill -0 "$pid" 2>/dev/null; then
            echo "[start.sh] A child process exited. Shutting down."
            exit 1
        fi
    done
    sleep 1
done
```

- [ ] **Step 2: Verify**

Run: `bash -n start.sh && echo syntax-ok`
Expected: `syntax-ok`.

The next check confirms the loop notices a dead child quickly. bash reaps background children itself, so `kill -0` fails once one has exited:

```bash
start=$(date +%s)
bash -c 'sleep 1 & A=$!; sleep 30 & B=$!; trap "kill $B 2>/dev/null" EXIT
while true; do for pid in $A $B; do if ! kill -0 "$pid" 2>/dev/null; then exit 1; fi; done; sleep 1; done'
echo "exit=$? after $(( $(date +%s) - start ))s"
```

Expected: `exit=1 after 1s` or `2s`, and never 30 s.

Run: `grep -n "wait -n" start.sh`
Expected: no output.

- [ ] **Step 3: Commit**

```bash
git add start.sh
git commit -m "Make start.sh run under macOS bash 3.2 and set the macOS volume"
```

---

### Task 6: Import an already downloaded Gemma in `download_model.sh`

**Files:**
- Modify: `download_model.sh` (between the "already imported" check and the free-space check, plus the space requirement)

**Interfaces:**
- Consumes: `models/gemma/gemma-4-E2B-it.litertlm`, the path `make models` writes (`translator-core/src/models.rs` `GEMMA_FILE_NAME` under `<model_dir>/gemma/`). It honours `MODEL_DIR`, as the Makefile does.

- [ ] **Step 1: Edit**

After the `if litert-lm list ... fi` block, insert:

```bash
# make models already downloads the same file into the model directory;
# import that copy instead of downloading it a second time.
LOCAL_MODEL="${MODEL_DIR:-models}/gemma/$MODEL_FILE"
```

Then replace the free-space block (from `REQUIRED_GB=6` through its closing `fi`) with:

```bash
# Importing copies ~2.6GB; downloading first needs another ~2.6GB for the
# Hugging Face cache. Require headroom before starting.
if [ -f "$LOCAL_MODEL" ]; then
    REQUIRED_GB=3
else
    REQUIRED_GB=6
fi
AVAIL_GB="$(df -Pk "$HOME" | awk 'NR==2 { print int($4 / 1024 / 1024) }')"
if [ -n "$AVAIL_GB" ] && [ "$AVAIL_GB" -lt "$REQUIRED_GB" ]; then
    echo "Error: only ${AVAIL_GB}GB free on the filesystem holding $HOME."
    echo "At least ${REQUIRED_GB}GB is needed to import the model."
    echo "Free up space and re-run this script."
    exit 1
fi

if [ -f "$LOCAL_MODEL" ]; then
    echo "Importing $LOCAL_MODEL as '$MODEL_NAME'..."
    litert-lm import "$LOCAL_MODEL" "$MODEL_NAME"
    echo "========================================="
    echo "Model import complete!"
    echo "You can now start the server with ./start.sh"
    echo "========================================="
    exit 0
fi
```

Leave the Hugging Face download path below unchanged.

- [ ] **Step 2: Verify**

Run: `bash -n download_model.sh && echo syntax-ok`
Expected: `syntax-ok`.

Run: `./download_model.sh` in a checkout with `venv/` (the main checkout, or a worktree linked as in "Working in a worktree").
Expected: `Model 'gemma4-e2b' is already imported and ready to use!` when it is already imported. The local-import path itself is part of the user's manual check (spec §8 step 1) because re-importing here would replace a working model.

- [ ] **Step 3: Commit**

```bash
git add download_model.sh
git commit -m "Import the Gemma file from make models instead of downloading it again"
```

---

### Task 7: `scripts/setup-env.sh` and `make setup`

**Files:**
- Create: `scripts/setup-env.sh` (mode 755)
- Modify: `Makefile` (new `setup` target; `models` recipe writes a stamp; header comment)

**Interfaces:**
- Consumes: `make native-libs`, `make models MODEL_DIR=...`, `./setup.sh`, `./download_model.sh` (Task 6).
- Produces: `make setup` / `scripts/setup-env.sh [--yes]`. It exits 0 when every item is `ok`, `installed` or `skipped`, 1 when a tool is missing or any item `FAILED`, and 2 on a bad option. `make models` now writes `$(MODEL_DIR)/.complete` after `translator-models` succeeds.

- [ ] **Step 1: Makefile**

At the end of the `models:` recipe, after the `$(CARGO) run --release -p translator-models ...` line, add:

```make
	touch $(MODEL_DIR)/.complete
```

Its comment line becomes `## models: download every model into MODEL_DIR (default ./models, ~3.8 GB; writes MODEL_DIR/.complete when done)`.

After the `help` target, add:

```make
## setup: check system tools and fetch everything the project downloads (SETUP_ARGS=--yes skips the model prompt)
.PHONY: setup
setup:
	scripts/setup-env.sh $(SETUP_ARGS)
```

Change the header's second line to: ``# `make run-mock` works on any Linux or macOS desktop; `build-pi` targets the Raspberry Pi.``

- [ ] **Step 2: Write the script**

Create `scripts/setup-env.sh`:

```bash
#!/bin/bash
# Checks the system tools this project needs, then fetches everything it
# downloads into the repository: venv/, the native libraries, the models,
# the Gemma import for `litert-lm serve`, and frontend/node_modules.
# Finished items are skipped, so it is safe to run again.
# Usage: scripts/setup-env.sh [--yes]   (--yes: download models without asking)
set -o pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT" || exit 1
OS="$(uname -s)"
ARCH="$(uname -m)"
MODEL_DIR="${MODEL_DIR:-$ROOT/models}"
GEMMA_FILE="gemma-4-E2B-it.litertlm"
GEMMA_MODEL_NAME="gemma4-e2b"
SHOULD_ASSUME_YES=0

for arg in "$@"; do
    case "$arg" in
        --yes | -y) SHOULD_ASSUME_YES=1 ;;
        --help | -h)
            echo "usage: $0 [--yes]   (--yes: download models without asking)"
            exit 0
            ;;
        *)
            echo "unknown option: $arg (usage: $0 [--yes])" >&2
            exit 2
            ;;
    esac
done

# ---- Step 1: system tools -------------------------------------------------

MISSING=()

install_hint() { # install_hint <brew formula> <apt packages>
    if [ "$OS" = Darwin ]; then echo "brew install $1"; else echo "sudo apt-get install -y $2"; fi
}

need_command() { # need_command <command> <install hint>
    command -v "$1" >/dev/null 2>&1 || MISSING+=("$1|$2")
}

case "$OS" in
    Darwin)
        if [ "$ARCH" != arm64 ]; then
            if [ "$(sysctl -n hw.optional.arm64 2>/dev/null)" = 1 ]; then
                echo "error: this shell runs under Rosetta; use a native arm64 terminal (arch -arm64 bash)." >&2
            else
                echo "error: Intel Macs are not supported (moonshine-voice has no x86_64 macOS wheel)." >&2
            fi
            exit 1
        fi
        MACOS_VERSION="$(sw_vers -productVersion)"
        if [ "${MACOS_VERSION%%.*}" -lt 15 ]; then
            echo "error: macOS 15 or later is required (this Mac runs $MACOS_VERSION)." >&2
            exit 1
        fi
        xcode-select -p >/dev/null 2>&1 || MISSING+=("Xcode Command Line Tools|xcode-select --install")
        ;;
    Linux)
        need_command make "$(install_hint make make)"
        need_command patchelf "$(install_hint patchelf patchelf)"
        need_command pkg-config "$(install_hint pkg-config pkg-config)"
        if command -v pkg-config >/dev/null 2>&1 && ! pkg-config --exists alsa; then
            MISSING+=("ALSA development files|sudo apt-get install -y libasound2-dev")
        fi
        LDCONFIG="$(command -v ldconfig || echo /sbin/ldconfig)"
        if ! "$LDCONFIG" -p 2>/dev/null | grep -q 'libvulkan\.so\.1'; then
            MISSING+=("libvulkan.so.1|sudo apt-get install -y libvulkan1")
        fi
        ;;
    *)
        echo "error: unsupported operating system $OS (Linux or macOS only)." >&2
        exit 1
        ;;
esac

need_command cargo "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
if ! python3 -c 'import sys, ensurepip, venv; sys.exit(sys.version_info < (3, 10))' >/dev/null 2>&1; then
    if [ "$OS" = Darwin ]; then
        PYTHON_HINT="brew install python@3.12, then put \$(brew --prefix)/bin before /usr/bin in PATH"
    else
        PYTHON_HINT="sudo apt-get install -y python3 python3-venv"
    fi
    MISSING+=("python3 >= 3.10 with venv|$PYTHON_HINT")
fi
need_command curl "$(install_hint curl curl)"
need_command unzip "$(install_hint unzip unzip)"
if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
    MISSING+=("sha256sum or shasum|sudo apt-get install -y coreutils")
fi

if [ "${#MISSING[@]}" -gt 0 ]; then
    echo "Missing system tools. Install them, then run make setup again:"
    for item in "${MISSING[@]}"; do
        printf '  %-28s %s\n' "${item%%|*}" "${item#*|}"
    done
    exit 1
fi
echo "System tools: ok"

# ---- Step 2: project items --------------------------------------------------

ITEM_NAMES=()
ITEM_STATES=()
ITEM_NOTES=()

record() { # record <item> <ok|installed|skipped|FAILED> [note]
    ITEM_NAMES+=("$1")
    ITEM_STATES+=("$2")
    ITEM_NOTES+=("${3:-}")
}

confirm() { # confirm <question>; yes with --yes, no when stdin is not a terminal
    [ "$SHOULD_ASSUME_YES" -eq 1 ] && return 0
    [ -t 0 ] || return 1
    local answer
    read -r -p "$1 [y/N] " answer
    case "$answer" in y | Y | yes | YES) return 0 ;; *) return 1 ;; esac
}

# Python venv (litert-lm serve, server.py)
if [ -x venv/bin/litert-lm ]; then
    record "Python venv" ok
elif ./setup.sh; then
    record "Python venv" installed
else
    record "Python venv" FAILED "run ./setup.sh"
fi

# Native libraries (inference_backend = "native")
HAS_NATIVE_LIBS=0
if [ -f "native/$ARCH/lib/.complete" ]; then
    record "Native libraries" ok
    HAS_NATIVE_LIBS=1
elif make native-libs; then
    record "Native libraries" installed
    HAS_NATIVE_LIBS=1
else
    record "Native libraries" FAILED "run make native-libs"
fi

# Models (~3.8 GB; make models skips files it already has)
if [ -f "$MODEL_DIR/.complete" ]; then
    record "Models" ok
elif [ "$HAS_NATIVE_LIBS" -eq 0 ]; then
    record "Models" skipped "needs the native libraries"
elif ! confirm "Download the models (~3.8 GB, files already present are kept) into $MODEL_DIR?"; then
    record "Models" skipped "run make setup SETUP_ARGS=--yes or make models"
elif make models MODEL_DIR="$MODEL_DIR"; then
    record "Models" installed
else
    record "Models" FAILED "run make models (it resumes where it stopped)"
fi

# Gemma imported into litert-lm (inference_backend = "http", ./start.sh)
if [ ! -x venv/bin/litert-lm ]; then
    record "Gemma for litert-lm serve" skipped "needs the Python venv"
elif venv/bin/litert-lm list 2>/dev/null | awk -v m="$GEMMA_MODEL_NAME" '$1 == m { found = 1 } END { exit !found }'; then
    record "Gemma for litert-lm serve" ok
elif [ ! -f "$MODEL_DIR/gemma/$GEMMA_FILE" ] \
    && ! confirm "Download Gemma (~2.6 GB) from Hugging Face for litert-lm serve?"; then
    record "Gemma for litert-lm serve" skipped "run ./download_model.sh"
elif MODEL_DIR="$MODEL_DIR" ./download_model.sh; then
    record "Gemma for litert-lm serve" installed
else
    record "Gemma for litert-lm serve" FAILED "run ./download_model.sh"
fi

# React web UI dependencies (./start.sh)
if [ -d frontend/node_modules ]; then
    record "frontend/node_modules" ok
elif ! command -v npm >/dev/null 2>&1; then
    record "frontend/node_modules" skipped "install Node.js 18+ for the React web UI"
elif npm --prefix frontend install; then
    record "frontend/node_modules" installed
else
    record "frontend/node_modules" FAILED "run npm --prefix frontend install"
fi

# ---- Step 3: summary ------------------------------------------------------------

HAS_FAILURE=0
echo
echo "Setup summary:"
for index in "${!ITEM_NAMES[@]}"; do
    printf '  %-28s %-10s %s\n' "${ITEM_NAMES[$index]}" "${ITEM_STATES[$index]}" "${ITEM_NOTES[$index]}"
    [ "${ITEM_STATES[$index]}" = FAILED ] && HAS_FAILURE=1
done
if [ "$HAS_FAILURE" -eq 1 ]; then
    exit 1
fi
echo
echo "Ready: make run-native, make run-tui, or ./start.sh"
```

Run: `chmod +x scripts/setup-env.sh && bash -n scripts/setup-env.sh && echo syntax-ok`
Expected: `syntax-ok`.

- [ ] **Step 3: Verify the non-interactive and missing-tool paths**

The expected results below are for this development machine. Gemma is already imported in the main venv, and `cargo` lives in `~/.cargo/bin`. On another machine, compare against what is actually installed.

Run: `scripts/setup-env.sh < /dev/null; echo "exit=$?"`
Expected:
- `System tools: ok`;
- `Python venv`, `Native libraries`, `Gemma for litert-lm serve` and `frontend/node_modules` are `ok`;
- `Models` is `skipped`, because no `.complete` stamp exists yet and stdin is not a terminal;
- no download starts, and the last line is `exit=0`.

Run: `env PATH=/usr/bin:/bin scripts/setup-env.sh < /dev/null; echo "exit=$?"`
Expected: `Missing system tools.` lists `cargo` with the rustup command, and the run ends with `exit=1`. `cargo` lives in `~/.cargo/bin`, which this PATH leaves out.

Run: `scripts/setup-env.sh --bogus; echo "exit=$?"`
Expected: `unknown option: --bogus …`, then `exit=2`.

- [ ] **Step 4: Verify the stamp and the re-run**

Run: `scripts/setup-env.sh --yes < /dev/null; echo "exit=$?"`
Expected: `make models` runs. Every file prints `present` and none prints `fetched`. `Models` is `installed`, `$MODEL_DIR/.complete` now exists, and the run ends with `exit=0`.

Run: `scripts/setup-env.sh < /dev/null | tail -8`
Expected: every item is `ok`, and the last line is `Ready: make run-native, make run-tui, or ./start.sh`.

Run: `make setup SETUP_ARGS=--help`
Expected: the usage line.

- [ ] **Step 5: Commit**

```bash
git add scripts/setup-env.sh Makefile
git commit -m "Add make setup to check tools and fetch the project's downloads"
```

---

### Task 8: Documentation and the macOS cross-check

**Files:**
- Modify: `README.md` (Prerequisites, Setup Instructions, the phase B section, a new macOS section before `## Project Structure`)
- Modify: `CLAUDE.md` (Commands block and the Rust UI list)

- [ ] **Step 1: README**

In `## Prerequisites`, replace `- Linux or macOS` with:

```markdown
- Linux, or macOS 15+ on Apple Silicon (see [macOS](#macos-apple-silicon))
```

At the start of `## Setup Instructions`, before item 1, insert:

```markdown
`make setup` checks the system tools, then creates `venv/`, fetches the native
libraries and models, imports Gemma for `litert-lm serve` and installs the
frontend packages, skipping anything already there. It prints the install
command for any missing tool instead of installing it; `make setup
SETUP_ARGS=--yes` downloads the ~3.8 GB of models without asking. The steps
below do the same by hand.
```

In `### In-process inference (phase B, experimental)`, change the line `sudo apt-get install -y patchelf unzip libvulkan1` so the block reads:

```bash
sudo apt-get install -y patchelf unzip libvulkan1   # Linux; macOS needs none of these
make native-libs     # pinned libmoonshine / liblitert-lm into native/<arch>/lib
```

Keep the rest of that block. Then add this section before `## Project Structure`:

````markdown
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
````

- [ ] **Step 2: CLAUDE.md**

In the Commands code block, add after `make test ...`:

```
make setup              # check system tools, fetch venv/native libs/models/node_modules (SETUP_ARGS=--yes)
```

In the Rust UI list, after the `make native-libs` needs `patchelf` line, add:

```markdown
- macOS (Apple Silicon, macOS 15+): the libraries are `.dylib` files that carry an `@loader_path` rpath, so no patchelf; volume goes through `osascript`; `start.sh` must stay bash 3.2-compatible (no `wait -n`).
```

- [ ] **Step 3: Cross-check the pure crates for macOS**

Run: `rustup target add aarch64-apple-darwin && cargo check --tests --target aarch64-apple-darwin -p moonshine-sys -p litert-lm-sys -p translator-core`
Expected: `Finished`. This confirms the `cfg(target_os = "macos")` constants and the macOS side of the new tests compile.

Run: `cargo check --tests --target aarch64-apple-darwin -p translator-io 2>&1 | tail -5`
Expected: either `Finished`, or a failure inside a dependency's build script (for example `coreaudio-sys` needing the macOS SDK). Record which one happened in the task report. A failure in our own crates' code is a bug to fix here.

- [ ] **Step 4: Final checks**

Run: `make test && make clippy && cargo fmt --all -- --check`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add README.md CLAUDE.md
git commit -m "Document macOS support and make setup"
```
