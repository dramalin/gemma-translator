# Speech recognition evaluation

Scores Moonshine through the app's own audio path (downmix, low-pass,
resample, silence trim, recognizer, transcript cleanup) on a fixed set of recordings, so a
change can be judged by numbers instead of single runs. The recordings are
personal voice data and stay out of git (`eval/corpus/` is ignored).

## 1. Record a clean set

Speak 10-20 cm from the microphone in a quiet room. Each line of
`sentences/<lang>.txt` becomes one clip; Enter starts and Enter stops, like the
push-to-talk key.

```bash
eval/record.sh zh clean
eval/record.sh en clean
```

The system default input is used unless `EVAL_DEVICE` names one, e.g.
`EVAL_DEVICE="MacBook Pro Microphone" eval/record.sh zh clean` on macOS or
`EVAL_DEVICE=plughw:1` on Linux. Check the default first: a Bluetooth headset
records at 16 kHz call quality.

Record real conditions the same way under another name, for example
`eval/record.sh zh 50cm` or, on the Pi, `eval/record.sh zh pi-usb`.

Without a speaker of the language, a synthetic set still catches gross
regressions (it understates noise sensitivity and accent variety). On macOS,
render each line with a system voice and pad it like a recording:

```bash
n=0; while IFS= read -r line; do n=$((n+1)); f=$(printf %02d $n)
  say -v Paulina --file-format=WAVE --data-format=LEI16@22050 -o /tmp/$f.wav "$line"
  sox /tmp/$f.wav -r 48000 eval/corpus/clean/es/$f.wav pad 0.5 0.5
done < eval/sentences/es.txt     # mkdir -p eval/corpus/clean/es first
```

(`say -v '?'` lists the voices: Kyoko for ja, Majed for ar.)

## 2. Add noise

Record noise where the translator will be used (30 s, nobody talking), then
mix it into the clean set at fixed signal-to-noise ratios:

```bash
eval/record.sh noise room
eval/record.sh noise street      # optional: any number of noise files
eval/mix-noise.sh                # SNR 20 15 10 5 dB -> corpus/<noise>-snr<N>/
```

As a guide, the MacBook Pro microphone measured about 15 dB at 30 cm and
11 dB at 50 cm in a quiet room.

## 3. Score

```bash
make eval-stt                                  # every condition, 3 runs per clip
make eval-stt EVAL_ARGS="--only snr10 --runs 1"
make eval-stt EVAL_ARGS="--trim energy"        # none | energy | vad | adaptive (default, as the app)
```

| Column | Meaning |
|---|---|
| CER/WER | Character error rate for zh/ja (Simplified, punctuation ignored), word error rate otherwise; digits count as their words (100 = 一百 = one hundred) |
| exact | Clips with no error |
| unstable | Clips whose runs gave different transcripts (should be 0) |
| stt ms | Mean time per clip for trimming (including the VAD) and recognition, after a warm-up call |
| RTF | Recognition time / audio length |

Per-clip transcripts go to `corpus/report.tsv`; keep a copy before a change
and diff it afterwards. Results from the Mac and the Pi differ, so compare
runs on the same machine.

Background noise that comes and goes (a fan, traffic, a door) costs far more
than steady noise at the same level: trimming by level cannot tell it from
speech and Moonshine turns it into words. The adaptive trim lets the Silero
VAD overrule the level in that case. A clean recording spoiled by such noise
is still worth keeping: move it to its own condition (e.g. `bursty`), or cut
its noise-only lead-ins into `noise/<name>.wav` for `eval/mix-noise.sh`.
