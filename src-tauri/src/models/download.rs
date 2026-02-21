use anyhow::Result;
use std::io::Write;
use std::path::Path;
use tauri::{AppHandle, Emitter};

struct ModelInfo {
    name: &'static str,
    url: &'static str,
    filename: &'static str,
}

const MODELS: &[ModelInfo] = &[
    ModelInfo {
        name: "Silero VAD",
        url: "https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx",
        filename: "silero_vad.onnx",
    },
    ModelInfo {
        name: "Whisper Base",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
        filename: "ggml-base.bin",
    },
    ModelInfo {
        name: "Qwen 2.5 3B",
        url: "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf",
        filename: "qwen2.5-3b-instruct-q4_k_m.gguf",
    },
];

#[derive(Clone, serde::Serialize)]
struct DownloadProgress {
    model: String,
    downloaded: u64,
    total: u64,
    model_index: usize,
    model_count: usize,
}

pub fn download_missing(models_dir: &Path, app: &AppHandle) -> Result<()> {
    let _ = std::fs::create_dir_all(models_dir);

    for (i, model) in MODELS.iter().enumerate() {
        let dest = models_dir.join(model.filename);
        if dest.exists() {
            tracing::info!("{} already present", model.name);
            continue;
        }

        tracing::info!("Downloading {} ...", model.name);
        let resp = ureq::get(model.url).call()
            .map_err(|e| anyhow::anyhow!("Download failed for {}: {}", model.name, e))?;

        let total: u64 = resp.headers().get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        let tmp = dest.with_extension("part");
        let mut file = std::fs::File::create(&tmp)?;
        let mut reader = resp.into_body().into_reader();
        let mut downloaded: u64 = 0;
        let mut buf = vec![0u8; 256 * 1024];
        let mut last_emit = std::time::Instant::now();

        loop {
            let n = std::io::Read::read(&mut reader, &mut buf)?;
            if n == 0 { break; }
            file.write_all(&buf[..n])?;
            downloaded += n as u64;

            if last_emit.elapsed().as_millis() > 200 {
                let _ = app.emit("download_progress", DownloadProgress {
                    model: model.name.to_string(),
                    downloaded,
                    total,
                    model_index: i,
                    model_count: MODELS.len(),
                });
                last_emit = std::time::Instant::now();
            }
        }
        file.flush()?;
        std::fs::rename(&tmp, &dest)?;
        tracing::info!("{} downloaded ({:.1} MB)", model.name, downloaded as f64 / 1_048_576.0);
    }

    Ok(())
}
