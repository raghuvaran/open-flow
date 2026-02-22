use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

pub struct PolishEngine {
    model_path: PathBuf,
    server: Mutex<Option<Child>>,
    port: u16,
}

impl PolishEngine {
    pub fn new(model_path: &Path) -> Result<Self> {
        if !model_path.exists() {
            anyhow::bail!("LLM model not found: {}", model_path.display());
        }
        let port = 8384;
        let engine = Self {
            model_path: model_path.to_path_buf(),
            server: Mutex::new(None),
            port,
        };
        engine.ensure_server()?;
        Ok(engine)
    }

    fn find_llama_server() -> Option<String> {
        // Check our bundled copy first
        let bundled = crate::config::AppConfig::default()
            .models_dir.join("llama-server");
        if bundled.exists() { return Some(bundled.to_string_lossy().into()); }

        let candidates = [
            "/opt/homebrew/bin/llama-server",
            "/usr/local/bin/llama-server",
        ];
        for p in candidates {
            if std::path::Path::new(p).exists() { return Some(p.to_string()); }
        }
        Command::new("which").arg("llama-server")
            .output().ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    fn ensure_server(&self) -> Result<()> {
        let mut guard = self.server.lock().unwrap();

        if let Some(ref mut child) = *guard {
            if child.try_wait()?.is_none() {
                return Ok(());
            }
        }

        // Kill any orphaned llama-server on our port from a previous crash
        let _ = Command::new("sh")
            .args(["-c", &format!("lsof -ti tcp:{} | xargs kill -9 2>/dev/null", self.port)])
            .status();

        let bin = Self::find_llama_server()
            .ok_or_else(|| anyhow::anyhow!("llama-server not found"))?;

        tracing::info!("Starting llama-server on port {}...", self.port);
        let child = Command::new(&bin)
            .args([
                "-m", self.model_path.to_str().unwrap(),
                "--port", &self.port.to_string(),
                "-ngl", "99",
                "--log-disable",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();

        match child {
            Ok(c) => {
                *guard = Some(c);
                drop(guard);
                // Wait for server to be ready (max 30s)
                let url = format!("http://127.0.0.1:{}/health", self.port);
                for _ in 0..60 {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    if ureq::get(&url).call().is_ok() {
                        tracing::info!("llama-server ready");
                        return Ok(());
                    }
                }
                anyhow::bail!("llama-server failed to start within 30s")
            }
            Err(e) => anyhow::bail!("Failed to start llama-server: {}", e),
        }
    }

    pub fn generate(&self, system_prompt: &str, user_text: &str, max_tokens: i32) -> Result<String> {
        self.ensure_server()?;

        let url = format!("http://127.0.0.1:{}/v1/chat/completions", self.port);
        let body = serde_json::json!({
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_text}
            ],
            "max_tokens": max_tokens,
            "temperature": 0.1,
            "stream": false
        });

        let mut resp = ureq::post(&url)
            .header("Content-Type", "application/json")
            .send(&serde_json::to_vec(&body)?)?;

        let body_str = resp.body_mut().read_to_string()?;
        let json: serde_json::Value = serde_json::from_str(&body_str)?;
        let text = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();

        if text.is_empty() {
            anyhow::bail!("LLM returned empty response")
        }
        Ok(text)
    }
}

impl Drop for PolishEngine {
    fn drop(&mut self) {
        if let Some(ref mut child) = *self.server.lock().unwrap() {
            let _ = child.kill();
        }
    }
}
