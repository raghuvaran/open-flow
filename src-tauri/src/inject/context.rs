use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppContext {
    pub app_name: String,
    pub bundle_id: String,
    pub category: String,
    pub tone: String,
    pub window_title: String,
    pub selected_text: String,
}

impl Default for AppContext {
    fn default() -> Self {
        Self {
            app_name: "Unknown".into(),
            bundle_id: String::new(),
            category: "default".into(),
            tone: "Natural, clear prose.".into(),
            window_title: String::new(),
            selected_text: String::new(),
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

    let (app_name, bundle_id) = match output {
        Ok(out) => {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let parts: Vec<&str> = s.split(", ").collect();
            (
                parts.first().unwrap_or(&"Unknown").to_string(),
                parts.get(1).unwrap_or(&"").to_string(),
            )
        }
        Err(_) => ("Unknown".into(), String::new()),
    };

    let category = categorize_app(&bundle_id);
    let tone = tone_for_category(&category);

    // Get window title and selected text via Accessibility API
    let (window_title, selected_text) = get_ax_context();

    AppContext { app_name, bundle_id, category, tone, window_title, selected_text }
}

#[cfg(target_os = "macos")]
fn get_ax_context() -> (String, String) {
    // For terminals, use AppleScript to get buffer content
    if let Some(text) = get_terminal_content() {
        let trimmed = text.trim().to_string();
        let ctx = tail_chars(&trimmed, 200);
        return (String::new(), ctx);
    }
    get_ax_text()
}

/// Terminal-specific: grab visible buffer via AppleScript
#[cfg(target_os = "macos")]
fn get_terminal_content() -> Option<String> {
    use std::process::Command;
    // Detect which terminal is frontmost and use its API
    let script = r#"
        tell application "System Events"
            set frontApp to bundle identifier of first application process whose frontmost is true
        end tell
        if frontApp is "com.apple.Terminal" then
            tell application "Terminal" to get contents of selected tab of front window
        else if frontApp starts with "com.googlecode.iterm2" then
            tell application "iTerm2" to get contents of current session of current tab of current window
        else
            return ""
        end if
    "#;
    let out = Command::new("osascript").arg("-e").arg(script).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

#[cfg(target_os = "macos")]
fn get_ax_text() -> (String, String) {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;

    unsafe {
        let sys_wide = AXUIElementCreateSystemWide();
        let mut focused_app: CFTypeRef = std::ptr::null();
        let key = CFString::new("AXFocusedApplication");
        if AXUIElementCopyAttributeValue(sys_wide, key.as_concrete_TypeRef(), &mut focused_app) != 0 {
            CFRelease(sys_wide as _);
            return (String::new(), String::new());
        }

        // Window title
        let mut focused_window: CFTypeRef = std::ptr::null();
        let win_key = CFString::new("AXFocusedWindow");
        let title = if AXUIElementCopyAttributeValue(focused_app as AXUIElementRef, win_key.as_concrete_TypeRef(), &mut focused_window) == 0 {
            let mut title_val: CFTypeRef = std::ptr::null();
            let title_key = CFString::new("AXTitle");
            if AXUIElementCopyAttributeValue(focused_window as AXUIElementRef, title_key.as_concrete_TypeRef(), &mut title_val) == 0 && !title_val.is_null() {
                let s = CFString::wrap_under_get_rule(title_val as _).to_string();
                CFRelease(title_val);
                s
            } else { String::new() }
        } else { String::new() };

        // Selected text from focused UI element
        let mut focused_elem: CFTypeRef = std::ptr::null();
        let elem_key = CFString::new("AXFocusedUIElement");
        let selected = if AXUIElementCopyAttributeValue(focused_app as AXUIElementRef, elem_key.as_concrete_TypeRef(), &mut focused_elem) == 0 {
            let mut sel_val: CFTypeRef = std::ptr::null();
            let sel_key = CFString::new("AXSelectedText");
            if AXUIElementCopyAttributeValue(focused_elem as AXUIElementRef, sel_key.as_concrete_TypeRef(), &mut sel_val) == 0 && !sel_val.is_null() {
                let s = CFString::wrap_under_get_rule(sel_val as _).to_string();
                CFRelease(sel_val);
                // Limit to 200 chars to keep prompt small
                if s.len() > 200 { s[..200].to_string() } else { s }
            } else {
                // Try AXValue as fallback (text field content near cursor)
                let mut val: CFTypeRef = std::ptr::null();
                let val_key = CFString::new("AXValue");
                if AXUIElementCopyAttributeValue(focused_elem as AXUIElementRef, val_key.as_concrete_TypeRef(), &mut val) == 0 && !val.is_null() {
                    let s = CFString::wrap_under_get_rule(val as _).to_string();
                    CFRelease(val);
                    // Take last 200 chars (text near cursor)
                    if s.len() > 200 { s[s.len()-200..].to_string() } else { s }
                } else { String::new() }
            }
        } else { String::new() };

        if !focused_window.is_null() { CFRelease(focused_window); }
        if !focused_elem.is_null() { CFRelease(focused_elem); }
        CFRelease(focused_app);
        CFRelease(sys_wide as _);

        (title, selected)
    }
}

pub(crate) fn tail_chars(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { s[s.len()-max..].to_string() }
}

#[cfg(target_os = "macos")]
use core_foundation::base::CFTypeRef;
#[cfg(target_os = "macos")]
extern "C" {
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(element: AXUIElementRef, attribute: core_foundation::string::CFStringRef, value: *mut CFTypeRef) -> i32;
    fn CFRelease(cf: CFTypeRef);
}
#[cfg(target_os = "macos")]
type AXUIElementRef = *const std::ffi::c_void;

#[cfg(not(target_os = "macos"))]
pub fn get_active_app() -> AppContext {
    AppContext::default()
}

pub(crate) fn categorize_app(bundle_id: &str) -> String {
    match bundle_id {
        b if b.contains("mail") || b.contains("Outlook") => "email",
        b if b.contains("slack") => "slack",
        b if b.contains("VSCode") || b.contains("Xcode") => "code",
        b if b.contains("notion") => "notes",
        b if b.contains("Terminal") || b.contains("iTerm") => "terminal",
        _ => "default",
    }.into()
}

pub(crate) fn tone_for_category(category: &str) -> String {
    match category {
        "email" => "Professional, concise.",
        "slack" => "Casual, conversational.",
        "code" => "Technical. Use backticks for code references.",
        "terminal" => "Command-like. Be terse.",
        "notes" => "Structured. Use headers and lists where appropriate.",
        _ => "Natural, clear prose.",
    }.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- categorize_app ---
    #[test]
    fn categorize_known_apps() {
        assert_eq!(categorize_app("com.apple.mail"), "email");
        assert_eq!(categorize_app("com.microsoft.Outlook"), "email");
        assert_eq!(categorize_app("com.tinyspeck.slackmacgap"), "slack");
        assert_eq!(categorize_app("com.microsoft.VSCode"), "code");
        assert_eq!(categorize_app("com.apple.dt.Xcode"), "code");
        assert_eq!(categorize_app("com.apple.Terminal"), "terminal");
        assert_eq!(categorize_app("com.googlecode.iTerm2"), "terminal");
        assert_eq!(categorize_app("notion.id"), "notes");
    }

    #[test]
    fn categorize_unknown_app() {
        assert_eq!(categorize_app("com.random.app"), "default");
        assert_eq!(categorize_app(""), "default");
    }

    // --- tone_for_category ---
    #[test]
    fn tone_mapping() {
        assert!(tone_for_category("email").contains("Professional"));
        assert!(tone_for_category("slack").contains("Casual"));
        assert!(tone_for_category("code").contains("Technical"));
        assert!(tone_for_category("terminal").contains("terse"));
        assert!(tone_for_category("notes").contains("Structured"));
        assert!(tone_for_category("default").contains("Natural"));
        assert!(tone_for_category("unknown").contains("Natural"));
    }

    // --- tail_chars ---
    #[test]
    fn tail_chars_short_string() {
        assert_eq!(tail_chars("hello", 10), "hello");
    }

    #[test]
    fn tail_chars_exact_length() {
        assert_eq!(tail_chars("hello", 5), "hello");
    }

    #[test]
    fn tail_chars_truncates() {
        assert_eq!(tail_chars("hello world", 5), "world");
    }

    #[test]
    fn tail_chars_empty() {
        assert_eq!(tail_chars("", 10), "");
    }

    // --- AppContext default ---
    #[test]
    fn app_context_default() {
        let ctx = AppContext::default();
        assert_eq!(ctx.app_name, "Unknown");
        assert_eq!(ctx.category, "default");
        assert!(ctx.tone.contains("Natural"));
        assert!(ctx.window_title.is_empty());
        assert!(ctx.selected_text.is_empty());
    }
}
