# Speech recognition evaluation

Scores Moonshine through the app's own audio path (downmix, low-pass,
resample, recognizer, transcript cleanup) on a fixed set of recordings, so a
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
```

| Column | Meaning |
|---|---|
| CER/WER | Character error rate for zh/ja (Simplified, punctuation ignored), word error rate otherwise |
| exact | Clips with no error |
| unstable | Clips whose runs gave different transcripts (should be 0) |
| stt ms | Mean recognition time per clip, after a warm-up call |
| RTF | Recognition time / audio length |

Per-clip transcripts go to `corpus/report.tsv`; keep a copy before a change
and diff it afterwards. Results from the Mac and the Pi differ, so compare
runs on the same machine.
