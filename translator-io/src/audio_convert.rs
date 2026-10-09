/// Averages interleaved frames into one channel.
pub fn downmix_to_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let channels = usize::from(channels.max(1));
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Linear-interpolation resampler, a port of the old `audioHelpers.js::resample`.
/// A zero rate is meaningless and yields no samples.
pub fn resample_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == 0 || to_rate == 0 {
        return Vec::new();
    }
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = f64::from(from_rate) / f64::from(to_rate);
    let output_len = (samples.len() as f64 / ratio).round() as usize;
    let last = samples.len() - 1;
    (0..output_len)
        .map(|i| {
            let position = i as f64 * ratio;
            let index = (position.floor() as usize).min(last);
            let fraction = (position - index as f64) as f32;
            let current = samples[index];
            let next = samples.get(index + 1).copied().unwrap_or(current);
            current + fraction * (next - current)
        })
        .collect()
}

pub fn f32_to_pcm16_le(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|sample| ((sample.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes())
        .collect()
}

/// `None` when the byte count is odd (truncated stream).
pub fn pcm16_le_to_f32(bytes: &[u8]) -> Option<Vec<f32>> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(2)
            .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
            .collect(),
    )
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
        let input: Vec<f32> = (0..48).map(|i| i as f32).collect();
        let output = resample_linear(&input, 48_000, 16_000);
        assert_eq!(output.len(), 16);
        assert_eq!(output[1], 3.0);
        let upsampled = resample_linear(&[0.0, 1.0], 16_000, 32_000);
        assert_eq!(upsampled, [0.0, 0.5, 1.0, 1.0]);
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

    #[test]
    fn pcm16_round_trip() {
        let samples = [0.0, 0.5, -0.5, 1.0, -1.0];
        let decoded = pcm16_le_to_f32(&f32_to_pcm16_le(&samples)).unwrap();
        for (a, b) in samples.iter().zip(&decoded) {
            assert!((a - b).abs() < 1e-4, "{a} vs {b}");
        }
        assert_eq!(f32_to_pcm16_le(&[2.0]), 32767i16.to_le_bytes());
    }

    #[test]
    fn odd_pcm_length_is_rejected() {
        assert_eq!(pcm16_le_to_f32(&[0, 1, 2]), None);
    }
}
