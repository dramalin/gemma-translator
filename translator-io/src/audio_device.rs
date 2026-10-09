use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use translator_core::pipeline::STT_SAMPLE_RATE;

use crate::audio_convert::{downmix_to_mono, resample_linear};
use crate::playback_queue::PlaybackQueue;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no audio {0} device available")]
    NoDevice(&'static str),
    #[error("audio device error: {0}")]
    Device(String),
    #[error("unsupported sample format {0}")]
    UnsupportedFormat(String),
}

fn device_error(error: impl ToString) -> AudioError {
    AudioError::Device(error.to_string())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Output samples (all channels) reserved for the mix buffer up front; larger
/// than the period of common devices.
const OUTPUT_BUFFER_RESERVE: usize = 16_384;

/// Recordings longer than this are cut off, so a stuck key cannot grow the
/// buffer without bound.
const MAX_RECORDING: Duration = Duration::from_secs(60);

/// One recording in the device's own format. Converting it for the speech
/// recognizer is left to the caller, so it can happen off the UI thread.
pub struct CapturedAudio {
    pub interleaved: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration: Duration,
}

impl CapturedAudio {
    pub fn has_samples(&self) -> bool {
        !self.interleaved.is_empty()
    }

    /// Mono at `STT_SAMPLE_RATE`.
    pub fn into_stt_samples(self) -> Vec<f32> {
        let mono = downmix_to_mono(&self.interleaved, self.channels);
        resample_linear(&mono, self.sample_rate, STT_SAMPLE_RATE)
    }
}

/// Shared with the input callback, which only ever pushes into the capacity
/// reserved by `start`, so it never allocates.
#[derive(Default)]
struct CaptureBuffer {
    is_capturing: bool,
    samples: Vec<f32>,
    started_at: Option<Instant>,
    recent_level: f32,
}

/// Keeps the microphone stream open; samples are kept only between `start` and `stop`.
pub struct AudioRecorder {
    _stream: cpal::Stream,
    buffer: Arc<Mutex<CaptureBuffer>>,
    sample_rate: u32,
    channels: u16,
}

impl AudioRecorder {
    pub fn open(device_name: Option<&str>) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = match device_name {
            Some(name) => host
                .input_devices()
                .map_err(device_error)?
                .find(|device| device.name().is_ok_and(|candidate| candidate == name))
                .ok_or(AudioError::NoDevice("input"))?,
            None => host
                .default_input_device()
                .ok_or(AudioError::NoDevice("input"))?,
        };
        let supported = device.default_input_config().map_err(device_error)?;
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let buffer = Arc::new(Mutex::new(CaptureBuffer::default()));
        let stream = match sample_format {
            SampleFormat::F32 => build_input::<f32>(&device, &config, Arc::clone(&buffer)),
            SampleFormat::I16 => build_input::<i16>(&device, &config, Arc::clone(&buffer)),
            SampleFormat::U16 => build_input::<u16>(&device, &config, Arc::clone(&buffer)),
            other => Err(AudioError::UnsupportedFormat(format!("{other:?}"))),
        }?;
        stream.play().map_err(device_error)?;
        Ok(AudioRecorder {
            _stream: stream,
            buffer,
            sample_rate: config.sample_rate.0,
            channels: config.channels,
        })
    }

    pub fn start(&self) {
        let capacity = MAX_RECORDING.as_secs() as usize
            * self.sample_rate as usize
            * usize::from(self.channels.max(1));
        // Allocate before taking the lock the audio callback also takes.
        let samples = Vec::with_capacity(capacity);
        let mut buffer = lock(&self.buffer);
        buffer.samples = samples;
        buffer.is_capturing = true;
        buffer.started_at = Some(Instant::now());
    }

    pub fn stop(&self) -> CapturedAudio {
        let (interleaved, duration) = {
            let mut buffer = lock(&self.buffer);
            buffer.is_capturing = false;
            let duration = buffer
                .started_at
                .take()
                .map(|start| start.elapsed())
                .unwrap_or_default();
            (std::mem::take(&mut buffer.samples), duration)
        };
        CapturedAudio {
            interleaved,
            sample_rate: self.sample_rate,
            channels: self.channels,
            duration,
        }
    }

    /// RMS of the most recent device buffer, for the visualizer.
    pub fn level(&self) -> f32 {
        lock(&self.buffer).recent_level
    }
}

fn build_input<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buffer: Arc<Mutex<CaptureBuffer>>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let mut buffer = lock(&buffer);
                let is_capturing = buffer.is_capturing;
                let mut sum_of_squares = 0.0f32;
                for sample in data {
                    let value = sample.to_sample::<f32>();
                    sum_of_squares += value * value;
                    // Past the reserved capacity the recording is cut off.
                    if is_capturing && buffer.samples.len() < buffer.samples.capacity() {
                        buffer.samples.push(value);
                    }
                }
                buffer.recent_level = if data.is_empty() {
                    0.0
                } else {
                    (sum_of_squares / data.len() as f32).sqrt()
                };
            },
            |error| eprintln!("[audio] input stream error: {error}"),
            None,
        )
        .map_err(device_error)
}

/// Keeps the speaker stream open and plays whatever is pushed into its queue.
pub struct AudioPlayer {
    _stream: cpal::Stream,
    queue: PlaybackQueue,
}

impl AudioPlayer {
    pub fn open(device_name: Option<&str>) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = match device_name {
            Some(name) => host
                .output_devices()
                .map_err(device_error)?
                .find(|device| device.name().is_ok_and(|candidate| candidate == name))
                .ok_or(AudioError::NoDevice("output"))?,
            None => host
                .default_output_device()
                .ok_or(AudioError::NoDevice("output"))?,
        };
        let supported = device.default_output_config().map_err(device_error)?;
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let queue = PlaybackQueue::new(config.sample_rate.0);
        let stream = match sample_format {
            SampleFormat::F32 => build_output::<f32>(&device, &config, queue.clone()),
            SampleFormat::I16 => build_output::<i16>(&device, &config, queue.clone()),
            SampleFormat::U16 => build_output::<u16>(&device, &config, queue.clone()),
            other => Err(AudioError::UnsupportedFormat(format!("{other:?}"))),
        }?;
        stream.play().map_err(device_error)?;
        Ok(AudioPlayer {
            _stream: stream,
            queue,
        })
    }

    pub fn queue(&self) -> PlaybackQueue {
        self.queue.clone()
    }
}

fn build_output<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    queue: PlaybackQueue,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels;
    // Reused across callbacks and allocated here, outside the real-time
    // callback; it grows only if the device asks for a larger buffer.
    let mut mixed: Vec<f32> = Vec::with_capacity(OUTPUT_BUFFER_RESERVE);
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                mixed.resize(data.len(), 0.0);
                queue.fill(&mut mixed, channels);
                for (out, &sample) in data.iter_mut().zip(&mixed) {
                    *out = T::from_sample(sample);
                }
            },
            |error| eprintln!("[audio] output stream error: {error}"),
            None,
        )
        .map_err(device_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_audio_converts_to_mono_at_the_stt_rate() {
        // 32 kHz stereo; the right channel is silent.
        let interleaved: Vec<f32> = (0..64).flat_map(|i| [i as f32, 0.0]).collect();
        let captured = CapturedAudio {
            interleaved,
            sample_rate: 32_000,
            channels: 2,
            duration: Duration::from_millis(2),
        };
        assert!(captured.has_samples());
        let samples = captured.into_stt_samples();
        assert_eq!(samples.len(), 32);
        assert_eq!(samples[1], 1.0);
    }

    #[test]
    fn empty_capture_has_no_samples() {
        let captured = CapturedAudio {
            interleaved: Vec::new(),
            sample_rate: 48_000,
            channels: 1,
            duration: Duration::from_secs(1),
        };
        assert!(!captured.has_samples());
    }
}
