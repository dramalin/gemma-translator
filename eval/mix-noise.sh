#!/usr/bin/env bash
# Mixes every noise file into the clean corpus at fixed SNRs, so distance and
# noisy places are simulated repeatably instead of re-recorded.
#
#   eval/mix-noise.sh [snr...]     default: 20 15 10 5
#
# eval/corpus/clean/<lang>/NN.wav + eval/corpus/noise/<name>.wav
#   -> eval/corpus/<name>-snr<N>/<lang>/NN.wav
# The SNR is the clip's RMS level (silence included) over the noise RMS, so it
# is a little pessimistic for clips with long pauses.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
corpus="${EVAL_CORPUS:-$here/corpus}"
snrs=("$@")
[[ ${#snrs[@]} -gt 0 ]] || snrs=(20 15 10 5)

rms_db() { sox "$1" -n stats 2>&1 | awk '/^RMS lev dB/ {print $4}'; }

shopt -s nullglob
noises=("$corpus"/noise/*.wav)
[[ ${#noises[@]} -gt 0 ]] || { echo "no noise files: run eval/record.sh noise room" >&2; exit 1; }

for noise in "${noises[@]}"; do
    name="$(basename "$noise" .wav)"
    noise_db="$(rms_db "$noise")"
    noise_len="$(soxi -D "$noise")"
    for clip in "$corpus"/clean/*/*.wav; do
        lang="$(basename "$(dirname "$clip")")"
        clip_db="$(rms_db "$clip")"
        clip_len="$(soxi -D "$clip")"
        # A fixed offset per clip, so a rerun mixes the same noise.
        offset="$(awk -v n="$noise_len" -v c="$clip_len" -v s="$(cksum <<<"$clip" | cut -d' ' -f1)" \
            'BEGIN { span = n - c; print (span > 0) ? (s % 1000) / 1000 * span : 0 }')"
        for snr in "${snrs[@]}"; do
            out="$corpus/$name-snr$snr/$lang"
            mkdir -p "$out"
            gain="$(awk -v c="$clip_db" -v n="$noise_db" -v s="$snr" 'BEGIN { print c - s - n }')"
            sox -m -v 1 "$clip" -v 1 "|sox \"$noise\" -p repeat 5 trim $offset $clip_len gain $gain" \
                "$out/$(basename "$clip")" 2>/dev/null
        done
    done
    echo "mixed $name at SNR ${snrs[*]} dB"
done
