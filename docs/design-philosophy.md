# OpenFlow Design Philosophy

## Core Principle: Never Make the User Leave

OpenFlow is a background utility. The moment we force the user to think about OpenFlow instead of their actual work, we've failed. Every design decision flows from this.

## The Three Rules

### 1. The Pill Is the Interface

The pill is always visible, always on top, and always the single source of truth. It's the only surface we control, so it must do everything: status, guidance, errors, progress.

- **No popups, no modals, no separate windows.** If we can't say it in the pill, we shouldn't say it.
- **No notifications.** The user might have DND on. The pill is already there.
- **No stealing focus.** The user is in another app. Show the pill, don't activate it.

The pill is a heads-up display, not a dialog box.

### 2. Guide In Place, Not In Docs

When the user needs to do something (grant a permission, wait for a download), tell them what to do right where they're already looking. Assume they will never read documentation.

- **Progressive disclosure.** Show the next step, not all steps. `⚠ Enable Accessibility →` becomes `Click + → add OpenFlow → toggle on` only after they click.
- **Assume the worst case.** On first launch, OpenFlow isn't in the Accessibility list. The hint must account for the full flow (add + enable), not just the toggle.
- **Auto-detect completion.** Poll for permission changes, check for model files. The moment the user finishes, the pill should react — no "click here to continue" buttons.
- **Celebrate subtly.** Transition from guidance to "Ready" is the reward. No confetti, no "Setup complete!" splash screens.

### 3. Zero Restarts, Zero External Steps

The app must go from first launch to working dictation without the user ever closing it, reopening it, or running a terminal command.

- **Download everything automatically.** Models, binaries, whatever the app needs. Show progress in the pill.
- **Hot-reload permissions.** Poll until granted. The user toggles a switch in System Settings, the pill clears the warning within seconds.
- **Degrade gracefully.** If the LLM isn't ready yet, dictation still works — just without polish. If accessibility isn't granted, dictation still works — just show the text in the pill instead of pasting.
- **Never error with "install X".** If we depend on it, we ship it or download it.

## Decision Framework

When facing a UX choice, ask in order:

1. **Can the pill handle it?** → Do it in the pill.
2. **Does the user need to act?** → Tell them the next step, not all steps. Auto-detect when they're done.
3. **Does it require a restart?** → Find a way to hot-reload it. If truly impossible, explain why in the pill and make restart one click.
4. **Does it require an external tool?** → Bundle it or download it. The user should never open a terminal.
5. **Can we skip it entirely?** → If a feature can work without this step (even degraded), let it. Ask for the permission when it's actually needed, not preemptively.

## Examples

| Situation | Wrong | Right |
|-----------|-------|-------|
| Accessibility not granted | Show warning forever, require restart | Clickable pill → opens Settings, shows step-by-step hint, auto-detects grant |
| Models missing | "Error: model not found" | Auto-download with progress in pill |
| llama-server not installed | "Install: brew install llama.cpp" | Download binary automatically alongside models |
| Mic permission denied | App crashes or hangs | Pill shows "Grant mic access", retry on next listen attempt |
| Download fails mid-way | Corrupt file, crash on load | `.part` temp files, resume on retry |
| LLM not ready but ASR works | Block everything until LLM loads | Dictate without polish, enable polish when ready |
