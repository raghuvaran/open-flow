//! Experiment harness: sweep Whisper configs × fixtures, measure WER and
//! latency, print a ranked table at the end. Run with:
//!
//!     cargo test --release --test e2e_experiments run_experiments -- --ignored --nocapture
//!
//! Each row is one (model, beam, threads, prompt) × fixture combo, repeated
//! N_RUNS times for timing stability.

use openflow_lib::asr::engine::{AsrEngine, AsrOpts};
use openflow_lib::config::AppConfig;
use openflow_lib::polish::engine::PolishEngine;
use openflow_lib::polish::prompt;
use openflow_lib::inject::context::AppContext;
use std::path::{Path, PathBuf};
use std::time::Instant;

const N_RUNS: usize = 5;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn models_dir() -> PathBuf {
    AppConfig::default().models_dir
}

fn load_wav_16k_mono(path: &Path) -> Vec<f32> {
    let mut r = hound::WavReader::open(path)
        .unwrap_or_else(|e| panic!("open {}: {}", path.display(), e));
    let spec = r.spec();
    assert_eq!(spec.sample_rate, 16_000);
    assert_eq!(spec.channels, 1);
    assert_eq!(spec.bits_per_sample, 16);
    r.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect()
}

/// Tokenize for WER: lowercase, strip punctuation, collapse whitespace.
/// Keeps intra-word characters like `/` and `.` when flanked by alphanumerics
/// (so "p0.md" and "docs/requirements" survive as single tokens).
fn tokens(s: &str) -> Vec<String> {
    let s = s.to_lowercase();
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = s.chars().collect();
    for i in 0..chars.len() {
        let c = chars[i];
        let is_alnum = c.is_alphanumeric();
        let is_intra = (c == '/' || c == '.' || c == '-' || c == '_')
            && i > 0 && i + 1 < chars.len()
            && chars[i - 1].is_alphanumeric() && chars[i + 1].is_alphanumeric();
        if is_alnum || is_intra {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() { out.push(cur); }
    out
}

/// Classic Levenshtein on token sequences → WER = edits / reference_len.
fn wer(reference: &str, hypothesis: &str) -> f64 {
    let r = tokens(reference);
    let h = tokens(hypothesis);
    if r.is_empty() { return if h.is_empty() { 0.0 } else { 1.0 }; }
    let n = r.len();
    let m = h.len();
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        cur[0] = i;
        for j in 1..=m {
            let cost = if r[i - 1] == h[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m] as f64 / n as f64
}

struct Fixture {
    name: String,
    audio: Vec<f32>,
    duration_s: f64,
    reference: String,
}

fn load_fixtures() -> Vec<Fixture> {
    let dir = fixtures_dir();
    let mut out = vec![];
    if !dir.exists() { return out; }
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = entry.path();
        if !p.is_dir() { continue; }
        let wav = p.join("audio.wav");
        if !wav.exists() { continue; }
        // Prefer reference.txt over expect.txt so users can refine without
        // touching the substring-assertion file used by e2e_all_fixtures.
        let ref_path = if p.join("reference.txt").exists() {
            p.join("reference.txt")
        } else {
            p.join("expect.txt")
        };
        let reference = std::fs::read_to_string(&ref_path)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#') && !l.trim().starts_with('!'))
            .collect::<Vec<_>>()
            .join(" ");
        if reference.is_empty() { continue; }
        let audio = load_wav_16k_mono(&wav);
        let duration_s = audio.len() as f64 / 16_000.0;
        out.push(Fixture {
            name: p.file_name().unwrap().to_string_lossy().to_string(),
            audio,
            duration_s,
            reference,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

struct Config {
    label: String,
    model_file: &'static str,
    opts: AsrOpts,
}

fn configs() -> Vec<Config> {
    // Each (model, beam, threads, prompt). Kept deliberately small; expand if
    // a clear winner emerges.
    vec![
        // Baseline today
        Config { label: "base-greedy-p".into(),         model_file: "ggml-base.bin",                opts: AsrOpts { beam_size: None,    n_threads: None,    use_initial_prompt: true  } },
        // Ablate initial prompt on base
        Config { label: "base-greedy-NOp".into(),       model_file: "ggml-base.bin",                opts: AsrOpts { beam_size: None,    n_threads: None,    use_initial_prompt: false } },
        // Beam 5 on base (quality-for-latency trade)
        Config { label: "base-beam5-p".into(),          model_file: "ggml-base.bin",                opts: AsrOpts { beam_size: Some(5), n_threads: None,    use_initial_prompt: true  } },
        // small.en greedy + prompt (expected accuracy win)
        Config { label: "small.en-greedy-p".into(),     model_file: "ggml-small.en-q5_1.bin",       opts: AsrOpts { beam_size: None,    n_threads: None,    use_initial_prompt: true  } },
        // small.en beam 5
        Config { label: "small.en-beam5-p".into(),      model_file: "ggml-small.en-q5_1.bin",       opts: AsrOpts { beam_size: Some(5), n_threads: None,    use_initial_prompt: true  } },
        // small.en greedy, 8 threads
        Config { label: "small.en-greedy-p-t8".into(),  model_file: "ggml-small.en-q5_1.bin",       opts: AsrOpts { beam_size: None,    n_threads: Some(8), use_initial_prompt: true  } },
        // large-v3-turbo — upper bound on accuracy, latency check
        Config { label: "turbo-greedy-p".into(),        model_file: "ggml-large-v3-turbo-q5_0.bin", opts: AsrOpts { beam_size: None,    n_threads: None,    use_initial_prompt: true  } },
        Config { label: "turbo-greedy-NOp".into(),      model_file: "ggml-large-v3-turbo-q5_0.bin", opts: AsrOpts { beam_size: None,    n_threads: None,    use_initial_prompt: false } },
        Config { label: "turbo-greedy-NOp-t8".into(),   model_file: "ggml-large-v3-turbo-q5_0.bin", opts: AsrOpts { beam_size: None,    n_threads: Some(8), use_initial_prompt: false } },
        Config { label: "turbo-beam5-p".into(),         model_file: "ggml-large-v3-turbo-q5_0.bin", opts: AsrOpts { beam_size: Some(5), n_threads: None,    use_initial_prompt: true  } },
    ]
}

struct Row {
    config: String,
    fixture: String,
    wer_raw: f64,
    wer_polished: f64,
    asr_ms_median: u128,
    polish_ms_median: u128,
    total_ms_median: u128,
    real_time_factor: f64,
    raw: String,
    polished: String,
}

fn median(mut xs: Vec<u128>) -> u128 {
    xs.sort();
    xs[xs.len() / 2]
}

#[test]
#[ignore]
fn run_experiments() {
    let fixtures = load_fixtures();
    assert!(!fixtures.is_empty(), "no fixtures with audio.wav + reference/expect.txt");

    let configs = configs();

    // One polish engine — it doesn't depend on the whisper model.
    let llm_path = models_dir().join("qwen2.5-3b-instruct-q4_k_m.gguf");
    let polish = PolishEngine::new(&llm_path).expect("load llm");
    let ctx = AppContext {
        app_name: "Test".into(), bundle_id: String::new(),
        category: "default".into(), tone: "Natural".into(),
        window_title: String::new(), selected_text: String::new(),
    };
    let sys_prompt = prompt::build_system_prompt(&ctx, &[]);

    let mut rows: Vec<Row> = Vec::new();

    for cfg in &configs {
        let model_path = models_dir().join(cfg.model_file);
        if !model_path.exists() {
            eprintln!("[skip] {} — model file not present: {}", cfg.label, model_path.display());
            continue;
        }
        eprintln!("\n=== Loading {} ===", cfg.label);
        let load_t = Instant::now();
        let asr = match AsrEngine::new(&model_path) {
            Ok(e) => e,
            Err(e) => { eprintln!("[skip] {} — load failed: {}", cfg.label, e); continue; }
        };
        eprintln!("    loaded in {:?}", load_t.elapsed());

        for fx in &fixtures {
            // Warmup: Metal shaders + KV cache init on first call.
            let _ = asr.transcribe_with_opts(&fx.audio, &[], &cfg.opts);

            let mut asr_times = Vec::with_capacity(N_RUNS);
            let mut polish_times = Vec::with_capacity(N_RUNS);
            let mut total_times = Vec::with_capacity(N_RUNS);
            let mut last_raw = String::new();
            let mut last_polished = String::new();

            for _ in 0..N_RUNS {
                let t0 = Instant::now();
                let raw = match asr.transcribe_with_opts(&fx.audio, &[], &cfg.opts) {
                    Ok(r) => r,
                    Err(e) => { eprintln!("ASR err on {}/{}: {}", cfg.label, fx.name, e); continue; }
                };
                let asr_t = t0.elapsed().as_millis();
                let t1 = Instant::now();
                let polished = polish.generate(&sys_prompt, &raw, 256).unwrap_or_else(|_| raw.clone());
                let polish_t = t1.elapsed().as_millis();
                let total_t = t0.elapsed().as_millis();
                asr_times.push(asr_t);
                polish_times.push(polish_t);
                total_times.push(total_t);
                last_raw = raw;
                last_polished = polished;
            }

            if asr_times.is_empty() { continue; }
            let asr_med = median(asr_times);
            let pol_med = median(polish_times);
            let tot_med = median(total_times);
            let wer_raw = wer(&fx.reference, &last_raw);
            let wer_pol = wer(&fx.reference, &last_polished);
            let rtf = asr_med as f64 / (fx.duration_s * 1000.0);

            println!(
                "RESULT config={} fixture={} asr_ms={} polish_ms={} total_ms={} wer_raw={:.3} wer_polished={:.3} audio_s={:.1} rtf={:.2}",
                cfg.label, fx.name, asr_med, pol_med, tot_med, wer_raw, wer_pol, fx.duration_s, rtf,
            );

            rows.push(Row {
                config: cfg.label.clone(),
                fixture: fx.name.clone(),
                wer_raw, wer_polished: wer_pol,
                asr_ms_median: asr_med,
                polish_ms_median: pol_med,
                total_ms_median: tot_med,
                real_time_factor: rtf,
                raw: last_raw, polished: last_polished,
            });
        }

        // Explicitly drop the whisper context before loading the next one —
        // Metal buffers sometimes linger briefly otherwise and compound into
        // an OOM kill when sweeping many models on an 8GB-ish workload.
        drop(asr);
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    // ---------- Summary tables ----------

    println!("\n======== RAW TRANSCRIPTS (last run of each) ========");
    for r in &rows {
        println!("[{}/{}]", r.config, r.fixture);
        println!("    RAW:      {}", r.raw);
        println!("    POLISHED: {}", r.polished);
    }

    println!("\n======== PER-CONFIG AVERAGES (across fixtures) ========");
    println!("{:<28} {:>10} {:>10} {:>10} {:>10} {:>8}",
             "config", "asr_ms", "total_ms", "wer_raw", "wer_pol", "rtf");
    let mut by_config: std::collections::BTreeMap<String, Vec<&Row>> = Default::default();
    for r in &rows {
        by_config.entry(r.config.clone()).or_default().push(r);
    }
    let mut agg: Vec<(String, f64, f64, f64, f64, f64)> = by_config.iter().map(|(k, v)| {
        let n = v.len() as f64;
        let asr = v.iter().map(|r| r.asr_ms_median as f64).sum::<f64>() / n;
        let total = v.iter().map(|r| r.total_ms_median as f64).sum::<f64>() / n;
        let wer_r = v.iter().map(|r| r.wer_raw).sum::<f64>() / n;
        let wer_p = v.iter().map(|r| r.wer_polished).sum::<f64>() / n;
        let rtf = v.iter().map(|r| r.real_time_factor).sum::<f64>() / n;
        (k.clone(), asr, total, wer_r, wer_p, rtf)
    }).collect();
    // Sort by wer_raw ascending (best accuracy first).
    agg.sort_by(|a, b| a.3.partial_cmp(&b.3).unwrap());
    for (label, asr, total, wer_r, wer_p, rtf) in &agg {
        println!("{:<28} {:>10.0} {:>10.0} {:>10.3} {:>10.3} {:>8.2}",
                 label, asr, total, wer_r, wer_p, rtf);
    }

    println!("\n======== PARETO NOTES ========");
    // Flag the lowest-WER config, the fastest-ASR config, and a balanced pick.
    if let Some(best_acc) = agg.iter().min_by(|a, b| a.3.partial_cmp(&b.3).unwrap()) {
        println!("best accuracy   : {} (wer_raw={:.3}, asr_ms={:.0})", best_acc.0, best_acc.3, best_acc.1);
    }
    if let Some(best_lat) = agg.iter().min_by(|a, b| a.1.partial_cmp(&b.1).unwrap()) {
        println!("fastest ASR     : {} (asr_ms={:.0}, wer_raw={:.3})", best_lat.0, best_lat.1, best_lat.3);
    }
    // Balanced: min(wer_raw + asr_ms / 5000) — weighting accuracy heavily but
    // penalizing absurd latency. Adjust if your preferences differ.
    if let Some(bal) = agg.iter().min_by(|a, b| {
        let sa = a.3 + a.1 / 5000.0;
        let sb = b.3 + b.1 / 5000.0;
        sa.partial_cmp(&sb).unwrap()
    }) {
        println!("balanced pick   : {} (wer_raw={:.3}, asr_ms={:.0})", bal.0, bal.3, bal.1);
    }
}
