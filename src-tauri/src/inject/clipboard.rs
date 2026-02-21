use anyhow::Result;
use arboard::Clipboard;
use std::thread;
use std::time::Duration;

extern "C" {
    fn AXIsProcessTrustedWithOptions(options: *const std::ffi::c_void) -> bool;
}

/// Check if the app has Accessibility permission.
pub fn check_accessibility() -> bool {
    unsafe { AXIsProcessTrustedWithOptions(std::ptr::null()) }
}

/// Simulate Cmd+V keystroke using CGEvent API directly.
fn simulate_paste() -> Result<()> {
    use core_graphics::event::{CGEvent, CGEventFlags, CGKeyCode};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| anyhow::anyhow!("Failed to create CGEventSource"))?;

    let key_v: CGKeyCode = 9;

    let key_down = CGEvent::new_keyboard_event(source.clone(), key_v, true)
        .map_err(|_| anyhow::anyhow!("Failed to create key down event"))?;
    key_down.set_flags(CGEventFlags::CGEventFlagCommand);

    let key_up = CGEvent::new_keyboard_event(source, key_v, false)
        .map_err(|_| anyhow::anyhow!("Failed to create key up event"))?;
    key_up.set_flags(CGEventFlags::CGEventFlagCommand);

    key_down.post(core_graphics::event::CGEventTapLocation::HID);
    key_up.post(core_graphics::event::CGEventTapLocation::HID);

    Ok(())
}

/// Inject text at cursor via clipboard paste simulation.
pub fn inject_text(text: &str) -> Result<()> {
    let mut clip = Clipboard::new()?;
    let original = clip.get_text().ok();

    clip.set_text(text)?;
    thread::sleep(Duration::from_millis(50));

    simulate_paste()?;

    thread::sleep(Duration::from_millis(100));
    if let Some(orig) = original {
        let _ = clip.set_text(&orig);
    }

    Ok(())
}
