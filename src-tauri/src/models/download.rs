use anyhow::Result;
use std::io::Write;
use std::path::Path;
use tauri::{AppHandle, Emitter};

struct ModelInfo {
    name: &'static str,
    url: &'static str,
    filename: &'static str,
}

const LLAMA_SERVER_URL: &str = "https://github.com/ggml-org/llama.cpp/releases/download/b8123/llama-b8123-bin-macos-arm64.tar.gz";
pub const LLAMA_SERVER_FILENAME: &str = "llama-server";

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

    // Download llama-server if not present
    let server_dest = models_dir.join(LLAMA_SERVER_FILENAME);
    if !server_dest.exists() {
        tracing::info!("Downloading llama-server ...");
        let _ = app.emit("download_progress", DownloadProgress {
            model: "llama-server".to_string(),
            downloaded: 0, total: 0,
            model_index: MODELS.len(), model_count: MODELS.len() + 1,
        });

        let resp = ureq::get(LLAMA_SERVER_URL).call()
            .map_err(|e| anyhow::anyhow!("Download failed for llama-server: {}", e))?;

        let total: u64 = resp.headers().get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        let tmp_tar = models_dir.join("llama-server.tar.gz");
        let mut file = std::fs::File::create(&tmp_tar)?;
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
                    model: "llama-server".to_string(),
                    downloaded, total,
                    model_index: MODELS.len(), model_count: MODELS.len() + 1,
                });
                last_emit = std::time::Instant::now();
            }
        }
        file.flush()?;
        drop(file);

        // Extract all files from tarball (server + dylibs)
        let status = std::process::Command::new("tar")
            .args(["xzf", tmp_tar.to_str().unwrap(), "--strip-components=1"])
            .current_dir(models_dir)
            .status()?;
        let _ = std::fs::remove_file(&tmp_tar);
        if !status.success() {
            anyhow::bail!("Failed to extract llama-server from tarball");
        }
        // Remove extra binaries we don't need, clean xattrs
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for entry in std::fs::read_dir(models_dir)? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !name.contains('.') && name != "llama-server" && entry.metadata()?.permissions().mode() & 0o111 != 0 {
                    let _ = std::fs::remove_file(entry.path());
                } else if name == "llama-server" || name.ends_with(".dylib") {
                    let _ = std::process::Command::new("xattr").args(["-c"]).arg(entry.path()).status();
                }
            }
            std::fs::set_permissions(&server_dest, std::fs::Permissions::from_mode(0o755))?;
        }
        tracing::info!("llama-server installed ({:.1} MB)", downloaded as f64 / 1_048_576.0);
    }

    Ok(())
}
