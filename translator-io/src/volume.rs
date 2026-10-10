use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use translator_core::language::RotateDirection;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(20);
const VOLUME_STEP_PERCENT: u16 = 5;
const MACOS_GET_VOLUME_SCRIPT: &str = "output volume of (get volume settings)";

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
        if cfg!(target_os = "linux") && std::env::var_os("XDG_RUNTIME_DIR").is_none() {
            if let Ok(metadata) = std::fs::metadata("/proc/self") {
                command.env("XDG_RUNTIME_DIR", format!("/run/user/{}", metadata.uid()));
            }
        }
        let mut child = command.spawn().ok()?;
        // Read stdout while the program runs: a program whose output fills the
        // pipe would otherwise block and never exit.
        let mut pipe = child.stdout.take()?;
        let (stdout_sender, stdout_receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut stdout = String::new();
            let _ = stdout_sender.send(pipe.read_to_string(&mut stdout).ok().map(|_| stdout));
        });
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        loop {
            match child.try_wait().ok()? {
                Some(status) => {
                    // Bounded: a daemon the command spawned may keep stdout open.
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    let stdout = stdout_receiver.recv_timeout(remaining).ok()??;
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

/// Which system tools control the output volume.
#[derive(Clone, Copy, Debug, PartialEq)]
enum VolumePlatform {
    /// wpctl (PipeWire), then pactl (PulseAudio), then amixer (ALSA).
    Linux,
    /// AppleScript's `output volume` through osascript.
    MacOs,
}

/// System output volume: osascript on macOS; wpctl, pactl, then amixer elsewhere.
pub struct VolumeControl<R: CommandRunner> {
    runner: R,
    platform: VolumePlatform,
}

impl<R: CommandRunner> VolumeControl<R> {
    pub fn new(runner: R) -> Self {
        let platform = if cfg!(target_os = "macos") {
            VolumePlatform::MacOs
        } else {
            VolumePlatform::Linux
        };
        VolumeControl { runner, platform }
    }

    pub fn current_percent(&self) -> Option<u16> {
        match self.platform {
            VolumePlatform::Linux => self.linux_percent(),
            VolumePlatform::MacOs => self.macos_percent(),
        }
    }

    /// Steps the volume by 5 % without going past 0–100 % and returns the new level.
    pub fn adjust(&self, direction: RotateDirection) -> Option<u16> {
        match self.platform {
            VolumePlatform::Linux => self.adjust_linux(direction),
            VolumePlatform::MacOs => self.adjust_macos(direction),
        }
    }

    fn macos_percent(&self) -> Option<u16> {
        self.runner
            .run("osascript", &["-e", MACOS_GET_VOLUME_SCRIPT])
            .and_then(|output| parse_osascript_volume(&output))
    }

    fn adjust_macos(&self, direction: RotateDirection) -> Option<u16> {
        let current = self.macos_percent()?;
        let target = if direction == RotateDirection::Next {
            (current + VOLUME_STEP_PERCENT).min(100)
        } else {
            current.saturating_sub(VOLUME_STEP_PERCENT)
        };
        let script = format!("set volume output volume {target}");
        self.runner.run("osascript", &["-e", &script])?;
        self.macos_percent()
    }

    fn linux_percent(&self) -> Option<u16> {
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

    fn adjust_linux(&self, direction: RotateDirection) -> Option<u16> {
        let is_up = direction == RotateDirection::Next;
        let wpctl_step = if is_up { "5%+" } else { "5%-" };
        let pactl_step = if is_up { "+5%" } else { "-5%" };
        let wpctl_args = [
            "set-volume",
            "-l",
            "1.0",
            "@DEFAULT_AUDIO_SINK@",
            wpctl_step,
        ];
        let pactl_args = ["set-sink-volume", "@DEFAULT_SINK@", pactl_step];
        let amixer_args = ["sset", "Master", wpctl_step];
        let attempts: [(&str, &[&str]); 3] = [
            ("wpctl", &wpctl_args),
            ("pactl", &pactl_args),
            ("amixer", &amixer_args),
        ];
        let changed_with = attempts
            .iter()
            .find(|(program, args)| self.runner.run(program, args).is_some())
            .map(|(program, _)| *program)?;
        let percent = self.linux_percent();
        // wpctl caps itself (-l) and amixer stops at its maximum; pactl does not.
        if changed_with == "pactl" && percent.is_some_and(|percent| percent > 100) {
            self.runner
                .run("pactl", &["set-sink-volume", "@DEFAULT_SINK@", "100%"])?;
            return self.linux_percent();
        }
        percent
    }
}

/// Runs `VolumeControl` on its own thread, because each command may take up to
/// `COMMAND_TIMEOUT` and the UI thread must not wait. The thread ends when the
/// worker is dropped.
pub struct VolumeWorker {
    adjustments: Sender<RotateDirection>,
    readings: Receiver<Option<u16>>,
}

impl VolumeWorker {
    /// Starts the thread, which first reads the current volume.
    pub fn spawn<R: CommandRunner + Send + 'static>(runner: R) -> Self {
        Self::spawn_control(VolumeControl::new(runner))
    }

    fn spawn_control<R: CommandRunner + Send + 'static>(control: VolumeControl<R>) -> Self {
        let (adjustments, adjustment_receiver) = mpsc::channel();
        let (reading_sender, readings) = mpsc::channel();
        thread::spawn(move || {
            if reading_sender.send(control.current_percent()).is_err() {
                return;
            }
            for direction in adjustment_receiver {
                if reading_sender.send(control.adjust(direction)).is_err() {
                    return;
                }
            }
        });
        VolumeWorker {
            adjustments,
            readings,
        }
    }

    /// Queues a 5 % step; the resulting level arrives through `latest_reading`.
    pub fn adjust(&self, direction: RotateDirection) {
        let _ = self.adjustments.send(direction);
    }

    /// The newest reading finished since the last call, if any; the reading
    /// itself is `None` when the volume is unknown.
    pub fn latest_reading(&self) -> Option<Option<u16>> {
        self.readings.try_iter().last()
    }
}

/// "Volume: 0.75" → 75
fn parse_wpctl_volume(output: &str) -> Option<u16> {
    let value = output.split("Volume:").nth(1)?.split_whitespace().next()?;
    let fraction: f32 = value.parse().ok()?;
    Some((fraction * 100.0).round() as u16)
}

/// "75\n" → 75; "missing value" (no output device) → None
fn parse_osascript_volume(output: &str) -> Option<u16> {
    output
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|percent| *percent <= 100)
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
    use std::sync::mpsc::{self, Receiver};
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

    fn linux_control<R: CommandRunner>(runner: R) -> VolumeControl<R> {
        VolumeControl {
            runner,
            platform: VolumePlatform::Linux,
        }
    }

    fn macos_control<R: CommandRunner>(runner: R) -> VolumeControl<R> {
        VolumeControl {
            runner,
            platform: VolumePlatform::MacOs,
        }
    }

    const OSASCRIPT_GET: &str = "osascript -e output volume of (get volume settings)";

    #[test]
    fn parses_osascript_volume() {
        assert_eq!(parse_osascript_volume("75\n"), Some(75));
        assert_eq!(parse_osascript_volume("0"), Some(0));
        assert_eq!(parse_osascript_volume("100\n"), Some(100));
        assert_eq!(parse_osascript_volume("missing value\n"), None);
        assert_eq!(parse_osascript_volume(""), None);
        assert_eq!(parse_osascript_volume("101"), None);
    }

    #[test]
    fn macos_reads_osascript() {
        let control = macos_control(FakeRunner::with(&[(OSASCRIPT_GET, "75\n")]));
        assert_eq!(control.current_percent(), Some(75));
    }

    #[test]
    fn macos_adjust_up_clamps_at_one_hundred() {
        let control = macos_control(FakeRunner::with(&[
            (OSASCRIPT_GET, "98\n"),
            ("osascript -e set volume output volume 100", ""),
        ]));
        // The fake keeps answering 98; the set command proves the clamp.
        assert_eq!(control.adjust(RotateDirection::Next), Some(98));
        let calls = control.runner.calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            [
                OSASCRIPT_GET,
                "osascript -e set volume output volume 100",
                OSASCRIPT_GET
            ]
        );
    }

    #[test]
    fn macos_adjust_down_clamps_at_zero() {
        let control = macos_control(FakeRunner::with(&[
            (OSASCRIPT_GET, "3\n"),
            ("osascript -e set volume output volume 0", ""),
        ]));
        control.adjust(RotateDirection::Previous);
        let calls = control.runner.calls.lock().unwrap().clone();
        assert!(calls.contains(&"osascript -e set volume output volume 0".to_string()));
    }

    #[test]
    fn macos_unknown_volume_sets_nothing() {
        let control = macos_control(FakeRunner::with(&[(OSASCRIPT_GET, "missing value\n")]));
        assert_eq!(control.adjust(RotateDirection::Next), None);
        let calls = control.runner.calls.lock().unwrap().clone();
        assert_eq!(calls, [OSASCRIPT_GET]);
    }

    #[test]
    fn macos_failed_set_gives_none() {
        let control = macos_control(FakeRunner::with(&[(OSASCRIPT_GET, "50\n")]));
        assert_eq!(control.adjust(RotateDirection::Next), None);
    }

    #[test]
    fn new_picks_the_build_platform() {
        let control = VolumeControl::new(FakeRunner::default());
        let expected = if cfg!(target_os = "macos") {
            VolumePlatform::MacOs
        } else {
            VolumePlatform::Linux
        };
        assert_eq!(control.platform, expected);
    }

    #[test]
    fn reads_wpctl_first() {
        let runner =
            FakeRunner::with(&[("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.75\n")]);
        assert_eq!(linux_control(runner).current_percent(), Some(75));
    }

    #[test]
    fn falls_back_to_pactl_then_amixer() {
        let pactl = FakeRunner::with(&[(
            "pactl get-sink-volume @DEFAULT_SINK@",
            "Volume: front-left: 49151 /  75% / -7.50 dB,   front-right: 49151 /  75% / -7.50 dB",
        )]);
        assert_eq!(linux_control(pactl).current_percent(), Some(75));
        let amixer = FakeRunner::with(&[(
            "amixer sget Master",
            "  Mono: Playback 52 [81%] [-12.00dB] [on]",
        )]);
        assert_eq!(linux_control(amixer).current_percent(), Some(81));
    }

    #[test]
    fn pactl_is_pulled_back_to_one_hundred_percent() {
        let runner = FakeRunner::with(&[
            ("pactl set-sink-volume @DEFAULT_SINK@ +5%", ""),
            (
                "pactl get-sink-volume @DEFAULT_SINK@",
                "Volume: front-left: 68811 / 105% / 1.27 dB",
            ),
            ("pactl set-sink-volume @DEFAULT_SINK@ 100%", ""),
        ]);
        let control = linux_control(runner);
        control.adjust(RotateDirection::Next);
        let calls = control.runner.calls.lock().unwrap().clone();
        assert!(calls.contains(&"pactl set-sink-volume @DEFAULT_SINK@ 100%".to_string()));
    }

    #[test]
    fn system_runner_reads_large_output() {
        let output = SystemCommandRunner
            .run("head", &["-c", "200000", "/dev/zero"])
            .expect("head is available");
        assert_eq!(output.len(), 200_000);
    }

    #[test]
    fn missing_tools_give_none_without_panicking() {
        let control = linux_control(FakeRunner::default());
        assert_eq!(control.current_percent(), None);
        assert_eq!(control.adjust(RotateDirection::Next), None);
    }

    #[test]
    fn adjust_uses_five_percent_steps_and_rereads() {
        let runner = FakeRunner::with(&[
            ("wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 5%-", ""),
            ("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.70"),
        ]);
        let control = linux_control(runner);
        assert_eq!(control.adjust(RotateDirection::Previous), Some(70));
        let calls = control.runner.calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            [
                "wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 5%-",
                "wpctl get-volume @DEFAULT_AUDIO_SINK@"
            ]
        );
    }

    /// Blocks every command until `release` is signalled or dropped.
    struct GatedRunner {
        inner: FakeRunner,
        release: Mutex<Receiver<()>>,
    }

    impl CommandRunner for GatedRunner {
        fn run(&self, program: &str, args: &[&str]) -> Option<String> {
            let _ = self
                .release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(2));
            self.inner.run(program, args)
        }
    }

    fn wait_for_reading(worker: &VolumeWorker) -> Option<u16> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(reading) = worker.latest_reading() {
                return reading;
            }
            assert!(Instant::now() < deadline, "no volume reading arrived");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn worker_reads_at_start_and_after_each_adjustment() {
        let worker = VolumeWorker::spawn_control(linux_control(FakeRunner::with(&[
            ("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.75"),
            ("wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 5%+", ""),
        ])));
        assert_eq!(wait_for_reading(&worker), Some(75));
        worker.adjust(RotateDirection::Next);
        assert_eq!(wait_for_reading(&worker), Some(75));
        assert_eq!(worker.latest_reading(), None);
    }

    #[test]
    fn worker_requests_do_not_wait_for_the_commands() {
        let (release, release_receiver) = mpsc::channel();
        let started_at = Instant::now();
        let worker = VolumeWorker::spawn_control(linux_control(GatedRunner {
            inner: FakeRunner::with(&[
                ("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.40"),
                ("wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 5%-", ""),
            ]),
            release: Mutex::new(release_receiver),
        }));
        worker.adjust(RotateDirection::Previous);
        assert!(started_at.elapsed() < Duration::from_millis(500));
        assert_eq!(worker.latest_reading(), None);
        drop(release);
        assert_eq!(wait_for_reading(&worker), Some(40));
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
            linux_control(runner).adjust(RotateDirection::Next),
            Some(80)
        );
    }
}
