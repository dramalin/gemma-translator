//! Downloads every model file `translator` needs into one directory
//! Run at deployment time; the translator never goes online.

mod catalog;
mod download;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use moonshine_sys::{DependencyFile, ModelArch, MoonshineLibrary};
use translator_core::config::Config;
use translator_core::language::LanguageCode;
use translator_core::models::{ModelPaths, SttModelArch, TtsSettings, TtsVoice};

use crate::download::{ensure_file, DownloadOutcome};

const USAGE: &str =
    "usage: translator-models --model-dir <dir> [--lib-dir <dir>] [--config <file>] [--list-voices]";
/// Files at least this large get progress lines while they download.
const PROGRESS_MIN_BYTES: u64 = 64 << 20;

#[derive(Debug, PartialEq)]
struct Options {
    model_dir: PathBuf,
    lib_dir: PathBuf,
    /// The translator's config, for its `tts_voices`; `None` uses the
    /// default path.
    config: Option<PathBuf>,
    /// Print every voice instead of downloading.
    list_voices: bool,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut model_dir = None;
    let mut lib_dir = None;
    let mut config = None;
    let mut list_voices = false;
    while let Some(argument) = args.next() {
        let target = match argument.as_str() {
            "--model-dir" => &mut model_dir,
            "--lib-dir" => &mut lib_dir,
            "--config" => &mut config,
            "--list-voices" => {
                list_voices = true;
                continue;
            }
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
        config,
        list_voices,
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
    let root = paths.tts_root();
    let root = root
        .to_str()
        .ok_or("model directory path is not valid UTF-8")?;
    if options.list_voices {
        return list_voices(&library, root);
    }
    let tts = load_tts_settings(options.config.as_deref())?;
    check_voices(&library, root, &tts)?;
    let tts_files = collect_tts_files(&library, root, &tts)?;

    let mut files = vec![catalog::gemma_file(&paths)];
    for language in LanguageCode::ALL {
        let model = language.stt_model();
        let manifest = library
            .stt_dependencies(language.code(), model_arch(model.arch))
            .map_err(|error| error.to_string())?;
        files.extend(catalog::manifest_files(&paths.stt_dir(model), &manifest));
    }
    files.push(catalog::vad_file(&paths));
    files.extend(catalog::manifest_files(
        &paths.tts_root(),
        tts_files.values(),
    ));

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
            Ok(DownloadOutcome::Replaced { bytes }) => {
                println!("updated  {} ({bytes} bytes)", spec.destination.display())
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
    for removed in remove_onnx_files(&paths.tts_root())? {
        println!("removed  {} (moonshine loads .ort only)", removed.display());
    }
    let wanted: Vec<&Path> = files
        .iter()
        .map(|spec| spec.destination.as_path())
        .collect();
    for removed in remove_other_stt_models(&paths, &wanted)? {
        println!(
            "removed  {} (not part of a current model)",
            removed.display()
        );
    }
    println!(
        "all {} model files ready in {}",
        files.len(),
        paths.root().display()
    );
    Ok(())
}

/// Deletes what under `stt/` is not a current model's file: the models of an
/// earlier version, and files a model no longer lists (Moonshine might load
/// an old `frontend.ort` in place of the new pair). They would also ship in
/// the Pi package.
fn remove_other_stt_models(paths: &ModelPaths, wanted: &[&Path]) -> Result<Vec<PathBuf>, String> {
    let root = paths.stt_root();
    let current: Vec<PathBuf> = LanguageCode::ALL
        .iter()
        .map(|language| paths.stt_dir(language.stt_model()))
        .collect();
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{}: {error}", root.display())),
    };
    let mut removed = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| format!("{}: {error}", root.display()))?
            .path();
        if !path.is_dir() {
            continue;
        }
        if !current.contains(&path) {
            std::fs::remove_dir_all(&path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            removed.push(path);
            continue;
        }
        for file in
            std::fs::read_dir(&path).map_err(|error| format!("{}: {error}", path.display()))?
        {
            let file = file
                .map_err(|error| format!("{}: {error}", path.display()))?
                .path();
            if file.is_file() && !wanted.contains(&file.as_path()) {
                std::fs::remove_file(&file)
                    .map_err(|error| format!("{}: {error}", file.display()))?;
                removed.push(file);
            }
        }
    }
    removed.sort();
    Ok(removed)
}

/// Deletes the `.onnx` models under `directory`: earlier Moonshine versions
/// used them, the current one loads `.ort` files only, and they would ship
/// in the Pi package. Piper's `.onnx.json` configs stay.
fn remove_onnx_files(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let mut removed = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("{}: {error}", directory.display())),
        };
        for entry in entries {
            let path = entry
                .map_err(|error| format!("{}: {error}", directory.display()))?
                .path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "onnx")
            {
                std::fs::remove_file(&path)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                removed.push(path);
            }
        }
    }
    removed.sort();
    Ok(removed)
}

/// Completed tenths (1..=10) of a large download, or `None` for small or
/// unknown-size files.
fn progress_tenth(received: u64, total: Option<u64>) -> Option<u64> {
    let total = total.filter(|total| *total >= PROGRESS_MIN_BYTES)?;
    Some(received.min(total) * 10 / total)
}

/// The voices and speed from `path`, or from the default config path. A
/// missing file means the built-in voices.
fn load_tts_settings(path: Option<&Path>) -> Result<TtsSettings, String> {
    let Some(path) = path.map(Path::to_path_buf).or_else(Config::default_path) else {
        return Ok(TtsSettings::default());
    };
    let config = Config::load(&path).map_err(|error| error.to_string())?;
    for (language, voice) in &config.tts_voices {
        println!(
            "voice    {} = {voice} (from {})",
            language.code(),
            path.display()
        );
    }
    Ok(config.tts_settings())
}

/// Fails on a configured voice that is not one of its language's voices:
/// Moonshine would quietly speak with its default voice instead.
fn check_voices(library: &MoonshineLibrary, root: &str, tts: &TtsSettings) -> Result<(), String> {
    let mut problems = Vec::new();
    for (&language, id) in &tts.voices {
        let tag = language.tts_voice().language_tag;
        let known = library
            .tts_voices(tag, &[("g2p_root", root)])
            .map_err(|error| error.to_string())?;
        if !known.iter().any(|voice| voice.id == *id) {
            let ids: Vec<&str> = known.iter().map(|voice| voice.id.as_str()).collect();
            problems.push(format!(
                "tts_voices.{} = {id:?} is not a {tag} voice; choose one of: {}",
                language.code(),
                ids.join(", ")
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// Every voice Moonshine offers per language, marking the downloaded ones.
fn list_voices(library: &MoonshineLibrary, root: &str) -> Result<(), String> {
    for language in LanguageCode::ALL {
        let built_in = language.tts_voice();
        let voices = library
            .tts_voices(built_in.language_tag, &[("g2p_root", root)])
            .map_err(|error| error.to_string())?;
        println!("{} ({}):", language.code(), built_in.language_tag);
        for voice in voices {
            let mark = if voice.found { "*" } else { " " };
            println!("  {mark} {}", voice.id);
        }
    }
    println!("* = downloaded. Set one per language under [tts_voices] in config.toml.");
    Ok(())
}

/// The files, by name relative to the TTS root, of each language's built-in
/// voice and its configured one.
/// The built-in voices are always fetched, so a package built with one
/// config still speaks under another config's defaults.
fn collect_tts_files(
    library: &MoonshineLibrary,
    root: &str,
    tts: &TtsSettings,
) -> Result<BTreeMap<String, DependencyFile>, String> {
    let mut files = BTreeMap::new();
    for language in LanguageCode::ALL {
        let configured = tts.voice(language);
        let built_in = language.tts_voice();
        let mut voices = vec![built_in];
        if configured.voice != voices[0].voice {
            voices.push(configured);
        }
        for voice in voices {
            for file in voice_files(library, root, &voice)? {
                files.insert(file.name.clone(), file);
            }
        }
    }
    Ok(files)
}

fn voice_files(
    library: &MoonshineLibrary,
    root: &str,
    voice: &TtsVoice,
) -> Result<Vec<DependencyFile>, String> {
    let options = voice.synthesizer_options(root);
    let options: Vec<(&str, &str)> = options
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let files = library
        .tts_dependencies(voice.language_tag, &options)
        .map_err(|error| error.to_string())?;
    if files.is_empty() {
        return Err(format!("no TTS assets listed for {}", voice.language_tag));
    }
    Ok(files)
}

fn model_arch(arch: SttModelArch) -> ModelArch {
    match arch {
        SttModelArch::Tiny => ModelArch::Tiny,
        SttModelArch::TinyStreaming => ModelArch::TinyStreaming,
        SttModelArch::SmallStreaming => ModelArch::SmallStreaming,
        SttModelArch::MediumStreaming => ModelArch::MediumStreaming,
    }
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
                config: None,
                list_voices: false,
            })
        );
    }

    #[test]
    fn parses_config_and_list_voices() {
        let options = parse_args(args(&[
            "--model-dir",
            "/m",
            "--lib-dir",
            "/l",
            "--config",
            "/c.toml",
            "--list-voices",
        ]))
        .unwrap();
        assert_eq!(options.config, Some(PathBuf::from("/c.toml")));
        assert!(options.list_voices);
    }

    #[test]
    fn only_unused_stt_models_and_files_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ModelPaths::new(dir.path());
        let current = paths.stt_dir(LanguageCode::Zh.stt_model());
        let old = paths.stt_root().join("base-zh");
        for directory in [&current, &old] {
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(directory.join("tokenizer.bin"), b"").unwrap();
        }
        std::fs::write(current.join("frontend.ort"), b"").unwrap();
        std::fs::write(paths.stt_root().join("notes.txt"), b"").unwrap();
        let wanted = current.join("tokenizer.bin");
        assert_eq!(
            remove_other_stt_models(&paths, &[wanted.as_path()]).unwrap(),
            [old, current.join("frontend.ort")]
        );
        assert!(current.join("tokenizer.bin").is_file());
        assert!(paths.stt_root().join("notes.txt").is_file());
    }

    #[test]
    fn only_onnx_models_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let voices = dir.path().join("ko/piper-voices");
        std::fs::create_dir_all(&voices).unwrap();
        for name in ["a.onnx", "a.onnx.json", "a.upstream.model.ort"] {
            std::fs::write(voices.join(name), b"").unwrap();
        }
        assert_eq!(
            remove_onnx_files(dir.path()).unwrap(),
            [voices.join("a.onnx")]
        );
        assert!(voices.join("a.onnx.json").is_file());
        assert!(voices.join("a.upstream.model.ort").is_file());
        assert!(remove_onnx_files(&dir.path().join("missing"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_missing_config_file_means_the_built_in_voices() {
        let dir = tempfile::tempdir().unwrap();
        let settings = load_tts_settings(Some(&dir.path().join("config.toml"))).unwrap();
        assert_eq!(settings, TtsSettings::default());
    }

    #[test]
    fn an_invalid_config_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[tts_voices]\nxx = \"a\"\n").unwrap();
        assert!(load_tts_settings(Some(&path)).is_err());
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
