use crate::asr::engine::AsrEngine;
use crate::inject::clipboard;
use crate::inject::context::get_active_app;
use crate::polish::commands::{self, VoiceCommand};
use crate::polish::engine::PolishEngine;
use crate::polish::prompt;
use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;
use tokio::sync::mpsc;

pub static POLISH_ENABLED: AtomicBool = AtomicBool::new(true);

pub enum PipelineEvent {
    AudioSegment(Vec<f32>),
    Flush,
    Stop,
}

pub struct Orchestrator {
    pub event_tx: mpsc::UnboundedSender<PipelineEvent>,
}

impl Orchestrator {
    pub fn start(
        asr: Arc<AsrEngine>,
        polish: Option<Arc<PolishEngine>>,
        app_handle: tauri::AppHandle,
    ) -> Self {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<PipelineEvent>();

        tokio::spawn(async move {
            let mut pending: Option<tokio::task::JoinHandle<()>> = None;
            while let Some(event) = event_rx.recv().await {
                match event {
                    PipelineEvent::AudioSegment(audio) => {
                        if let Some(h) = pending.take() { let _ = h.await; }
                        let asr = asr.clone();
                        let polish = polish.clone();
                        let handle = app_handle.clone();
                        let _ = handle.emit("pipeline_state", "processing");
                        pending = Some(tokio::task::spawn_blocking(move || {
                            if let Err(e) = process_segment(&asr, polish.as_deref(), &audio) {
                                tracing::error!("Pipeline error: {}", e);
                            }
                            let _ = handle.emit("pipeline_state", "idle");
                        }));
                    }
                    PipelineEvent::Stop => {
                        if let Some(h) = pending.take() { let _ = h.await; }
                        break;
                    }
                    PipelineEvent::Flush => {}
                }
            }
        });

        Self { event_tx }
    }
}

fn process_segment(asr: &AsrEngine, polish: Option<&PolishEngine>, audio: &[f32]) -> Result<()> {
    let start = std::time::Instant::now();

    let raw_text = asr.transcribe(audio)?;
    tracing::info!("ASR ({:?}): {}", start.elapsed(), &raw_text);

    if raw_text.is_empty() || raw_text.starts_with('[') || raw_text.starts_with('(') {
        return Ok(());
    }

    let cmd = commands::parse_command(&raw_text);
    if let Some(text) = commands::command_text(&cmd) {
        clipboard::inject_text(text)?;
        return Ok(());
    }

    let use_polish = POLISH_ENABLED.load(Ordering::Relaxed);

    let final_text = match (&cmd, polish) {
        (VoiceCommand::None(text), Some(engine)) if use_polish => {
            let ctx = get_active_app();
            let sys_prompt = prompt::build_system_prompt(&ctx, &[]);
            match engine.generate(&sys_prompt, text, 256) {
                Ok(polished) => polished,
                Err(e) => {
                    tracing::warn!("LLM polish failed, using raw: {}", e);
                    text.clone()
                }
            }
        }
        (VoiceCommand::None(text), _) => text.clone(),
        _ => return Ok(()),
    };

    tracing::info!("Total pipeline ({:?}): {}", start.elapsed(), &final_text);
    clipboard::inject_text(&final_text)?;
    Ok(())
}
