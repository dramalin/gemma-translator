//! Downloads every model file `translator` needs into one directory
//! Run at deployment time; the translator never goes online.

mod catalog;
mod download;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;

use moonshine_sys::MoonshineLibrary;
use translator_core::language::LanguageCode;
use translator_core::models::ModelPaths;

use crate::download::{ensure_file, DownloadOutcome};

const USAGE: &str = "usage: translator-models --model-dir <dir> [--lib-dir <dir>]";
/// Files at least this large get progress lines while they download.
const PROGRESS_MIN_BYTES: u64 = 64 << 20;

#[derive(Debug, PartialEq)]
struct Options {
    model_dir: PathBuf,
    lib_dir: PathBuf,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut model_dir = None;
    let mut lib_dir = None;
    while let Some(argument) = args.next() {
        let target = match argument.as_str() {
            "--model-dir" => &mut model_dir,
            "--lib-dir" => &mut lib_dir,
            other => return Err(format!("unknown argument {other:?}")),
        };
        let value = args
            .next()
            .ok_or_else(|| format!("{argument} needs a value"))?;
        *target = Some(PathBuf::from(value));
    }
    Ok(Options {
        model_dir: model_dir.ok_or("--model-dir is required")?,
        lib_dir: match lib_dir {
            Some(directory) => directory,
            None => default_lib_dir()?,
        },
    })
}

fn default_lib_dir() -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let directory = executable
        .parent()
        .ok_or("executable has no parent directory")?;
    Ok(directory.join("lib"))
}

fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(options: &Options) -> Result<(), String> {
    let paths = ModelPaths::new(&options.model_dir);
    let library = MoonshineLibrary::load(&options.lib_dir).map_err(|error| error.to_string())?;
    let tts_keys = collect_tts_keys(&library, &paths)?;

    let mut files = vec![catalog::gemma_file(&paths)];
    files.extend(catalog::stt_files(&paths));
    files.push(catalog::vad_file(&paths));
    files.extend(catalog::tts_files(&paths, &tts_keys));

    // litert-lm writes its compiled-model cache here at run time; create it
    // now, while the deploying user can still write to the model directory.
    let cache_dir = paths.llm_cache_dir();
    std::fs::create_dir_all(&cache_dir)
        .map_err(|error| format!("{}: {error}", cache_dir.display()))?;

    let agent = download::agent();
    let mut failures = 0;
    for spec in &files {
        let mut last_tenth = 0;
        let mut on_progress = |received: u64, total: Option<u64>| {
            if let Some(tenth) = progress_tenth(received, total) {
                if tenth > last_tenth {
                    last_tenth = tenth;
                    println!("  {}% of {}", tenth * 10, spec.destination.display());
                }
            }
        };
        match ensure_file(&agent, spec, &mut on_progress) {
            Ok(DownloadOutcome::AlreadyPresent) => {
                println!("present  {}", spec.destination.display())
            }
            Ok(DownloadOutcome::Downloaded { bytes }) => {
                println!("fetched  {} ({bytes} bytes)", spec.destination.display())
            }
            Err(error) => {
                eprintln!("failed   {error}");
                failures += 1;
            }
        }
    }
    if failures > 0 {
        return Err(format!(
            "{failures} of {} files failed; run again to retry",
            files.len()
        ));
    }
    println!(
        "all {} model files ready in {}",
        files.len(),
        paths.root().display()
    );
    Ok(())
}

/// Completed tenths (1..=10) of a large download, or `None` for small or
/// unknown-size files.
fn progress_tenth(received: u64, total: Option<u64>) -> Option<u64> {
    let total = total.filter(|total| *total >= PROGRESS_MIN_BYTES)?;
    Some(received.min(total) * 10 / total)
}

fn collect_tts_keys(
    library: &MoonshineLibrary,
    paths: &ModelPaths,
) -> Result<BTreeSet<String>, String> {
    let root = paths.tts_root();
    let root = root
        .to_str()
        .ok_or("model directory path is not valid UTF-8")?;
    let mut keys = BTreeSet::new();
    for language in LanguageCode::ALL {
        let voice = language.tts_voice();
        let dependencies = library
            .tts_dependencies(voice.language_tag, &voice.synthesizer_options(root))
            .map_err(|error| error.to_string())?;
        // Keys without a '/' are G2P override labels, not files.
        let files: Vec<String> = dependencies
            .into_iter()
            .filter(|key| key.contains('/'))
            .collect();
        if files.is_empty() {
            return Err(format!("no TTS assets listed for {}", voice.language_tag));
        }
        keys.extend(files);
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn parses_both_directories() {
        assert_eq!(
            parse_args(args(&["--model-dir", "/m", "--lib-dir", "/l"])),
            Ok(Options {
                model_dir: PathBuf::from("/m"),
                lib_dir: PathBuf::from("/l"),
            })
        );
    }

    #[test]
    fn progress_is_reported_in_tenths_for_large_files_only() {
        let total = Some(PROGRESS_MIN_BYTES * 10);
        assert_eq!(progress_tenth(PROGRESS_MIN_BYTES * 3 + 1, total), Some(3));
        assert_eq!(progress_tenth(PROGRESS_MIN_BYTES * 10, total), Some(10));
        assert_eq!(progress_tenth(100, Some(1000)), None);
        assert_eq!(progress_tenth(PROGRESS_MIN_BYTES * 3, None), None);
    }

    #[test]
    fn model_dir_is_required_and_unknown_flags_rejected() {
        assert!(parse_args(args(&["--lib-dir", "/l"])).is_err());
        assert!(parse_args(args(&["--model-dir"])).is_err());
        assert!(parse_args(args(&["--model-dir", "/m", "--fast"])).is_err());
    }
}
