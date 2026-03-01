use anyhow::Result;
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct AsrEngine {
    ctx: WhisperContext,
}

impl AsrEngine {
    pub fn new(model_path: &Path) -> Result<Self> {
        let mut params = WhisperContextParameters::default();
        params.use_gpu(false);

        let ctx = WhisperContext::new_with_params(
            model_path.to_str().unwrap(),
            params,
        )
        .map_err(|e| anyhow::anyhow!("Failed to load whisper model: {}", e))?;
        Ok(Self { ctx })
    }

    pub fn transcribe(&self, audio: &[f32]) -> Result<String> {
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
