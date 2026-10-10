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

"""System output volume: osascript on macOS; wpctl, pactl, then amixer elsewhere."""

import os
import re
import subprocess
import sys

STEP_PERCENT = 5
COMMAND_TIMEOUT_SECONDS = 2
MACOS_GET_VOLUME_SCRIPT = "output volume of (get volume settings)"


def run_command(args, env=None):
    """Returns the program's stdout, or None when it is missing, fails or times out."""
    try:
        return subprocess.run(
            args, check=True, capture_output=True, text=True,
            timeout=COMMAND_TIMEOUT_SECONDS, env=env,
        ).stdout
    except (OSError, subprocess.SubprocessError):
        return None


def linux_env():
    """PipeWire's wpctl finds its socket through XDG_RUNTIME_DIR, which systemd services may lack."""
    env = os.environ.copy()
    env.setdefault("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}")
    return env


def parse_osascript_volume(output):
    """"75\\n" -> 75; "missing value" (no output device) -> None."""
    text = output.strip()
    if not (text.isascii() and text.isdigit()):
        return None
    percent = int(text)
    return percent if percent <= 100 else None


def step_percent(current, direction):
    """One 5 % step up or down, kept within 0-100."""
    delta = STEP_PERCENT if direction == "up" else -STEP_PERCENT
    return max(0, min(100, current + delta))


def first_match(output, pattern):
    if output is None:
        return None
    match = re.search(pattern, output)
    return match.group(1) if match else None


def get_volume(platform=sys.platform, run=run_command):
    """The output volume in percent, or None when it cannot be read."""
    if platform == "darwin":
        output = run(["osascript", "-e", MACOS_GET_VOLUME_SCRIPT])
        return None if output is None else parse_osascript_volume(output)
    env = linux_env()
    value = first_match(run(["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"], env), r"Volume:\s+([0-9.]+)")
    if value is not None:
        return round(float(value) * 100)
    value = first_match(run(["pactl", "get-sink-volume", "@DEFAULT_SINK@"], env), r"(\d+)%")
    if value is not None:
        return int(value)
    value = first_match(run(["amixer", "sget", "Master"], env), r"\[(\d+)%\]")
    if value is not None:
        return int(value)
    return None


def adjust_volume(direction, platform=sys.platform, run=run_command):
    """Steps the volume up or down by 5 %; True when a command succeeded."""
    if platform == "darwin":
        current = get_volume(platform, run)
        if current is None:
            return False
        script = f"set volume output volume {step_percent(current, direction)}"
        return run(["osascript", "-e", script]) is not None
    env = linux_env()
    wpctl_step = "5%+" if direction == "up" else "5%-"
    pactl_step = "+5%" if direction == "up" else "-5%"
    attempts = [
        ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", wpctl_step],
        ["pactl", "set-sink-volume", "@DEFAULT_SINK@", pactl_step],
        ["amixer", "sset", "Master", wpctl_step],
    ]
    return any(run(args, env) is not None for args in attempts)
