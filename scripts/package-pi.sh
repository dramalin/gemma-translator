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
installed_targets="$(rustup target list --installed)"
grep -qx "$PI_TARGET" <<<"$installed_targets" || fail "missing Rust target: rustup target add $PI_TARGET"
command -v readelf >/dev/null || fail "readelf is missing: sudo apt-get install -y binutils"
[ -f "$MODEL_DIR/.complete" ] || fail "the models in $MODEL_DIR are not complete; run make models first"
MODEL_DIR="$(cd "$MODEL_DIR" && pwd)"
rm -f "$OUT"

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
