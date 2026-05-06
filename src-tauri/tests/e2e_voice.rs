//! End-to-end voice → transcript tests. Reads WAV fixtures from
//! `tests/fixtures/` and runs them through the real ASR + polish stack.
//!
//! All tests are `#[ignore]` — they need models, `llama-server`, and WAV
//! fixtures to be present. Run with:
//!
//!     cargo test --release --test e2e_voice -- --ignored --nocapture
//!
//! Use `--nocapture` so the structured timing lines print. See
//! `tests/fixtures/README.md` for how to record fixtures with your own voice.

use openflow_lib::asr::engine::AsrEngine;
use openflow_lib::audio::chunker::Chunker;
use openflow_lib::audio::vad::SileroVad;
use openflow_lib::config::AppConfig;
use openflow_lib::polish::engine::PolishEngine;
use openflow_lib::polish::prompt;
use openflow_lib::inject::context::AppContext;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn models_dir() -> PathBuf {
    AppConfig::default().models_dir
}

/// Decode a 16-bit PCM WAV to mono f32 at 16 kHz. Fails loudly if the fixture
/// isn't already in that format — recording scripts produce it natively.
fn load_wav_16k_mono(path: &Path) -> Vec<f32> {
    let reader = hound::WavReader::open(path)
        .unwrap_or_else(|e| panic!("open {}: {}", path.display(), e));
    let spec = reader.spec();
    assert_eq!(spec.sample_rate, 16_000, "{} must be 16kHz (got {})", path.display(), spec.sample_rate);
    assert_eq!(spec.channels, 1, "{} must be mono (got {})", path.display(), spec.channels);
    assert_eq!(spec.bits_per_sample, 16, "{} must be 16-bit PCM", path.display());

    let mut r = reader;
    r.samples::<i16>()
        .map(|s| s.unwrap() as f32 / 32768.0)
        .collect()
}

fn asr_model() -> PathBuf {
    let base = models_dir().join("ggml-base.bin");
    if base.exists() { base } else { models_dir().join("ggml-small.bin") }
}

fn llm_model() -> PathBuf {
    models_dir().join("qwen2.5-3b-instruct-q4_k_m.gguf")
}

fn vad_model() -> PathBuf {
    models_dir().join("silero_vad.onnx")
}

/// Discover fixtures. Each fixture is a directory containing `audio.wav` and
/// an optional `expect.txt` (substrings that MUST appear, one per line; lines
/// starting with `!` MUST NOT appear; lines starting with `#` are comments).
fn fixtures() -> Vec<PathBuf> {
    let dir = fixtures_dir();
    if !dir.exists() { return vec![]; }
    std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("audio.wav").exists())
        .collect()
}

struct Expectations {
    must_contain: Vec<String>,
    must_not_contain: Vec<String>,
}

fn load_expectations(fixture: &Path) -> Expectations {
    let p = fixture.join("expect.txt");
    if !p.exists() {
        return Expectations { must_contain: vec![], must_not_contain: vec![] };
    }
    let text = std::fs::read_to_string(&p).unwrap();
    let mut must = vec![];
    let mut must_not = vec![];
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if let Some(rest) = line.strip_prefix('!') {
            must_not.push(rest.trim().to_lowercase());
        } else {
            must.push(line.to_lowercase());
        }
    }
    Expectations { must_contain: must, must_not_contain: must_not }
}

fn print_timing(name: &str, asr_ms: u128, polish_ms: u128, total_ms: u128, words: usize, text: &str) {
    // Structured line so regressions are grep-able from CI logs.
    println!(
        "E2E fixture={name} asr_ms={asr_ms} polish_ms={polish_ms} total_ms={total_ms} words={words}"
    );
    println!("     -> {text}");
}

#[test]
#[ignore] // requires whisper model + llama-server + fixtures
fn e2e_all_fixtures() {
    let fixtures = fixtures();
    if fixtures.is_empty() {
        panic!(
            "no fixtures found in {} — see tests/fixtures/README.md to record some",
            fixtures_dir().display()
        );
    }

    let asr = AsrEngine::new(&asr_model()).expect("load whisper");
    let polish = PolishEngine::new(&llm_model()).expect("load llm (is llama-server installed?)");
    let ctx = AppContext {
        app_name: "Test".into(),
        bundle_id: String::new(),
        category: "default".into(),
        tone: "Natural".into(),
        window_title: String::new(),
        selected_text: String::new(),
    };
    let sys_prompt = prompt::build_system_prompt(&ctx, &[]);

    let mut failed = Vec::new();

    for fixture in &fixtures {
        let name = fixture.file_name().unwrap().to_string_lossy().to_string();
        let audio = load_wav_16k_mono(&fixture.join("audio.wav"));
        let expect = load_expectations(fixture);

        let t0 = Instant::now();
        let raw = asr.transcribe_with_vocab(&audio, &[]).expect("transcribe");
        let asr_ms = t0.elapsed().as_millis();

        let t1 = Instant::now();
        let polished = polish.generate(&sys_prompt, &raw, 256)
            .unwrap_or_else(|e| { eprintln!("polish failed: {e}"); raw.clone() });
        let polish_ms = t1.elapsed().as_millis();

        let total_ms = t0.elapsed().as_millis();
        let words = polished.split_whitespace().count();
        print_timing(&name, asr_ms, polish_ms, total_ms, words, &polished);

        let lower = polished.to_lowercase();
        for needle in &expect.must_contain {
            if !lower.contains(needle) {
                failed.push(format!("{name}: MISSING \"{needle}\" in: {polished}"));
            }
        }
        for needle in &expect.must_not_contain {
            if lower.contains(needle) {
                failed.push(format!("{name}: UNEXPECTED \"{needle}\" in: {polished}"));
            }
        }
    }

    if !failed.is_empty() {
        panic!("{} assertion failures:\n{}", failed.len(), failed.join("\n"));
    }
}

/// Microbenchmark: runs the first fixture N times and prints p50/p95 per stage.
/// Useful for catching regressions after changing models or flags.
#[test]
#[ignore]
fn e2e_benchmark() {
    const RUNS: usize = 5;
    let fixtures = fixtures();
    let fixture = fixtures.first().expect("need at least one fixture");
    let audio = load_wav_16k_mono(&fixture.join("audio.wav"));

    let asr = AsrEngine::new(&asr_model()).expect("load whisper");
    let polish = PolishEngine::new(&llm_model()).expect("load llm");
    let ctx = AppContext {
        app_name: "Test".into(), bundle_id: String::new(),
        category: "default".into(), tone: "Natural".into(),
        window_title: String::new(), selected_text: String::new(),
    };
    let sys_prompt = prompt::build_system_prompt(&ctx, &[]);

    let mut asr_times = Vec::with_capacity(RUNS);
    let mut polish_times = Vec::with_capacity(RUNS);
    let mut total_times = Vec::with_capacity(RUNS);

    // Warmup: first run includes Metal shader compile + KV cache init.
    let _ = asr.transcribe_with_vocab(&audio, &[]);

    for _ in 0..RUNS {
        let t0 = Instant::now();
        let raw = asr.transcribe_with_vocab(&audio, &[]).unwrap();
        let asr_t = t0.elapsed();
        let t1 = Instant::now();
        let _ = polish.generate(&sys_prompt, &raw, 256).unwrap_or_default();
        let polish_t = t1.elapsed();
        let total = t0.elapsed();
        asr_times.push(asr_t);
        polish_times.push(polish_t);
        total_times.push(total);
    }

    fn summarize(label: &str, mut times: Vec<Duration>) {
        times.sort();
        let p50 = times[times.len() / 2].as_millis();
        let p95_idx = ((times.len() as f64) * 0.95) as usize;
        let p95 = times[p95_idx.min(times.len() - 1)].as_millis();
        let min = times.first().unwrap().as_millis();
        let max = times.last().unwrap().as_millis();
        println!("BENCH {label}: min={min}ms p50={p50}ms p95={p95}ms max={max}ms");
    }

    summarize("asr", asr_times);
    summarize("polish", polish_times);
    summarize("total", total_times);
}

/// Chunker e2e: feed a WAV with speech+pauses through Silero VAD and verify
/// we get the expected segment count. Fixture must have `expect_segments.txt`
/// containing a single integer.
#[test]
#[ignore]
fn e2e_chunker_segments() {
    let fixture_dir = fixtures_dir().join("chunker_pauses");
    let wav = fixture_dir.join("audio.wav");
    let expect_file = fixture_dir.join("expect_segments.txt");
    if !wav.exists() || !expect_file.exists() {
        panic!(
            "need {} and {} — see tests/fixtures/README.md",
            wav.display(), expect_file.display()
        );
    }
    let expected: usize = std::fs::read_to_string(&expect_file).unwrap()
        .trim().parse().expect("expect_segments.txt must be an integer");

    let audio = load_wav_16k_mono(&wav);
    let mut vad = SileroVad::new(&vad_model(), 0.5).expect("load vad");
    let mut chunker = Chunker::new(AppConfig::default().silence_threshold_ms);

    // Silero expects 30ms frames at 16kHz = 480 samples.
    let mut segments = 0;
    for frame in audio.chunks(480) {
        if frame.len() < 480 { break; }
        let is_speech = vad.is_speech(frame).unwrap_or(false);
        if chunker.feed(frame, is_speech).is_some() {
            segments += 1;
        }
    }
    if chunker.flush().is_some() {
        segments += 1;
    }

    assert_eq!(
        segments, expected,
        "chunker split into {segments} segments, expected {expected}"
    );
}
