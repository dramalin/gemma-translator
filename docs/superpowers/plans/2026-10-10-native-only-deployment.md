# Native-Only Deployment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make in-process (native) inference the only path, delete the Python backend, React frontend and their scripts, and replace the Pi deployment with a cross-compiled tar plus `sudo ./install.sh`.

**Architecture:** The HTTP clients and the four HTTP config keys go; `AppState` always builds native services. A run-time `--model-dir` override reaches only the native services, so it never changes `Config` or the saved file. `make package-pi` cross-compiles the kms `translator` with cross-rs inside a Debian bookworm image (same glibc and libinput as the Pi). It checks symbol versions and tars the binary, the aarch64 libraries, the models and `deploy/install.sh`. The installer checks everything before touching the system, then installs to `/opt` and `/var/lib`, enables the user's PipeWire session with linger, and boots to the console.

**Tech Stack:** Rust (Slint, cpal), GNU make 3.81+, bash, cross-rs 0.2.5, Docker, Debian bookworm, systemd.

**Spec:** `docs/superpowers/specs/2026-10-10-native-only-deployment-design.md`

## Global Constraints

- Pi target: Raspberry Pi OS Bookworm, 64-bit, with desktop. Symbol ceilings `GLIBC_2.36` and `GLIBCXX_3.4.30`; the installer requires glibc ≥ 2.34.
- Cross build: `cross` 0.2.5 (`cargo install cross --version 0.2.5 --locked`), image `deploy/Dockerfile.pi` based on `debian:bookworm`, target `aarch64-unknown-linux-gnu`, command `cross build --release -p translator-ui --no-default-features --features kms --target aarch64-unknown-linux-gnu`.
- Package: `dist/gemma-translator-pi.tar`, uncompressed, with top-level folder `gemma-translator/` holding `translator`, `lib/`, `models/` (without `cache/` and `.gitignore`), `gemma-translator.service` and `install.sh`.
- Install paths: `/opt/gemma-translator/` (binary + `lib/`), `/var/lib/gemma-translator/models/`, `/etc/systemd/system/gemma-translator.service`.
- Runtime package list checked by `install.sh` (verbatim): `libegl1 libgles2 libgbm1 libgl1-mesa-dri libvulkan1 mesa-vulkan-drivers libinput10 libxkbcommon0 xkb-data libudev1 libasound2 libfontconfig1 libdrm2 pipewire pipewire-alsa wireplumber alsa-utils fonts-dejavu-core fonts-noto-cjk fonts-noto-core`.
- `install.sh` never calls apt and changes nothing on the system before every check passes.
- `--model-dir <path>`: run-time only. Missing value → error `--model-dir needs a path`. Ignored with `--mock`. Never written to the config file.
- Linux and macOS desktops: `make setup && make run` must work; `MODEL_DIR` defaults to `$(CURDIR)/models`.
- Project rules:
  - booleans start with `is`/`has`/`should`/`can`;
  - commit messages are imperative, with no `Co-Authored-By` or other AI attribution;
  - no TODOs, no unused helpers;
  - stage only the files the task names, never `git add -A`/`git add .`.
- Rust checks before each Rust commit: `cargo test --workspace`, `make clippy`, `cargo fmt --all -- --check`.

## Review Focus

1. An existing `config.toml` written by the HTTP-era app (`inference_backend`, `speech_service_url`, `llm_endpoint`, `llm_model`) must load with every other setting intact. The next save drops the old keys (Task 1 test `old_http_keys_are_ignored_and_dropped_on_save`).
2. `make run` passes `--model-dir ./models`, and the user then changes a setting. The saved file keeps its own `model_dir`, never `./models` (Task 2 test `override_reaches_only_the_model_paths` + structural: `Config` is not touched).
3. `make package-pi` with incomplete models (no `.complete`) or with a binary that needs a newer glibc/libstdc++ than the Pi has must refuse and name the problem, and must leave no half-written tar (Task 6 Steps 2 and 4).
4. `install.sh` run without sudo, or on a non-aarch64 machine, must exit before changing anything (Task 7 Step 3).
5. Re-running `install.sh` (an update) must stop the old service, skip unchanged models and restart into the new binary (Task 7 Step 2 design; Pi manual check 4).

## Refinements to the spec

- Spec §3.2 adds `model_dir_override` to `AppState::headless` for a save test. `headless` always uses mock services, which never read model paths, so the parameter would go unused there. This plan instead routes the override through one function, `model_root(config, override)`, used only by `native_services`, and unit-tests it. `Config` is never mutated, so nothing can write the override back.
- Spec §3.1 prunes `.gitignore` rules for removed files. This plan keeps them: users still have local `venv/`, `frontend/node_modules/` and `backend/__pycache__/`, which would otherwise show up as untracked files and risk being committed. The root `package-lock.json` is already untracked and ignored.
- Spec §3.4 step 4 is implemented as a separate script, `scripts/check-symbol-versions.sh`, so it can be tested on host files without a full cross build.
- `make run-tui ARGS=--mock` now depends on `native-libs`, like `make run`. A first `--mock` run on a fresh machine fetches the libraries once.

## Working in a worktree

Before Task 1, link the main checkout's git-ignored downloads so checks do not re-download gigabytes:

```bash
MAIN="$(git worktree list --porcelain | awk 'NR==1 { print $2 }')"
for item in native models; do
    [ -e "$item" ] || ln -s "$MAIN/$item" "$item"
done
```

`native` and `models` are already excluded in `.git/info/exclude`.

---

## File Structure

| File | Change |
|---|---|
| `translator-core/src/config.rs` | Drop `InferenceBackend` and the 4 HTTP fields; update tests |
| `translator-io/src/{lib.rs,speech_client.rs,llm_client.rs,http_error.rs}`, `translator-io/tests/{speech_client.rs,llm_client.rs,support/}`, `translator-io/Cargo.toml` | Delete the HTTP clients and their deps |
| `translator-app/src/state.rs` | Native services only; `model_dir_override`; `model_root` |
| `translator-app/src/cli.rs` | `--model-dir` |
| `translator-ui/src/main.rs`, `translator-tui/src/main.rs` | Pass the override; usage strings |
| `Makefile` | `test`, `run`, `run-tui`, `package-pi`; drop `run-native`, `build-pi` |
| `scripts/setup-env.sh` | Tools + native libs + models only |
| `backend/`, `frontend/`, `setup.sh`, `start.sh`, `download_model.sh`, `deploy-pi.sh`, `deploy/gemma-translator.service` | Delete |
| `deploy/Dockerfile.pi`, `Cross.toml` (new) | Bookworm arm64 cross image |
| `scripts/check-symbol-versions.sh`, `scripts/package-pi.sh` (new) | Symbol check; tar builder |
| `deploy/install.sh`, `deploy/gemma-translator.service.in` (new) | Pi installer and unit template |
| `README.md`, `CLAUDE.md`, `docs/superpowers/specs/2026-10-09-rust-frontend-design.md`, `docs/superpowers/specs/2026-10-09-tui-frontend-design.md` | Docs |

---

### Task 1: Remove the HTTP inference path

**Files:**
- Modify: `translator-core/src/config.rs` (enum at ~102-108, struct fields ~118-121, defaults ~138-141, tests ~281-289 and ~345-364)
- Modify: `translator-app/src/state.rs` (imports lines 10, 22, 26; `new` ~156-160; `configured_services` ~369-384)
- Modify: `translator-io/src/lib.rs`, `translator-io/Cargo.toml`
- Delete: `translator-io/src/speech_client.rs`, `translator-io/src/llm_client.rs`, `translator-io/src/http_error.rs`, `translator-io/tests/speech_client.rs`, `translator-io/tests/llm_client.rs`, `translator-io/tests/support/`

**Interfaces:**
- Produces: `Config` without `inference_backend`, `speech_service_url`, `llm_endpoint`, `llm_model`; no `InferenceBackend` type. `fn native_services(config: &Config) -> Services` stays in `state.rs` (Task 2 changes its signature).

- [ ] **Step 1: Write the failing test**

In `translator-core/src/config.rs` `mod tests`, add:

```rust
    #[test]
    fn old_http_keys_are_ignored_and_dropped_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "inference_backend = \"http\"\n\
             speech_service_url = \"http://localhost:3000\"\n\
             llm_endpoint = \"http://localhost:9379/v1\"\n\
             llm_model = \"gemma4-e2b\"\n\
             keyboard_mode = \"vertical\"\n\
             model_dir = \"/home/pi/models\"\n",
        )
        .unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!(config.keyboard_mode, KeyboardMode::Vertical);
        assert_eq!(config.model_dir, PathBuf::from("/home/pi/models"));
        config.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        for key in [
            "inference_backend",
            "speech_service_url",
            "llm_endpoint",
            "llm_model",
        ] {
            assert!(!text.contains(key), "{key} was written back:\n{text}");
        }
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p translator-core old_http_keys`
Expected: FAIL with `inference_backend was written back`.

- [ ] **Step 3: Remove the HTTP config**

In `translator-core/src/config.rs`:
- delete the `InferenceBackend` enum and its doc comment;
- delete the four fields `speech_service_url`, `llm_endpoint`, `llm_model`, `inference_backend` from `Config` and from `Default`.

In the existing test `partial_file_fills_in_defaults`, replace `assert_eq!(config.llm_model, "gemma4-e2b");` with:

```rust
        assert_eq!(
            config.model_dir,
            PathBuf::from("/var/lib/gemma-translator/models")
        );
```

Replace the whole test `inference_backend_and_model_dir_have_defaults_and_parse` with:

```rust
    #[test]
    fn model_dir_has_a_default_and_parses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(
            Config::load(&path).unwrap().model_dir,
            PathBuf::from("/var/lib/gemma-translator/models")
        );
        std::fs::write(&path, "model_dir = \"/home/pi/models\"\n").unwrap();
        assert_eq!(
            Config::load(&path).unwrap().model_dir,
            PathBuf::from("/home/pi/models")
        );
    }
```

- [ ] **Step 4: Remove the HTTP clients**

```bash
git rm -q translator-io/src/speech_client.rs translator-io/src/llm_client.rs translator-io/src/http_error.rs \
    translator-io/tests/speech_client.rs translator-io/tests/llm_client.rs
git rm -q -r translator-io/tests/support
```

In `translator-io/src/lib.rs`, delete the lines `mod http_error;`, `pub mod llm_client;` and `pub mod speech_client;`. Change the crate doc line to `//! Device- and process-bound parts of the translator.`

In `translator-io/Cargo.toml`, delete `ureq = …`. Then run `grep -rn "serde" translator-io/src translator-io/tests`. If nothing matches, also delete the `serde = …` and `serde_json = …` lines.

In `translator-app/src/state.rs`:
- change line 10 to `use translator_core::config::Config;`;
- delete the `use translator_io::llm_client::LlmClient;` and `use translator_io::speech_client::SpeechServiceClient;` lines;
- delete the whole `configured_services` function and its doc comment;
- in `AppState::new`, replace `configured_services(&config)` with `native_services(&config)`.

- [ ] **Step 5: Run the checks**

Run: `cargo test -p translator-core config && cargo test --workspace && make clippy && cargo fmt --all -- --check`
Expected: all pass. `grep -rn "InferenceBackend\|llm_endpoint\|speech_service_url\|llm_model\|ureq" --include=*.rs --include=Cargo.toml translator-* moonshine-sys litert-lm-sys` prints only `translator-models` lines.

- [ ] **Step 6: Commit**

```bash
git add translator-core/src/config.rs translator-app/src/state.rs translator-io/src/lib.rs translator-io/Cargo.toml Cargo.lock
git commit -m "Remove the HTTP inference path and its config keys"
```

---

### Task 2: `--model-dir` run-time override

**Files:**
- Modify: `translator-app/src/cli.rs` (`CliOptions`, `parse_args`, tests)
- Modify: `translator-app/src/state.rs` (`AppState::new`, `native_services`, new `model_root`, tests)
- Modify: `translator-ui/src/main.rs` (USAGE line 17, `AppState::new` call ~41-46)
- Modify: `translator-tui/src/main.rs` (USAGE line 25, `AppState::new` call ~68)

**Interfaces:**
- Consumes: `native_services(config: &Config) -> Services` from Task 1.
- Produces:
  - `CliOptions { is_mock: bool, config_path: Option<PathBuf>, model_dir: Option<PathBuf> }`;
  - `AppState::new(config: Config, config_path: Option<PathBuf>, is_mock: bool, record_style: RecordStyle, model_dir_override: Option<PathBuf>) -> AppState`;
  - private `fn model_root(config: &Config, model_dir_override: Option<&Path>) -> PathBuf`;
  - `fn native_services(config: &Config, model_dir_override: Option<&Path>) -> Services`.

- [ ] **Step 1: Write the failing tests**

In `translator-app/src/cli.rs` tests, add:

```rust
    #[test]
    fn parses_model_dir() {
        assert_eq!(
            parse_args(args(&["--model-dir", "/repo/models", "--config", "/tmp/c.toml"])),
            Ok(CliOptions {
                is_mock: false,
                config_path: Some(PathBuf::from("/tmp/c.toml")),
                model_dir: Some(PathBuf::from("/repo/models")),
            })
        );
        assert_eq!(
            parse_args(args(&["--model-dir"])),
            Err("--model-dir needs a path".to_string())
        );
    }
```

In the existing `parses_mock_and_config` test, add `model_dir: None` to both expected `CliOptions` literals.

In `translator-app/src/state.rs` tests, add:

```rust
    #[test]
    fn override_reaches_only_the_model_paths() {
        let config = Config {
            model_dir: PathBuf::from("/var/lib/gemma-translator/models"),
            ..Config::default()
        };
        assert_eq!(
            model_root(&config, Some(Path::new("/repo/models"))),
            PathBuf::from("/repo/models")
        );
        assert_eq!(
            model_root(&config, None),
            PathBuf::from("/var/lib/gemma-translator/models")
        );
        // The override is a separate value: the config that gets saved is untouched.
        assert_eq!(
            config.model_dir,
            PathBuf::from("/var/lib/gemma-translator/models")
        );
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p translator-app`
Expected: compile errors: `model_dir` is not a field of `CliOptions`, and `model_root` is not found.

- [ ] **Step 3: Implement**

`cli.rs`:

```rust
#[derive(Debug, PartialEq, Eq)]
pub struct CliOptions {
    pub is_mock: bool,
    pub config_path: Option<PathBuf>,
    /// Model directory for this run only; never written to the config file.
    pub model_dir: Option<PathBuf>,
}
```

In `parse_args`, initialise `model_dir: None` and add a match arm after `--config`:

```rust
            "--model-dir" => {
                let path = args.next().ok_or("--model-dir needs a path")?;
                options.model_dir = Some(PathBuf::from(path));
            }
```

`state.rs`:
- change `use std::path::PathBuf;` to `use std::path::{Path, PathBuf};`.
- `AppState::new` gains a last parameter `model_dir_override: Option<PathBuf>`. Its doc comment gains the line `/// \`model_dir_override\` (from \`--model-dir\`) replaces \`config.model_dir\` for this run only; the saved config keeps its own value.`
- In `AppState::new`, the services become `native_services(&config, model_dir_override.as_deref())`.
- Replace `native_services` with:

```rust
/// In-process inference: one speech thread (preloading both lanes' languages)
/// and one model thread, both loading their libraries from `<exe dir>/lib`.
fn native_services(config: &Config, model_dir_override: Option<&Path>) -> Services {
    let lib_dir = native::default_lib_dir();
    let paths = ModelPaths::new(model_root(config, model_dir_override));
    let speech = Arc::new(NativeSpeech::spawn(
        lib_dir.clone(),
        paths.clone(),
        vec![config.lane1_language, config.lane2_language],
    ));
    Services {
        stt: speech.clone(),
        translator: Arc::new(NativeTranslator::spawn(lib_dir, paths)),
        tts: speech,
    }
}

/// The model directory: the `--model-dir` override for this run, else the config's.
fn model_root(config: &Config, model_dir_override: Option<&Path>) -> PathBuf {
    model_dir_override.map_or_else(|| config.model_dir.clone(), Path::to_path_buf)
}
```

`ModelPaths::new` takes `impl Into<PathBuf>`, so the returned `PathBuf` goes in directly.

`translator-ui/src/main.rs`: USAGE becomes `"usage: translator [--mock] [--config <path>] [--model-dir <path>]"`, and the call becomes `AppState::new(config, config_path, options.is_mock, RecordStyle::Hold, options.model_dir)`.

`translator-tui/src/main.rs`: USAGE becomes `"usage: translator-tui [--mock] [--config <path>] [--model-dir <path>]"`, and the call becomes `AppState::new(config, config_path, options.is_mock, RecordStyle::Toggle, options.model_dir)`.

`options.config_path` is moved earlier in both mains by `options.config_path.or_else(...)`. If the borrow checker complains about a partial move, bind `let model_dir = options.model_dir;` before that line and pass `model_dir`.

- [ ] **Step 4: Run the checks**

Run: `cargo test -p translator-app && cargo test --workspace && make clippy && cargo fmt --all -- --check`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add translator-app/src/cli.rs translator-app/src/state.rs translator-ui/src/main.rs translator-tui/src/main.rs
git commit -m "Add a --model-dir run-time override for the native models"
```

---

### Task 3: Makefile targets and `make setup` without Python or Node

**Files:**
- Modify: `Makefile` (`test`, `run-tui`, `run`, `run-native`)
- Modify: `scripts/setup-env.sh` (whole file)

**Interfaces:**
- Consumes: `--model-dir` from Task 2.
- Produces:
  - `make run` and `make run-tui` (both depend on `native-libs`, link `lib`, pass `--model-dir $(MODEL_DIR) $(ARGS)`);
  - `make test` runs only `cargo test --workspace`;
  - `make setup` checks tools, native libs and models only.

- [ ] **Step 1: Makefile**

Replace the `test` recipe body and comment:

```make
## test: run the Rust tests
.PHONY: test
test:
	$(CARGO) test --workspace
```

Delete the old `run-tui`, `run` and `run-native` targets (with their `##` comments). In their place, add:

```make
# Builds package $(1), links native/<arch>/lib next to its debug executable
# (the native libraries load from <exe dir>/lib) and runs binary $(2) on the
# models in MODEL_DIR.
define run_native
	$(CARGO) build -p $(1)
	target_dir=$$($(CARGO) metadata --format-version 1 --no-deps \
		| sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p'); \
	test -n "$$target_dir" || { echo "cargo metadata failed" >&2; exit 1; }; \
	ln -sfn $(NATIVE_LIB_DIR) "$$target_dir/debug/lib" && \
	"$$target_dir/debug/$(2)" --model-dir $(MODEL_DIR) $(ARGS)
endef

## run: run the Slint UI with in-process inference on MODEL_DIR (default ./models)
.PHONY: run
run: native-libs
	$(call run_native,translator-ui,translator)

## run-tui: run the terminal UI with in-process inference (ARGS=--mock for mock services)
.PHONY: run-tui
run-tui: native-libs
	$(call run_native,translator-tui,translator-tui)
```

`MODEL_DIR ?=` is defined further down the Makefile. Recipes expand late, so that order is fine.

- [ ] **Step 2: Rewrite `scripts/setup-env.sh`**

Replace the file with:

```bash
#!/bin/bash
# Checks the system tools this project needs, then fetches what it downloads
# into the repository: the native libraries and the models. Finished items are
# skipped, so it is safe to run again.
# Usage: scripts/setup-env.sh [--yes]   (--yes: download models without asking)
set -o pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT" || exit 1
OS="$(uname -s)"
ARCH="$(uname -m)"
MODEL_DIR="${MODEL_DIR:-$ROOT/models}"
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
        if command -v pkg-config >/dev/null 2>&1; then
            pkg-config --exists alsa || MISSING+=("ALSA development files|sudo apt-get install -y libasound2-dev")
            pkg-config --exists fontconfig || MISSING+=("fontconfig development files|sudo apt-get install -y libfontconfig1-dev")
        fi
        LDCONFIG="$(command -v ldconfig || echo /sbin/ldconfig)"
        LDCONFIG_LIBRARIES="$("$LDCONFIG" -p 2>/dev/null)"
        if ! grep -q 'libvulkan\.so\.1' <<<"$LDCONFIG_LIBRARIES"; then
            MISSING+=("libvulkan.so.1|sudo apt-get install -y libvulkan1")
        fi
        ;;
    *)
        echo "error: unsupported operating system $OS (Linux or macOS only)." >&2
        exit 1
        ;;
esac

need_command cargo "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
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

# Native libraries
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
echo "Ready: make run or make run-tui"
```

- [ ] **Step 3: Verify**

Run: `bash -n scripts/setup-env.sh && echo syntax-ok`
Expected: `syntax-ok`.

Run: `scripts/setup-env.sh < /dev/null; echo "exit=$?"`
Expected: `System tools: ok`; `Native libraries ok`; `Models ok`; `Ready: make run or make run-tui`; `exit=0`.

Run: `make test 2>&1 | tail -3`
Expected: cargo test output only, with no `python3` line, and passing.

Run: `make -n run | tail -2` and `make -n run-tui ARGS=--mock | tail -1`
Expected: the last line runs `.../debug/translator --model-dir <repo>/models`. For run-tui it runs `.../debug/translator-tui --model-dir <repo>/models --mock`.

Run: `timeout 20 make run-tui ARGS=--mock < /dev/null; echo "exit=$?"`
Expected: it builds, then exits with code 2 and the message `translator-tui needs a terminal …`, because stdout is not a TTY. That proves the binary and arguments are wired.

- [ ] **Step 4: Commit**

```bash
git add Makefile scripts/setup-env.sh
git commit -m "Run the native UI from make run and drop Python and Node from make setup"
```

---

### Task 4: Delete the Python backend, React frontend and their scripts

**Files:**
- Delete: `backend/`, `frontend/`, `setup.sh`, `start.sh`, `download_model.sh`, `deploy-pi.sh`, `deploy/gemma-translator.service`

**Interfaces:**
- Consumes: Task 3 removed every reference from `scripts/setup-env.sh` and the Makefile.

- [ ] **Step 1: Delete**

```bash
git rm -q -r backend frontend
git rm -q setup.sh start.sh download_model.sh deploy-pi.sh deploy/gemma-translator.service
```

Leave `.gitignore` unchanged (see "Refinements to the spec").

- [ ] **Step 2: Verify nothing still points at them**

Run: `grep -rn "start\.sh\|setup\.sh\|download_model\|deploy-pi\|server\.py\|backend/\|frontend/" --exclude-dir=target --exclude-dir=docs --exclude-dir=.git --exclude-dir=node_modules --exclude-dir=venv --exclude-dir=native --exclude-dir=models . | grep -v "^./README.md\|^./CLAUDE.md"`
Expected: no output. README and CLAUDE.md are rewritten in Task 8. A code comment such as `moonshine-sys/src/transcriber.rs` "as `server.py` did" is history, not a reference, and may stay. Any other hit must be fixed here.

Run: `cargo test --workspace && make clippy`
Expected: pass.

- [ ] **Step 3: Commit**

```bash
git commit -m "Delete the Python backend, React frontend and their scripts"
```

---

### Task 5: Bookworm cross-compilation image for the Pi

**Files:**
- Create: `deploy/Dockerfile.pi`
- Create: `Cross.toml`

**Interfaces:**
- Produces: `cross build --release -p translator-ui --no-default-features --features kms --target aarch64-unknown-linux-gnu` works on the x86_64 host and writes `target/aarch64-unknown-linux-gnu/release/translator`.

- [ ] **Step 1: Host prerequisites**

The controller has approved these user-environment installs.

```bash
cargo install cross --version 0.2.5 --locked
rustup target add aarch64-unknown-linux-gnu
docker info >/dev/null && echo docker-ok
```

Expected: `docker-ok`. If Docker is not usable by this user, stop and report BLOCKED with the error. Do not change Docker or group settings.

- [ ] **Step 2: Write `deploy/Dockerfile.pi`**

```dockerfile
# Cross-compilation image for the Raspberry Pi 5 (Raspberry Pi OS Bookworm,
# arm64). Debian bookworm has the Pi's glibc (2.36) and libinput (1.22, which
# Slint's kms backend needs); cross 0.2.5's own aarch64 image is too old and
# its development image too new. Used by cross through Cross.toml.
FROM debian:bookworm

RUN dpkg --add-architecture arm64 \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        crossbuild-essential-arm64 \
        pkg-config \
        libasound2-dev:arm64 \
        libdrm-dev:arm64 \
        libfontconfig1-dev:arm64 \
        libgbm-dev:arm64 \
        libinput-dev:arm64 \
        libudev-dev:arm64 \
        libxkbcommon-dev:arm64 \
    && rm -rf /var/lib/apt/lists/*

ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++ \
    PKG_CONFIG_ALLOW_CROSS=1 \
    PKG_CONFIG_LIBDIR=/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig
```

- [ ] **Step 3: Write `Cross.toml`**

```toml
# Build the Pi binary in a Debian bookworm image that matches Raspberry Pi OS
# (see deploy/Dockerfile.pi) instead of cross's default aarch64 image.
[target.aarch64-unknown-linux-gnu]
dockerfile = "deploy/Dockerfile.pi"
```

- [ ] **Step 4: Build**

Run: `cross build --release -p translator-ui --no-default-features --features kms --target aarch64-unknown-linux-gnu 2>&1 | tail -5`
Expected: `Finished release`. The first run builds the image and the whole dependency tree, which takes several minutes, so use a long timeout. If a `*-sys` build script fails with a missing pkg-config package, add the matching `lib…-dev:arm64` package to the Dockerfile list, keeping it alphabetical, and rebuild. List every package you added in the report.

Run: `file target/aarch64-unknown-linux-gnu/release/translator`
Expected: `ELF 64-bit LSB pie executable, ARM aarch64`.

Run: `readelf -d target/aarch64-unknown-linux-gnu/release/translator | grep NEEDED`
Expected: libasound, libinput, libudev, libgbm, libdrm, libxkbcommon, libfontconfig, libc and so on. There must be no `libEGL` or `libGLESv2` line, because EGL is dlopen'd at run time. If one appears, report it as a concern.

- [ ] **Step 5: Commit**

```bash
git add deploy/Dockerfile.pi Cross.toml
git commit -m "Cross-compile the Pi binary in a Debian bookworm image"
```

---

### Task 6: `make package-pi`

**Files:**
- Create: `scripts/check-symbol-versions.sh` (mode 755)
- Create: `scripts/package-pi.sh` (mode 755)
- Modify: `Makefile` (replace `build-pi`; header comment)

**Interfaces:**
- Consumes:
  - Task 5's cross setup;
  - `scripts/fetch-native-libs.sh aarch64`;
  - `$(MODEL_DIR)/.complete`;
  - `deploy/install.sh` and `deploy/gemma-translator.service.in` from Task 7. Task 6 runs before Task 7, so Step 4 creates throwaway stubs for those two paths to test packaging and removes them afterwards. Task 7 writes the real files.
- Produces: `make package-pi`, which writes `dist/gemma-translator-pi.tar`; and `scripts/check-symbol-versions.sh FILE...`, which exits 1 naming the first file over a limit.

- [ ] **Step 1: Write `scripts/check-symbol-versions.sh`**

```bash
#!/bin/bash
# Fails when any given ELF file needs a newer glibc or libstdc++ than
# Raspberry Pi OS Bookworm has (GLIBC_2.36, GLIBCXX_3.4.30).
# Usage: scripts/check-symbol-versions.sh FILE...
set -euo pipefail

LIMITS="GLIBC:2.36 GLIBCXX:3.4.30"

highest_version() { # highest_version <prefix> <file>
    { readelf --dyn-syms --wide "$2" 2>/dev/null || true; } \
        | { grep -oE "@${1}_[0-9]+(\.[0-9]+)*" || true; } \
        | sed "s/^@${1}_//" | sort -V | tail -n 1
}

is_newer() { # is_newer <version> <limit>
    [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n 1)" = "$1" ]
}

if [ "$#" -eq 0 ]; then
    echo "usage: $0 FILE..." >&2
    exit 2
fi
for file in "$@"; do
    for limit in $LIMITS; do
        prefix="${limit%%:*}"
        highest="$(highest_version "$prefix" "$file")"
        if [ -n "$highest" ] && is_newer "$highest" "${limit#*:}"; then
            echo "error: $file needs ${prefix}_$highest; the Pi has ${prefix}_${limit#*:}" >&2
            exit 1
        fi
    done
done
echo "Symbol versions ok: $*"
```

The pattern `@GLIBC_` requires an underscore right after `GLIBC`, so it never matches `GLIBCXX`.

- [ ] **Step 2: Verify the checker**

Run: `chmod +x scripts/check-symbol-versions.sh && scripts/check-symbol-versions.sh "$(ldconfig -p | awk '/libstdc\+\+\.so\.6 .*x86-64/ { print $NF; exit }')"; echo "exit=$?"`
Expected: `error: … needs GLIBCXX_3.4.3x …` (the host libstdc++ is newer than Bookworm's), then `exit=1`.

Run: `scripts/check-symbol-versions.sh target/aarch64-unknown-linux-gnu/release/translator native/aarch64/lib/lib*; echo "exit=$?"`
If `native/aarch64/lib` is missing, run `scripts/fetch-native-libs.sh aarch64` first.
Expected: `Symbol versions ok: …`, `exit=0`.

- [ ] **Step 3: Write `scripts/package-pi.sh`**

```bash
#!/bin/bash
# Builds dist/gemma-translator-pi.tar for a Raspberry Pi 5: the kms translator
# (cross-compiled in deploy/Dockerfile.pi), the aarch64 native libraries, the
# models and the installer. Run it through `make package-pi` on x86_64 Linux.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
PI_TARGET=aarch64-unknown-linux-gnu
MODEL_DIR="${MODEL_DIR:-$ROOT/models}"
LIB_DIR="$ROOT/native/aarch64/lib"
BINARY="$ROOT/target/$PI_TARGET/release/translator"
OUT="$ROOT/dist/gemma-translator-pi.tar"

fail() {
    echo "error: $*" >&2
    exit 1
}

[ "$(uname -s)/$(uname -m)" = Linux/x86_64 ] \
    || fail "make package-pi runs on x86_64 Linux only (this is $(uname -s)/$(uname -m))"
command -v cross >/dev/null || fail "cross is not installed: cargo install cross --version 0.2.5 --locked"
docker info >/dev/null 2>&1 || fail "Docker is not running or not usable by $(id -un)"
rustup target list --installed | grep -qx "$PI_TARGET" || fail "missing Rust target: rustup target add $PI_TARGET"
command -v readelf >/dev/null || fail "readelf is missing: sudo apt-get install -y binutils"
[ -f "$MODEL_DIR/.complete" ] || fail "the models in $MODEL_DIR are not complete; run make models first"

cross build --release -p translator-ui --no-default-features --features kms --target "$PI_TARGET"
[ -f "$LIB_DIR/.complete" ] || "$ROOT/scripts/fetch-native-libs.sh" aarch64
"$ROOT/scripts/check-symbol-versions.sh" "$BINARY" "$LIB_DIR"/lib*

mkdir -p "$ROOT/dist"
STAGE="$(mktemp -d "$ROOT/dist/.stage.XXXXXX")"
trap 'rm -rf "$STAGE" "$OUT.partial"' EXIT
PACKAGE="$STAGE/gemma-translator"
mkdir -p "$PACKAGE/lib"
cp "$BINARY" "$PACKAGE/translator"
cp "$LIB_DIR"/lib* "$PACKAGE/lib/"
cp "$ROOT/deploy/install.sh" "$PACKAGE/install.sh"
cp "$ROOT/deploy/gemma-translator.service.in" "$PACKAGE/gemma-translator.service"
chmod 755 "$PACKAGE/translator" "$PACKAGE/install.sh"
# The models are linked, not copied; tar -h follows the link. litert-lm's
# cache/ is machine-specific, so the Pi rebuilds it on first start.
ln -s "$MODEL_DIR" "$PACKAGE/models"
tar -C "$STAGE" -chf "$OUT.partial" \
    --exclude=gemma-translator/models/cache \
    --exclude=gemma-translator/models/.gitignore \
    gemma-translator
mv "$OUT.partial" "$OUT"
echo "Wrote $OUT ($(du -h "$OUT" | cut -f1)). Copy it to the Pi, then:"
echo "  tar -xf gemma-translator-pi.tar && cd gemma-translator && sudo ./install.sh"
```

- [ ] **Step 4: Makefile, then verify**

Delete the `build-pi` target and its `##` comment, and the `PI_TARGET :=` line. Add:

```make
## package-pi: build dist/gemma-translator-pi.tar for the Raspberry Pi 5 (x86_64 Linux; needs cross 0.2.5 and Docker)
.PHONY: package-pi
package-pi:
	MODEL_DIR=$(MODEL_DIR) scripts/package-pi.sh
```

Change the header's second line to: ``# `make run-mock` works on any Linux or macOS desktop; `package-pi` builds the Raspberry Pi package.``

Create throwaway stubs for Task 7's files. Do not commit them:

```bash
[ -e deploy/install.sh ] || printf '#!/bin/bash\n' > deploy/install.sh
[ -e deploy/gemma-translator.service.in ] || : > deploy/gemma-translator.service.in
```

Run: `bash -n scripts/package-pi.sh && make package-pi MODEL_DIR=/nonexistent; echo "exit=$?"; ls dist/`
Expected: `error: the models in /nonexistent are not complete; run make models first`, a non-zero exit, and no `gemma-translator-pi.tar` or `.partial` in `dist/`.

Run: `make package-pi && tar -tvf dist/gemma-translator-pi.tar | awk '{ print $6 }' | grep -v '^gemma-translator/models/.*/' | sort`
Expected: the file is about 4.7 GB, and the listing holds `gemma-translator/`, `translator`, `install.sh`, `gemma-translator.service`, `lib/` with `libmoonshine.so`, `liblitert-lm.so` and `libonnxruntime-*.so.1`, `models/` with `.complete` and the top-level model folders (`gemma`, `stt`, `tts`). It must have no `models/cache` entry and no `.gitignore`.

Run: `tar -tf dist/gemma-translator-pi.tar | grep -c "models/cache"`
Expected: `0`.

Then delete the stubs if this step created them (`git status --short deploy/` shows them as untracked), and delete `dist/gemma-translator-pi.tar`.

- [ ] **Step 5: Commit**

```bash
git add scripts/check-symbol-versions.sh scripts/package-pi.sh Makefile
git commit -m "Add make package-pi to build the Raspberry Pi package"
```

---

### Task 7: Pi installer and systemd unit

**Files:**
- Create: `deploy/install.sh` (mode 755)
- Create: `deploy/gemma-translator.service.in`

**Interfaces:**
- Consumes: the package layout from Task 6 (`translator`, `lib/`, `models/`, `gemma-translator.service`, `install.sh` side by side).
- Produces: `sudo ./install.sh`, which installs to `/opt/gemma-translator`, `/var/lib/gemma-translator/models` and `/etc/systemd/system/gemma-translator.service`.

- [ ] **Step 1: Write `deploy/gemma-translator.service.in`**

```ini
[Unit]
Description=Gemma Offline Translator
# PipeWire runs in the user's session; install.sh enables linger so it starts at boot.
Wants=user@{{UID}}.service
After=sound.target user@{{UID}}.service getty@tty1.service
# The translator draws on the display through KMS; keep the tty1 login prompt
# off the screen and away from the keyboard while it runs.
Conflicts=getty@tty1.service

[Service]
Type=simple
User={{USER}}
SupplementaryGroups=video input render audio
Environment=XDG_RUNTIME_DIR=/run/user/{{UID}}
ExecStart=/opt/gemma-translator/translator
Restart=always
RestartSec=2

[Install]
WantedBy=multi-user.target
```

- [ ] **Step 2: Write `deploy/install.sh`**

```bash
#!/bin/bash
# Installs the Gemma Translator package on a Raspberry Pi 5 running Raspberry
# Pi OS Bookworm (64-bit, desktop): the translator into /opt/gemma-translator,
# the models into /var/lib/gemma-translator/models, and a systemd service that
# starts it at boot on the console instead of the desktop.
# Usage (from the unpacked package): sudo ./install.sh
set -euo pipefail

PACKAGE_DIR="$(cd "$(dirname "$0")" && pwd)"
INSTALL_DIR=/opt/gemma-translator
DATA_DIR=/var/lib/gemma-translator
MODEL_DIR="$DATA_DIR/models"
SERVICE=gemma-translator.service
SERVICE_FILE="/etc/systemd/system/$SERVICE"
MIN_GLIBC=2.34
RUNTIME_PACKAGES="libegl1 libgles2 libgbm1 libgl1-mesa-dri libvulkan1 mesa-vulkan-drivers libinput10 libxkbcommon0 xkb-data libudev1 libasound2 libfontconfig1 libdrm2 pipewire pipewire-alsa wireplumber alsa-utils fonts-dejavu-core fonts-noto-cjk fonts-noto-core"

fail() {
    echo "error: $*" >&2
    exit 1
}

version_at_least() { # version_at_least <version> <minimum>
    [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | head -n 1)" = "$2" ]
}

package_for_library() { # package_for_library <soname>; fails for unknown names
    case "$1" in
        libasound.so.2) echo libasound2 ;;
        libdrm.so.2) echo libdrm2 ;;
        libfontconfig.so.1) echo libfontconfig1 ;;
        libgbm.so.1) echo libgbm1 ;;
        libgomp.so.1) echo libgomp1 ;;
        libinput.so.10) echo libinput10 ;;
        libstdc++.so.6) echo libstdc++6 ;;
        libudev.so.1) echo libudev1 ;;
        libvulkan.so.1) echo libvulkan1 ;;
        libxkbcommon.so.0) echo libxkbcommon0 ;;
        *) return 1 ;;
    esac
}

copy_models() { # copy_models; used when rsync is missing. Skips same size and mtime.
    (cd "$PACKAGE_DIR/models" && find . -type d) | while read -r dir; do
        mkdir -p "$MODEL_DIR/$dir"
    done
    (cd "$PACKAGE_DIR/models" && find . -type f) | while read -r file; do
        if [ -f "$MODEL_DIR/$file" ] \
            && [ "$(stat -c '%s %Y' "$PACKAGE_DIR/models/$file")" = "$(stat -c '%s %Y' "$MODEL_DIR/$file")" ]; then
            continue
        fi
        cp -p "$PACKAGE_DIR/models/$file" "$MODEL_DIR/$file"
    done
}

# ---- Checks: nothing on the system changes before all of them pass ----------

[ "$(id -u)" -eq 0 ] || fail "run it with sudo: sudo ./install.sh"
SERVICE_USER="${SUDO_USER:-}"
if [ -z "$SERVICE_USER" ] || [ "$SERVICE_USER" = root ]; then
    fail "run it with sudo from the account that should run the translator (SUDO_USER is not set)"
fi
SERVICE_UID="$(id -u "$SERVICE_USER")"
SERVICE_HOME="$(getent passwd "$SERVICE_USER" | cut -d: -f6)"
[ "$(uname -m)" = aarch64 ] \
    || fail "this package is for a 64-bit Raspberry Pi (aarch64); this machine is $(uname -m)"
GLIBC_VERSION="$(ldd --version | head -n 1 | grep -oE '[0-9]+\.[0-9]+$')"
version_at_least "$GLIBC_VERSION" "$MIN_GLIBC" \
    || fail "glibc $GLIBC_VERSION is too old (needs $MIN_GLIBC); use Raspberry Pi OS Bookworm"
for item in translator lib models gemma-translator.service; do
    [ -e "$PACKAGE_DIR/$item" ] || fail "$PACKAGE_DIR/$item is missing; unpack the whole package"
done

MISSING_PACKAGES=()
for package in $RUNTIME_PACKAGES; do
    dpkg -s "$package" >/dev/null 2>&1 || MISSING_PACKAGES+=("$package")
done
UNKNOWN_LIBRARIES=()
while read -r library; do
    if package="$(package_for_library "$library")"; then
        MISSING_PACKAGES+=("$package")
    else
        UNKNOWN_LIBRARIES+=("$library")
    fi
done < <(LD_LIBRARY_PATH="$PACKAGE_DIR/lib" ldd "$PACKAGE_DIR/translator" "$PACKAGE_DIR"/lib/lib* 2>/dev/null \
    | awk '/not found/ { print $1 }' | sort -u)
if [ "${#MISSING_PACKAGES[@]}" -gt 0 ] || [ "${#UNKNOWN_LIBRARIES[@]}" -gt 0 ]; then
    echo "Missing system packages. Install them (this step needs the network), then run install.sh again:" >&2
    if [ "${#MISSING_PACKAGES[@]}" -gt 0 ]; then
        echo "  sudo apt-get install -y $(printf '%s\n' "${MISSING_PACKAGES[@]}" | sort -u | tr '\n' ' ')" >&2
    fi
    if [ "${#UNKNOWN_LIBRARIES[@]}" -gt 0 ]; then
        echo "  libraries with no known package: ${UNKNOWN_LIBRARIES[*]}" >&2
    fi
    exit 1
fi

# ---- Install ------------------------------------------------------------------

echo "[1/7] Stopping the running translator service..."
systemctl stop "$SERVICE" 2>/dev/null || true
rm -rf "$INSTALL_DIR.new" "$INSTALL_DIR.old"

echo "[2/7] Installing the models into $MODEL_DIR (unchanged files are skipped)..."
mkdir -p "$MODEL_DIR"
if command -v rsync >/dev/null; then
    rsync -a "$PACKAGE_DIR/models/" "$MODEL_DIR/"
else
    copy_models
fi
mkdir -p "$MODEL_DIR/cache"
chown -R "$SERVICE_USER:" "$DATA_DIR"

echo "[3/7] Installing the translator into $INSTALL_DIR..."
mkdir -p "$INSTALL_DIR.new/lib"
cp "$PACKAGE_DIR/translator" "$INSTALL_DIR.new/translator"
cp "$PACKAGE_DIR"/lib/lib* "$INSTALL_DIR.new/lib/"
chmod 755 "$INSTALL_DIR.new/translator"
if [ -e "$INSTALL_DIR" ]; then
    mv "$INSTALL_DIR" "$INSTALL_DIR.old"
fi
mv "$INSTALL_DIR.new" "$INSTALL_DIR"
rm -rf "$INSTALL_DIR.old"

echo "[4/7] Writing $SERVICE_FILE..."
sed -e "s|{{USER}}|$SERVICE_USER|g" -e "s|{{UID}}|$SERVICE_UID|g" \
    "$PACKAGE_DIR/gemma-translator.service" > "$SERVICE_FILE"
chmod 644 "$SERVICE_FILE"
sudo -u "$SERVICE_USER" mkdir -p "$SERVICE_HOME/.config/gemma-translator"

echo "[5/7] Starting $SERVICE_USER's session at boot, for PipeWire audio..."
loginctl enable-linger "$SERVICE_USER"

echo "[6/7] Booting to the console instead of the desktop..."
systemctl set-default multi-user.target
AUTOSTART="$SERVICE_HOME/.config/lxsession/rpd-x/autostart"
if [ -f "$AUTOSTART" ]; then
    sed -i '/@chromium/d' "$AUTOSTART"
fi

echo "[7/7] Starting the translator..."
systemctl daemon-reload
systemctl enable "$SERVICE"
systemctl restart "$SERVICE"

echo
echo "Installed. After a reboot the Pi starts straight into the translator;"
echo "the first start takes longer while litert-lm rebuilds its cache."
echo "To boot to the desktop again:"
echo "  sudo systemctl disable $SERVICE && sudo systemctl set-default graphical.target"
```

- [ ] **Step 3: Verify**

Run: `chmod +x deploy/install.sh && bash -n deploy/install.sh && echo syntax-ok`
Expected: `syntax-ok`.

If `shellcheck` is installed, run `shellcheck deploy/install.sh` and fix any warnings. Otherwise note in the report that it was skipped.

Run: `deploy/install.sh; echo "exit=$?"` (not as root)
Expected: `error: run it with sudo: sudo ./install.sh`, `exit=1`.

Run the check functions on their own, without root:

```bash
bash -c 'source <(sed -n "/^version_at_least()/,/^}/p; /^package_for_library()/,/^}/p" deploy/install.sh)
version_at_least 2.36 2.34 && echo ok-newer
version_at_least 2.31 2.34 || echo ok-older
package_for_library libvulkan.so.1
package_for_library libmystery.so.9 || echo ok-unknown'
```

Expected: `ok-newer`, `ok-older`, `libvulkan1`, `ok-unknown`.

Run `make package-pi` again to confirm the real files are packed. Then run `tar -xOf dist/gemma-translator-pi.tar gemma-translator/gemma-translator.service | head -3` and expect `[Unit]`. Delete `dist/gemma-translator-pi.tar` afterwards.

- [ ] **Step 4: Commit**

```bash
git add deploy/install.sh deploy/gemma-translator.service.in
git commit -m "Add the Raspberry Pi installer and systemd unit"
```

---

### Task 8: Documentation

**Files:**
- Modify: `README.md`, `CLAUDE.md`
- Modify: `docs/superpowers/specs/2026-10-09-rust-frontend-design.md` (§11)
- Modify: `docs/superpowers/specs/2026-10-09-tui-frontend-design.md` (success criteria line ~21)

**Interfaces:**
- Consumes: everything above (`make setup`, `make run`, `make run-tui`, `make package-pi`, `sudo ./install.sh`, `--model-dir`).

- [ ] **Step 1: README.md**

Rewrite it for a single Rust program. Keep the project intro, hardware, keyboard shortcut tables, credits and disclaimer, and update them where they mention the browser. The result must contain:
- **Prerequisites:**
  - Rust (rustup);
  - Linux: `make`, `patchelf`, `pkg-config`, `libasound2-dev`, `libfontconfig1-dev`, `libvulkan1`, `unzip`;
  - macOS 15+ on Apple Silicon: the Xcode Command Line Tools.
  - Name `make setup` as the tool that checks these.
- **Desktop (Linux or macOS):** `make setup`, `make run` (Slint window), `make run-tui` (terminal UI), `make run-mock` / `make run-tui ARGS=--mock`. Mention `--model-dir` and that the models live in `./models`.
- **Raspberry Pi 5:**
  - Supported system: Raspberry Pi OS Bookworm, 64-bit, desktop.
  - On an x86_64 Linux machine: `cargo install cross --version 0.2.5 --locked`, `rustup target add aarch64-unknown-linux-gnu`, Docker, then `make models` and `make package-pi`.
  - The tar is about 4.7 GB. Copy it to the Pi, then `tar -xf gemma-translator-pi.tar && cd gemma-translator && sudo ./install.sh`.
  - No network is needed on the Pi.
  - After a reboot it starts straight into the translator, and the first start is slower.
  - How to go back to the desktop: `sudo systemctl disable gemma-translator && sudo systemctl set-default graphical.target`.
  - If install.sh lists missing packages, install them with the printed apt command.
- **Settings:** `~/.config/gemma-translator/config.toml`. Replace every `localStorage` / browser mention in the keyboard sections with "the settings panel (Esc) and `config.toml`".
- **Upgrading from the Python/React version:** delete `venv/` and `frontend/node_modules/` by hand; old HTTP settings in `config.toml` are ignored.
- **Development:** `make test`, `make clippy`, `make test-ffi`, `make test-soak`.

Remove every mention of `setup.sh`, `start.sh`, `download_model.sh`, `deploy-pi.sh`, `server.py`, `npm`, Vite, React, Chromium kiosk, `inference_backend` and the "phase A/phase B" split.

- [ ] **Step 2: CLAUDE.md**

Rewrite **Overview**, **Commands** and **Architecture** for the single Rust program.
- Keep the TUI, native-library, macOS and Git bullets that are still true.
- Delete the three-process table, the Python and React sections, the HTTP backend lines and the `inference_backend` line.
- `make run-native` becomes `make run`. Add `make package-pi` and `sudo ./install.sh` with one line each on what they do.
- Add one bullet: "Pi packaging: `deploy/Dockerfile.pi` (Debian bookworm, arm64 dev packages) via `Cross.toml`; `scripts/check-symbol-versions.sh` caps GLIBC_2.36 / GLIBCXX_3.4.30; `deploy/install.sh` checks a fixed runtime package list before changing anything and enables linger so PipeWire runs without a desktop session."

- [ ] **Step 3: Specs**

- In `docs/superpowers/specs/2026-10-09-rust-frontend-design.md` §11, after phase A step 6 and phase B step 11, add `（已完成，見 docs/superpowers/specs/2026-10-10-native-only-deployment-design.md）`.
- In `docs/superpowers/specs/2026-10-09-tui-frontend-design.md`, change the success-criterion line listing the three modes (`--mock`, `inference_backend = "http"`, `inference_backend = "native"`) to `--mock` and native inference.

- [ ] **Step 4: Verify**

Run: `grep -n "start\.sh\|setup\.sh\|download_model\|deploy-pi\|server\.py\|inference_backend\|localStorage\|run-native\|npm " README.md CLAUDE.md`
Expected: no output, except an upgrade note that names `venv/`/`node_modules`.

Run: `make help`
Expected: the target list shows `setup`, `test`, `run`, `run-tui`, `run-mock`, `native-libs`, `models`, `test-ffi`, `test-soak`, `package-pi`, `fmt` and `clippy`, and no `run-native` or `build-pi`.

- [ ] **Step 5: Commit**

```bash
git add README.md CLAUDE.md docs/superpowers/specs/2026-10-09-rust-frontend-design.md docs/superpowers/specs/2026-10-09-tui-frontend-design.md
git commit -m "Document the native-only setup and the Pi package"
```
