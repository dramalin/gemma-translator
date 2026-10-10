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
