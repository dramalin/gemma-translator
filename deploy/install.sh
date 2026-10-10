#!/bin/bash
# Installs the Gemma Translator package on a Raspberry Pi 5 running Raspberry
# Pi OS Bookworm (64-bit, desktop): the translator into /opt/gemma-translator,
# the models into /var/lib/gemma-translator/models, and a systemd service that
# starts it at boot on the console instead of the desktop.
# Usage (from the unpacked package): sudo ./install.sh
set -euo pipefail

PACKAGE_DIR="$(cd "$(dirname "$0")" && pwd -P)"
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
LDD_OUTPUT="$(ldd --version 2>&1 | head -n 1 || true)"
GLIBC_VERSION="$(grep -oE '[0-9]+\.[0-9]+$' <<<"$LDD_OUTPUT" || true)"
[ -n "$GLIBC_VERSION" ] || fail "cannot read the glibc version"
version_at_least "$GLIBC_VERSION" "$MIN_GLIBC" \
    || fail "glibc $GLIBC_VERSION is too old (needs $MIN_GLIBC); use Raspberry Pi OS Bookworm"
for item in translator lib models models/.complete gemma-translator.service; do
    [ -e "$PACKAGE_DIR/$item" ] || fail "$PACKAGE_DIR/$item is missing; the package is incomplete, copy and unpack it again"
done
for protected in "$INSTALL_DIR" "$DATA_DIR"; do
    case "$PACKAGE_DIR/" in
        "$protected"/*) fail "unpack the package somewhere else (e.g. your home directory), not in $PACKAGE_DIR" ;;
    esac
done

MISSING_PACKAGES=()
for package in $RUNTIME_PACKAGES; do
    dpkg -s "$package" >/dev/null 2>&1 || MISSING_PACKAGES+=("$package")
done
UNRESOLVED_LIBRARIES=""
for binary in "$PACKAGE_DIR/translator" "$PACKAGE_DIR"/lib/lib*; do
    LDD_LISTING="$(LD_LIBRARY_PATH="$PACKAGE_DIR/lib" ldd "$binary" 2>&1)" \
        || fail "cannot inspect $binary with ldd"
    UNRESOLVED_LIBRARIES+="$(awk '/not found/ { print $1 }' <<<"$LDD_LISTING")"$'\n'
done
UNKNOWN_LIBRARIES=()
while read -r library; do
    [ -n "$library" ] || continue
    if package="$(package_for_library "$library")"; then
        MISSING_PACKAGES+=("$package")
    else
        UNKNOWN_LIBRARIES+=("$library")
    fi
done < <(sort -u <<<"$UNRESOLVED_LIBRARIES")
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

# The program files land next to the old ones before the swap; the models are
# counted only on a first install, as rsync copies just the changed files.
REQUIRED_KB="$(du -sk "$PACKAGE_DIR/translator" "$PACKAGE_DIR/lib/" | awk '{ total += $1 } END { print total }')" \
    || fail "cannot measure the size of the package in $PACKAGE_DIR"
if [ ! -d "$MODEL_DIR" ]; then
    MODELS_KB="$(du -sk "$PACKAGE_DIR/models/" | awk '{ print $1 }')" \
        || fail "cannot measure the size of $PACKAGE_DIR/models"
    REQUIRED_KB=$((REQUIRED_KB + MODELS_KB))
fi
SPACE_CHECK_DIR="$DATA_DIR"
while [ ! -d "$SPACE_CHECK_DIR" ]; do
    SPACE_CHECK_DIR="$(dirname "$SPACE_CHECK_DIR")"
done
AVAILABLE_KB="$(df -Pk "$SPACE_CHECK_DIR" | awk 'NR == 2 { print $4 }')" \
    || fail "cannot read the free space under $SPACE_CHECK_DIR"
if [ "$AVAILABLE_KB" -lt "$REQUIRED_KB" ]; then
    fail "need about $(((REQUIRED_KB + 1023) / 1024)) MB free under $SPACE_CHECK_DIR, only $((AVAILABLE_KB / 1024)) MB available"
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

echo "[7/7] Enabling the translator service..."
systemctl daemon-reload
systemctl enable "$SERVICE"
echo
if systemctl is-active --quiet graphical.target; then
    # The desktop holds the display, so the translator could not show anything.
    echo "Installed. The desktop is running, so the translator starts after a reboot:"
    echo "  sudo reboot"
else
    systemctl restart "$SERVICE"
    echo "Installed and running. The Pi starts straight into the translator at every boot."
fi
echo "The first start takes longer while litert-lm rebuilds its cache."
echo "To boot to the desktop again:"
echo "  sudo systemctl disable $SERVICE && sudo systemctl set-default graphical.target"
