pub mod audio;
pub mod asr;
pub mod polish;
pub mod inject;
pub mod db;
pub mod pipeline;
pub mod state;
pub mod config;
pub mod models;

use asr::engine::AsrEngine;
use audio::chunker::Chunker;
use audio::vad::SileroVad;
use config::AppConfig;
use db::{schema, settings};
use pipeline::orchestrator::{Orchestrator, PipelineEvent, POLISH_ENABLED};
use polish::engine::PolishEngine;
use state::AppState;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;
use tauri::Manager;
use tauri::menu::{MenuBuilder, MenuItemBuilder, CheckMenuItemBuilder, SubmenuBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::image::Image;
use tokio::sync::Mutex;
use tokio::sync::mpsc;

pub static WALKIE_TALKIE: AtomicBool = AtomicBool::new(false);
static LAST_DICTATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct AppResources {
    state: AppState,
    asr: Option<Arc<AsrEngine>>,
    polish: Option<Arc<PolishEngine>>,
    stop_tx: Option<mpsc::Sender<()>>,
    config: AppConfig,
}

type SharedResources = Arc<Mutex<AppResources>>;

async fn load_on_thread<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> Result<T, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .name("model-loader".into())
        .spawn(move || { let _ = tx.send(f()); })
        .map_err(|e| e.to_string())?;
    let result = rx.await.map_err(|e| e.to_string())?.map_err(|e| e.to_string());
    handle.join().map_err(|_| "Model loader thread panicked".to_string())?;
    result
}

fn launchd_plist_path() -> std::path::PathBuf {
    dirs::home_dir().unwrap().join("Library/LaunchAgents/com.openflow.app.plist")
}

fn set_autostart(enabled: bool) {
    let plist_path = launchd_plist_path();
    if enabled {
        let app_path = std::env::current_exe().unwrap_or_default();
        let plist = format!(
r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>com.openflow.app</string>
    <key>ProgramArguments</key><array><string>{}</string></array>
    <key>RunAtLoad</key><true/>
</dict>
</plist>"#, app_path.display());
        let _ = std::fs::write(&plist_path, plist);
    } else {
        let _ = std::fs::remove_file(&plist_path);
    }
}

fn is_autostart_enabled() -> bool {
    launchd_plist_path().exists()
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let config = AppConfig::default();
    let conn = schema::init_db(&config.db_path).ok();

    let polish_on = POLISH_ENABLED.load(Ordering::Relaxed);
    let walkie_on = WALKIE_TALKIE.load(Ordering::Relaxed);
    let autostart_on = is_autostart_enabled();

    let mic_granted = conn.as_ref()
        .and_then(|c| settings::get(c, "mic_granted").ok().flatten())
        .is_some();
    let mics = if mic_granted {
        audio::capture::list_input_devices().unwrap_or_default()
    } else {
        vec![]
    };
    let saved_mic = conn.as_ref()
        .and_then(|c| settings::get(c, "mic_device").ok().flatten())
        .unwrap_or_default();

    let show_item = MenuItemBuilder::new("Show OpenFlow")
        .id("show").build(app)?;
    let polish_item = CheckMenuItemBuilder::new("AI Polish")
        .id("polish").checked(polish_on).build(app)?;
    let walkie_item = CheckMenuItemBuilder::new("Walkie-Talkie Mode")
        .id("walkie").checked(walkie_on).build(app)?;
    let autostart_item = CheckMenuItemBuilder::new("Start on Login")
        .id("autostart").checked(autostart_on).build(app)?;

    let default_mic = CheckMenuItemBuilder::new("Default")
        .id("mic__default").checked(saved_mic.is_empty()).build(app)?;
    let mut mic_sub = SubmenuBuilder::new(app, "Microphone").item(&default_mic);
    let mut mic_items = vec![default_mic.clone()];
    for mic in &mics {
        let item = CheckMenuItemBuilder::new(mic.as_str())
            .id(format!("mic__{}", mic)).checked(*mic == saved_mic).build(app)?;
        mic_items.push(item.clone());
        mic_sub = mic_sub.item(&item);
    }
    let mic_menu = mic_sub.build()?;

    let quit_item = MenuItemBuilder::new("Quit OpenFlow")
        .id("quit").accelerator("CmdOrCtrl+Q").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&show_item)
        .separator()
        .item(&polish_item)
        .item(&walkie_item)
        .item(&autostart_item)
        .item(&mic_menu)
        .separator()
        .item(&quit_item)
        .build()?;

    let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))
        .expect("tray icon missing");

    let _tray = TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .tooltip("OpenFlow")
        .on_menu_event(move |app, event| {
            let id = event.id().0.as_str();
            match id {
                "quit" => { std::process::exit(0); }
                "show" => { let _ = app.emit("show_window", ()); }
                "polish" => {
                    let current = POLISH_ENABLED.load(Ordering::Relaxed);
                    POLISH_ENABLED.store(!current, Ordering::Relaxed);
                }
                "walkie" => {
                    let new_val = !WALKIE_TALKIE.load(Ordering::Relaxed);
                    WALKIE_TALKIE.store(new_val, Ordering::Relaxed);
                    let cfg = AppConfig::default();
                    if let Ok(c) = schema::init_db(&cfg.db_path) {
                        let _ = settings::set(&c, "walkie_talkie", if new_val { "1" } else { "0" });
                    }
                    let _ = app.emit("walkie_changed", new_val);
                }
                "autostart" => {
                    let now_on = !is_autostart_enabled();
                    set_autostart(now_on);
                }
                _ if id.starts_with("mic__") => {
                    let name = if id == "mic__default" { String::new() } else { id[5..].to_string() };
                    for item in &mic_items {
                        let _ = item.set_checked(item.id().0.as_str() == id);
                    }
                    let cfg = AppConfig::default();
                    if let Ok(c) = schema::init_db(&cfg.db_path) {
                        let _ = settings::set(&c, "mic_device", &name);
                    }
                    let _ = app.emit("mic_changed", ());
                }
                _ => {}
            }
        })
        .build(app)?;

    Ok(())
}

#[tauri::command]
async fn check_models() -> Result<serde_json::Value, String> {
    let config = AppConfig::default();
    let vad = config.models_dir.join("silero_vad.onnx").exists();
    let asr = config.models_dir.join("ggml-base.bin").exists()
           || config.models_dir.join("ggml-small.bin").exists();
    let llm = config.models_dir.join("qwen2.5-3b-instruct-q4_k_m.gguf").exists();
    let server = config.models_dir.join(models::download::LLAMA_SERVER_FILENAME).exists();
    Ok(serde_json::json!({
        "vad": vad, "asr": asr, "llm": llm, "server": server,
        "models_dir": config.models_dir.to_string_lossy(),
        "all_ready": vad && asr && llm && server,
    }))
}

#[tauri::command]
async fn download_models(app: tauri::AppHandle) -> Result<String, String> {
    let config = AppConfig::default();
    let handle = app.clone();
    tokio::task::spawn_blocking(move || {
        models::download::download_missing(&config.models_dir, &handle)
    }).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    Ok("done".into())
}

#[tauri::command]
async fn load_models(res: tauri::State<'_, SharedResources>) -> Result<String, String> {
    let config = AppConfig::default();
    let mut r = res.lock().await;
    if r.asr.is_none() {
        let p = config.models_dir.join("ggml-base.bin");
        if !p.exists() {
            let p2 = config.models_dir.join("ggml-small.bin");
            if !p2.exists() { return Err("ASR model not found".into()); }
            tracing::info!("Loading ASR model (small)...");
            let engine = load_on_thread(move || AsrEngine::new(&p2)).await?;
            r.asr = Some(Arc::new(engine));
        } else {
            tracing::info!("Loading ASR model (base)...");
            let engine = load_on_thread(move || AsrEngine::new(&p)).await?;
            r.asr = Some(Arc::new(engine));
        }
        tracing::info!("ASR loaded");
    }
    if r.polish.is_none() {
        let p = config.models_dir.join("qwen2.5-3b-instruct-q4_k_m.gguf");
        if p.exists() {
            tracing::info!("Loading LLM model...");
            let engine = load_on_thread(move || PolishEngine::new(&p)).await?;
            r.polish = Some(Arc::new(engine));
            tracing::info!("LLM loaded");
        }
    }
    Ok("Models loaded".into())
}

#[tauri::command]
async fn start_listening(app: tauri::AppHandle, res: tauri::State<'_, SharedResources>) -> Result<String, String> {
    let mut r = res.lock().await;
    if r.stop_tx.is_some() { return Ok("already listening".into()); }

    let asr = r.asr.clone().ok_or("Models not loaded")?;
    let polish = r.polish.clone();
    let vad_path = r.config.models_dir.join("silero_vad.onnx");
    let mic_name = {
        let conn = schema::init_db(&r.config.db_path).ok();
        conn.and_then(|c| settings::get(&c, "mic_device").ok().flatten())
    };

    let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
    r.stop_tx = Some(stop_tx);
    r.state.set_listening();
    let app_handle = app.clone();

    // Start orchestrator on Tauri's multi-threaded runtime (survives audio thread shutdown)
    let orchestrator = Orchestrator::start(asr.clone(), polish.clone(), app_handle.clone());
    let event_tx = orchestrator.event_tx.clone();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all().build().unwrap();
        rt.block_on(async {
            let (audio_tx, mut audio_rx) = mpsc::unbounded_channel::<Vec<f32>>();
            let mut capture = match audio::capture::start_capture(audio_tx, mic_name.as_deref()) {
                Ok(c) => {
                    // Mark mic permission granted so tray can enumerate devices next time
                    if let Ok(conn) = schema::init_db(&AppConfig::default().db_path) {
                        let _ = settings::set(&conn, "mic_granted", "1");
                    }
                    c
                },
                Err(e) => { tracing::error!("Capture failed: {}", e); return; }
            };

            let mut vad = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                SileroVad::new(&vad_path, 0.5).ok()
            })).unwrap_or(None);
            if vad.is_none() {
                tracing::warn!("Silero VAD unavailable, using energy-based detection");
            }
            let mut chunker = Chunker::new(700);
            let mut frame_buf: Vec<f32> = Vec::with_capacity(480);
            let mut level_acc = 0.0f32;
            let mut level_count = 0u32;

            loop {
                tokio::select! {
                    Some(chunk) = audio_rx.recv() => {
                        frame_buf.extend_from_slice(&chunk);
                        while frame_buf.len() >= 480 {
                            let frame: Vec<f32> = frame_buf.drain(..480).collect();
                            let rms = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt();

                            level_acc += rms;
                            level_count += 1;
                            if level_count >= 3 {
                                let _ = app_handle.emit("audio_level", level_acc / level_count as f32);
                                level_acc = 0.0;
                                level_count = 0;
                            }

                            let is_speech = match &mut vad {
                                Some(v) => v.is_speech(&frame).unwrap_or(false),
                                None => rms > 0.01,
                            };

                            // In walkie-talkie mode, buffer audio but don't auto-dispatch on silence
                            let walkie = WALKIE_TALKIE.load(Ordering::Relaxed);
                            if walkie {
                                // Just accumulate in chunker, no auto-segment dispatch
                                chunker.feed(&frame, true); // always "speech" so it never auto-flushes
                            } else {
                                if let Some(segment) = chunker.feed(&frame, is_speech) {
                                    if segment.len() > 4800 {
                                        let _ = event_tx.send(PipelineEvent::AudioSegment(segment));
                                    }
                                }
                            }
                        }
                    }
                    _ = stop_rx.recv() => {
                        capture.stop();
                        if let Some(segment) = chunker.flush() {
                            if segment.len() > 4800 {
                                let _ = event_tx.send(PipelineEvent::AudioSegment(segment));
                            }
                        }
                        let _ = event_tx.send(PipelineEvent::Stop);
                        break;
                    }
                }
            }
        });
    });

    Ok("listening".into())
}

#[tauri::command]
async fn stop_listening(res: tauri::State<'_, SharedResources>) -> Result<String, String> {
    let mut r = res.lock().await;
    if let Some(tx) = r.stop_tx.take() { let _ = tx.send(()).await; }
    r.state.set_idle();
    Ok("idle".into())
}

#[tauri::command]
async fn toggle_polish(enabled: bool) -> Result<bool, String> {
    POLISH_ENABLED.store(enabled, Ordering::Relaxed);
    Ok(enabled)
}

#[tauri::command]
async fn get_polish_enabled() -> Result<bool, String> {
    Ok(POLISH_ENABLED.load(Ordering::Relaxed))
}

#[tauri::command]
async fn list_mics() -> Result<Vec<String>, String> {
    audio::capture::list_input_devices().map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_mic(res: tauri::State<'_, SharedResources>, name: String) -> Result<(), String> {
    let r = res.lock().await;
    let conn = schema::init_db(&r.config.db_path).map_err(|e| e.to_string())?;
    settings::set(&conn, "mic_device", &name).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_mic(res: tauri::State<'_, SharedResources>) -> Result<String, String> {
    let r = res.lock().await;
    let conn = schema::init_db(&r.config.db_path).map_err(|e| e.to_string())?;
    Ok(settings::get(&conn, "mic_device").map_err(|e| e.to_string())?.unwrap_or_default())
}

#[tauri::command]
async fn save_window_pos(res: tauri::State<'_, SharedResources>, x: f64, y: f64) -> Result<(), String> {
    let r = res.lock().await;
    let conn = schema::init_db(&r.config.db_path).map_err(|e| e.to_string())?;
    settings::set(&conn, "win_x", &x.to_string()).map_err(|e| e.to_string())?;
    settings::set(&conn, "win_y", &y.to_string()).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn get_window_pos(res: tauri::State<'_, SharedResources>) -> Result<serde_json::Value, String> {
    let r = res.lock().await;
    let conn = schema::init_db(&r.config.db_path).map_err(|e| e.to_string())?;
    let x = settings::get(&conn, "win_x").map_err(|e| e.to_string())?;
    let y = settings::get(&conn, "win_y").map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "x": x, "y": y }))
}

#[tauri::command]
async fn get_app_state(res: tauri::State<'_, SharedResources>) -> Result<String, String> {
    let r = res.lock().await;
    Ok(serde_json::to_string(&r.state.phase).unwrap_or_else(|_| format!("{:?}", r.state.phase)))
}

#[tauri::command]
async fn open_accessibility_settings() -> Result<(), String> {
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn get_active_app_info() -> Result<String, String> {
    serde_json::to_string(&inject::context::get_active_app()).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_hint() -> Result<String, String> {
    let config = AppConfig::default();
    let conn = schema::init_db(&config.db_path).map_err(|e| e.to_string())?;
    // Return a random hint from today's cache
    let hint: Option<String> = conn.prepare(
        "SELECT hint FROM hint_cache WHERE generated_date = date('now') ORDER BY RANDOM() LIMIT 1"
    ).and_then(|mut s| {
        let mut rows = s.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.next().and_then(|r| r.ok()))
    }).unwrap_or(None);
    Ok(hint.unwrap_or_default())
}

async fn start_hint_generator(res: SharedResources) {
    let mut first = true;
    loop {
            let delay = if first { first = false; 60 } else { 300 };
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;

            // Skip if user dictated in the last 30s (LLM might be busy)
            let last = LAST_DICTATION.load(Ordering::Relaxed);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
            if last > 0 && now - last < 30 { continue; }

            generate_hints(&res).await;
    }
}

async fn generate_hints(res: &SharedResources) {
    let config = AppConfig::default();
    let conn = match schema::init_db(&config.db_path) {
        Ok(c) => c, Err(_) => return,
    };

    // Use top apps from history, or seed with common apps on first run
    let mut apps = db::hints::top_apps(&conn, 5).unwrap_or_default();
    if apps.is_empty() {
        apps = vec!["Safari", "Chrome", "Slack", "VS Code", "Notes"]
            .into_iter().map(String::from).collect();
    }

    // Skip if all apps already have today's hints
    let need: Vec<_> = apps.iter()
        .filter(|a| db::hints::get_hint(&conn, a).ok().flatten().is_none())
        .cloned().collect();
    if need.is_empty() { return; }

    let polish = {
        let r = res.lock().await;
        r.polish.clone()
    };
    let engine = match polish {
        Some(e) => e, None => return,
    };

    let app_list = need.join(", ");
    let prompt = format!(
        "Generate a short, subtle voice dictation hint (max 6 words) for each app. \
         The hint should remind the user they can dictate instead of typing. \
         Be creative, varied, not generic. One hint per line, format: AppName: hint\n\nApps: {}",
        app_list
    );

    let result = tokio::task::spawn_blocking(move || {
        engine.generate("You write ultra-concise UI microcopy.", &prompt, 128)
    }).await;

    if let Ok(Ok(text)) = result {
        for line in text.lines() {
            if let Some((app, hint)) = line.split_once(':') {
                let app = app.trim();
                let hint = hint.trim().trim_matches('"');
                if !hint.is_empty() && need.iter().any(|a| a == app) {
                    let _ = db::hints::save_hint(&conn, app, hint);
                }
            }
        }
        tracing::info!("Generated hints for: {}", app_list);
    }
}

#[tauri::command]
async fn add_dictionary_word(spoken: String, written: String) -> Result<(), String> {
    let config = AppConfig::default();
    let conn = schema::init_db(&config.db_path).map_err(|e| e.to_string())?;
    db::dictionary::add(&conn, &spoken, &written, "general").map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_dictionary() -> Result<Vec<String>, String> {
    let config = AppConfig::default();
    let conn = schema::init_db(&config.db_path).map_err(|e| e.to_string())?;
    db::dictionary::get_all(&conn).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt::init();

    let config = AppConfig::default();
    let _ = std::fs::create_dir_all(&config.models_dir);
    let _ = schema::init_db(&config.db_path);

    // Restore walkie-talkie setting from DB
    if let Ok(conn) = schema::init_db(&config.db_path) {
        if let Ok(Some(v)) = settings::get(&conn, "walkie_talkie") {
            WALKIE_TALKIE.store(v == "1", Ordering::Relaxed);
        }
    }

    if !launchd_plist_path().exists() {
        let cfg = AppConfig::default();
        if let Ok(conn) = schema::init_db(&cfg.db_path) {
            if settings::get(&conn, "autostart_initialized").ok().flatten().is_none() {
                set_autostart(true);
                let _ = settings::set(&conn, "autostart_initialized", "1");
            }
        }
    }

    let resources: SharedResources = Arc::new(Mutex::new(AppResources {
        state: AppState::new(),
        asr: None,
        polish: None,
        stop_tx: None,
        config,
    }));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(resources)
        .setup(|app| {
            setup_tray(app)?;

            // Start background hint generator
            let hint_res: SharedResources = app.state::<SharedResources>().inner().clone();
            std::thread::spawn(move || {
                // Wait for Tauri runtime to be ready
                std::thread::sleep(std::time::Duration::from_secs(5));
                tauri::async_runtime::spawn(async move {
                    start_hint_generator(hint_res).await;
                });
            });

            // Poll accessibility permission until granted
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(2));
                loop {
                    if inject::clipboard::check_accessibility() {
                        let _ = handle.emit("accessibility_granted", ());
                        break;
                    }
                    let _ = handle.emit("accessibility_missing", ());
                    std::thread::sleep(std::time::Duration::from_secs(3));
                }
            });

            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            let app_handle = app.handle().clone();
            app.global_shortcut().on_shortcut("ctrl+shift+space", move |_app, _shortcut, event| {
                use tauri_plugin_global_shortcut::ShortcutState;
                let handle = app_handle.clone();
                match event.state {
                    ShortcutState::Pressed => {
                        tauri::async_runtime::spawn(async move {
                            if WALKIE_TALKIE.load(Ordering::Relaxed) {
                                let _ = handle.emit("walkie_press", ());
                            } else {
                                let _ = handle.emit("toggle_listening", ());
                            }
                        });
                    }
                    ShortcutState::Released => {
                        if WALKIE_TALKIE.load(Ordering::Relaxed) {
                            tauri::async_runtime::spawn(async move {
                                let _ = handle.emit("walkie_release", ());
                            });
                        }
                    }
                }
            })?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            check_models, download_models, load_models,
            start_listening, stop_listening,
            get_app_state, open_accessibility_settings, get_active_app_info,
            get_hint,
            add_dictionary_word, get_dictionary,
            toggle_polish, get_polish_enabled,
            list_mics, set_mic, get_mic,
            save_window_pos, get_window_pos,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
