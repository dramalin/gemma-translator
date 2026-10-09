# Copyright 2026 Google LLC
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Request/response helpers for the raw-PCM /v2 speech endpoints used by the Rust UI."""

import json

import numpy as np

# Moonshine STT expects 16 kHz mono.
STT_SAMPLE_RATE = 16000


def decode_pcm16le(body):
    """16-bit little-endian PCM bytes -> float32 samples in [-1, 1)."""
    if len(body) == 0:
        raise ValueError("Empty audio body")
    if len(body) % 2:
        raise ValueError("PCM body must have an even number of bytes")
    return np.frombuffer(body, dtype="<i2").astype(np.float32) / 32768.0


def encode_pcm16le(samples):
    """Float samples (clipped to [-1, 1]) -> 16-bit little-endian PCM bytes.

    Rounds half away from zero, exactly like the Rust side's `f32_to_pcm16_le`
    (numpy's own `round` rounds half to even)."""
    # Scale in float32 as Rust does, then add the half in float64, where the
    # sum is exact (in float32 it can round up across a power of two).
    scaled = (np.clip(np.asarray(samples, dtype=np.float32), -1.0, 1.0) * np.float32(32767.0)).astype(np.float64)
    return np.trunc(scaled + np.copysign(0.5, scaled)).astype("<i2").tobytes()


def parse_tts_request(body):
    """JSON body {"text", "language"} -> (text, language). Raises ValueError."""
    data = json.loads(body.decode("utf-8"))
    if not isinstance(data, dict):
        raise ValueError("Request body must be a JSON object")
    text = data.get("text")
    language = data.get("language", "en")
    if not isinstance(text, str) or not text.strip():
        raise ValueError('Missing "text"')
    if not isinstance(language, str):
        raise ValueError('"language" must be a string')
    return text, language
