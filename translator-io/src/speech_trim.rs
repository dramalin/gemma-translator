//! Cuts the silence or noise before and after the speech in a recording.
//! Moonshine returns nothing for some English clips that start with a second
//! or more of silence, and continues Chinese clips that end with half a second
//! of it with invented words, so only `KEEP_MS` is left on each side.
//!
//! Speech is found by level (`energy_speech`) or by the Silero voice activity
//! detector (`vad_speech`). The level alone is fooled by noise that comes and
//! goes, the detector alone sometimes cuts off a quiet first syllable;
//! `TrimMode::Adaptive` uses the level unless the detector shows that a
//! stretch the level counted as speech is not.

use std::ops::Range;

/// Frame length for the level measurements, in ms.
const FRAME_MS: u32 = 20;
/// Silence kept before and after the speech, in ms. On the eval corpus
/// 200-300 ms scored best; at 500 ms Chinese clips again gained invented
/// endings.
const KEEP_MS: u32 = 300;
/// Loud frames must be this far (dB) above the noise floor to trim at all.
const MIN_RANGE_DB: f32 = 12.0;
/// Speech starts this many loud frames (or detector chunks) in a row, so a
/// key click does not count.
const MIN_RUN: usize = 3;

/// Samples per Silero VAD chunk at 16 kHz.
pub const VAD_CHUNK: usize = 512;
/// Speech probability a detector chunk needs. The detector gives fan and
/// traffic noise 0.6-0.9 and speech above 0.95.
const VAD_THRESHOLD: f32 = 0.95;
/// `Adaptive` switches to the detector's bounds when they cut this much
/// (ms) more at either end than the level's.
const ADAPTIVE_GAP_MS: u32 = 500;

/// How a recording is trimmed before speech recognition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimMode {
    /// Not at all.
    None,
    /// To where the level is clearly above the noise floor.
    Energy,
    /// To where the voice activity detector hears speech.
    Vad,
    /// `Energy`, or `Vad` when its speech is shorter by `ADAPTIVE_GAP_MS` at
    /// either end. On the eval corpus (two voices, steady and intermittent
    /// noise) this beat both: 2.7% errors against 3.0% (`Energy`) and 2.9%
    /// (`Vad`), mostly on clips with intermittent noise.
    Adaptive,
}

impl TrimMode {
    pub fn parse(name: &str) -> Option<TrimMode> {
        match name {
            "none" => Some(TrimMode::None),
            "energy" => Some(TrimMode::Energy),
            "vad" => Some(TrimMode::Vad),
            "adaptive" => Some(TrimMode::Adaptive),
            _ => None,
        }
    }

    /// Whether this mode runs the voice activity detector.
    pub fn uses_vad(self) -> bool {
        matches!(self, TrimMode::Vad | TrimMode::Adaptive)
    }
}

/// Trims `samples` to the speech `mode` finds, plus `KEEP_MS` each side.
/// `vad` holds the detector's speech probability per `VAD_CHUNK` samples;
/// without it the VAD modes fall back to `Energy`. Audio in which no speech
/// is found is returned whole.
pub fn trim_speech(
    mut samples: Vec<f32>,
    sample_rate: u32,
    mode: TrimMode,
    vad: Option<&[f32]>,
) -> Vec<f32> {
    let speech = match (mode, vad) {
        (TrimMode::None, _) => None,
        (TrimMode::Energy, _) | (_, None) => energy_speech(&samples, sample_rate),
        (TrimMode::Vad, Some(vad)) => vad_speech(vad, samples.len()),
        (TrimMode::Adaptive, Some(vad)) => adaptive_speech(
            energy_speech(&samples, sample_rate),
            vad_speech(vad, samples.len()),
            samples.len(),
            sample_rate,
        ),
    };
    let Some(speech) = speech else {
        return samples;
    };
    let keep = (sample_rate * KEEP_MS / 1000) as usize;
    let end = (speech.end + keep).min(samples.len());
    samples.truncate(end);
    samples.drain(..speech.start.saturating_sub(keep));
    samples
}

/// Samples from the first to the last run of `MIN_RUN` frames louder than
/// halfway between the noise floor (10th percentile of frame levels) and
/// the speech level (95th). `None` when those are closer than `MIN_RANGE_DB`
/// or the clip is too short to trim.
pub fn energy_speech(samples: &[f32], sample_rate: u32) -> Option<Range<usize>> {
    let frame = (sample_rate * FRAME_MS / 1000) as usize;
    let keep = (sample_rate * KEEP_MS / 1000) as usize;
    if frame == 0 || samples.len() <= 2 * keep {
        return None;
    }
    let levels: Vec<f32> = samples
        .chunks(frame)
        .map(|chunk| {
            let power = chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32;
            10.0 * power.max(1e-12).log10()
        })
        .collect();
    let mut sorted = levels.clone();
    sorted.sort_by(f32::total_cmp);
    let percentile = |p: usize| sorted[(sorted.len() - 1) * p / 100];
    let (noise, speech) = (percentile(10), percentile(95));
    if speech - noise < MIN_RANGE_DB {
        return None;
    }
    let threshold = (noise + speech) / 2.0;
    let (first, last) = runs(&levels, |level| level > threshold)?;
    Some(first * frame..((last + MIN_RUN) * frame).min(samples.len()))
}

/// Samples from the first to the last run of `MIN_RUN` detector chunks at or
/// above `VAD_THRESHOLD`, given one probability per `VAD_CHUNK` samples.
pub fn vad_speech(probabilities: &[f32], sample_count: usize) -> Option<Range<usize>> {
    let (first, last) = runs(probabilities, |p| p >= VAD_THRESHOLD)?;
    let start = (first * VAD_CHUNK).min(sample_count);
    Some(start..((last + MIN_RUN) * VAD_CHUNK).min(sample_count))
}

/// The first and last index starting `MIN_RUN` values in a row that pass.
fn runs(values: &[f32], passes: impl Fn(f32) -> bool) -> Option<(usize, usize)> {
    let run = |window: &[f32]| window.iter().all(|&value| passes(value));
    Some((
        values.windows(MIN_RUN).position(run)?,
        values.windows(MIN_RUN).rposition(run)?,
    ))
}

/// The detector's speech when it starts `ADAPTIVE_GAP_MS` later or ends that
/// much earlier than the level's (the whole clip when the level found none),
/// otherwise the level's.
fn adaptive_speech(
    energy: Option<Range<usize>>,
    vad: Option<Range<usize>>,
    sample_count: usize,
    sample_rate: u32,
) -> Option<Range<usize>> {
    let Some(vad) = vad else {
        return energy;
    };
    let gap = (sample_rate * ADAPTIVE_GAP_MS / 1000) as usize;
    let level = energy.clone().unwrap_or(0..sample_count);
    if vad.start > level.start + gap || level.end > vad.end + gap {
        Some(vad)
    } else {
        energy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;

    /// Deterministic white noise of the given amplitude.
    fn noise(amplitude: f32, count: usize) -> Vec<f32> {
        let mut state = 1_u32;
        (0..count)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                amplitude * ((state >> 8) as f32 / (1 << 23) as f32 - 1.0)
            })
            .collect()
    }

    /// `before` s of noise, `speech` s of a 300 Hz tone at `level`, `after` s of noise.
    fn clip(before: f32, speech: f32, after: f32, level: f32) -> Vec<f32> {
        let rate = RATE as f32;
        let mut samples = vec![0.0; (before * rate) as usize];
        samples.extend(
            (0..(speech * rate) as usize)
                .map(|i| level * (2.0 * std::f32::consts::PI * 300.0 * i as f32 / rate).sin()),
        );
        samples.extend(vec![0.0; (after * rate) as usize]);
        let background = noise(0.001, samples.len());
        samples
            .iter_mut()
            .zip(background)
            .for_each(|(s, n)| *s += n);
        samples
    }

    /// 1 s of speech plus 0.3 s each side, to within a frame.
    fn assert_kept_one_second(samples: &[f32]) {
        let expected = 16_000 + 2 * 4_800;
        assert!(samples.len().abs_diff(expected) <= 640, "{}", samples.len());
    }

    #[test]
    fn energy_keeps_a_little_silence_around_speech() {
        let trimmed = trim_speech(clip(1.5, 1.0, 1.0, 0.1), RATE, TrimMode::Energy, None);
        assert_kept_one_second(&trimmed);
    }

    #[test]
    fn energy_ignores_a_click_in_the_silence() {
        let mut samples = clip(1.5, 1.0, 1.0, 0.1);
        samples[3_200..3_360].fill(0.5);
        assert_kept_one_second(&trim_speech(samples, RATE, TrimMode::Energy, None));
    }

    #[test]
    fn energy_leaves_audio_alone_when_speech_is_not_clearly_louder() {
        let quiet = clip(1.5, 1.0, 1.0, 0.002);
        assert_eq!(
            trim_speech(quiet.clone(), RATE, TrimMode::Energy, None),
            quiet
        );
        let silent = vec![0.0; 48_000];
        assert_eq!(
            trim_speech(silent.clone(), RATE, TrimMode::Energy, None),
            silent
        );
        assert_eq!(
            trim_speech(vec![0.5; 100], RATE, TrimMode::Energy, None),
            [0.5; 100]
        );
    }

    #[test]
    fn none_keeps_everything() {
        let samples = clip(1.5, 1.0, 1.0, 0.1);
        assert_eq!(
            trim_speech(samples.clone(), RATE, TrimMode::None, Some(&[1.0; 110])),
            samples
        );
    }

    /// Detector output for `chunks` chunks, speech in `speech`.
    fn probabilities(chunks: usize, speech: Range<usize>, background: f32) -> Vec<f32> {
        (0..chunks)
            .map(|i| {
                if speech.contains(&i) {
                    0.99
                } else {
                    background
                }
            })
            .collect()
    }

    #[test]
    fn vad_trims_to_the_chunks_it_hears_speech_in() {
        let range = vad_speech(&probabilities(100, 40..70, 0.1), 51_200).unwrap();
        assert_eq!(range, 40 * VAD_CHUNK..70 * VAD_CHUNK);
        // Background the detector is unsure about is not speech.
        assert_eq!(
            vad_speech(&probabilities(100, 40..70, 0.9), 51_200),
            Some(range)
        );
        // Two loud chunks are a click.
        assert_eq!(vad_speech(&probabilities(100, 40..42, 0.1), 51_200), None);
    }

    #[test]
    fn without_detector_output_vad_modes_fall_back_to_energy() {
        let samples = clip(1.5, 1.0, 1.0, 0.1);
        for mode in [TrimMode::Vad, TrimMode::Adaptive] {
            assert_kept_one_second(&trim_speech(samples.clone(), RATE, mode, None));
        }
    }

    #[test]
    fn adaptive_follows_the_detector_only_when_it_cuts_much_more() {
        let seconds = |s: f32| (s * RATE as f32) as usize;
        let level = Some(seconds(1.0)..seconds(3.0));
        // Close to the level's bounds: keep the level's.
        let near = Some(seconds(1.3)..seconds(2.9));
        assert_eq!(
            adaptive_speech(level.clone(), near, seconds(4.0), RATE),
            level
        );
        // Starts 0.8 s later: the level took noise for speech.
        let later = Some(seconds(1.8)..seconds(3.0));
        assert_eq!(
            adaptive_speech(level.clone(), later.clone(), seconds(4.0), RATE),
            later
        );
        // Ends 0.6 s earlier.
        let earlier = Some(seconds(1.0)..seconds(2.4));
        assert_eq!(
            adaptive_speech(level.clone(), earlier.clone(), seconds(4.0), RATE),
            earlier
        );
        // The level found nothing: the detector is compared with the whole clip.
        let middle = Some(seconds(1.5)..seconds(2.5));
        assert_eq!(
            adaptive_speech(None, middle.clone(), seconds(4.0), RATE),
            middle
        );
        assert_eq!(
            adaptive_speech(None, Some(0..seconds(3.8)), seconds(4.0), RATE),
            None
        );
        // The detector found nothing.
        assert_eq!(
            adaptive_speech(level.clone(), None, seconds(4.0), RATE),
            level
        );
    }
}
