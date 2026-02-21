use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct AppConfig {
    pub models_dir: PathBuf,
    pub db_path: PathBuf,
    pub silence_threshold_ms: u64,
    pub sample_rate: u32,
}

impl Default for AppConfig {
    fn default() -> Self {
        let base = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("openflow");
        Self {
            models_dir: base.join("models"),
            db_path: base.join("openflow.db"),
            silence_threshold_ms: 700,
            sample_rate: 16000,
        }
    }
}
