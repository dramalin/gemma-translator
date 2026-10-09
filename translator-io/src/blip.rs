use std::f32::consts::TAU;

use translator_core::controller::Blip;
use translator_core::pipeline::AudioClip;

pub const BLIP_SAMPLE_RATE: u32 = 48_000;
const SILENT_GAIN: f32 = 0.001;

/// Envelope of one sound, copied from the old `audio-blip.js` oscillator settings.
struct BlipShape {
    start_hz: f32,
    end_hz: f32,
    sweep_seconds: f32,
    peak_gain: f32,
    attack_seconds: f32,
    duration_seconds: f32,
}

fn shape(blip: Blip) -> BlipShape {
    match blip {
        Blip::Speaker => BlipShape {
            start_hz: 600.0,
            end_hz: 800.0,
            sweep_seconds: 0.05,
            peak_gain: 0.08,
            attack_seconds: 0.01,
            duration_seconds: 0.1,
        },
        Blip::Ping => BlipShape {
            start_hz: 880.0,
            end_hz: 880.0,
            sweep_seconds: 0.0,
            peak_gain: 0.1,
            attack_seconds: 0.02,
            duration_seconds: 0.3,
        },
        Blip::Language => BlipShape {
            start_hz: 400.0,
            end_hz: 200.0,
            sweep_seconds: 0.05,
            peak_gain: 0.08,
            attack_seconds: 0.01,
            duration_seconds: 0.1,
        },
    }
}

/// Sine with an exponential pitch sweep, linear attack and exponential decay.
pub fn synthesize_blip(blip: Blip, sample_rate: u32) -> AudioClip {
    let shape = shape(blip);
    let rate = sample_rate as f32;
    let count = (shape.duration_seconds * rate).round() as usize;
    let mut phase = 0.0f32;
    let samples = (0..count)
        .map(|i| {
            let t = i as f32 / rate;
            let frequency = if t < shape.sweep_seconds {
                shape.start_hz * (shape.end_hz / shape.start_hz).powf(t / shape.sweep_seconds)
            } else {
                shape.end_hz
            };
            let gain = if t < shape.attack_seconds {
                shape.peak_gain * t / shape.attack_seconds
            } else {
                let progress =
                    (t - shape.attack_seconds) / (shape.duration_seconds - shape.attack_seconds);
                shape.peak_gain * (SILENT_GAIN / shape.peak_gain).powf(progress)
            };
            let sample = gain * phase.sin();
            phase = (phase + TAU * frequency / rate) % TAU;
            sample
        })
        .collect();
    AudioClip {
        samples,
        sample_rate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blip_lengths_match_the_old_web_audio_sounds() {
        assert_eq!(synthesize_blip(Blip::Speaker, 48_000).samples.len(), 4_800);
        assert_eq!(synthesize_blip(Blip::Language, 48_000).samples.len(), 4_800);
        assert_eq!(synthesize_blip(Blip::Ping, 48_000).samples.len(), 14_400);
    }

    #[test]
    fn blips_stay_quiet_and_start_silent() {
        for blip in [Blip::Speaker, Blip::Language, Blip::Ping] {
            let clip = synthesize_blip(blip, BLIP_SAMPLE_RATE);
            assert_eq!(clip.sample_rate, BLIP_SAMPLE_RATE);
            assert_eq!(clip.samples[0], 0.0);
            assert!(clip.samples.iter().all(|s| s.abs() <= 0.1 + 1e-6));
            assert!(clip.samples.iter().any(|s| s.abs() > 0.01));
        }
    }
}
