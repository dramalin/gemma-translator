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

"""Handler-level tests for the raw-PCM /v2 endpoints, with fake speech engines."""

import http.server
import json
import threading
import unittest
import urllib.error
import urllib.request
from types import SimpleNamespace
from unittest import mock

import numpy as np

import server


class FakeRecognizer:
    def __init__(self):
        self.calls = []

    def transcribe_without_streaming(self, samples, sample_rate):
        self.calls.append((len(samples), sample_rate))
        return SimpleNamespace(lines=[SimpleNamespace(text="hello"), SimpleNamespace(text="world")])


class FakeEngine:
    def synthesize(self, text):
        return [0.0, 0.5, -0.5], 24000


class V2EndpointTests(unittest.TestCase):
    def setUp(self):
        self.recognizer = FakeRecognizer()
        patches = [
            mock.patch.object(server, "get_stt_recognizer", lambda language: self.recognizer),
            mock.patch.object(server, "get_tts_engine", lambda language: FakeEngine()),
        ]
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)
        self.httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), server.ProxyHTTPRequestHandler)
        self.httpd.daemon_threads = True
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        self.addCleanup(self.httpd.server_close)
        self.addCleanup(self.httpd.shutdown)
        self.base_url = f"http://127.0.0.1:{self.httpd.server_address[1]}"

    def post(self, path, body, content_type):
        request = urllib.request.Request(
            self.base_url + path, data=body, headers={"Content-Type": content_type}, method="POST"
        )
        try:
            with urllib.request.urlopen(request, timeout=5) as response:
                return response.status, dict(response.headers), response.read()
        except urllib.error.HTTPError as error:
            return error.code, dict(error.headers), error.read()

    def test_stt_transcribes_raw_pcm(self):
        pcm = np.zeros(1600, dtype="<i2").tobytes()
        status, _, body = self.post("/v2/stt?language=ja", pcm, "application/octet-stream")
        self.assertEqual(status, 200)
        self.assertEqual(json.loads(body), {"text": "hello world"})
        self.assertEqual(self.recognizer.calls, [(1600, 16000)])

    def test_stt_rejects_unknown_language_and_empty_body(self):
        status, _, body = self.post("/v2/stt?language=xx", b"\x00\x00", "application/octet-stream")
        self.assertEqual(status, 400)
        self.assertIn("error", json.loads(body))
        status, _, body = self.post("/v2/stt?language=en", b"", "application/octet-stream")
        self.assertEqual(status, 400)
        self.assertIn("error", json.loads(body))

    def test_tts_returns_pcm_with_its_sample_rate(self):
        body = json.dumps({"text": "hola", "language": "es"}).encode("utf-8")
        status, headers, audio = self.post("/v2/tts", body, "application/json")
        self.assertEqual(status, 200)
        self.assertEqual(headers["X-Sample-Rate"], "24000")
        np.testing.assert_array_equal(np.frombuffer(audio, dtype="<i2"), [0, 16384, -16384])

    def test_tts_rejects_a_bad_request(self):
        status, _, body = self.post("/v2/tts", b"{not json", "application/json")
        self.assertEqual(status, 400)
        self.assertIn("error", json.loads(body))

    def test_only_the_exact_v2_paths_are_routed(self):
        pcm = np.zeros(16, dtype="<i2").tobytes()
        status, _, _ = self.post("/v2/sttx?language=en", pcm, "application/octet-stream")
        self.assertEqual(status, 404)
        self.assertEqual(self.recognizer.calls, [])


if __name__ == "__main__":
    unittest.main()
