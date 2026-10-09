use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use translator_core::language::RotateDirection;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Runs an external program and returns its stdout, or `None` when the program
/// is missing, fails or times out.
pub trait CommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> Option<String>;
}

pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> Option<String> {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // PipeWire's wpctl finds its socket through XDG_RUNTIME_DIR, which systemd
        // services may lack.
        if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
            if let Ok(metadata) = std::fs::metadata("/proc/self") {
                command.env("XDG_RUNTIME_DIR", format!("/run/user/{}", metadata.uid()));
            }
        }
        let mut child = command.spawn().ok()?;
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        loop {
            match child.try_wait().ok()? {
                Some(status) => {
                    let mut stdout = String::new();
                    child.stdout.take()?.read_to_string(&mut stdout).ok()?;
                    return status.success().then_some(stdout);
                }
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                None => thread::sleep(COMMAND_POLL_INTERVAL),
            }
        }
    }
}

/// System output volume via wpctl (PipeWire), then pactl (PulseAudio), then amixer (ALSA).
pub struct VolumeControl<R: CommandRunner> {
    runner: R,
}

impl<R: CommandRunner> VolumeControl<R> {
    pub fn new(runner: R) -> Self {
        VolumeControl { runner }
    }

    pub fn current_percent(&self) -> Option<u16> {
        self.runner
            .run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])
            .and_then(|output| parse_wpctl_volume(&output))
            .or_else(|| {
                self.runner
                    .run("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])
                    .and_then(|output| parse_pactl_volume(&output))
            })
            .or_else(|| {
                self.runner
                    .run("amixer", &["sget", "Master"])
                    .and_then(|output| parse_amixer_volume(&output))
            })
    }

    /// Steps the volume by 5 % and returns the new level.
    pub fn adjust(&self, direction: RotateDirection) -> Option<u16> {
        let is_up = direction == RotateDirection::Next;
        let attempts: [(&str, [&str; 3]); 3] = [
            (
                "wpctl",
                [
                    "set-volume",
                    "@DEFAULT_AUDIO_SINK@",
                    if is_up { "5%+" } else { "5%-" },
                ],
            ),
            (
                "pactl",
                [
                    "set-sink-volume",
                    "@DEFAULT_SINK@",
                    if is_up { "+5%" } else { "-5%" },
                ],
            ),
            (
                "amixer",
                ["sset", "Master", if is_up { "5%+" } else { "5%-" }],
            ),
        ];
        let is_changed = attempts
            .iter()
            .any(|(program, args)| self.runner.run(program, args).is_some());
        if is_changed {
            self.current_percent()
        } else {
            None
        }
    }
}

/// "Volume: 0.75" → 75
fn parse_wpctl_volume(output: &str) -> Option<u16> {
    let value = output.split("Volume:").nth(1)?.split_whitespace().next()?;
    let fraction: f32 = value.parse().ok()?;
    Some((fraction * 100.0).round() as u16)
}

/// "... / 75% / ..." → 75
fn parse_pactl_volume(output: &str) -> Option<u16> {
    output
        .split_whitespace()
        .find_map(|token| token.strip_suffix('%')?.parse().ok())
}

/// "... [81%] ..." → 81
fn parse_amixer_volume(output: &str) -> Option<u16> {
    output
        .split('[')
        .find_map(|part| part.split_once("%]")?.0.parse().ok())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    /// Answers commands by their full text; anything unscripted "is not installed".
    #[derive(Default)]
    struct FakeRunner {
        outputs: HashMap<String, String>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeRunner {
        fn with(outputs: &[(&str, &str)]) -> Self {
            FakeRunner {
                outputs: outputs
                    .iter()
                    .map(|(c, o)| (c.to_string(), o.to_string()))
                    .collect(),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, program: &str, args: &[&str]) -> Option<String> {
            let command = format!("{program} {}", args.join(" "));
            self.calls.lock().unwrap().push(command.clone());
            self.outputs.get(&command).cloned()
        }
    }

    #[test]
    fn reads_wpctl_first() {
        let runner =
            FakeRunner::with(&[("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.75\n")]);
        assert_eq!(VolumeControl::new(runner).current_percent(), Some(75));
    }

    #[test]
    fn falls_back_to_pactl_then_amixer() {
        let pactl = FakeRunner::with(&[(
            "pactl get-sink-volume @DEFAULT_SINK@",
            "Volume: front-left: 49151 /  75% / -7.50 dB,   front-right: 49151 /  75% / -7.50 dB",
        )]);
        assert_eq!(VolumeControl::new(pactl).current_percent(), Some(75));
        let amixer = FakeRunner::with(&[(
            "amixer sget Master",
            "  Mono: Playback 52 [81%] [-12.00dB] [on]",
        )]);
        assert_eq!(VolumeControl::new(amixer).current_percent(), Some(81));
    }

    #[test]
    fn missing_tools_give_none_without_panicking() {
        let control = VolumeControl::new(FakeRunner::default());
        assert_eq!(control.current_percent(), None);
        assert_eq!(control.adjust(RotateDirection::Next), None);
    }

    #[test]
    fn adjust_uses_five_percent_steps_and_rereads() {
        let runner = FakeRunner::with(&[
            ("wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-", ""),
            ("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.70"),
        ]);
        let control = VolumeControl::new(runner);
        assert_eq!(control.adjust(RotateDirection::Previous), Some(70));
        let calls = control.runner.calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            [
                "wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-",
                "wpctl get-volume @DEFAULT_AUDIO_SINK@"
            ]
        );
    }

    #[test]
    fn adjust_up_falls_back_to_pactl() {
        let runner = FakeRunner::with(&[
            ("pactl set-sink-volume @DEFAULT_SINK@ +5%", ""),
            (
                "pactl get-sink-volume @DEFAULT_SINK@",
                "Volume: front-left: 1 / 80% / 0 dB",
            ),
        ]);
        assert_eq!(
            VolumeControl::new(runner).adjust(RotateDirection::Next),
            Some(80)
        );
    }
}
