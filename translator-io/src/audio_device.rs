use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use translator_core::pipeline::STT_SAMPLE_RATE;

use crate::audio_convert::{downmix_to_mono, resample_linear, rms};
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

/// One recording, converted for the speech recognizer.
pub struct CapturedAudio {
    pub samples: Vec<f32>,
    pub duration: Duration,
}

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
        let mut buffer = lock(&self.buffer);
        buffer.samples.clear();
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
        let mono = downmix_to_mono(&interleaved, self.channels);
        CapturedAudio {
            samples: resample_linear(&mono, self.sample_rate, STT_SAMPLE_RATE),
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
                let samples: Vec<f32> = data
                    .iter()
                    .map(|sample| sample.to_sample::<f32>())
                    .collect();
                let mut buffer = lock(&buffer);
                buffer.recent_level = rms(&samples);
                if buffer.is_capturing {
                    buffer.samples.extend_from_slice(&samples);
                }
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
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let mut mixed = vec![0.0f32; data.len()];
                queue.fill(&mut mixed, channels);
                for (out, sample) in data.iter_mut().zip(mixed) {
                    *out = T::from_sample(sample);
                }
            },
            |error| eprintln!("[audio] output stream error: {error}"),
            None,
        )
        .map_err(device_error)
}
