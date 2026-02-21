use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppContext {
    pub app_name: String,
    pub bundle_id: String,
    pub category: String,
    pub tone: String,
}

impl Default for AppContext {
    fn default() -> Self {
        Self {
            app_name: "Unknown".into(),
            bundle_id: String::new(),
            category: "default".into(),
            tone: "Natural, clear prose.".into(),
        }
    }
}

/// Detect the active application on macOS.
#[cfg(target_os = "macos")]
pub fn get_active_app() -> AppContext {
    use std::process::Command;
    // Use osascript to get frontmost app info
    let output = Command::new("osascript")
        .arg("-e")
        .arg("tell application \"System Events\" to get {name, bundle identifier} of first application process whose frontmost is true")
        .output();

    match output {
        Ok(out) => {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let parts: Vec<&str> = s.split(", ").collect();
            let app_name = parts.first().unwrap_or(&"Unknown").to_string();
            let bundle_id = parts.get(1).unwrap_or(&"").to_string();
            let category = categorize_app(&bundle_id);
            let tone = tone_for_category(&category);
            AppContext { app_name, bundle_id, category, tone }
        }
        Err(_) => AppContext::default(),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn get_active_app() -> AppContext {
    AppContext::default()
}

fn categorize_app(bundle_id: &str) -> String {
    match bundle_id {
        b if b.contains("mail") || b.contains("Outlook") => "email",
        b if b.contains("slack") => "slack",
        b if b.contains("VSCode") || b.contains("Xcode") => "code",
        b if b.contains("notion") => "notes",
        b if b.contains("Terminal") || b.contains("iTerm") => "terminal",
        _ => "default",
    }.into()
}

fn tone_for_category(category: &str) -> String {
    match category {
        "email" => "Professional, concise.",
        "slack" => "Casual, conversational.",
        "code" => "Technical. Use backticks for code references.",
        "terminal" => "Command-like. Be terse.",
        "notes" => "Structured. Use headers and lists where appropriate.",
        _ => "Natural, clear prose.",
    }.into()
}
