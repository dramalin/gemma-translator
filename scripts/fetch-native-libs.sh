#!/bin/bash
# Downloads the pinned moonshine-voice and litert-lm-api wheels, verifies their
# sha256, and unpacks the native libraries into native/<arch>/lib/.
# Usage: scripts/fetch-native-libs.sh [x86_64|aarch64]   (default: this machine)
set -euo pipefail

ARCH="${1:-$(uname -m)}"
PYPI=https://files.pythonhosted.org/packages
case "$ARCH" in
    x86_64)
        MOONSHINE_URL="$PYPI/de/b9/f45749837ecb75a7de2ffed261c0ec12333f66e2e7696b15655717a0e28f/moonshine_voice-0.0.65-py3-none-manylinux_2_34_x86_64.whl"
        MOONSHINE_SHA256=946e0a056dd3a13ee8cc28dec84c0f01676743a841bae5e6c52a87483b10241b
        LITERT_URL="$PYPI/36/27/bb0c2e084d59938bc12f960db50e7e8e056337ed286f31468aaa39bc9d9a/litert_lm_api-0.13.1-py3-none-manylinux_2_27_x86_64.whl"
        LITERT_SHA256=b981d55e057be9664f30670d5c1faa6e738ad516a2617dbdac10dc6b5b409cca
        ;;
    aarch64)
        MOONSHINE_URL="$PYPI/c4/b7/fe75106ffb8001ef43da1c4b75968c82f0322d9880dd3efccca1f315b556/moonshine_voice-0.0.65-py3-none-manylinux_2_34_aarch64.whl"
        MOONSHINE_SHA256=e6200c95950f6385a346b220ad046e586221d9a38db30152fa0cd12a809e9e86
        LITERT_URL="$PYPI/12/a7/c23db30dc19ad07a188eeeab48fd8d676554e8647dac6e9dbf62be0f82f1/litert_lm_api-0.13.1-py3-none-manylinux_2_27_aarch64.whl"
        LITERT_SHA256=4092c4f7ad75fb23d83bc5b00a9aa6896a19d75f148515efeef0d079ba22ef74
        ;;
    *)
        echo "usage: $0 [x86_64|aarch64]" >&2
        exit 2
        ;;
esac

for tool in curl sha256sum unzip patchelf; do
    if ! command -v "$tool" >/dev/null; then
        echo "error: '$tool' is required (sudo apt-get install -y $tool)" >&2
        exit 1
    fi
done

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/native/$ARCH/lib"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fetch() {
    local url="$1" sha256="$2" out="$3"
    curl -fL --retry 3 -o "$out" "$url"
    echo "$sha256  $out" | sha256sum -c --quiet -
}

fetch "$MOONSHINE_URL" "$MOONSHINE_SHA256" "$WORK/moonshine.whl"
fetch "$LITERT_URL" "$LITERT_SHA256" "$WORK/litert.whl"

rm -rf "$DEST"
mkdir -p "$DEST"
printf '*\n' > "$ROOT/native/.gitignore"
unzip -q -j "$WORK/moonshine.whl" 'moonshine_voice/libmoonshine.so' \
    'moonshine_voice.libs/libonnxruntime-*.so.1' -d "$DEST"
unzip -q -j "$WORK/litert.whl" 'litert_lm/liblitert-lm.so' -d "$DEST"
# libmoonshine.so looks for onnxruntime in ../moonshine_voice.libs; point it at its own directory.
patchelf --set-rpath '$ORIGIN' "$DEST/libmoonshine.so"
# Written last: make treats the libraries as fetched only once every step worked.
touch "$DEST/.complete"

echo "Native libraries for $ARCH in $DEST:"
ls -1 "$DEST"
