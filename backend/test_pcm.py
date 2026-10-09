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

import json
import unittest

import numpy as np

from pcm import STT_SAMPLE_RATE, decode_pcm16le, encode_pcm16le, parse_tts_request


class DecodePcmTests(unittest.TestCase):
    def test_decodes_little_endian_int16_to_unit_floats(self):
        body = np.array([0, 16384, -32768], dtype="<i2").tobytes()
        samples = decode_pcm16le(body)
        self.assertEqual(samples.dtype, np.float32)
        np.testing.assert_allclose(samples, [0.0, 0.5, -1.0])

    def test_rejects_empty_body(self):
        with self.assertRaises(ValueError):
            decode_pcm16le(b"")

    def test_rejects_odd_length_body(self):
        with self.assertRaises(ValueError):
            decode_pcm16le(b"\x00\x01\x02")


class EncodePcmTests(unittest.TestCase):
    def test_clips_and_scales_to_int16(self):
        encoded = encode_pcm16le([0.0, 0.5, 2.0, -2.0])
        np.testing.assert_array_equal(
            np.frombuffer(encoded, dtype="<i2"), [0, 16383, 32767, -32767]
        )


class ParseTtsRequestTests(unittest.TestCase):
    def test_reads_text_and_language(self):
        body = json.dumps({"text": "hola", "language": "es"}).encode("utf-8")
        self.assertEqual(parse_tts_request(body), ("hola", "es"))

    def test_language_defaults_to_english(self):
        body = json.dumps({"text": "hi"}).encode("utf-8")
        self.assertEqual(parse_tts_request(body), ("hi", "en"))

    def test_rejects_missing_or_blank_text(self):
        for payload in ({}, {"text": "  "}, {"text": 5}):
            with self.subTest(payload=payload), self.assertRaises(ValueError):
                parse_tts_request(json.dumps(payload).encode("utf-8"))

    def test_rejects_non_string_language(self):
        with self.assertRaises(ValueError):
            parse_tts_request(json.dumps({"text": "hi", "language": 1}).encode("utf-8"))

    def test_rejects_invalid_json(self):
        with self.assertRaises(ValueError):
            parse_tts_request(b"{not json")


class ConstantsTests(unittest.TestCase):
    def test_stt_sample_rate_is_16k(self):
        self.assertEqual(STT_SAMPLE_RATE, 16000)


if __name__ == "__main__":
    unittest.main()
