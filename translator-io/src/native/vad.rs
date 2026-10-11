//! The Silero voice activity detector (v5) on the ONNX Runtime that ships
//! with libmoonshine. Moonshine has the same model built in, but keeps its
//! state across every call in the process, so the same recording got a
//! different segmentation each time; here every recording starts fresh.

use std::path::Path;
use std::rc::Rc;

use onnxruntime_sys::{Input, OrtError, OrtLibrary, Session};
use translator_core::pipeline::STT_SAMPLE_RATE;

use crate::speech_trim::VAD_CHUNK;

/// Samples of the previous chunk the model sees in front of each chunk.
const CONTEXT: usize = 64;
/// The recurrent state: 2 x 1 x 128 floats.
const STATE_SHAPE: [i64; 3] = [2, 1, 128];
const STATE_LEN: usize = 2 * 128;

/// A loaded detector. Not `Send`: it lives on the speech thread.
pub struct SileroVad {
    session: Session,
}

impl SileroVad {
    pub fn load(library: &Rc<OrtLibrary>, model: &Path) -> Result<SileroVad, OrtError> {
        Ok(SileroVad {
            session: Session::load(library, model)?,
        })
    }

    /// Speech probability for each `VAD_CHUNK` samples of 16 kHz audio,
    /// starting from silence. A last partial chunk is padded with zeros.
    pub fn probabilities(&mut self, samples: &[f32]) -> Result<Vec<f32>, OrtError> {
        let sample_rate = [i64::from(STT_SAMPLE_RATE)];
        let mut state = vec![0.0f32; STATE_LEN];
        let mut input = vec![0.0f32; CONTEXT + VAD_CHUNK];
        let mut probabilities = Vec::with_capacity(samples.len().div_ceil(VAD_CHUNK));
        for chunk in samples.chunks(VAD_CHUNK) {
            // The tail of the previous chunk stays in front as context.
            input.copy_within(VAD_CHUNK.., 0);
            input[CONTEXT..CONTEXT + chunk.len()].copy_from_slice(chunk);
            input[CONTEXT + chunk.len()..].fill(0.0);
            let outputs = self.session.run(
                &[
                    (
                        c"input",
                        Input::F32 {
                            data: &input,
                            shape: &[1, (CONTEXT + VAD_CHUNK) as i64],
                        },
                    ),
                    (
                        c"state",
                        Input::F32 {
                            data: &state,
                            shape: &STATE_SHAPE,
                        },
                    ),
                    (
                        c"sr",
                        Input::I64 {
                            data: &sample_rate,
                            shape: &[],
                        },
                    ),
                ],
                &[c"output", c"stateN"],
            )?;
            let [probability, next_state] = <[Vec<f32>; 2]>::try_from(outputs)
                .map_err(|_| OrtError::Api("Silero VAD: expected two outputs".to_string()))?;
            if probability.len() != 1 || next_state.len() != STATE_LEN {
                return Err(OrtError::Api(format!(
                    "Silero VAD: output sizes {} and {}",
                    probability.len(),
                    next_state.len()
                )));
            }
            probabilities.push(probability[0]);
            state = next_state;
        }
        Ok(probabilities)
    }
}
