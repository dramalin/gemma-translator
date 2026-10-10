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

import unittest

from volume import adjust_volume, get_volume, parse_osascript_volume, step_percent

OSASCRIPT_GET = ("osascript", "-e", "output volume of (get volume settings)")


class FakeRun:
    """Answers commands by their argument tuple; anything unscripted fails."""

    def __init__(self, outputs):
        self.outputs = outputs
        self.calls = []

    def __call__(self, args, env=None):
        self.calls.append(tuple(args))
        return self.outputs.get(tuple(args))


class ParseTests(unittest.TestCase):
    def test_parses_osascript_volume(self):
        self.assertEqual(parse_osascript_volume("75\n"), 75)
        self.assertEqual(parse_osascript_volume("0"), 0)
        self.assertEqual(parse_osascript_volume("100"), 100)
        self.assertIsNone(parse_osascript_volume("missing value\n"))
        self.assertIsNone(parse_osascript_volume(""))
        self.assertIsNone(parse_osascript_volume("101"))
        self.assertIsNone(parse_osascript_volume("²"))

    def test_step_clamps(self):
        self.assertEqual(step_percent(98, "up"), 100)
        self.assertEqual(step_percent(50, "up"), 55)
        self.assertEqual(step_percent(3, "down"), 0)
        self.assertEqual(step_percent(50, "down"), 45)


class MacTests(unittest.TestCase):
    def test_reads_osascript(self):
        run = FakeRun({OSASCRIPT_GET: "75\n"})
        self.assertEqual(get_volume(platform="darwin", run=run), 75)

    def test_adjust_up_sets_clamped_value(self):
        run = FakeRun({
            OSASCRIPT_GET: "98\n",
            ("osascript", "-e", "set volume output volume 100"): "",
        })
        self.assertTrue(adjust_volume("up", platform="darwin", run=run))
        self.assertIn(("osascript", "-e", "set volume output volume 100"), run.calls)

    def test_macos_unknown_volume_sets_nothing(self):
        run = FakeRun({OSASCRIPT_GET: "missing value\n"})
        self.assertFalse(adjust_volume("up", platform="darwin", run=run))
        self.assertEqual(run.calls, [OSASCRIPT_GET])


class LinuxTests(unittest.TestCase):
    def test_reads_wpctl_first(self):
        run = FakeRun({("wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"): "Volume: 0.75\n"})
        self.assertEqual(get_volume(platform="linux", run=run), 75)

    def test_falls_back_to_pactl_then_amixer(self):
        pactl = FakeRun({("pactl", "get-sink-volume", "@DEFAULT_SINK@"): "front-left: 1 /  75% / 0 dB"})
        self.assertEqual(get_volume(platform="linux", run=pactl), 75)
        amixer = FakeRun({("amixer", "sget", "Master"): "  Mono: Playback 52 [81%] [on]"})
        self.assertEqual(get_volume(platform="linux", run=amixer), 81)

    def test_adjust_tries_wpctl_then_pactl_then_amixer(self):
        run = FakeRun({("amixer", "sset", "Master", "5%-"): ""})
        self.assertTrue(adjust_volume("down", platform="linux", run=run))
        self.assertEqual(run.calls, [
            ("wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"),
            ("pactl", "set-sink-volume", "@DEFAULT_SINK@", "-5%"),
            ("amixer", "sset", "Master", "5%-"),
        ])

    def test_missing_tools(self):
        run = FakeRun({})
        self.assertIsNone(get_volume(platform="linux", run=run))
        self.assertFalse(adjust_volume("up", platform="linux", run=run))


if __name__ == "__main__":
    unittest.main()
