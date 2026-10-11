//! Speech recognition evaluation over the corpus in `eval/` (see eval/README.md).
//!
//! Every `<corpus>/<condition>/<lang>/NN.wav` goes through the app's own path
//! (downmix, low-pass, resample, silence trim, Moonshine, transcript cleanup)
//! `--runs` times, and is scored against line NN of `<sentences>/<lang>.txt`:
//! character error rate for Chinese and Japanese, word error rate otherwise,
//! with numbers written out in words on both sides. Clips whose runs disagree
//! are counted as unstable. Per-clip results go to `<corpus>/report.tsv`.
//! `--trim none|energy|vad|adaptive` picks how the silence is trimmed
//! (default adaptive, as in the app), to compare against.
//!
//! make eval-stt [EVAL_ARGS="--runs 3 --only clean --trim energy"]

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use translator_core::language::LanguageCode;
use translator_core::models::{ModelPaths, TtsSettings};
use translator_core::pipeline::{SpeechToText, STT_SAMPLE_RATE};
use translator_io::audio_device::CapturedAudio;
use translator_io::chinese_script::spoken_text;
use translator_io::native::speech::NativeSpeech;
use translator_io::speech_trim::TrimMode;

struct Args {
    corpus: PathBuf,
    sentences: PathBuf,
    lib_dir: PathBuf,
    model_dir: PathBuf,
    runs: usize,
    only: Option<String>,
    trim: TrimMode,
}

fn parse_args() -> Result<Args, String> {
    let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let mut args = Args {
        corpus: PathBuf::from("eval/corpus"),
        sentences: PathBuf::from("eval/sentences"),
        lib_dir: env("TRANSLATOR_NATIVE_LIB_DIR").unwrap_or_else(|| PathBuf::from("lib")),
        model_dir: env("TRANSLATOR_MODEL_DIR").unwrap_or_else(|| PathBuf::from("models")),
        runs: 3,
        only: None,
        trim: TrimMode::Adaptive,
    };
    let mut iter = std::env::args().skip(1);
    while let Some(flag) = iter.next() {
        let mut value = || iter.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--corpus" => args.corpus = value()?.into(),
            "--sentences" => args.sentences = value()?.into(),
            "--lib-dir" => args.lib_dir = value()?.into(),
            "--model-dir" => args.model_dir = value()?.into(),
            "--runs" => {
                args.runs = value()?
                    .parse()
                    .ok()
                    .filter(|&runs| runs > 0)
                    .ok_or("--runs needs a positive number")?;
            }
            "--only" => args.only = Some(value()?),
            "--trim" => {
                args.trim = TrimMode::parse(&value()?)
                    .ok_or("--trim takes none, energy, vad or adaptive")?;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(args)
}

/// One recording to score.
struct Clip {
    condition: String,
    language: LanguageCode,
    path: PathBuf,
    reference: String,
}

fn find_clips(args: &Args) -> Result<Vec<Clip>, String> {
    let mut clips = Vec::new();
    let mut conditions = read_dir_sorted(&args.corpus)?;
    conditions.retain(|path| path.is_dir() && !path.ends_with("noise"));
    for condition_dir in conditions {
        let condition = file_name(&condition_dir);
        if args
            .only
            .as_ref()
            .is_some_and(|only| !condition.contains(only.as_str()))
        {
            continue;
        }
        for lang_dir in read_dir_sorted(&condition_dir)? {
            let Some(language) = LanguageCode::from_code(&file_name(&lang_dir)) else {
                continue;
            };
            let sentences_file = args.sentences.join(format!("{}.txt", language.code()));
            let sentences = fs::read_to_string(&sentences_file)
                .map_err(|error| format!("{}: {error}", sentences_file.display()))?;
            let sentences: Vec<&str> = sentences.lines().collect();
            for path in read_dir_sorted(&lang_dir)? {
                let number: Option<usize> = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .and_then(|stem| stem.parse().ok());
                let reference = number
                    .filter(|_| path.extension().is_some_and(|ext| ext == "wav"))
                    .and_then(|number| sentences.get(number.checked_sub(1)?));
                if let Some(reference) = reference {
                    clips.push(Clip {
                        condition: condition.clone(),
                        language,
                        path,
                        reference: reference.to_string(),
                    });
                }
            }
        }
    }
    Ok(clips)
}

fn read_dir_sorted(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|error| format!("{}: {error}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    paths.sort();
    Ok(paths)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Interleaved samples, rate and channel count of a 16-bit PCM or 32-bit
/// float WAV.
fn read_wav(path: &Path) -> Result<(Vec<f32>, u32, u16), String> {
    let fail = |message: &str| format!("{}: {message}", path.display());
    let bytes = fs::read(path).map_err(|error| fail(&error.to_string()))?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(fail("not a WAV file"));
    }
    let mut format = None;
    let mut position = 12;
    while position + 8 <= bytes.len() {
        let id = &bytes[position..position + 4];
        let size = u32::from_le_bytes(bytes[position + 4..position + 8].try_into().unwrap());
        let body_start = position + 8;
        let body_end = (body_start + size as usize).min(bytes.len());
        let body = &bytes[body_start..body_end];
        match id {
            b"fmt " if body.len() >= 16 => {
                let tag = u16::from_le_bytes([body[0], body[1]]);
                let channels = u16::from_le_bytes([body[2], body[3]]);
                let rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                let bits = u16::from_le_bytes([body[14], body[15]]);
                // 0xFFFE (extensible) carries the real format in its sub-format GUID.
                let tag = if tag == 0xFFFE && body.len() >= 26 {
                    u16::from_le_bytes([body[24], body[25]])
                } else {
                    tag
                };
                format = Some((tag, channels, rate, bits));
            }
            b"data" => {
                let (tag, channels, rate, bits) = format.ok_or_else(|| fail("data before fmt"))?;
                let samples: Vec<f32> = match (tag, bits) {
                    (1, 16) => body
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32768.0)
                        .collect(),
                    (3, 32) => body
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|quad| f32::from_le_bytes(*quad))
                        .collect(),
                    _ => return Err(fail(&format!("unsupported format {tag}/{bits}-bit"))),
                };
                return Ok((samples, rate, channels));
            }
            _ => {}
        }
        position = body_end + (size as usize & 1);
    }
    Err(fail("no data chunk"))
}

/// Simplified characters zhconv and Moonshine disagree on, mapped to one form.
const SAME_CHARACTERS: &[(char, char)] = &[('帐', '账')];

/// The units errors are counted in: characters for Chinese and Japanese
/// (in Simplified, punctuation and spaces dropped), lower-case words otherwise.
/// Numbers in digits count as their words ("100" is 一百 or "one hundred").
fn units(text: &str, language: LanguageCode) -> Vec<String> {
    match language {
        LanguageCode::Zh | LanguageCode::Ja => {
            let text = spoken_text(text, language);
            let mut characters = Vec::new();
            let mut digits = String::new();
            for c in text.chars().chain([' ']) {
                if c.is_ascii_digit() {
                    digits.push(c);
                    continue;
                }
                // "1 0 0" is one number.
                if c.is_whitespace() && !digits.is_empty() {
                    continue;
                }
                if !digits.is_empty() {
                    characters.extend(chinese_number(&digits).chars().map(String::from));
                    digits.clear();
                }
                if c.is_alphanumeric() {
                    let c = SAME_CHARACTERS
                        .iter()
                        .find(|(from, _)| *from == c)
                        .map_or(c, |&(_, to)| to);
                    characters.push(String::from(c));
                }
            }
            characters
        }
        _ => text
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '\'' {
                    c
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .flat_map(|word| match word.parse::<u32>() {
                Ok(number) if number < 1000 && !word.starts_with('0') || word == "0" => {
                    english_number(number)
                }
                _ => vec![word.to_string()],
            })
            .collect(),
    }
}

const CHINESE_DIGITS: [char; 10] = ['零', '一', '二', '三', '四', '五', '六', '七', '八', '九'];

/// 一百, 十五, 两千零七 style for numbers below 10000; other digit strings
/// (phone numbers, leading zeros) are read digit by digit.
fn chinese_number(digits: &str) -> String {
    let values: Vec<usize> = digits.bytes().map(|b| usize::from(b - b'0')).collect();
    if values.len() > 4 || values.len() > 1 && values[0] == 0 {
        return values.iter().map(|&d| CHINESE_DIGITS[d]).collect();
    }
    if values.iter().all(|&d| d == 0) {
        return "零".to_string();
    }
    const PLACES: [&str; 4] = ["", "十", "百", "千"];
    let mut out = String::new();
    let mut pending_zero = false;
    for (i, &digit) in values.iter().enumerate() {
        let place = values.len() - 1 - i;
        if digit == 0 {
            pending_zero = !out.is_empty();
            continue;
        }
        if pending_zero {
            out.push('零');
            pending_zero = false;
        }
        // 十五, not 一十五.
        if !(digit == 1 && place == 1 && out.is_empty()) {
            out.push(CHINESE_DIGITS[digit]);
        }
        out.push_str(PLACES[place]);
    }
    out
}

/// Words for 0..1000 ("one hundred twenty one").
fn english_number(number: u32) -> Vec<String> {
    const ONES: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: [&str; 10] = [
        "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    let mut words = Vec::new();
    let (hundreds, rest) = (number / 100, number % 100);
    if hundreds > 0 {
        words.extend([ONES[hundreds as usize], "hundred"]);
    }
    if rest >= 20 {
        words.push(TENS[(rest / 10) as usize]);
        if rest % 10 > 0 {
            words.push(ONES[(rest % 10) as usize]);
        }
    } else if rest > 0 || words.is_empty() {
        words.push(ONES[rest as usize]);
    }
    words.into_iter().map(String::from).collect()
}

/// Levenshtein distance: substitutions, insertions and deletions.
fn edit_distance(reference: &[String], hypothesis: &[String]) -> usize {
    let mut previous: Vec<usize> = (0..=hypothesis.len()).collect();
    for (i, r) in reference.iter().enumerate() {
        let mut current = vec![i + 1; hypothesis.len() + 1];
        for (j, h) in hypothesis.iter().enumerate() {
            let substitution = previous[j] + usize::from(r != h);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        previous = current;
    }
    previous[hypothesis.len()]
}

#[derive(Default)]
struct Totals {
    clips: usize,
    errors: usize,
    units: usize,
    unstable: usize,
    exact: usize,
    stt_time: Duration,
    audio: Duration,
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("stt_eval: {error}");
            return ExitCode::FAILURE;
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("stt_eval: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<(), String> {
    let clips = find_clips(args)?;
    if clips.is_empty() {
        return Err(format!(
            "no clips under {}: record some with eval/record.sh",
            args.corpus.display()
        ));
    }
    let mut languages: Vec<LanguageCode> = clips.iter().map(|clip| clip.language).collect();
    languages.sort_by_key(|language| language.code());
    languages.dedup();
    let speech = NativeSpeech::spawn_with(
        args.lib_dir.clone(),
        ModelPaths::new(args.model_dir.clone()),
        languages.clone(),
        args.trim,
        TtsSettings::default(),
    );
    // The first call to each model is slow; keep it out of the timings.
    for &language in &languages {
        speech
            .transcribe(&[0.0; STT_SAMPLE_RATE as usize], language)
            .map_err(|error| format!("loading the {} model: {error}", language.code()))?;
    }

    let report_path = args.corpus.join("report.tsv");
    let mut report = fs::File::create(&report_path)
        .map_err(|error| format!("{}: {error}", report_path.display()))?;
    let _ = writeln!(
        report,
        "condition\tlang\tclip\terrors\tunits\tstable\tstt_ms\treference\thypothesis"
    );

    let mut totals: BTreeMap<(String, &str), Totals> = BTreeMap::new();
    for clip in &clips {
        let (interleaved, rate, channels) = read_wav(&clip.path)?;
        let frames = interleaved.len() / usize::from(channels.max(1));
        let audio_length = Duration::from_secs_f64(frames as f64 / f64::from(rate.max(1)));
        let samples =
            CapturedAudio::new(interleaved, rate, channels, audio_length).into_stt_samples();
        let mut hypotheses = Vec::with_capacity(args.runs);
        let mut stt_time = Duration::ZERO;
        for _ in 0..args.runs {
            let started = Instant::now();
            let text = speech
                .transcribe(&samples, clip.language)
                .map_err(|error| format!("{}: {error}", clip.path.display()))?;
            stt_time += started.elapsed();
            hypotheses.push(text);
        }
        let stable = hypotheses.iter().all(|text| *text == hypotheses[0]);
        let reference = units(&clip.reference, clip.language);
        let hypothesis = units(&hypotheses[0], clip.language);
        let errors = edit_distance(&reference, &hypothesis);
        let average = stt_time / args.runs as u32;

        let entry = totals
            .entry((clip.condition.clone(), clip.language.code()))
            .or_default();
        entry.clips += 1;
        entry.errors += errors;
        entry.units += reference.len();
        entry.unstable += usize::from(!stable);
        entry.exact += usize::from(errors == 0);
        entry.stt_time += average;
        entry.audio += audio_length;

        let _ = writeln!(
            report,
            "{}\t{}\t{}\t{errors}\t{}\t{stable}\t{}\t{}\t{}",
            clip.condition,
            clip.language.code(),
            file_name(&clip.path),
            reference.len(),
            average.as_millis(),
            clip.reference,
            hypotheses[0].replace(['\t', '\n'], " "),
        );
        eprint!(".");
    }
    eprintln!();

    println!(
        "{:<20} {:<4} {:>5} {:>8} {:>7} {:>8} {:>9} {:>6}",
        "condition", "lang", "clips", "CER/WER", "exact", "unstable", "stt ms", "RTF"
    );
    for ((condition, lang), t) in &totals {
        let rate = 100.0 * t.errors as f64 / t.units.max(1) as f64;
        let rtf = t.stt_time.as_secs_f64() / t.audio.as_secs_f64().max(1e-9);
        println!(
            "{:<20} {:<4} {:>5} {:>7.1}% {:>7} {:>8} {:>9} {:>6.3}",
            condition,
            lang,
            t.clips,
            rate,
            format!("{}/{}", t.exact, t.clips),
            t.unstable,
            (t.stt_time / t.clips as u32).as_millis(),
            rtf,
        );
    }
    println!("per clip: {}", report_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_is_scored_by_character_ignoring_script_and_punctuation() {
        let reference = units("請問最近的火車站在哪裡？", LanguageCode::Zh);
        let hypothesis = units("请 问 最 近 的 火 车 站 在 哪 里", LanguageCode::Zh);
        assert_eq!(reference.len(), 11);
        assert_eq!(edit_distance(&reference, &hypothesis), 0);
        let wrong = units("请问最近的火车站在那", LanguageCode::Zh);
        // 哪 -> 那 substituted, 裡 deleted.
        assert_eq!(edit_distance(&reference, &wrong), 2);
    }

    #[test]
    fn english_is_scored_by_lower_case_word() {
        let reference = units("Excuse me, where is the station?", LanguageCode::En);
        let hypothesis = units("excuse me we are in the station", LanguageCode::En);
        assert_eq!(reference.len(), 6);
        // where -> we, is -> are, + in.
        assert_eq!(edit_distance(&reference, &hypothesis), 3);
        assert_eq!(
            units("I'd like it.", LanguageCode::En),
            ["i'd", "like", "it"]
        );
    }

    #[test]
    fn numbers_are_scored_as_words() {
        assert_eq!(
            units("Exchange 100 dollars at 7 or 21.", LanguageCode::En),
            units(
                "exchange one hundred dollars at seven or twenty-one",
                LanguageCode::En
            )
        );
        assert_eq!(units("007", LanguageCode::En), ["007"]);
        assert_eq!(
            units("我想换 1 0 0 美金", LanguageCode::Zh),
            units("我想換一百美金", LanguageCode::Zh)
        );
        for (digits, words) in [
            ("7", "七"),
            ("15", "十五"),
            ("20", "二十"),
            ("105", "一百零五"),
            ("1007", "一千零七"),
            ("1500", "一千五百"),
            ("0", "零"),
            ("0912", "零九一二"),
        ] {
            assert_eq!(chinese_number(digits), words, "{digits}");
        }
        assert_eq!(
            units("结帐", LanguageCode::Zh),
            units("結賬", LanguageCode::Zh)
        );
    }
}
