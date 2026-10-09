use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use translator_core::pipeline::AudioClip;

use crate::audio_convert::resample_linear;

/// Where synthesized speech and UI sounds go.
pub trait PlaybackSink: Send + Sync {
    fn enqueue(&self, clip: AudioClip);
    /// Queues `clip` only if `cancel` is still clear, checking the flag under
    /// the same lock `clear` takes. A canceller that sets the flag *before*
    /// calling `clear` therefore never finds the clip queued after its clear.
    fn enqueue_unless_cancelled(&self, clip: AudioClip, cancel: &AtomicBool);
    fn clear(&self);
    fn is_idle(&self) -> bool;
}

/// Mono sample FIFO drained by the output device callback. Clones share the buffer.
#[derive(Clone)]
pub struct PlaybackQueue {
    samples: Arc<Mutex<VecDeque<f32>>>,
    output_rate: u32,
}

impl PlaybackQueue {
    pub fn new(output_rate: u32) -> Self {
        PlaybackQueue {
            samples: Arc::new(Mutex::new(VecDeque::new())),
            output_rate,
        }
    }

    /// Fills an interleaved device buffer; frames past the queued audio are silent.
    pub fn fill(&self, out: &mut [f32], channels: u16) {
        let mut samples = self.lock();
        for frame in out.chunks_mut(usize::from(channels.max(1))) {
            frame.fill(samples.pop_front().unwrap_or(0.0));
        }
    }

    fn lock(&self) -> MutexGuard<'_, VecDeque<f32>> {
        self.samples.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl PlaybackSink for PlaybackQueue {
    fn enqueue(&self, clip: AudioClip) {
        let resampled = resample_linear(&clip.samples, clip.sample_rate, self.output_rate);
        self.lock().extend(resampled);
    }

    fn enqueue_unless_cancelled(&self, clip: AudioClip, cancel: &AtomicBool) {
        let resampled = resample_linear(&clip.samples, clip.sample_rate, self.output_rate);
        let mut samples = self.lock();
        if !cancel.load(Ordering::SeqCst) {
            samples.extend(resampled);
        }
    }

    fn clear(&self) {
        self.lock().clear();
    }

    fn is_idle(&self) -> bool {
        self.lock().is_empty()
    }
}

/// Used when no output device exists: audio is dropped and jobs still finish.
pub struct DiscardingSink;

impl PlaybackSink for DiscardingSink {
    fn enqueue(&self, _clip: AudioClip) {}

    fn enqueue_unless_cancelled(&self, _clip: AudioClip, _cancel: &AtomicBool) {}

    fn clear(&self) {}

    fn is_idle(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_duplicates_mono_into_every_channel_then_pads_with_silence() {
        let queue = PlaybackQueue::new(48_000);
        queue.enqueue(AudioClip {
            samples: vec![0.1, 0.2],
            sample_rate: 48_000,
        });
        let mut out = [9.0; 6];
        queue.fill(&mut out, 2);
        assert_eq!(out, [0.1, 0.1, 0.2, 0.2, 0.0, 0.0]);
        assert!(queue.is_idle());
    }

    #[test]
    fn enqueue_resamples_to_the_output_rate() {
        let queue = PlaybackQueue::new(48_000);
        queue.enqueue(AudioClip {
            samples: vec![0.0; 160],
            sample_rate: 16_000,
        });
        let mut out = vec![1.0; 480];
        queue.fill(&mut out, 1);
        assert!(queue.is_idle());
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn clear_empties_the_queue() {
        let queue = PlaybackQueue::new(48_000);
        queue.enqueue(AudioClip {
            samples: vec![0.5; 10],
            sample_rate: 48_000,
        });
        assert!(!queue.is_idle());
        queue.clear();
        assert!(queue.is_idle());
    }

    #[test]
    fn clones_share_the_same_buffer() {
        let queue = PlaybackQueue::new(48_000);
        let clone = queue.clone();
        clone.enqueue(AudioClip {
            samples: vec![0.5; 4],
            sample_rate: 48_000,
        });
        assert!(!queue.is_idle());
    }

    #[test]
    fn cancelled_clips_are_not_queued() {
        let queue = PlaybackQueue::new(48_000);
        let clip = AudioClip {
            samples: vec![0.5; 10],
            sample_rate: 48_000,
        };
        queue.enqueue_unless_cancelled(clip.clone(), &AtomicBool::new(true));
        assert!(queue.is_idle());
        queue.enqueue_unless_cancelled(clip, &AtomicBool::new(false));
        assert!(!queue.is_idle());
    }

    #[test]
    fn discarding_sink_is_always_idle() {
        let sink = DiscardingSink;
        sink.enqueue(AudioClip {
            samples: vec![0.5; 10],
            sample_rate: 48_000,
        });
        assert!(sink.is_idle());
    }
}
