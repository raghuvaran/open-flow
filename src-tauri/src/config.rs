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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_paths() {
        let c = AppConfig::default();
        assert!(c.models_dir.to_string_lossy().contains("openflow"));
        assert!(c.db_path.to_string_lossy().ends_with("openflow.db"));
        assert_eq!(c.silence_threshold_ms, 700);
        assert_eq!(c.sample_rate, 16000);
    }

    #[test]
    fn models_dir_is_subdir_of_db_parent() {
        let c = AppConfig::default();
        let db_parent = c.db_path.parent().unwrap();
        let models_parent = c.models_dir.parent().unwrap();
        assert_eq!(db_parent, models_parent);
    }
}
