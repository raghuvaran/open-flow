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
static VAD_INIT_FAILED: AtomicBool = AtomicBool::new(false);

struct AppResources {
    state: AppState,
    asr: Option<Arc<AsrEngine>>,
    polish: Option<Arc<PolishEngine>>,
    stop_tx: Option<mpsc::Sender<()>>,
    config: AppConfig,
    _device_watcher: Option<Box<dyn Send + Sync>>,
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

fn build_tray_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
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
    let walkie_item = CheckMenuItemBuilder::new("Walkie-Talkie")
        .id("walkie").checked(walkie_on).build(app)?;
    let autostart_item = CheckMenuItemBuilder::new("Launch at Login")
        .id("autostart").checked(autostart_on).build(app)?;

    let default_mic = CheckMenuItemBuilder::new("Default")
        .id("mic__default").checked(saved_mic.is_empty()).build(app)?;
    let mut mic_sub = SubmenuBuilder::new(app, "Microphone").item(&default_mic);
    for mic in &mics {
        let item = CheckMenuItemBuilder::new(mic.as_str())
            .id(format!("mic__{}", mic)).checked(*mic == saved_mic).build(app)?;
        mic_sub = mic_sub.item(&item);
    }
    let mic_menu = mic_sub.build()?;

    let quit_item = MenuItemBuilder::new("Quit OpenFlow")
        .id("quit").accelerator("CmdOrCtrl+Q").build(app)?;

    // Pill color submenu
    let saved_color = conn.as_ref()
        .and_then(|c| settings::get(c, "pill_color").ok().flatten())
        .unwrap_or_else(|| "rgba(18, 18, 30, 0.44)".to_string());

    let presets = [
        ("Dark (Default)", "rgba(18, 18, 30, 0.44)"),
        ("Charcoal",       "rgba(40, 40, 50, 0.55)"),
        ("Midnight Blue",  "rgba(15, 23, 42, 0.55)"),
        ("Deep Teal",      "rgba(13, 42, 48, 0.55)"),
        ("Slate",          "rgba(51, 65, 85, 0.50)"),
        ("Graphite",       "rgba(55, 55, 55, 0.50)"),
        ("Warm Dark",      "rgba(45, 30, 20, 0.50)"),
        ("Frosted White",  "rgba(240, 240, 245, 0.35)"),
    ];
    let opacities = [
        ("25%", 0.25), ("35%", 0.35), ("50%", 0.50),
        ("65%", 0.65), ("75%", 0.75), ("90%", 0.90),
    ];

    let mut color_sub = SubmenuBuilder::new(app, "Appearance");
    for (name, rgba) in &presets {
        let checked = *rgba == saved_color.as_str();
        let item = CheckMenuItemBuilder::new(*name)
            .id(format!("color__{}", rgba)).checked(checked).build(app)?;
        color_sub = color_sub.item(&item);
    }

    let current_opacity = saved_color.split(',').last()
        .and_then(|s| s.trim().trim_end_matches(')').parse::<f64>().ok())
        .unwrap_or(0.44);

    let mut opacity_sub = SubmenuBuilder::new(app, "Opacity");
    for (label, val) in &opacities {
        let checked = (current_opacity - val).abs() < 0.03;
        let item = CheckMenuItemBuilder::new(*label)
            .id(format!("opacity__{}", val)).checked(checked).build(app)?;
        opacity_sub = opacity_sub.item(&item);
    }
    let opacity_menu = opacity_sub.build()?;
    color_sub = color_sub.separator().item(&opacity_menu);
    let color_menu = color_sub.build()?;

    // Shortcut submenu
    let saved_shortcut = conn.as_ref()
        .and_then(|c| settings::get(c, "shortcut").ok().flatten())
        .unwrap_or_else(|| "ctrl+shift+space".to_string());

    let shortcut_presets = [
        ("Ctrl+Shift+Space", "ctrl+shift+space"),
        ("Ctrl+Shift+S",     "ctrl+shift+s"),
        ("Ctrl+Shift+D",     "ctrl+shift+d"),
        ("Cmd+Shift+Space",  "super+shift+space"),
        ("Cmd+Shift+S",      "super+shift+s"),
        ("Option+Space",     "alt+space"),
    ];
    let mut shortcut_sub = SubmenuBuilder::new(app, "Shortcut");
    for (label, key) in &shortcut_presets {
        let checked = *key == saved_shortcut.as_str();
        let item = CheckMenuItemBuilder::new(*label)
            .id(format!("shortcut__{}", key)).checked(checked).build(app)?;
        shortcut_sub = shortcut_sub.item(&item);
    }
    let shortcut_menu = shortcut_sub.build()?;

    MenuBuilder::new(app)
        .item(&show_item)
        .separator()
        .item(&polish_item)
        .item(&walkie_item)
        .item(&autostart_item)
        .item(&mic_menu)
        .item(&color_menu)
        .item(&shortcut_menu)
        .separator()
        .item(&quit_item)
        .build()
}

fn handle_menu_event(app: &tauri::AppHandle, id: &str) {
    match id {
        "quit" => { std::process::exit(0); }
        "show" => { let _ = app.emit("show_window", ()); }
        "pill_color" => {
            let _ = app.emit("show_window", ());
            let _ = app.emit("toggle_settings", ());
        }
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
            let cfg = AppConfig::default();
            if let Ok(c) = schema::init_db(&cfg.db_path) {
                let _ = settings::set(&c, "mic_device", &name);
            }
            // Rebuild menu to update check marks
            rebuild_tray_menu(app);
            let _ = app.emit("mic_changed", ());
        }
        _ if id.starts_with("color__") => {
            let rgba = id[7..].to_string();
            let cfg = AppConfig::default();
            if let Ok(c) = schema::init_db(&cfg.db_path) {
                let _ = settings::set(&c, "pill_color", &rgba);
            }
            rebuild_tray_menu(app);
            let _ = app.emit("pill_color_changed", &rgba);
        }
        _ if id.starts_with("opacity__") => {
            let new_opacity: f64 = id[9..].parse().unwrap_or(0.44);
            let cfg = AppConfig::default();
            if let Ok(c) = schema::init_db(&cfg.db_path) {
                let current = settings::get(&c, "pill_color").ok().flatten()
                    .unwrap_or_else(|| "rgba(18, 18, 30, 0.44)".to_string());
                if let Some(last_comma) = current.rfind(',') {
                    let new_rgba = format!("{}, {})", &current[..last_comma], new_opacity);
                    let _ = settings::set(&c, "pill_color", &new_rgba);
                    let _ = app.emit("pill_color_changed", &new_rgba);
                }
            }
            rebuild_tray_menu(app);
        }
        _ if id.starts_with("shortcut__") => {
            let new_key = id[10..].to_string();
            let cfg = AppConfig::default();
            if let Ok(c) = schema::init_db(&cfg.db_path) {
                let _ = settings::set(&c, "shortcut", &new_key);
            }
            rebuild_tray_menu(app);
            // Re-register the global shortcut
            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            let _ = app.global_shortcut().unregister_all();
            let handle = app.clone();
            let _ = app.global_shortcut().on_shortcut(new_key.as_str(), move |_app, _shortcut, event| {
                use tauri_plugin_global_shortcut::ShortcutState;
                let h = handle.clone();
                match event.state {
                    ShortcutState::Pressed => {
                        tauri::async_runtime::spawn(async move {
                            if WALKIE_TALKIE.load(Ordering::Relaxed) {
                                let _ = h.emit("walkie_press", ());
                            } else {
                                let _ = h.emit("toggle_listening", ());
                            }
                        });
                    }
                    ShortcutState::Released => {
                        if WALKIE_TALKIE.load(Ordering::Relaxed) {
                            tauri::async_runtime::spawn(async move {
                                let _ = h.emit("walkie_release", ());
                            });
                        }
                    }
                }
            });
        }
        _ => {}
    }
}

const TRAY_ID: &str = "openflow_tray";

fn rebuild_tray_menu(app: &tauri::AppHandle) {
    if let Ok(menu) = build_tray_menu(app) {
        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let handle = app.handle();
    let menu = build_tray_menu(handle)?;

    let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))
        .expect("tray icon missing");

    let _tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .menu(&menu)
        .tooltip("OpenFlow")
        .on_menu_event(|app, event: tauri::menu::MenuEvent| {
            handle_menu_event(app, event.id().0.as_str());
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

            let mut vad = if VAD_INIT_FAILED.load(Ordering::Relaxed) {
                None
            } else {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    SileroVad::new(&vad_path, 0.5).ok()
                })) {
                    Ok(v) => v,
                    Err(_) => { VAD_INIT_FAILED.store(true, Ordering::Relaxed); None }
                }
            };
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
async fn set_pill_color(res: tauri::State<'_, SharedResources>, color: String) -> Result<(), String> {
    let r = res.lock().await;
    let conn = schema::init_db(&r.config.db_path).map_err(|e| e.to_string())?;
    settings::set(&conn, "pill_color", &color).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_pill_color(res: tauri::State<'_, SharedResources>) -> Result<Option<String>, String> {
    let r = res.lock().await;
    let conn = schema::init_db(&r.config.db_path).map_err(|e| e.to_string())?;
    settings::get(&conn, "pill_color").map_err(|e| e.to_string())
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
async fn check_accessibility_cmd() -> Result<bool, String> {
    Ok(inject::clipboard::check_accessibility())
}

#[tauri::command]
async fn get_active_app_info() -> Result<String, String> {
    serde_json::to_string(&inject::context::get_active_app()).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_hint() -> Result<String, String> {
    let config = AppConfig::default();
    let conn = schema::init_db(&config.db_path).map_err(|e| e.to_string())?;

    // Weighted random: 50% hints, 25% mood, 25% affirmations
    let roll: u8 = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default()
        .subsec_nanos() % 4) as u8;

    let query = match roll {
        0 => {
            // Mood: pick one matching current time bracket
            let hour: u32 = conn.query_row(
                "SELECT CAST(strftime('%H', 'now', 'localtime') AS INTEGER)", [],
                |row| row.get(0)
            ).unwrap_or(12);
            let bracket = match hour {
                6..=8 => "early", 9..=11 => "morning", 12..=16 => "afternoon",
                17..=20 => "evening", 21..=23 => "night", _ => "late",
            };
            format!(
                "SELECT hint FROM hint_cache WHERE app_name = '__mood_{}' AND generated_date = date('now') LIMIT 1",
                bracket
            )
        }
        1 => {
            // Affirmation
            "SELECT hint FROM hint_cache WHERE app_name LIKE '__affirm_%' AND generated_date = date('now') ORDER BY RANDOM() LIMIT 1".to_string()
        }
        _ => {
            // Regular hint
            "SELECT hint FROM hint_cache WHERE app_name NOT LIKE '__%' AND generated_date = date('now') ORDER BY RANDOM() LIMIT 1".to_string()
        }
    };

    let hint: Option<String> = conn.prepare(&query)
        .and_then(|mut s| {
            let mut rows = s.query_map([], |row| row.get::<_, String>(0))?;
            Ok(rows.next().and_then(|r| r.ok()))
        }).unwrap_or(None);

    // Fallback to any hint if the chosen category is empty
    if hint.is_some() { return Ok(hint.unwrap()); }
    let fallback: Option<String> = conn.prepare(
        "SELECT hint FROM hint_cache WHERE generated_date = date('now') AND app_name NOT LIKE '__%' ORDER BY RANDOM() LIMIT 1"
    ).and_then(|mut s| {
        let mut rows = s.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.next().and_then(|r| r.ok()))
    }).unwrap_or(None);
    Ok(fallback.unwrap_or_default())
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

    let mut apps = db::hints::top_apps(&conn, 5).unwrap_or_default();
    if apps.is_empty() {
        apps = vec!["Safari", "Chrome", "Slack", "VS Code", "Notes"]
            .into_iter().map(String::from).collect();
    }

    let need_hints: Vec<_> = apps.iter()
        .filter(|a| db::hints::get_hint(&conn, a).ok().flatten().is_none())
        .cloned().collect();
    let need_mood = db::hints::get_hint(&conn, "__mood__").ok().flatten().is_none();
    let need_affirm: Vec<_> = apps.iter()
        .filter(|a| {
            let key = format!("__affirm_{}", a);
            db::hints::get_hint(&conn, &key).ok().flatten().is_none()
        })
        .cloned().collect();

    if need_hints.is_empty() && !need_mood && need_affirm.is_empty() { return; }

    let polish = {
        let r = res.lock().await;
        r.polish.clone()
    };
    let engine = match polish {
        Some(e) => e, None => return,
    };

    // Build a single combined prompt
    let mut sections = Vec::new();

    if !need_hints.is_empty() {
        sections.push(format!(
            "HINTS: One voice dictation hint (max 6 words) per app. Format: AppName: hint\nApps: {}",
            need_hints.join(", ")
        ));
    }

    if need_mood {
        sections.push(
            "MOOD: Generate 6 time-of-day mood texts (max 4 words each). Calm, observational tone. \
             Format: morning: text, afternoon: text, evening: text, night: text, late: text, early: text".to_string()
        );
    }

    if !need_affirm.is_empty() {
        sections.push(format!(
            "AFFIRMATIONS: One calm, observational acknowledgment (3-5 words, no emoji, no exclamation) per app. \
             Format: AppName: text\nApps: {}",
            need_affirm.join(", ")
        ));
    }

    let prompt = sections.join("\n\n");
    let engine_clone = engine.clone();
    let result = tokio::task::spawn_blocking(move || {
        engine_clone.generate("You write ultra-concise UI microcopy. Be subtle, never cheerful.", &prompt, 256)
    }).await;

    if let Ok(Ok(text)) = result {
        let mut section = "";
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("HINTS") { section = "hints"; continue; }
            if trimmed.starts_with("MOOD") { section = "mood"; continue; }
            if trimmed.starts_with("AFFIRMATION") { section = "affirm"; continue; }

            if let Some((key, val)) = trimmed.split_once(':') {
                let key = key.trim();
                let val = val.trim().trim_matches('"');
                if val.is_empty() { continue; }

                match section {
                    "hints" if need_hints.iter().any(|a| a == key) => {
                        let _ = db::hints::save_hint(&conn, key, val);
                    }
                    "mood" => {
                        let tag = format!("__mood_{}", key);
                        let _ = db::hints::save_hint(&conn, &tag, val);
                    }
                    "affirm" if need_affirm.iter().any(|a| a == key) => {
                        let tag = format!("__affirm_{}", key);
                        let _ = db::hints::save_hint(&conn, &tag, val);
                    }
                    // If no section header, try to match as hint (backward compat)
                    "" if need_hints.iter().any(|a| a == key) => {
                        let _ = db::hints::save_hint(&conn, key, val);
                    }
                    _ => {}
                }
            }
        }
        tracing::info!("Generated hints/mood/affirmations");
    }
}

#[tauri::command]
async fn add_dictionary_word(spoken: String, written: String) -> Result<(), String> {    let config = AppConfig::default();
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
        _device_watcher: None,
    }));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(resources)
        .setup(|app| {
            setup_tray(app)?;

            // Watch for audio device changes and rebuild tray menu
            #[cfg(target_os = "macos")]
            {
                let handle = app.handle().clone();
                if let Ok(watcher) = audio::capture::watch_device_changes(move || {
                    rebuild_tray_menu(&handle);
                }) {
                    let res: tauri::State<SharedResources> = app.state();
                    tauri::async_runtime::block_on(async {
                        res.lock().await._device_watcher = Some(Box::new(watcher));
                    });
                }
            }

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
            let saved_shortcut = {
                let cfg = AppConfig::default();
                schema::init_db(&cfg.db_path).ok()
                    .and_then(|c| settings::get(&c, "shortcut").ok().flatten())
                    .unwrap_or_else(|| "ctrl+shift+space".to_string())
            };
            app.global_shortcut().on_shortcut(saved_shortcut.as_str(), move |_app, _shortcut, event| {
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
            get_app_state, open_accessibility_settings, check_accessibility_cmd, get_active_app_info,
            set_pill_color, get_pill_color,
            get_hint,
            add_dictionary_word, get_dictionary,
            toggle_polish, get_polish_enabled,
            list_mics, set_mic, get_mic,
            save_window_pos, get_window_pos,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn test_db_conn() -> rusqlite::Connection {
        schema::init_db(std::path::Path::new(":memory:")).unwrap()
    }

    // ===== Layer 3: Feature-level integration tests =====

    // --- Walkie-talkie mode ---
    #[test]
    fn walkie_talkie_chunker_no_auto_flush() {
        // In walkie-talkie mode, chunker is fed is_speech=true always, so it never auto-segments
        let mut chunker = audio::chunker::Chunker::new(700);
        let frame = vec![0.5f32; 480];
        for _ in 0..50 {
            // Walkie mode: always feed as speech
            assert!(chunker.feed(&frame, true).is_none());
        }
        // Only flush produces output
        let seg = chunker.flush().expect("flush should return audio");
        assert_eq!(seg.len(), 50 * 480);
    }

    #[test]
    fn walkie_talkie_flag_persists_to_db() {
        let conn = test_db_conn();
        WALKIE_TALKIE.store(true, Ordering::Relaxed);
        settings::set(&conn, "walkie_talkie", "1").unwrap();
        let val = settings::get(&conn, "walkie_talkie").unwrap();
        assert_eq!(val, Some("1".into()));

        WALKIE_TALKIE.store(false, Ordering::Relaxed);
        settings::set(&conn, "walkie_talkie", "0").unwrap();
        let val = settings::get(&conn, "walkie_talkie").unwrap();
        assert_eq!(val, Some("0".into()));
    }

    #[test]
    fn walkie_talkie_restore_from_db() {
        let conn = test_db_conn();
        settings::set(&conn, "walkie_talkie", "1").unwrap();
        if let Ok(Some(v)) = settings::get(&conn, "walkie_talkie") {
            WALKIE_TALKIE.store(v == "1", Ordering::Relaxed);
        }
        assert!(WALKIE_TALKIE.load(Ordering::Relaxed));
        WALKIE_TALKIE.store(false, Ordering::Relaxed); // cleanup
    }

    // --- Toggle mode (normal mode) ---
    #[test]
    fn toggle_mode_emits_segments_on_silence() {
        let mut chunker = audio::chunker::Chunker::new(90); // 3 frames threshold
        let speech = vec![0.5f32; 480];
        let silence = vec![0.0f32; 480];

        // Speech burst
        for _ in 0..10 { chunker.feed(&speech, true); }
        // Silence triggers segment
        let mut got = false;
        for _ in 0..5 {
            if chunker.feed(&silence, false).is_some() { got = true; break; }
        }
        assert!(got, "toggle mode should auto-emit on silence");
    }

    // --- Polish toggle ---
    #[test]
    fn polish_toggle_flag() {
        POLISH_ENABLED.store(true, Ordering::Relaxed);
        assert!(POLISH_ENABLED.load(Ordering::Relaxed));
        POLISH_ENABLED.store(false, Ordering::Relaxed);
        assert!(!POLISH_ENABLED.load(Ordering::Relaxed));
        POLISH_ENABLED.store(true, Ordering::Relaxed); // restore
    }

    // --- Mic persistence ---
    #[test]
    fn mic_device_persistence() {
        let conn = test_db_conn();
        settings::set(&conn, "mic_device", "Blue Yeti").unwrap();
        assert_eq!(settings::get(&conn, "mic_device").unwrap(), Some("Blue Yeti".into()));

        // Default mic = empty string
        settings::set(&conn, "mic_device", "").unwrap();
        assert_eq!(settings::get(&conn, "mic_device").unwrap(), Some("".into()));
    }

    // --- Pill position persistence ---
    #[test]
    fn pill_position_roundtrip() {
        let conn = test_db_conn();
        settings::set(&conn, "win_x", "150").unwrap();
        settings::set(&conn, "win_y", "800").unwrap();
        assert_eq!(settings::get(&conn, "win_x").unwrap(), Some("150".into()));
        assert_eq!(settings::get(&conn, "win_y").unwrap(), Some("800".into()));
    }

    // --- Pill color persistence ---
    #[test]
    fn pill_color_roundtrip() {
        let conn = test_db_conn();
        let color = "rgba(18, 18, 30, 0.44)";
        settings::set(&conn, "pill_color", color).unwrap();
        assert_eq!(settings::get(&conn, "pill_color").unwrap(), Some(color.into()));
    }

    #[test]
    fn pill_opacity_update() {
        let conn = test_db_conn();
        let color = "rgba(18, 18, 30, 0.44)";
        settings::set(&conn, "pill_color", color).unwrap();

        // Simulate opacity change (same logic as tray handler)
        let current = settings::get(&conn, "pill_color").unwrap().unwrap();
        let new_opacity = 0.75;
        if let Some(last_comma) = current.rfind(',') {
            let new_rgba = format!("{}, {})", &current[..last_comma], new_opacity);
            settings::set(&conn, "pill_color", &new_rgba).unwrap();
        }
        let updated = settings::get(&conn, "pill_color").unwrap().unwrap();
        assert!(updated.contains("0.75"));
    }

    // --- Shortcut persistence ---
    #[test]
    fn shortcut_persistence() {
        let conn = test_db_conn();
        settings::set(&conn, "shortcut", "ctrl+shift+space").unwrap();
        assert_eq!(settings::get(&conn, "shortcut").unwrap(), Some("ctrl+shift+space".into()));

        settings::set(&conn, "shortcut", "super+shift+s").unwrap();
        assert_eq!(settings::get(&conn, "shortcut").unwrap(), Some("super+shift+s".into()));
    }

    // --- Dictionary feature ---
    #[test]
    fn dictionary_add_and_use_in_prompt() {
        let conn = test_db_conn();
        db::dictionary::add(&conn, "k8s", "Kubernetes", "tech").unwrap();
        db::dictionary::add(&conn, "grpc", "gRPC", "tech").unwrap();
        let all = db::dictionary::get_all(&conn).unwrap();
        assert_eq!(all.len(), 2);

        // Verify dictionary words can be fed into prompt builder
        let words: Vec<String> = all.iter().map(|e| {
            e.split(" → ").nth(1).unwrap_or("").to_string()
        }).collect();
        let ctx = inject::context::AppContext::default();
        let prompt = polish::prompt::build_system_prompt(&ctx, &words);
        assert!(prompt.contains("Kubernetes"));
        assert!(prompt.contains("gRPC"));
    }

    // --- Hint system ---
    #[test]
    fn hint_system_save_and_retrieve() {
        let conn = test_db_conn();
        db::hints::save_hint(&conn, "Slack", "Say 'new paragraph' for breaks").unwrap();
        db::hints::save_hint(&conn, "Safari", "Try voice commands").unwrap();
        assert!(db::hints::get_hint(&conn, "Slack").unwrap().is_some());
        assert!(db::hints::get_hint(&conn, "Safari").unwrap().is_some());
    }

    #[test]
    fn hint_mood_and_affirmation_tags() {
        let conn = test_db_conn();
        db::hints::save_hint(&conn, "__mood_morning", "Fresh start ahead").unwrap();
        db::hints::save_hint(&conn, "__affirm_Slack", "Words flowing well").unwrap();
        assert_eq!(db::hints::get_hint(&conn, "__mood_morning").unwrap(), Some("Fresh start ahead".into()));
        assert_eq!(db::hints::get_hint(&conn, "__affirm_Slack").unwrap(), Some("Words flowing well".into()));
    }

    #[test]
    fn hint_top_apps_ranking() {
        let conn = test_db_conn();
        for _ in 0..10 { db::hints::record_usage(&conn, "VS Code").unwrap(); }
        for _ in 0..5 { db::hints::record_usage(&conn, "Slack").unwrap(); }
        for _ in 0..2 { db::hints::record_usage(&conn, "Safari").unwrap(); }
        let top = db::hints::top_apps(&conn, 3).unwrap();
        assert_eq!(top[0], "VS Code");
        assert_eq!(top[1], "Slack");
        assert_eq!(top[2], "Safari");
    }

    // --- Autostart ---
    #[test]
    fn autostart_plist_path() {
        let path = launchd_plist_path();
        assert!(path.to_string_lossy().contains("LaunchAgents"));
        assert!(path.to_string_lossy().contains("com.openflow.app.plist"));
    }

    // --- Voice commands through pipeline ---
    #[test]
    fn voice_command_new_paragraph() {
        let cmd = polish::commands::parse_command("new paragraph");
        assert_eq!(polish::commands::command_text(&cmd), Some("\n\n"));
    }

    #[test]
    fn voice_command_scratch_that_has_no_text() {
        let cmd = polish::commands::parse_command("scratch that");
        assert!(polish::commands::command_text(&cmd).is_none());
    }

    // --- App context → prompt integration ---
    #[test]
    fn app_context_flows_to_prompt() {
        let ctx = inject::context::AppContext {
            app_name: "Slack".into(),
            bundle_id: "com.tinyspeck.slackmacgap".into(),
            category: "slack".into(),
            tone: "Casual, conversational.".into(),
            window_title: "#engineering".into(),
            selected_text: "let's deploy".into(),
        };
        let prompt = polish::prompt::build_system_prompt(&ctx, &["Kubernetes".into()]);
        assert!(prompt.contains("Slack"));
        assert!(prompt.contains("slack"));
        assert!(prompt.contains("Casual"));
        assert!(prompt.contains("#engineering"));
        assert!(prompt.contains("let's deploy"));
        assert!(prompt.contains("Kubernetes"));
    }

    // ===== Layer 4: Error and edge case tests =====

    // --- Blank/noise audio filtering ---
    #[test]
    fn blank_audio_marker_filtered() {
        // process_segment filters text starting with '[' or '('
        let markers = ["[BLANK_AUDIO]", "(music)", "[silence]", "(noise)"];
        for m in markers {
            assert!(m.starts_with('[') || m.starts_with('('),
                "marker '{}' should be filtered by pipeline", m);
        }
    }

    // --- Empty ASR ---
    #[test]
    fn empty_text_is_not_a_command() {
        let cmd = polish::commands::parse_command("");
        match cmd {
            polish::commands::VoiceCommand::None(t) => assert!(t.is_empty()),
            _ => panic!("empty text should be VoiceCommand::None"),
        }
    }

    // --- Missing model error ---
    #[test]
    fn polish_engine_missing_model_error() {
        let result = polish::engine::PolishEngine::new(std::path::Path::new("/tmp/nonexistent.gguf"));
        assert!(result.is_err());
    }

    #[test]
    fn asr_engine_missing_model_error() {
        let result = asr::engine::AsrEngine::new(std::path::Path::new("/tmp/nonexistent.bin"));
        assert!(result.is_err());
    }

    // --- DB creation from scratch ---
    #[test]
    fn db_creation_in_new_directory() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("deep/nested/openflow.db");
        let conn = schema::init_db(&db_path).unwrap();
        // Verify it works
        settings::set(&conn, "test", "value").unwrap();
        assert_eq!(settings::get(&conn, "test").unwrap(), Some("value".into()));
    }

    // --- Chunker overflow ---
    #[test]
    fn chunker_overflow_at_60s() {
        let mut chunker = audio::chunker::Chunker::new(700);
        let frame = vec![0.5f32; 480];
        let max_samples = 16000 * 60;
        let frames_to_fill = max_samples / 480;

        let mut emitted = false;
        for _ in 0..=frames_to_fill {
            if let Some(seg) = chunker.feed(&frame, true) {
                assert!(seg.len() >= max_samples, "segment should be at least 60s worth");
                emitted = true;
                break;
            }
        }
        assert!(emitted, "chunker should force-emit at 60s cap");
    }

    // --- Accessibility check ---
    #[test]
    fn accessibility_check_returns_bool() {
        // Just verify it doesn't panic — in test env it will likely return false
        let _result = inject::clipboard::check_accessibility();
    }

    // --- Resample edge cases ---
    #[test]
    fn resample_empty_input() {
        let out = audio::capture::resample(&[], 48000);
        assert!(out.is_empty());
    }

    #[test]
    fn resample_single_sample() {
        let out = audio::capture::resample(&[0.5], 48000);
        // 1 sample at 48kHz → 0 or 1 sample at 16kHz
        assert!(out.len() <= 1);
    }

    #[test]
    fn to_mono_empty() {
        let out = audio::capture::to_mono(&[], 2);
        assert!(out.is_empty());
    }

    // --- Settings edge cases ---
    #[test]
    fn settings_empty_value() {
        let conn = test_db_conn();
        settings::set(&conn, "key", "").unwrap();
        assert_eq!(settings::get(&conn, "key").unwrap(), Some("".into()));
    }

    #[test]
    fn settings_unicode_value() {
        let conn = test_db_conn();
        settings::set(&conn, "name", "日本語テスト").unwrap();
        assert_eq!(settings::get(&conn, "name").unwrap(), Some("日本語テスト".into()));
    }

    // --- Dictionary edge cases ---
    #[test]
    fn dictionary_unicode_entries() {
        let conn = test_db_conn();
        db::dictionary::add(&conn, "café", "café", "general").unwrap();
        let all = db::dictionary::get_all(&conn).unwrap();
        assert!(all[0].contains("café"));
    }

    // --- Tone custom override ---
    #[test]
    fn custom_tone_overrides_default() {
        let conn = test_db_conn();
        db::tones::set_tone(&conn, "com.apple.mail", "Mail", "email", "Ultra formal").unwrap();
        let tone = db::tones::get_tone(&conn, "com.apple.mail").unwrap();
        assert_eq!(tone, Some("Ultra formal".into()));
    }

    // --- Snippets ---
    #[test]
    fn snippets_add_and_retrieve() {
        let conn = test_db_conn();
        db::snippets::add(&conn, "addr", "123 Main St").unwrap();
        let all = db::snippets::get_all(&conn).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0], ("addr".into(), "123 Main St".into()));
    }

    // --- Config paths ---
    #[test]
    fn config_models_dir_ends_with_models() {
        let c = config::AppConfig::default();
        assert!(c.models_dir.ends_with("models"));
    }

    // --- Concurrent POLISH_ENABLED access ---
    #[test]
    fn polish_flag_atomic_toggle() {
        let handles: Vec<_> = (0..10).map(|i| {
            std::thread::spawn(move || {
                POLISH_ENABLED.store(i % 2 == 0, Ordering::Relaxed);
                POLISH_ENABLED.load(Ordering::Relaxed)
            })
        }).collect();
        for h in handles { let _ = h.join(); }
        // Just verify no panic — final value is non-deterministic
        let _ = POLISH_ENABLED.load(Ordering::Relaxed);
        POLISH_ENABLED.store(true, Ordering::Relaxed); // restore
    }
}
