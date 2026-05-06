use anyhow::Result;
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct AsrEngine {
    ctx: WhisperContext,
}

/// Seed vocabulary biasing Whisper toward common software/tech terms. Kept short
/// because whisper's initial prompt is capped at ~224 tokens and longer prompts
/// bias decoding toward prompt-style phrasing.
const TECH_VOCAB_SEED: &str = "Kubernetes, Docker, React, Svelte, Tauri, Rust, TypeScript, JavaScript, Python, Go, gRPC, REST, JSON, YAML, SQL, PostgreSQL, Redis, GraphQL, OAuth, JWT, API, CLI, SDK, IDE, LLM, GPU, CPU, async, await, GitHub, PR, CI, CD, CoreAudio, Metal, whisper.cpp, llama.cpp.";

impl AsrEngine {
    pub fn new(model_path: &Path) -> Result<Self> {
        let mut params = WhisperContextParameters::default();
        params.use_gpu(true);

        let ctx = WhisperContext::new_with_params(
            model_path.to_str().unwrap(),
            params,
        )
        .map_err(|e| anyhow::anyhow!("Failed to load whisper model: {}", e))?;
        Ok(Self { ctx })
    }

    pub fn transcribe(&self, audio: &[f32]) -> Result<String> {
        self.transcribe_with_vocab(audio, &[])
    }

    /// Transcribe with an optional personal dictionary biasing Whisper's decoding.
    /// `personal_dict` entries may be bare terms or "spoken → written" pairs.
    pub fn transcribe_with_vocab(&self, audio: &[f32], personal_dict: &[String]) -> Result<String> {
        let mut state = self.ctx.create_state()
            .map_err(|e| anyhow::anyhow!("Failed to create state: {}", e))?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("en"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        params.set_no_timestamps(true);

        let prompt = build_initial_prompt(personal_dict);
        params.set_initial_prompt(&prompt);

        state.full(params, audio)
            .map_err(|e| anyhow::anyhow!("Transcription failed: {}", e))?;

        let n = state.full_n_segments()
            .map_err(|e| anyhow::anyhow!("Failed to get segments: {}", e))?;
        let mut text = String::new();
        for i in 0..n {
            if let Ok(seg) = state.full_get_segment_text(i) {
                text.push_str(&seg);
            }
        }
        Ok(text.trim().to_string())
    }
}

fn build_initial_prompt(personal_dict: &[String]) -> String {
    let mut terms: Vec<&str> = Vec::new();
    for entry in personal_dict {
        // Accept "spoken → written" pairs from the dictionary module — prefer the
        // written form since that's what we want Whisper to learn to emit.
        let written = entry.split('→').next_back().unwrap_or(entry).trim();
        if !written.is_empty() {
            terms.push(written);
        }
    }
    if terms.is_empty() {
        TECH_VOCAB_SEED.to_string()
    } else {
        format!("{} {}.", TECH_VOCAB_SEED, terms.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    fn asr_model_path() -> std::path::PathBuf {
        let cfg = AppConfig::default();
        let base = cfg.models_dir.join("ggml-base.bin");
        if base.exists() { base } else { cfg.models_dir.join("ggml-small.bin") }
    }

    #[test]
    #[ignore] // requires whisper model
    fn transcribe_silence_returns_empty() {
        let engine = AsrEngine::new(&asr_model_path()).unwrap();
        let silence = vec![0.0f32; 16000]; // 1 second of silence
        let text = engine.transcribe(&silence).unwrap();
        // Whisper on silence typically returns empty or bracketed noise markers
        assert!(text.is_empty() || text.starts_with('[') || text.starts_with('('));
    }

    #[test]
    #[ignore] // requires whisper model
    fn transcribe_returns_string() {
        let engine = AsrEngine::new(&asr_model_path()).unwrap();
        // Generate a 2-second 440Hz tone — won't produce real words but tests the pipeline
        let audio: Vec<f32> = (0..32000).map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16000.0).sin() * 0.5).collect();
        let text = engine.transcribe(&audio).unwrap();
        // Just verify it doesn't crash and returns a string
        assert!(text.len() < 10000);
    }
}
