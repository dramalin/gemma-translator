/// Averages interleaved frames into one channel.
pub fn downmix_to_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let channels = usize::from(channels.max(1));
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Linear-interpolation resampler.
/// When downsampling, the input is low-pass filtered first so content above
/// the new Nyquist frequency does not alias into the band.
/// A zero rate is meaningless and yields no samples.
pub fn resample_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == 0 || to_rate == 0 {
        return Vec::new();
    }
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = f64::from(from_rate) / f64::from(to_rate);
    let low_pass = (from_rate > to_rate).then(|| LowPass::new(from_rate, to_rate));
    let at = |index: usize| match &low_pass {
        Some(filter) => filter.apply(samples, index),
        None => samples[index],
    };
    let output_len = (samples.len() as f64 / ratio).round() as usize;
    let last = samples.len() - 1;
    (0..output_len)
        .map(|i| {
            let position = i as f64 * ratio;
            let index = (position.floor() as usize).min(last);
            let fraction = (position - index as f64) as f32;
            let current = at(index);
            if fraction == 0.0 || index == last {
                return current;
            }
            current + fraction * (at(index + 1) - current)
        })
        .collect()
}

/// Cutoff as a fraction of the output rate: just under its Nyquist (0.5), so
/// the transition band ends near the Nyquist frequency.
const LOW_PASS_CUTOFF: f64 = 0.47;
/// Transition band width as a fraction of the output rate.
const LOW_PASS_TRANSITION: f64 = 0.06;

/// Blackman-windowed sinc FIR, evaluated only at the input samples the
/// resampler reads. Samples past either end count as zero.
struct LowPass {
    taps: Vec<f32>,
}

impl LowPass {
    fn new(from_rate: u32, to_rate: u32) -> Self {
        let ratio = f64::from(from_rate) / f64::from(to_rate);
        // A Blackman window's transition band is about 5.5 / taps of the rate.
        let half = (5.5 * ratio / LOW_PASS_TRANSITION / 2.0).ceil() as usize;
        let len = 2 * half + 1;
        let cutoff = LOW_PASS_CUTOFF / ratio;
        let mut taps: Vec<f64> = (0..len)
            .map(|n| {
                let t = n as f64 - half as f64;
                let sinc = if t == 0.0 {
                    2.0 * cutoff
                } else {
                    (2.0 * std::f64::consts::PI * cutoff * t).sin() / (std::f64::consts::PI * t)
                };
                let phase = 2.0 * std::f64::consts::PI * n as f64 / (len - 1) as f64;
                sinc * (0.42 - 0.5 * phase.cos() + 0.08 * (2.0 * phase).cos())
            })
            .collect();
        // Unity gain at DC.
        let sum: f64 = taps.iter().sum();
        taps.iter_mut().for_each(|tap| *tap /= sum);
        LowPass {
            taps: taps.into_iter().map(|tap| tap as f32).collect(),
        }
    }

    fn apply(&self, samples: &[f32], index: usize) -> f32 {
        let half = self.taps.len() / 2;
        let first = index.saturating_sub(half);
        let last = (index + half).min(samples.len() - 1);
        samples[first..=last]
            .iter()
            .zip(&self.taps[first + half - index..])
            .map(|(sample, tap)| sample * tap)
            .sum()
    }
}

/// Frame length for the level measurements in `trim_silence`, in ms.
const TRIM_FRAME_MS: u32 = 20;
/// Silence kept before the first and after the last speech frame, in ms.
/// On the eval corpus 200-300 ms scored best; at 500 ms Chinese clips again
/// gained invented endings.
const TRIM_KEEP_MS: u32 = 300;
/// Loud frames must be this far (dB) above the noise floor to trim at all.
const TRIM_MIN_RANGE_DB: f32 = 12.0;
/// Speech starts this many loud frames in a row, so a key click does not count.
const TRIM_MIN_RUN: usize = 3;

/// Cuts leading and trailing silence down to `TRIM_KEEP_MS`.
/// Moonshine returns nothing for some English clips that start with a second
/// or more of silence, and continues Chinese clips that end with half a second
/// of it with invented words. Speech is where frames are louder than halfway between the noise
/// floor (10th percentile of frame levels) and the speech level (95th); when
/// those are closer than `TRIM_MIN_RANGE_DB` the audio is returned as it is.
pub fn trim_silence(mut samples: Vec<f32>, sample_rate: u32) -> Vec<f32> {
    let frame = (sample_rate * TRIM_FRAME_MS / 1000) as usize;
    let keep = (sample_rate * TRIM_KEEP_MS / 1000) as usize;
    if frame == 0 || samples.len() <= 2 * keep {
        return samples;
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
    if speech - noise < TRIM_MIN_RANGE_DB {
        return samples;
    }
    let threshold = (noise + speech) / 2.0;
    let loud = |run: &[f32]| run.iter().all(|&level| level > threshold);
    let (Some(first), Some(last)) = (
        levels.windows(TRIM_MIN_RUN).position(loud),
        levels.windows(TRIM_MIN_RUN).rposition(loud),
    ) else {
        return samples;
    };
    let start = (first * frame).saturating_sub(keep);
    let end = ((last + TRIM_MIN_RUN) * frame + keep).min(samples.len());
    samples.truncate(end);
    samples.drain(..start);
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_averages_channels() {
        assert_eq!(downmix_to_mono(&[1.0, 0.0, 0.5, 0.5], 2), [0.5, 0.5]);
        assert_eq!(downmix_to_mono(&[0.25, -0.25], 1), [0.25, -0.25]);
    }

    #[test]
    fn resample_keeps_same_rate_and_interpolates() {
        assert_eq!(resample_linear(&[0.1, 0.2], 16_000, 16_000), [0.1, 0.2]);
        let input = vec![0.25; 4_800];
        let output = resample_linear(&input, 48_000, 16_000);
        assert_eq!(output.len(), 1_600);
        // The filter has unity gain at DC; away from the zero-padded ends a
        // constant comes through unchanged.
        assert!((output[800] - 0.25).abs() < 1e-5, "{}", output[800]);
        let upsampled = resample_linear(&[0.0, 1.0], 16_000, 32_000);
        assert_eq!(upsampled, [0.0, 0.5, 1.0, 1.0]);
    }

    fn sine(frequency: f64, rate: u32, count: usize) -> Vec<f32> {
        (0..count)
            .map(|i| {
                (2.0 * std::f64::consts::PI * frequency * i as f64 / f64::from(rate)).sin() as f32
            })
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn downsampling_keeps_speech_and_removes_what_would_alias() {
        let rate = 48_000;
        // Skip the edges, where the filter sees zero padding.
        let middle = |samples: &[f32]| rms(&samples[400..samples.len() - 400]);
        let speech = resample_linear(&sine(1_000.0, rate, 48_000), rate, 16_000);
        assert!(
            (middle(&speech) - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01,
            "{}",
            middle(&speech)
        );
        // 12 kHz would fold to 4 kHz at 16 kHz.
        let high = resample_linear(&sine(12_000.0, rate, 48_000), rate, 16_000);
        assert!(middle(&high) < 0.001, "{}", middle(&high));
        let near_nyquist = resample_linear(&sine(8_500.0, rate, 48_000), rate, 16_000);
        assert!(middle(&near_nyquist) < 0.01, "{}", middle(&near_nyquist));
    }

    #[test]
    fn resample_with_a_zero_rate_is_empty() {
        assert!(resample_linear(&[0.5, 0.25], 0, 16_000).is_empty());
        assert!(resample_linear(&[0.5, 0.25], 48_000, 0).is_empty());
    }

    #[test]
    fn resample_of_empty_input_is_empty() {
        assert!(resample_linear(&[], 48_000, 16_000).is_empty());
    }

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
        let rate = 16_000.0;
        let mut samples = noise(0.001, (before * rate) as usize);
        let tone = sine(300.0, 16_000, (speech * rate) as usize);
        samples.extend(tone.iter().map(|s| s * level));
        samples.extend(noise(0.001, (after * rate) as usize));
        let background = noise(0.001, samples.len());
        samples
            .iter_mut()
            .zip(background)
            .for_each(|(s, n)| *s += n);
        samples
    }

    #[test]
    fn trim_keeps_a_little_silence_around_speech() {
        let trimmed = trim_silence(clip(1.5, 1.0, 1.0, 0.1), 16_000);
        // 1 s of speech plus 0.3 s each side, to within a frame.
        let expected = 16_000 + 2 * 4_800;
        assert!(trimmed.len().abs_diff(expected) <= 640, "{}", trimmed.len());
    }

    #[test]
    fn trim_ignores_a_click_in_the_silence() {
        let mut samples = clip(1.5, 1.0, 1.0, 0.1);
        samples[3_200..3_360].fill(0.5);
        let trimmed = trim_silence(samples, 16_000);
        assert!(
            trimmed.len().abs_diff(16_000 + 2 * 4_800) <= 640,
            "{}",
            trimmed.len()
        );
    }

    #[test]
    fn trim_leaves_audio_alone_when_speech_is_not_clearly_louder() {
        let quiet = clip(1.5, 1.0, 1.0, 0.002);
        assert_eq!(trim_silence(quiet.clone(), 16_000), quiet);
        let silent = vec![0.0; 48_000];
        assert_eq!(trim_silence(silent.clone(), 16_000), silent);
        assert_eq!(trim_silence(vec![0.5; 100], 16_000), [0.5; 100]);
    }
}
