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
}
