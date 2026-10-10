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
