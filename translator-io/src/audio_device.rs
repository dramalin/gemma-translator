use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
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

/// Audio kept after the key is released, so the end of the last word is not
/// cut off when the key goes up a little early.
pub const RECORDING_TAIL: Duration = Duration::from_millis(500);

/// One recording in the device's own format. Converting it for the speech
/// recognizer is left to the caller, so it can happen off the UI thread.
pub struct CapturedAudio {
    interleaved: Vec<f32>,
    sample_rate: u32,
    channels: u16,
    /// Up to the key release; the tail is not counted.
    pub duration: Duration,
    /// Set while the device is still recording the tail.
    tail: Option<Tail>,
}

impl CapturedAudio {
    pub fn new(interleaved: Vec<f32>, sample_rate: u32, channels: u16, duration: Duration) -> Self {
        CapturedAudio {
            interleaved,
            sample_rate,
            channels,
            duration,
            tail: None,
        }
    }

    /// Whether anything was recorded before the key release.
    pub fn has_samples(&self) -> bool {
        // A tail is only kept for a recording that has samples.
        !self.interleaved.is_empty() || self.tail.is_some()
    }

    /// Mono at `STT_SAMPLE_RATE`. With a tail pending this blocks until the
    /// tail is recorded, so call it off the UI thread.
    pub fn into_stt_samples(mut self) -> Vec<f32> {
        if let Some(tail) = self.tail.take() {
            thread::sleep(tail.ends_at.saturating_duration_since(Instant::now()));
            self.interleaved = tail.finish();
        }
        let mono = downmix_to_mono(&self.interleaved, self.channels);
        resample_linear(&mono, self.sample_rate, STT_SAMPLE_RATE)
    }
}

/// The samples of one recording, handed over when its tail ends.
#[derive(Default)]
struct Take(Mutex<Option<Vec<f32>>>);

/// A recording whose tail the device is still capturing.
struct Tail {
    buffer: Arc<Mutex<CaptureBuffer>>,
    take: Arc<Take>,
    ends_at: Instant,
}

impl Tail {
    /// Ends the capture (if `start` has not already) and returns every sample,
    /// tail included.
    fn finish(&self) -> Vec<f32> {
        let mut buffer = lock(&self.buffer);
        if buffer
            .tail
            .as_ref()
            .is_some_and(|take| Arc::ptr_eq(take, &self.take))
        {
            buffer.tail = None;
            buffer.is_capturing = false;
            return std::mem::take(&mut buffer.samples);
        }
        drop(buffer);
        lock(&self.take.0).take().unwrap_or_default()
    }
}

impl Drop for Tail {
    /// A discarded recording stops capturing now instead of at the next `start`.
    fn drop(&mut self) {
        self.finish();
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
    /// The stopped recording whose tail is still going into `samples`. Only
    /// `start` and `Tail` touch it, never the audio callback.
    tail: Option<Arc<Take>>,
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
        // A new recording cuts the previous one's tail short.
        let finished = std::mem::replace(&mut buffer.samples, samples);
        let previous = buffer.tail.take();
        buffer.is_capturing = true;
        buffer.started_at = Some(Instant::now());
        drop(buffer);
        if let Some(take) = previous {
            *lock(&take.0) = Some(finished);
        }
    }

    /// Ends the recording at the key release. The device keeps capturing for
    /// `RECORDING_TAIL`; `CapturedAudio::into_stt_samples` waits for it.
    pub fn stop(&self) -> CapturedAudio {
        let take = Arc::new(Take::default());
        let mut buffer = lock(&self.buffer);
        let duration = buffer
            .started_at
            .take()
            .map(|start| start.elapsed())
            .unwrap_or_default();
        let has_samples = !buffer.samples.is_empty();
        let tail = (buffer.is_capturing && has_samples).then(|| {
            buffer.tail = Some(Arc::clone(&take));
            Tail {
                buffer: Arc::clone(&self.buffer),
                take,
                ends_at: Instant::now() + RECORDING_TAIL,
            }
        });
        if tail.is_none() {
            buffer.is_capturing = false;
        }
        // With a tail the samples stay in the buffer until `Tail::finish`.
        let interleaved = if tail.is_some() {
            Vec::new()
        } else {
            std::mem::take(&mut buffer.samples)
        };
        drop(buffer);
        CapturedAudio {
            interleaved,
            sample_rate: self.sample_rate,
            channels: self.channels,
            duration,
            tail,
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
        let interleaved: Vec<f32> = (0..3_200).flat_map(|_| [0.5, 0.0]).collect();
        let captured = CapturedAudio::new(interleaved, 32_000, 2, Duration::from_millis(100));
        assert!(captured.has_samples());
        let samples = captured.into_stt_samples();
        assert_eq!(samples.len(), 1_600);
        assert!((samples[800] - 0.25).abs() < 1e-5, "{}", samples[800]);
    }

    #[test]
    fn empty_capture_has_no_samples() {
        let captured = CapturedAudio::new(Vec::new(), 48_000, 1, Duration::from_secs(1));
        assert!(!captured.has_samples());
    }
}
