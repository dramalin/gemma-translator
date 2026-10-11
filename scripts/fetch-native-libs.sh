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
        MOONSHINE_URL="$PYPI/a9/9d/228f738b48e0e7cc1de97c3f842b5470c6a3c5f7a0bffbf13c3d00eefb87/moonshine_voice-0.1.5-py3-none-manylinux_2_34_x86_64.whl"
        MOONSHINE_SHA256=1ed9e0ccf94be4845d69e7fa862dcf4c8b5801a758796966494c694f48184496
        LITERT_URL="$PYPI/36/27/bb0c2e084d59938bc12f960db50e7e8e056337ed286f31468aaa39bc9d9a/litert_lm_api-0.13.1-py3-none-manylinux_2_27_x86_64.whl"
        LITERT_SHA256=b981d55e057be9664f30670d5c1faa6e738ad516a2617dbdac10dc6b5b409cca
        ;;
    Linux/aarch64)
        MOONSHINE_URL="$PYPI/56/b3/ca8c6f3eb47498208e92094d68e3dc2a0ce50213fdf980e212f293872455/moonshine_voice-0.1.5-py3-none-manylinux_2_34_aarch64.whl"
        MOONSHINE_SHA256=fd9fa38ab58c6a8e64eb43b81b0f491138159c7ce45584bb0736ed7efa539766
        LITERT_URL="$PYPI/12/a7/c23db30dc19ad07a188eeeab48fd8d676554e8647dac6e9dbf62be0f82f1/litert_lm_api-0.13.1-py3-none-manylinux_2_27_aarch64.whl"
        LITERT_SHA256=4092c4f7ad75fb23d83bc5b00a9aa6896a19d75f148515efeef0d079ba22ef74
        ;;
    Darwin/arm64)
        MOONSHINE_URL="$PYPI/86/2a/9eca2f000e4990315d18aa360c6eb679417b29252e6307c273bcbef38461/moonshine_voice-0.1.5-py3-none-macosx_15_0_arm64.whl"
        MOONSHINE_SHA256=9866b1956d12fff11923ad42f1140bcb314bf05bf8d8576fb7b3aa819ec40e51
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
