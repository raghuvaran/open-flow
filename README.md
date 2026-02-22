# OpenFlow

[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/raghuvaran/open-flow)

Local-first, system-wide AI voice dictation for macOS. Speak anywhere, get polished text — entirely on-device.

OpenFlow runs as a transparent pill-shaped overlay that sits above your dock. Press a shortcut, speak naturally, and your words appear as clean text in whatever app has focus. No cloud APIs, no subscriptions, no data leaving your machine.

## How It Works

```
Mic → VAD → Chunker → Whisper ASR (~300ms) → LLM Polish (~500ms) → Paste
```

1. **Capture** — Records from any mic, resamples to 16kHz mono
2. **Voice Activity Detection** — Energy-based detection segments speech from silence
3. **Transcription** — Whisper base model converts speech to text locally
4. **Polish** — Qwen 2.5 3B fixes grammar, punctuation, and formatting via llama-server
5. **Inject** — Pastes the result into the active app via CGEvent Cmd+V simulation

Everything runs on your Mac. The 15 MB app auto-downloads ~2 GB of models on first launch.

## Features

- **System-wide dictation** — Works in any app. Speak in your editor, browser, terminal, Slack
- **AI polish** — Raw transcription is cleaned up by a local LLM (toggleable)
- **Walkie-talkie mode** — Hold shortcut to record, release to process. No auto-triggering
- **Toggle mode** — Press shortcut to start listening, press again to stop
- **Transparent overlay** — Draggable pill with waveform visualization, always on top
- **Menu bar tray** — AI Polish toggle, walkie-talkie mode, mic selector, auto-start
- **Auto-download** — Models download automatically on first launch with progress UI
- **Position memory** — Pill position persists across launches

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    macOS System                          │
│  ┌──────────┐    ┌──────────────────────────────────┐   │
│  │  Any App  │◄──│  CGEvent Cmd+V (clipboard paste)  │   │
│  └──────────┘    └──────────────┬───────────────────┘   │
│                                 │                        │
│  ┌──────────────────────────────┴───────────────────┐   │
│  │              Tauri v2 (Rust Backend)              │   │
│  │                                                   │   │
│  │  ┌─────────┐  ┌─────────┐  ┌──────────────────┐  │   │
│  │  │  Audio   │  │   ASR   │  │   LLM Polish     │  │   │
│  │  │ Capture  │─►│ Whisper │─►│  Qwen 2.5 3B     │  │   │
│  │  │  (cpal)  │  │  (.cpp) │  │ (llama-server)   │  │   │
│  │  └────┬────┘  └─────────┘  └──────────────────┘  │   │
│  │       │                                           │   │
│  │  ┌────┴────┐  ┌─────────┐  ┌──────────────────┐  │   │
│  │  │   VAD   │  │ SQLite  │  │  Model Download   │  │   │
│  │  │ Silero  │  │Settings │  │  (HuggingFace)    │  │   │
│  │  └─────────┘  └─────────┘  └──────────────────┘  │   │
│  └──────────────────────────────────────────────────┘   │
│                          │                               │
│  ┌───────────────────────┴──────────────────────────┐   │
│  │           SvelteKit Frontend (Webview)            │   │
│  │     Pill overlay · Waveform · Controls · Tray     │   │
│  └──────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
```

## Requirements

- macOS 11.0+ (Apple Silicon or Intel)
- ~2.5 GB disk space for models
- Microphone permission
- Accessibility permission (for text paste)

## Install

### Download (recommended)

1. Go to [Releases](https://github.com/raghuvaran/open-flow/releases/latest)
2. Download the `.dmg` file
3. Open the `.dmg` and drag OpenFlow to `/Applications`

### From Source

```bash
rustup target add aarch64-apple-darwin  # if on Apple Silicon

npm install
MACOSX_DEPLOYMENT_TARGET=11.0 npm run tauri build

cp -R src-tauri/target/release/bundle/macos/OpenFlow.app /Applications/
```

### macOS Gatekeeper Notice

Since OpenFlow is not yet code-signed with Apple, macOS will show **"OpenFlow Not Opened"** on first launch. To fix this:

**Option A — Terminal (quickest):**
```bash
xattr -cr /Applications/OpenFlow.app
```

**Option B — System Settings:**
1. Try to open OpenFlow (you'll see the warning — click **Done**, not "Move to Trash")
2. Open **System Settings → Privacy & Security**
3. Scroll down — you'll see *"OpenFlow was blocked"* with an **Open Anyway** button
4. Click **Open Anyway** and authenticate

> **Note:** On macOS Sequoia (15+), the old right-click → Open workaround no longer works. Use one of the methods above.

Once opened the first time, macOS will remember your choice.

### First Launch

1. Open OpenFlow from Spotlight or `/Applications`
2. Grant **Microphone** access when prompted
3. Click the **⚠ Enable Accessibility →** link in the pill (opens System Settings directly), then add OpenFlow
4. Models and llama-server download automatically (~2 GB, takes a few minutes)
5. Pill shows "Ready" — you're good to go

## Usage

| Action | How |
|--------|-----|
| **Toggle listening** | `Ctrl+Shift+Space` |
| **Walkie-talkie** | Hold `Ctrl+Shift+Space`, speak, release |
| **Show pill** | Tray → Show OpenFlow |
| **Hide pill** | Hover pill → click ✕ (stops listening) |
| **Move pill** | Drag the pill anywhere |
| **Toggle AI polish** | Tray → AI Polish |
| **Switch mic** | Tray → Microphone → select device |
| **Quit** | Tray → Quit OpenFlow (`Cmd+Q`) |

## Project Structure

```
├── src/                          # Svelte frontend (pill overlay UI)
│   └── routes/+page.svelte      # Waveform, controls, event listeners
├── src-tauri/
│   └── src/
│       ├── lib.rs                # App setup, tray, commands, audio loop
│       ├── audio/
│       │   ├── capture.rs        # cpal mic input, resample, mono
│       │   ├── vad.rs            # Silero VAD wrapper
│       │   └── chunker.rs        # Speech segment detection
│       ├── asr/engine.rs         # whisper.cpp transcription
│       ├── polish/
│       │   ├── engine.rs         # llama-server HTTP client
│       │   └── prompt.rs         # System prompt builder
│       ├── pipeline/
│       │   └── orchestrator.rs   # ASR → polish → inject pipeline
│       ├── inject/clipboard.rs   # CGEvent Cmd+V paste
│       ├── models/download.rs    # Auto-download from HuggingFace
│       ├── db/                   # SQLite settings persistence
│       └── config.rs             # Paths and defaults
└── tauri.conf.json
```

## Models

Downloaded to `~/Library/Application Support/openflow/models/`:

| Model | Size | Purpose |
|-------|------|---------|
| `silero_vad.onnx` | 2 MB | Voice activity detection |
| `ggml-base.bin` | 141 MB | Whisper base — speech to text |
| `qwen2.5-3b-instruct-q4_k_m.gguf` | 2 GB | Text polish and formatting |

## Tech Stack

- **Tauri 2** + **Svelte** — App framework and UI
- **whisper-rs** — Whisper.cpp Rust bindings for ASR
- **llama.cpp** — LLM inference via llama-server
- **cpal** — Cross-platform audio capture
- **core-graphics** — macOS CGEvent API for keystroke simulation
- **SQLite** — Settings and position persistence

## Recommended IDE Setup

[VS Code](https://code.visualstudio.com/) + [Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## License

MIT
