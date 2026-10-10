#!/usr/bin/env bash
# Records the evaluation corpus with sox, push-to-talk style.
#
#   eval/record.sh <lang> <condition>     one clip per line of eval/sentences/<lang>.txt
#                                         into eval/corpus/<condition>/<lang>/NN.wav
#   eval/record.sh noise <name> [secs]    room or street noise into eval/corpus/noise/<name>.wav
#
# For the clean set (condition "clean"), speak 10-20 cm from the microphone in
# a quiet room; eval/mix-noise.sh adds noise to it later. Other conditions
# ("50cm", "pi-usb") are recorded as they are.
#
# EVAL_DEVICE picks the input instead of the system default, e.g.
#   EVAL_DEVICE="MacBook Pro Microphone" (macOS)  or  EVAL_DEVICE=plughw:1 (Linux ALSA).
# Check it: a Bluetooth headset as default input records at 16 kHz call quality.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
corpus="${EVAL_CORPUS:-$here/corpus}"
command -v sox >/dev/null || { echo "sox not found: install sox" >&2; exit 1; }

input=(-d)
if [[ -n "${EVAL_DEVICE:-}" ]]; then
    if [[ "$(uname)" == Darwin ]]; then input=(-t coreaudio "$EVAL_DEVICE"); else input=(-t alsa "$EVAL_DEVICE"); fi
fi
format=(-c 1 -r 48000 -b 16)

record() { # <file> : Enter starts, Enter stops
    # sox itself in the background (not a function), so $! is its PID.
    sox -q "${input[@]}" "${format[@]}" "$1" &
    local pid=$!
    read -r -s
    # A background job of a script ignores SIGINT; sox closes the file on TERM too.
    kill -TERM "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
}

if [[ "${1:-}" == noise ]]; then
    name="${2:?usage: eval/record.sh noise <name> [seconds]}"
    seconds="${3:-30}"
    mkdir -p "$corpus/noise"
    echo "Recording ${seconds} s of noise into $corpus/noise/$name.wav; stay quiet."
    sox -q "${input[@]}" "${format[@]}" "$corpus/noise/$name.wav" trim 0 "$seconds"
    exit 0
fi

lang="${1:?usage: eval/record.sh <lang> <condition>}"
condition="${2:?usage: eval/record.sh <lang> <condition>}"
sentences="$here/sentences/$lang.txt"
[[ -f "$sentences" ]] || { echo "no $sentences" >&2; exit 1; }
out="$corpus/$condition/$lang"
mkdir -p "$out"

echo "Per sentence: Enter starts recording, Enter stops; s skips, q quits."
echo "Clips that exist are skipped; delete one to record it again."
n=0
while IFS= read -r sentence <&3; do
    n=$((n + 1))
    file="$out/$(printf '%02d' "$n").wav"
    [[ -f "$file" ]] && continue
    while true; do
        printf '\n[%02d] %s\n' "$n" "$sentence"
        read -r -p "Enter to record (s skip, q quit): " key
        case "$key" in
            q) exit 0 ;;
            s) break ;;
        esac
        echo "  recording... Enter to stop"
        record "$file"
        read -r -p "  kept. Enter next, r redo, p play: " key
        case "$key" in
            p) play -q "$file"; read -r -p "  Enter next, r redo: " key ;;
        esac
        [[ "$key" == r ]] || break
        rm -f "$file"
    done
done 3< "$sentences"
echo "Done: $out"
