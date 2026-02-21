# OpenFlow — Local-First AI Voice Dictation

## Final Design Document

**Project:** Open-source, fully local AI voice dictation
**Codename:** OpenFlow
**Status:** Implemented & Working (v0.1.0)
**Date:** February 2026
**Target Platforms:** macOS (primary), Windows, Linux (stretch)
**License:** MIT

> **Implementation Note (Feb 2026):** This document has been updated with
> hard-won lessons from the actual implementation. Sections marked with ⚡
> contain critical deviations from the original design that were discovered
> during development. The app is functional: mic → ASR (~300ms) → LLM polish
> (~600-1400ms) → clipboard paste. Total pipeline: 1-2s per utterance.

---

## 1. Product Vision

Build a system-wide, always-on voice dictation layer that runs 100% on-device.
The user presses a hotkey, speaks naturally (with fillers, self-corrections,
pauses, accents, or even whispers), and polished, context-aware text appears at
their cursor in any application with sub-second perceived latency and zero data
leaving the machine.

### 1.1 Core Capabilities

| Capability | Description | Priority |
|---|---|---|
| Universal text injection | Works in every app with a text field | P0 |
| Filler removal & polish | Removes "um/uh/like/you know"; fixes grammar & punctuation | P0 |
| Self-correction understanding | "Meet at 2pm — no, 4pm tomorrow" → outputs only the correction | P0 |
| Context-aware formatting | Detects active app, adjusts tone (email vs Slack vs code) | P0 |
| Sub-800ms perceived latency | From end-of-speech to polished text at cursor | P0 |
| Push-to-talk + toggle modes | Fn double-tap (long dictation), hold (quick burst) | P0 |
| Voice commands | "New paragraph", "make this a list", "rewrite professionally" | P1 |
| Whisper mode | Works with quiet/whispered speech in shared spaces | P1 |
| Personal dictionary | Learns custom names, acronyms, jargon | P1 |
| 100+ language support | Auto-detect language, multilingual dictation | P1 |
| Code variable recognition | Formats spoken vars as camelCase/snake_case per context | P2 |

### 1.2 Non-Goals (v1)

- Mobile (iOS/Android) — separate project, different constraints entirely
- Cloud fallback — strictly offline, no network calls ever
- Online model training — optional offline LoRA is a stretch goal
- Real-time collaboration features

---

## 2. Target Hardware & Performance Budget

### 2.1 Reference Hardware

| Tier | Example | Expected E2E Latency | Notes |
|---|---|---|---|
| Optimal | Apple M3 Pro+ / M4, RTX 4060+ | < 500ms | Full large models, GPU accel |
| Good | Apple M1/M2, RTX 3060, AMD 7800 | 500–800ms | Medium models, quantized |
| Minimum | Any 8-core CPU, 16GB RAM, no GPU | 800–1500ms | Small models, CPU-only fallback |

### 2.2 Latency Budget Breakdown (Target: < 800ms total)

```
┌─────────────────────────────────────────────────────────┐
│ Phase              │ Budget  │ Strategy                  │
├────────────────────┼─────────┼───────────────────────────┤
│ VAD + silence det  │  50ms   │ Silero VAD (ONNX, ~2ms)   │
│ Audio buffering    │  0ms    │ Streamed, no extra wait    │
│ ASR inference      │ 200ms   │ whisper.cpp, GPU/Metal     │
│ LLM polish         │ 350ms   │ llama.cpp, 3B Q4 model    │
│ Text injection     │  50ms   │ Clipboard paste simulation │
│ Overhead/margin    │ 150ms   │ IPC, context gathering     │
│ TOTAL              │ 800ms   │                            │
└─────────────────────────────────────────────────────────┘
```

> ⚡ **Actual measured latency (MacBook Pro, M-series, CPU-only whisper, Metal llama-server):**
> ```
> VAD (energy-based fallback):  < 1ms per 30ms frame
> ASR (whisper-base, CPU):      300-400ms per 1-5s segment
> LLM polish (Qwen 3B, Metal):  600-1400ms (via llama-server HTTP)
> Text injection (osascript):    ~150ms (clipboard + paste)
> TOTAL:                         1.0-2.0s per utterance
> ```
> The 800ms target is achievable with GPU-accelerated whisper (Metal) and
> shorter utterances. The LLM polish step is the bottleneck — llama-server
> model loading is ~11s on first start but stays resident thereafter.

Key insight from 2026 benchmarks: Whisper v3 Turbo achieves 30x real-time on
M4 Pro/Max. A 3-second utterance transcribes in ~100ms. The LLM polish step
is the actual bottleneck — hence the choice of a small, fast model (3B params).

---

## 3. Architecture Overview

### 3.1 High-Level Data Flow

```
                          ┌──────────────────────────┐
                          │   System Tray / Settings  │
                          │   (Tauri 2 + Svelte)      │
                          └────────────┬─────────────┘
                                       │ IPC (Tauri commands)
┌──────────┐    ┌──────────────────────▼──────────────────────────┐
│  Global   │    │              RUST CORE DAEMON                   │
│  Hotkey   │───▶│                                                 │
│ Listener  │    │  ┌─────────┐  ┌─────────┐  ┌───────────────┐  │
└──────────┘    │  │  Audio   │  │   ASR   │  │  LLM Polish   │  │
                │  │ Pipeline │─▶│ Engine  │─▶│    Engine      │  │
                │  │(cpal+VAD)│  │(whisper │  │ (llama.cpp)   │  │
                │  └─────────┘  │  .cpp)  │  └───────┬───────┘  │
                │               └─────────┘          │           │
                │  ┌──────────────┐  ┌───────────────▼────────┐  │
                │  │   Context    │  │    Text Injector       │  │
                │  │  Detector    │──│  (Accessibility API /  │  │
                │  │ (active app) │  │   Clipboard + Paste)   │  │
                │  └──────────────┘  └────────────────────────┘  │
                │  ┌──────────────┐                               │
                │  │  Local DB    │  (SQLite: dict, snippets,    │
                │  │  (rusqlite)  │   history, app tone maps)    │
                │  └──────────────┘                               │
                └─────────────────────────────────────────────────┘
```

### 3.2 Why This Architecture

1. **Rust core** — Zero-cost abstractions, no GC pauses, direct C/C++ FFI to
   whisper.cpp and llama.cpp. Critical for hitting latency targets.

2. **Tauri 2 shell** — Native webview (not Electron/Chromium), idle RAM < 50MB.
   Perfect for an always-on background tool. System tray, global shortcuts,
   and auto-update are first-class Tauri plugins.

3. **Embedded inference** — whisper.cpp is linked directly into the Rust binary
   via the `whisper-rs` FFI crate. No separate server, no Python, no Docker.

   > ⚡ **Critical lesson: ggml symbol collision.** The original design called
   > for both `whisper-rs` and `llama-cpp-rs` linked into the same binary.
   > This causes a **fatal SIGABRT/SIGSEGV** on macOS. Both crates bundle
   > their own copy of ggml. `llama-cpp-sys-2` unconditionally compiles the
   > Metal backend via cmake auto-detection (even without the `metal` cargo
   > feature). During whisper's `whisper_init_with_params_no_state`, the ggml
   > backend registry calls `ggml_backend_metal_reg_device_get` from llama's
   > ggml copy, which hits `GGML_ASSERT(index == 0)` and aborts.
   >
   > **Solution:** LLM inference runs out-of-process via `llama-server`
   > (HTTP API on localhost:8384). The server is spawned as a child process
   > on model load and killed on app exit. This completely eliminates the
   > symbol collision while keeping everything local. `brew install llama.cpp`
   > provides the `llama-server` binary.

4. **Modular pipeline** — Each stage (audio → ASR → polish → inject) is a
   separate async task communicating via tokio channels. Stages can be
   independently upgraded, benchmarked, or swapped.

---

## 4. Component Deep Dives

### 4.1 Activation & UX Layer

**Stack:** Tauri 2.x + Svelte (minimal UI) + tauri-plugin-global-shortcut

The UI is intentionally minimal — this is a background tool, not an app you
stare at. The visible surface area is:

- **System tray icon** with listening/idle/processing states (animated)
- **Floating recording indicator** — small pill at screen bottom (waveform bar), shows recording state + live partial transcript
- **Settings panel** — model download/selection, hotkey config, personal
  dictionary editor, per-app tone mappings, hardware acceleration toggle
- **Onboarding flow** — first-run model download, accessibility permission
  grant, hotkey tutorial

**Activation modes:**

| Mode | Trigger | Behavior |
|---|---|---|
| Quick dictation | Hold `Fn` (or configured key) | Records while held, processes on release |
| Long dictation | Double-tap `Fn` | Toggle on/off, auto-stops on extended silence |
| Command mode | `Fn` + speak command | "Rewrite this professionally", "make a list" |

**Implementation notes:**
- `tauri-plugin-global-shortcut` for cross-platform hotkey registration
- On macOS, intercept `Fn` key via `CGEventTap` (requires Accessibility permission)
- Floating indicator is a borderless, always-on-top Tauri window with
  `set_decorations(false)` and `set_always_on_top(true)`
- State machine: `Idle → Listening → Processing → Injecting → Idle`

### 4.2 Audio Pipeline

**Stack:** `cpal` (Rust) + Silero VAD (ONNX Runtime) + RNNoise

```
Microphone (cpal, 16kHz mono PCM)
    │
    ▼
Ring Buffer (lock-free, ringbuf crate)
    │
    ├──▶ Silero VAD (ONNX, runs every 30ms frame)
    │       │
    │       ├── Speech detected → start/continue recording
    │       └── Silence > threshold → trigger ASR
    │
    ├──▶ RNNoise (optional noise suppression)
    │
    └──▶ Audio chunks → ASR Engine
```

> ⚡ **Actual implementation differs significantly:**
> ```
> Microphone (cpal, device default config — typically 48kHz)
>     │
>     ▼
> Resample to 16kHz mono (linear interpolation in cpal callback)
>     │
>     ▼
> mpsc::UnboundedSender<Vec<f32>> (no ring buffer)
>     │
>     ├──▶ Energy-based VAD (RMS > 0.01 threshold per 480-sample frame)
>     │       │  (Silero VAD fallback — ort crate panics without libonnxruntime.dylib)
>     │       │
>     │       ├── Speech detected → chunker accumulates
>     │       └── Silence > 700ms → segment finalized
>     │
>     └──▶ Segments > 4800 samples (0.3s) → Orchestrator
> ```

**Key design decisions:**

- **Sample rate:** 16kHz, 1-channel — this is what Whisper expects natively.
  No resampling overhead.

  > ⚡ **MacBook Pro mic does NOT support 16kHz directly.** It runs at 48kHz.
  > Forcing 16kHz via cpal config causes `"The requested stream configuration
  > is not supported by the device"`. Must use `device.default_input_config()`
  > and resample in the audio callback. Linear interpolation + mono downmix
  > adds negligible overhead.

- **Ring buffer:** Lock-free `ringbuf` crate. Audio capture thread never blocks.
  Consumer (VAD + ASR) reads at its own pace.

  > ⚡ **Replaced with `mpsc::UnboundedSender`.** Simpler, no ring buffer
  > sizing issues. The cpal callback sends resampled chunks directly. The
  > consumer loop processes them in a `tokio::select!` alongside the stop
  > signal.

- **VAD:** Silero VAD v5 via ONNX Runtime. Runs in < 2ms per 30ms frame on CPU.

  > ⚡ **Silero VAD requires `libonnxruntime.dylib` installed on the system.**
  > The `ort` crate with `load-dynamic` feature panics at runtime if the dylib
  > is missing. Wrapped in `std::panic::catch_unwind` with fallback to
  > energy-based RMS detection (threshold 0.01). The energy-based fallback
  > works surprisingly well for normal speech but triggers on loud background
  > noise. Install ONNX Runtime (`brew install onnxruntime`) for proper VAD.
- **Silence threshold:** Configurable, default 700ms. When VAD detects this
  much continuous silence, the audio buffer is finalized and sent to ASR.
  For "quick dictation" (hold mode), release of the key triggers immediately.
- **Whisper mode support:** Lower VAD sensitivity threshold + RNNoise
  amplification for quiet speech. Silero handles this well — it was trained
  on whispered speech datasets.
- **Noise suppression:** RNNoise (via `nnnoiseless` Rust crate) is optional
  and togglable. Adds ~1ms latency. Recommended for noisy environments.

### 4.3 ASR Engine (Speech-to-Text)

**Stack:** whisper.cpp via `whisper-rs` crate

> ⚡ **Implementation note:** Using `whisper-rs = "0.13"` (not 0.12 as
> originally planned). GPU/Metal is disabled (`params.use_gpu(false)`) because
> enabling it while llama-cpp-sys was in the binary caused the ggml symbol
> collision. Now that llama-cpp is out-of-process, Metal could be re-enabled
> for whisper — this would cut ASR time from ~300ms to ~100ms.

**Why whisper.cpp over alternatives:**

| Option | Pros | Cons | Verdict |
|---|---|---|---|
| whisper.cpp | Fastest local Whisper, Metal/CUDA/CPU, mature, GGML quantization | Not truly streaming (encoder-decoder) | **Primary choice** |
| MLX Whisper | Insane speed on Apple Silicon (30x RT on M4) | macOS only, Python bindings | macOS fast-path option |
| NVIDIA Parakeet | Native streaming, punctuation-aware | ONNX only, less multilingual | Future alternative |
| Distil-Whisper | Smaller, faster | Lower accuracy on accents | CPU fallback option |

**Model selection strategy (auto-detected on first run):**

```
if GPU/Metal available AND RAM >= 16GB:
    model = "whisper-large-v3-turbo"     # Best accuracy, 30x RT on M4
elif GPU available AND RAM >= 8GB:
    model = "whisper-medium"              # Good balance
elif CPU only:
    model = "whisper-small" or "distil-whisper-small"  # Fast on CPU
```

**Pseudo-streaming approach (critical for perceived latency):**

Whisper is an encoder-decoder model — it needs complete audio segments. True
token-by-token streaming isn't possible. Instead, we use VAD-segmented chunking:

```
1. VAD detects speech onset → start buffering
2. VAD detects pause (300ms) within speech → segment boundary
3. Send completed segment to whisper.cpp immediately
4. Continue buffering next segment in parallel
5. VAD detects end-of-speech (700ms silence) → send final segment
```

This gives the illusion of streaming: short sentences appear almost instantly
because each 2-4 second segment processes in ~100-200ms on GPU hardware.

**Output format:** Raw text with timestamps per segment. Language auto-detected
per segment (Whisper's built-in capability). Passed directly to LLM polish.

**Hardware acceleration compilation flags:**
- macOS: `WHISPER_METAL=1` (Metal GPU) + Core ML for Neural Engine
- Windows: `WHISPER_CUDA=1` (NVIDIA) or `WHISPER_DIRECTML=1` (AMD/Intel)
- Linux: `WHISPER_CUDA=1` or `WHISPER_VULKAN=1`
- Fallback: CPU with AVX2/NEON SIMD

### 4.4 LLM Polish Engine (The "Magic" Layer)

This is the core differentiator between a raw transcription tool and a polished
dictation experience. The LLM takes dirty ASR output and produces clean,
contextual, publication-ready text.

**Stack:** llama.cpp via `llama-server` (localhost HTTP API, out-of-process)

> ⚡ **Original design called for `llama-cpp-rs` embedded in-process.** This
> is impossible due to the ggml symbol collision (see §3.2). Two alternatives
> were tried:
>
> 1. **`llama-cli` subprocess** — spawns a new process per polish call. Too
>    slow: model loads from disk every time (~10-15s), and `llama-cli` in
>    generation mode doesn't stop cleanly (keeps generating tokens forever
>    without proper stop conditions).
>
> 2. **`llama-server` (chosen)** — spawned once as a child process on model
>    load. Exposes OpenAI-compatible `/v1/chat/completions` endpoint on
>    `localhost:8384`. Model stays in memory. Each polish call is a simple
>    HTTP POST via `ureq`. Response time: 600-1400ms. Server is killed on
>    app exit via `Drop` impl.
>
> Requires `brew install llama.cpp` which provides both `llama-cli` and
> `llama-server`. The server uses Metal GPU acceleration (`-ngl 99`) by
> default on Apple Silicon.

**Model selection:**

| Model | Size (Q4_K_M) | Speed (M3 Pro) | Quality | Recommendation |
|---|---|---|---|---|
| Qwen2.5-3B-Instruct | ~2.0 GB | ~120 tok/s | Good | **Default — best speed/quality** |
| Llama-3.2-3B-Instruct | ~2.0 GB | ~110 tok/s | Good | Alternative default |
| Phi-4-mini (3.8B) | ~2.3 GB | ~100 tok/s | Very good | If RAM allows |
| Qwen2.5-7B-Instruct | ~4.5 GB | ~60 tok/s | Excellent | Power users, 32GB+ RAM |
| Gemma-2-2B-IT | ~1.5 GB | ~150 tok/s | Decent | Low-RAM fallback |

**Why 3B and not 7B+ as default:** At Q4 quantization, a 3B model generates
100+ tokens/second on Apple Silicon. A typical polished sentence is 15-30
tokens. That's 150-300ms for the polish step — within our budget. A 7B model
at 60 tok/s would push us to 250-500ms, eating into our margin. The 3B models
in 2026 are remarkably capable at text editing tasks.

**System prompt (token-efficient, deterministic):**

```
You are a dictation polisher. You receive raw speech transcripts and output
ONLY the clean, polished text. Rules:

1. Remove ALL filler words (um, uh, like, you know, basically, actually, so)
2. Remove false starts and self-corrections — keep only the final intent
   Example: "Let's meet at 2pm no wait make it 4pm" → "Let's meet at 4pm"
3. Fix grammar, spelling, punctuation, and capitalization
4. Add paragraph breaks where the speaker clearly shifts topics
5. If the speaker says list-like content, format as a bullet list
6. Handle voice commands literally:
   - "new paragraph" → insert paragraph break
   - "new line" → insert line break
   - "period/comma/question mark" → insert punctuation
7. Match the tone specified in the context below
8. Preserve technical terms, proper nouns, and the speaker's vocabulary exactly
9. Output ONLY the polished text. No explanations, no quotes, no markdown
   fences.

CONTEXT:
- App: {active_app_name} ({app_category})
- Tone: {tone_directive}
- Personal vocab: {personal_dict_entries}
- Previous sentence: {last_injected_sentence}

RAW TRANSCRIPT:
{asr_output}

POLISHED:
```

**Tone directives per app category (stored in SQLite, user-editable):**

```json
{
  "email": "Professional, concise. Use proper salutations if starting a new message.",
  "slack": "Casual, conversational. Okay to use contractions.",
  "vscode": "Technical. Format code references in backticks. Use camelCase for variables.",
  "terminal": "Command-like. Be terse.",
  "notion": "Structured. Use headers and lists where appropriate.",
  "default": "Natural, clear prose. Match the formality of the surrounding text."
}
```

**Streaming generation:** llama.cpp supports token-by-token callback. We use
this to start injecting text as soon as the first tokens are generated, giving
the user immediate visual feedback. The full flow:

```
ASR complete → LLM starts generating → first token in ~50ms →
tokens stream to Text Injector → user sees text appearing in real-time
```

### 4.5 Voice Command Parser

Voice commands are detected before the LLM polish step. This is a lightweight
rule-based parser (not LLM-powered) for speed.

**Command detection flow:**

```
ASR output → Command Parser → if command detected:
                                  execute command action
                               else:
                                  pass to LLM polish
```

**Supported commands (v1):**

| Spoken Command | Action |
|---|---|
| "new paragraph" / "next paragraph" | Insert `\n\n` at cursor |
| "new line" / "next line" | Insert `\n` at cursor |
| "period" / "full stop" | Insert `.` |
| "comma" | Insert `,` |
| "question mark" | Insert `?` |
| "exclamation mark" / "exclamation point" | Insert `!` |
| "delete that" / "scratch that" | Undo last injection (select + delete) |
| "select all" | Select all text in current field |
| "undo" | Trigger Cmd+Z / Ctrl+Z |
| "make this a list" | Re-process last injection as bullet list via LLM |
| "rewrite this professionally" | Re-process last injection with formal tone |
| "rewrite this casually" | Re-process last injection with casual tone |
| "bold this" / "italicize this" | Wrap last injection in formatting (app-dependent) |

**Implementation:** Simple keyword matching on the raw ASR output. Commands are
stripped before any text reaches the LLM. For complex rewrite commands, the
last injected text is retrieved from the history buffer and re-sent to the LLM
with a modified prompt.

### 4.6 Context Detector

**Purpose:** Determine what app the user is typing in, and optionally read
surrounding text for better LLM context.

**macOS implementation (primary platform):**

```rust
// Via Swift FFI bridge (AXUIElement APIs require a proper app bundle)
fn get_active_context() -> AppContext {
    // 1. Get frontmost application
    let app = NSWorkspace::shared().frontmostApplication();
    let bundle_id = app.bundleIdentifier();  // e.g., "com.apple.mail"
    let app_name = app.localizedName();       // e.g., "Mail"

    // 2. Get focused UI element
    let focused = AXUIElementCopyAttributeValue(system_wide, kAXFocusedUIElement);

    // 3. Read surrounding text (if accessible)
    let value = AXUIElementCopyAttributeValue(focused, kAXValueAttribute);
    let selection = AXUIElementCopyAttributeValue(focused, kAXSelectedTextRangeAttribute);

    // 4. Get window title for additional context
    let title = AXUIElementCopyAttributeValue(window, kAXTitleAttribute);

    AppContext { bundle_id, app_name, window_title, surrounding_text, cursor_position }
}
```

**Windows implementation:**
- UIAutomation API via `uiautomation` crate
- `GetForegroundWindow` + `GetWindowText` for window title
- `IUIAutomationTextPattern` for surrounding text

**App category mapping (bundle_id → category):**

```
com.apple.mail, com.microsoft.Outlook     → "email"
com.tinyspeck.slackmacgap                 → "slack"
com.microsoft.VSCode, com.apple.dt.Xcode  → "code"
com.electron.notion                        → "notes"
com.googleusercontent.apps.chrome          → "browser" (refine by tab title)
*                                          → "default"
```

This mapping is stored in SQLite and user-extensible via the settings UI.

### 4.7 Text Injector

**The most critical component for the "it just works everywhere" feeling.**

Text must appear at the user's cursor in any application, seamlessly, as if
they typed it. This is harder than it sounds.

**Strategy: Clipboard Paste Simulation (primary) + Accessibility API (enhanced)**

> ⚡ **`enigo` crate crashes on macOS when called from non-main thread.**
> The `enigo` keyboard simulation calls `TSMGetInputSourceProperty` which
> triggers `dispatch_assert_queue_fail` — it must run on the main thread,
> but our pipeline runs on a tokio worker thread. **Replaced entirely with
> `osascript`:**
> ```rust
> Command::new("osascript")
>     .arg("-e")
>     .arg(r#"tell application "System Events" to keystroke "v" using command down"#)
>     .output()?;
> ```
> This requires Accessibility permission in System Preferences but works
> reliably from any thread. The `enigo` dependency was removed entirely.

```
Primary method (works everywhere):
1. Save current clipboard contents (via `arboard` crate)
2. Copy polished text to clipboard
3. Simulate Cmd+V (macOS) or Ctrl+V (Windows) via `enigo` crate
4. Restore original clipboard contents (after 100ms delay)

Enhanced method (macOS, when accessible):
1. Get focused AXUIElement
2. Read kAXSelectedTextRangeAttribute (cursor position)
3. Set kAXSelectedTextAttribute with polished text (direct insertion)
4. Falls back to clipboard method if AX API returns error
```

**Why clipboard-paste over keystroke simulation:**
- Keystroke simulation (`enigo` key-by-key) is too slow for large blocks
  (200+ chars would take 1-2 seconds of visible typing)
- Clipboard paste is instant regardless of text length
- Works in virtually every application including Electron apps, terminals,
  web browsers, and native apps

**Why we still need the Accessibility API path:**
- Some apps intercept Cmd+V differently (rich text editors, terminals)
- Direct AX insertion preserves undo history in the target app
- Allows reading surrounding text for better LLM context
- Required for "delete that" / "scratch that" commands (need to select
  the last injected range)

**Streaming injection approach:**

For longer dictations, we don't wait for the full LLM output. Instead:

```
1. LLM generates tokens → buffer until sentence boundary (. ! ? \n)
2. Inject completed sentence via clipboard-paste
3. Continue buffering next sentence
4. Result: text appears sentence-by-sentence with ~200ms gaps
```

This feels natural — like watching someone type very fast — rather than a
jarring block-paste after a long delay.

**Edge cases handled:**
- **Password fields:** Detect `kAXIsSecureTextField` → skip injection, notify user
- **Read-only fields:** Detect via AX attributes → skip, notify
- **Focus loss:** If user clicks away during processing, cancel injection
- **Undo support:** Track injected text ranges in a stack. "Delete that"
  selects the last range and deletes it. Cmd+Z also works because we use
  paste (which is undoable in most apps).

### 4.8 Persistence & Personalization

**Stack:** SQLite via `rusqlite`

**Schema:**

```sql
-- Personal dictionary: custom words, names, acronyms
CREATE TABLE personal_dict (
    id INTEGER PRIMARY KEY,
    spoken_form TEXT NOT NULL,      -- what the user says
    written_form TEXT NOT NULL,     -- what should be written
    category TEXT DEFAULT 'general', -- 'name', 'acronym', 'technical', 'general'
    usage_count INTEGER DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- App tone mappings
CREATE TABLE app_tones (
    id INTEGER PRIMARY KEY,
    bundle_id TEXT NOT NULL UNIQUE,
    app_name TEXT NOT NULL,
    category TEXT NOT NULL,          -- 'email', 'slack', 'code', etc.
    tone_directive TEXT NOT NULL,    -- injected into LLM system prompt
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Injection history (for undo, corrections, and learning)
CREATE TABLE injection_history (
    id INTEGER PRIMARY KEY,
    raw_transcript TEXT NOT NULL,
    polished_text TEXT NOT NULL,
    app_bundle_id TEXT,
    app_name TEXT,
    language TEXT,
    latency_ms INTEGER,
    user_edited INTEGER DEFAULT 0,  -- 1 if user manually corrected after
    corrected_text TEXT,            -- what user changed it to (if edited)
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Voice snippets: trigger phrases that expand to longer text
CREATE TABLE snippets (
    id INTEGER PRIMARY KEY,
    trigger_phrase TEXT NOT NULL,    -- "my address", "email signature"
    expansion TEXT NOT NULL,         -- the full expanded text
    match_type TEXT DEFAULT 'fuzzy', -- 'exact' or 'fuzzy'
    usage_count INTEGER DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Model configuration
CREATE TABLE model_config (
    id INTEGER PRIMARY KEY,
    model_type TEXT NOT NULL,        -- 'asr' or 'llm'
    model_name TEXT NOT NULL,
    model_path TEXT NOT NULL,
    quantization TEXT,
    is_active INTEGER DEFAULT 0,
    download_url TEXT,
    file_size_bytes INTEGER,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

**Personalization pipeline:**

```
1. User speaks → ASR → LLM polish → inject
2. If user manually edits the injected text within 30 seconds:
   a. Detect edit via Accessibility API (poll focused element value)
   b. Store correction in injection_history (user_edited=1, corrected_text=...)
   c. If same correction pattern appears 3+ times:
      → Auto-add to personal_dict
      → Notify user: "Learned: 'kubernetes' should be 'Kubernetes'"
3. Personal dict entries are injected into LLM prompt on every call
4. Snippet triggers are matched against raw ASR output before LLM processing
```

### 4.9 Model Management

**First-run experience:**

```
1. App launches → detects no models installed
2. Hardware detection: GPU type, VRAM/unified memory, CPU cores
3. Recommends optimal model pair (ASR + LLM) based on hardware
4. Downloads from Hugging Face (GGUF format) with progress bar
5. Verifies SHA256 checksum
6. Stores in ~/.openflow/models/
7. Loads models into memory, runs a quick self-test transcription
8. Ready to use
```

**Model registry (bundled JSON, updatable):**

```json
{
  "asr_models": [
    {
      "name": "whisper-large-v3-turbo",
      "file": "ggml-large-v3-turbo-q5_0.bin",
      "size_mb": 1100,
      "min_ram_gb": 8,
      "requires_gpu": false,
      "gpu_recommended": true,
      "languages": 100,
      "hf_repo": "ggerganov/whisper.cpp",
      "tier": "optimal"
    },
    {
      "name": "whisper-medium",
      "file": "ggml-medium-q5_0.bin",
      "size_mb": 500,
      "min_ram_gb": 4,
      "requires_gpu": false,
      "tier": "good"
    },
    {
      "name": "whisper-small",
      "file": "ggml-small.bin",
      "size_mb": 250,
      "min_ram_gb": 2,
      "requires_gpu": false,
      "tier": "minimum"
    }
  ],
  "llm_models": [
    {
      "name": "Qwen2.5-3B-Instruct",
      "file": "qwen2.5-3b-instruct-q4_k_m.gguf",
      "size_mb": 2000,
      "min_ram_gb": 4,
      "speed_tier": "fast",
      "quality_tier": "good",
      "tier": "default"
    },
    {
      "name": "Qwen2.5-7B-Instruct",
      "file": "qwen2.5-7b-instruct-q4_k_m.gguf",
      "size_mb": 4500,
      "min_ram_gb": 8,
      "speed_tier": "medium",
      "quality_tier": "excellent",
      "tier": "power"
    },
    {
      "name": "Gemma-2-2B-IT",
      "file": "gemma-2-2b-it-q4_k_m.gguf",
      "size_mb": 1500,
      "min_ram_gb": 2,
      "speed_tier": "very_fast",
      "quality_tier": "decent",
      "tier": "lightweight"
    }
  ]
}
```

**Hot-swapping:** Models can be changed at runtime via settings. The old model
is unloaded, new one loaded. Takes 2-5 seconds. No app restart needed.

---

## 5. Tech Stack Summary

| Layer | Technology | Why |
|---|---|---|
| App shell | Tauri 2.x | Native webview, < 50MB idle RAM, system tray, auto-update |
| Frontend | Svelte 5 | Tiny bundle, fast, minimal UI needs |
| Core runtime | Rust (tokio async) | Zero-cost, no GC, direct FFI to C/C++ inference libs |
| Audio capture | cpal | Cross-platform, low-latency, pure Rust |
| VAD | Silero VAD v5 (ONNX) | < 2ms per frame, enterprise-grade accuracy |
| Noise suppression | nnnoiseless (RNNoise) | ~1ms, Rust-native RNNoise port |
| ASR | whisper.cpp (whisper-rs) | Fastest local Whisper, Metal/CUDA/CPU |
| LLM | llama.cpp (llama-server, out-of-process) | ⚡ Cannot embed in-process (ggml collision) |
| Text injection | osascript + arboard | ⚡ enigo crashes from non-main thread on macOS |
| Accessibility | AXUIElement (macOS) / UIAutomation (Win) | Context reading, direct text insertion |
| Database | SQLite (rusqlite) | Embedded, zero-config, fast |
| Hotkeys | tauri-plugin-global-shortcut | Cross-platform, Tauri-native |
| IPC | tokio::sync::mpsc channels | Lock-free async communication between pipeline stages |
| Build | Cargo + Tauri CLI | Single command builds for all platforms |
| CI/CD | GitHub Actions | macOS + Windows + Linux matrix builds |
| Distribution | Tauri bundler | .dmg (macOS), .msi (Windows), .AppImage (Linux) |

### 5.1 Rust Crate Dependency Map

> ⚡ **Actual dependencies (as implemented):**

```toml
[dependencies]
# App framework
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-opener = "2"
tauri-plugin-global-shortcut = "2"

# Async runtime
tokio = { version = "1", features = ["full"] }

# Audio
cpal = "0.15"
# ringbuf REMOVED — mpsc channels used instead

# Inference
whisper-rs = "0.13"          # whisper.cpp bindings (GPU disabled to avoid Metal issues)
# llama-cpp-2 REMOVED — causes ggml symbol collision with whisper-rs
ort = { version = "2.0.0-rc.11", features = ["load-dynamic"] }  # Silero VAD (optional)
ndarray = "0.17"             # For ort tensor operations

# HTTP client (for llama-server communication)
ureq = "3"

# Text injection
# enigo REMOVED — crashes on macOS from non-main thread
arboard = "3"                # Clipboard access (paste via osascript)

# Database
rusqlite = { version = "0.31", features = ["bundled"] }

# Utilities
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
dirs = "5"
anyhow = "1"
```

> **Removed from original design:**
> - `llama-cpp-rs` / `llama-cpp-2` — ggml symbol collision (SIGABRT)
> - `ringbuf` — replaced with tokio mpsc channels
> - `enigo` — `dispatch_assert_queue_fail` on macOS non-main thread
> - `encoding_rs` — was only needed by llama-cpp-2
> - `nnnoiseless` — not yet integrated (stretch goal)
> - `reqwest` — not yet needed (model download not implemented)
> - `sha2` — not yet needed (checksum verification not implemented)
>
> **Added:**
> - `ureq` — HTTP client for llama-server API calls
> - `ndarray` — tensor operations for ort/Silero VAD

---

## 6. Concurrency Model & Internal Architecture

### 6.1 Task Architecture (tokio async)

```
┌─────────────────────────────────────────────────────────────────┐
│                        MAIN THREAD                              │
│  Tauri event loop + UI rendering                                │
└──────────────────────────┬──────────────────────────────────────┘
                           │ Tauri IPC commands
┌──────────────────────────▼──────────────────────────────────────┐
│                     RUST ASYNC RUNTIME (tokio)                  │
│                                                                  │
│  ┌──────────────┐    mpsc     ┌──────────────┐    mpsc          │
│  │ Audio Task   │────────────▶│  VAD Task    │────────────▶     │
│  │ (cpal callback│            │ (Silero ONNX)│              │   │
│  │  → ring buf) │            └──────────────┘              │   │
│  └──────────────┘                                           │   │
│                                                              │   │
│  ┌──────────────┐    mpsc     ┌──────────────┐    mpsc     │   │
│  │  ASR Task    │◀────────────│ Chunk Router │◀────────────┘   │
│  │ (whisper.cpp)│            │ (segments +  │                  │
│  │  [dedicated  │            │  silence det)│                  │
│  │   thread]    │            └──────────────┘                  │
│  └──────┬───────┘                                               │
│         │ mpsc                                                   │
│  ┌──────▼───────┐    mpsc     ┌──────────────┐                  │
│  │ Command      │────────────▶│ LLM Task     │                  │
│  │ Parser       │   (if not   │ (llama.cpp)  │                  │
│  │              │    command)  │  [dedicated  │                  │
│  └──────┬───────┘             │   thread]    │                  │
│         │ (if command)        └──────┬───────┘                  │
│         │                            │ mpsc (token stream)      │
│  ┌──────▼───────┐             ┌──────▼───────┐                  │
│  │ Command      │             │ Text         │                  │
│  │ Executor     │             │ Injector     │                  │
│  └──────────────┘             └──────────────┘                  │
│                                                                  │
│  ┌──────────────┐  (background, low priority)                   │
│  │ Context      │  Polls active app every 500ms                 │
│  │ Detector     │  Updates shared AppContext atomically          │
│  └──────────────┘                                               │
└──────────────────────────────────────────────────────────────────┘
```

### 6.2 Thread Allocation

| Task | Thread Model | Why |
|---|---|---|
| Audio capture | cpal's own callback thread | Required by cpal API |
| VAD | tokio task (CPU-bound, but < 2ms) | Fast enough for async |
| ASR (whisper.cpp) | Dedicated OS thread via `tokio::task::spawn_blocking` | GPU/CPU-bound, 100-500ms |
| LLM (llama.cpp) | Dedicated OS thread via `tokio::task::spawn_blocking` | GPU/CPU-bound, 200-500ms |
| Command parser | tokio task | Lightweight string matching |
| Text injector | tokio task | I/O-bound (clipboard + key sim) |
| Context detector | tokio task, 500ms interval | Light AX API polling |
| UI/Tauri | Main thread | Required by Tauri |

> ⚡ **Actual thread architecture:**
>
> | Task | Thread Model | Why |
> |---|---|---|
> | Audio capture | cpal callback thread | Required by cpal; resamples 48kHz→16kHz in-callback |
> | Pipeline loop (VAD + chunker) | Dedicated `std::thread::spawn` with own `tokio::runtime::Builder::new_current_thread()` | cpal `Stream` is `!Send` — cannot live on tokio's multi-thread runtime |
> | ASR + LLM + inject | `tokio::task::spawn_blocking` inside pipeline thread's runtime | CPU-bound work |
> | LLM server | Separate OS process (`llama-server`) | Out-of-process to avoid ggml collision |
> | Model loading | Dedicated thread with 64MB stack | whisper.cpp needs large stack; `std::thread::Builder::new().stack_size(64 * 1024 * 1024)` |
> | UI/Tauri | Main thread | Required by Tauri |
>
> **Key insight:** cpal's `Stream` type is `!Send` on macOS. You cannot hold
> it across `.await` points on a multi-threaded tokio runtime. The entire
> listening pipeline must run on a dedicated OS thread with its own
> single-threaded tokio runtime.

### 6.3 State Machine

```
         ┌─────────┐
         │  IDLE   │◀──────────────────────────────────┐
         └────┬────┘                                    │
              │ hotkey pressed                          │
         ┌────▼────┐                                    │
         │LISTENING│──── hotkey released ───┐           │
         └────┬────┘    (quick mode)        │           │
              │ silence detected            │           │
              │ (toggle mode)               │           │
         ┌────▼────────────────────────┐    │           │
         │      PROCESSING             │◀───┘           │
         │  ┌─────────┐  ┌─────────┐  │                │
         │  │   ASR   │─▶│   LLM   │  │                │
         │  └─────────┘  └────┬────┘  │                │
         └─────────────────────┼──────┘                │
                               │                        │
         ┌─────────────────────▼──────┐                │
         │       INJECTING            │                │
         │  (streaming tokens to app) │────────────────┘
         └────────────────────────────┘
```

---

## 7. macOS-Specific Implementation Notes

macOS is the primary platform and has the richest (but also quirkiest)
integration surface. These notes capture the hard-won knowledge from existing
open-source dictation tools.

### 7.1 Accessibility Permissions

OpenFlow requires Accessibility permission to:
- Read the active app and focused text field
- Inject text via AXUIElement APIs
- Detect user edits for the learning pipeline

**First-run flow:**
1. Check `AXIsProcessTrusted()` — if false, show permission dialog
2. Open System Settings → Privacy & Security → Accessibility
3. Guide user to add OpenFlow to the allowed list
4. Re-check on a 1-second timer until granted
5. Proceed to onboarding

**Critical gotcha:** Accessibility APIs only work from a proper `.app` bundle,
not from a CLI binary or `cargo run`. The Tauri bundler handles this, but
during development, you must use `cargo tauri dev` (which creates a temporary
app bundle) or sign the debug binary with the appropriate entitlements.

### 7.2 Fn Key Interception

The `Fn` key on macOS is special — it's not a regular modifier key and isn't
exposed through standard keyboard APIs. Options:

1. **CGEventTap** (recommended): Create a tap on `kCGEventKeyDown` /
   `kCGEventKeyUp` events. The Fn key generates events with
   `kCGKeyboardEventKeyboardType` and specific keycodes. Requires
   Accessibility permission (already granted).

2. **IOKit HID**: Lower-level, can intercept Fn directly from the keyboard
   hardware. More reliable but more complex. Use as fallback.

3. **Alternative default hotkey**: If Fn interception proves unreliable,
   default to `Ctrl+Shift+Space` (configurable). Many open-source tools
   use this approach.

### 7.3 Metal Acceleration

Both whisper.cpp and llama.cpp support Metal natively:
- whisper.cpp: Compile with `WHISPER_METAL=1`. Uses Metal compute shaders
  for encoder/decoder. ~3-5x speedup over CPU on M-series chips.
- llama.cpp: Compile with `LLAMA_METAL=1`. Offloads all layers to GPU by
  default on Apple Silicon. ~4-6x speedup.

Both libraries detect Metal availability at runtime and fall back to CPU
automatically. No user configuration needed.

> ⚡ **Metal is a minefield when both libraries are in the same process.**
>
> - `whisper-rs` with the `metal` cargo feature works fine in isolation.
> - `llama-cpp-sys-2` unconditionally compiles Metal backend via cmake
>   auto-detection on macOS, even without the `metal` cargo feature.
>   Setting `CMAKE_GGML_METAL=OFF` in `build.rs` does not reliably prevent
>   this.
> - When both are linked, duplicate ggml Metal symbols cause SIGABRT during
>   backend registry initialization.
>
> **Current state:**
> - whisper: `use_gpu(false)` — runs on CPU (~300ms for base model)
> - llama: runs in separate `llama-server` process with `-ngl 99` (full
>   Metal GPU offload)
>
> **To re-enable Metal for whisper:** Since llama-cpp is now out-of-process,
> it should be safe to add `features = ["metal"]` to `whisper-rs` in
> Cargo.toml and set `params.use_gpu(true)`. This would cut ASR time to
> ~100ms. Not yet tested.

### 7.4 App Notarization & Distribution

For macOS distribution outside the App Store:
1. Sign with Developer ID certificate
2. Notarize via `xcrun notarytool`
3. Staple the notarization ticket to the .dmg
4. Tauri's bundler supports this workflow via `tauri.conf.json` signing config

---

## 8. Windows-Specific Implementation Notes

### 8.1 Text Injection

Windows text injection is more fragmented than macOS:
- **UIAutomation** (`IUIAutomationValuePattern::SetValue`): Works for most
  native Win32 and WPF apps
- **SendInput**: Simulates keystrokes. Works everywhere but is slow for
  long text
- **Clipboard + Ctrl+V**: Most reliable universal method (same as macOS)
- **WM_SETTEXT / EM_REPLACESEL**: Direct Win32 messages for edit controls

Strategy: Try UIAutomation first → fall back to clipboard paste.

### 8.2 GPU Acceleration

- NVIDIA: CUDA (compile with `WHISPER_CUDA=1`, `LLAMA_CUDA=1`)
- AMD/Intel: DirectML (compile with `WHISPER_DIRECTML=1`)
- Fallback: CPU with AVX2 SIMD

### 8.3 Distribution

- Tauri produces .msi and .exe installers
- No code signing required for sideloading, but SmartScreen will warn
- Optional: Sign with EV code signing certificate to avoid warnings

---

## 9. Project Structure

```
openflow/
├── Cargo.toml                    # Workspace root
├── Cargo.lock
├── tauri.conf.json               # Tauri configuration
├── README.md
├── LICENSE                       # MIT
│
├── src-tauri/                    # Rust backend (Tauri app)
│   ├── Cargo.toml
│   ├── build.rs                  # whisper.cpp/llama.cpp compile flags
│   ├── src/
│   │   ├── main.rs               # Tauri entry point, plugin registration
│   │   ├── lib.rs                # Module declarations
│   │   ├── state.rs              # Shared app state (Arc<Mutex<...>>)
│   │   ├── config.rs             # User configuration, model registry
│   │   │
│   │   ├── audio/
│   │   │   ├── mod.rs
│   │   │   ├── capture.rs        # cpal microphone capture → ring buffer
│   │   │   ├── vad.rs            # Silero VAD wrapper (ONNX Runtime)
│   │   │   ├── noise.rs          # RNNoise wrapper (nnnoiseless)
│   │   │   └── chunker.rs        # VAD-driven segment boundary detection
│   │   │
│   │   ├── asr/
│   │   │   ├── mod.rs
│   │   │   ├── engine.rs         # whisper.cpp wrapper (whisper-rs)
│   │   │   ├── models.rs         # Model loading, selection, hot-swap
│   │   │   └── language.rs       # Language detection utilities
│   │   │
│   │   ├── polish/
│   │   │   ├── mod.rs
│   │   │   ├── engine.rs         # llama.cpp wrapper (llama-cpp-rs)
│   │   │   ├── prompt.rs         # System prompt builder (context-aware)
│   │   │   ├── commands.rs       # Voice command parser & executor
│   │   │   └── models.rs         # LLM model loading, selection
│   │   │
│   │   ├── inject/
│   │   │   ├── mod.rs
│   │   │   ├── clipboard.rs      # Clipboard save/restore/paste simulation
│   │   │   ├── accessibility.rs  # Platform-specific AX/UIAutomation
│   │   │   ├── context.rs        # Active app detection, category mapping
│   │   │   └── history.rs        # Injection history stack (for undo)
│   │   │
│   │   ├── db/
│   │   │   ├── mod.rs
│   │   │   ├── schema.rs         # SQLite schema + migrations
│   │   │   ├── dictionary.rs     # Personal dictionary CRUD
│   │   │   ├── snippets.rs       # Voice snippets CRUD
│   │   │   └── tones.rs          # App tone mappings CRUD
│   │   │
│   │   ├── pipeline/
│   │   │   ├── mod.rs
│   │   │   └── orchestrator.rs   # Main pipeline: audio→ASR→polish→inject
│   │   │
│   │   └── platform/
│   │       ├── mod.rs
│   │       ├── macos.rs          # macOS-specific: CGEventTap, AXUIElement
│   │       └── windows.rs        # Windows-specific: UIAutomation, SendInput
│   │
│   ├── swift-bridge/             # Swift FFI for macOS Accessibility APIs
│   │   ├── Package.swift
│   │   └── Sources/
│   │       └── AccessibilityBridge.swift
│   │
│   └── models/                   # Bundled model registry JSON
│       └── registry.json
│
├── src/                          # Frontend (Svelte)
│   ├── App.svelte                # Root component
│   ├── main.ts                   # Entry point
│   ├── lib/
│   │   ├── components/
│   │   │   ├── RecordingIndicator.svelte   # Floating waveform bar
│   │   │   ├── SystemTray.svelte           # Tray menu
│   │   │   ├── Settings.svelte             # Settings panel
│   │   │   ├── ModelManager.svelte         # Download/select models
│   │   │   ├── DictionaryEditor.svelte     # Personal dictionary UI
│   │   │   └── Onboarding.svelte           # First-run wizard
│   │   ├── stores/
│   │   │   ├── app.ts            # App state store
│   │   │   └── settings.ts       # Settings store
│   │   └── api.ts                # Tauri IPC command wrappers
│   └── styles/
│       └── global.css
│
├── assets/
│   ├── icons/                    # App icons (tray, dock, installer)
│   └── sounds/                   # Optional: activation/deactivation sounds
│
└── tests/
    ├── audio_pipeline_test.rs
    ├── asr_accuracy_test.rs
    ├── polish_quality_test.rs
    ├── injection_test.rs
    └── e2e_latency_test.rs
```

---

## 10. Key Algorithms & Techniques

### 10.1 Incremental Injection (Zero-Flicker Strategy)

The biggest UX challenge: the user finishes speaking and expects text to appear
immediately, but the LLM needs 200-400ms to polish. Two strategies:

**Strategy A: Buffer-then-inject (recommended for v1)**
```
1. User finishes speaking
2. Show "processing" animation on floating indicator
3. ASR runs (100-200ms) → raw text
4. LLM polish runs (200-400ms) → polished text streams out
5. Inject polished tokens as they arrive (sentence-by-sentence)
6. Total perceived wait: ~300ms to first visible text
```

**Strategy B: Raw-then-replace (advanced, v2)**
```
1. User finishes speaking
2. ASR runs → inject RAW text immediately (user sees unpolished text)
3. LLM polish runs in background
4. When polish complete, select the raw text range and replace with polished
5. Feels instant but has a visible "correction" moment
```

Strategy A is simpler and avoids the jarring correction. Strategy B feels
faster but requires precise text range tracking. Start with A.

### 10.2 Self-Correction Detection

OpenFlow's killer feature: understanding when the user corrects themselves.

```
Input:  "Send the report to John no wait send it to Sarah by Friday"
Output: "Send the report to Sarah by Friday"
```

This is handled entirely by the LLM polish step. The system prompt explicitly
instructs the model to detect correction patterns ("no wait", "actually",
"I mean", "scratch that", "no no") and keep only the final intent. Modern
3B instruction-tuned models handle this reliably.

**Fallback for edge cases:** If the LLM misses a correction, the user can
say "scratch that" (voice command) and re-dictate.

### 10.3 Adaptive Silence Detection

Fixed silence thresholds don't work well for all speech patterns. The system
adapts:

```
base_threshold = 700ms (configurable)

if speaking_rate > 180 WPM:
    threshold = base_threshold * 0.7    # Fast speaker, shorter pauses
elif speaking_rate < 100 WPM:
    threshold = base_threshold * 1.5    # Slow/thoughtful speaker
else:
    threshold = base_threshold

if mode == "long_dictation":
    threshold *= 1.3                     # More tolerance in long mode

if consecutive_short_pauses > 3:
    threshold *= 1.2                     # Speaker uses lots of brief pauses
```

Speaking rate is estimated from the ASR output (words per second of audio).

---

## 11. Testing Strategy

### 11.1 Accuracy Benchmarks

| Test Suite | Source | Target Metric |
|---|---|---|
| LibriSpeech test-clean | Standard ASR benchmark | < 3% WER (raw ASR) |
| LibriSpeech test-other | Noisy/accented speech | < 7% WER (raw ASR) |
| Custom filler corpus | Synthetic "um/uh" injected speech | 99%+ filler removal rate |
| Self-correction corpus | "no wait" / "I mean" patterns | 95%+ correct resolution |
| Multilingual (CommonVoice) | 10 languages, accented English | < 5% WER per language |
| Whisper mode | Quiet speech recordings | < 10% WER degradation vs normal |

### 11.2 Latency Benchmarks

Automated benchmark suite that measures each pipeline stage independently:

```rust
#[bench]
fn bench_full_pipeline() {
    // Record: hotkey press → text appears in target app
    // Target: < 800ms on reference hardware (M3 Pro)
    let audio = load_test_audio("3_second_sentence.wav");
    let start = Instant::now();

    let vad_result = vad.process(&audio);           // ~2ms
    let asr_result = asr.transcribe(&audio);        // ~150ms
    let polished = llm.polish(&asr_result, &ctx);   // ~300ms
    injector.inject(&polished);                      // ~50ms

    assert!(start.elapsed() < Duration::from_millis(800));
}
```

### 11.3 Integration Tests

- **5-app smoke test:** Inject text into Mail, Slack, VS Code, Chrome, Terminal
- **Clipboard preservation:** Verify original clipboard is restored after injection
- **Undo test:** Inject text, press Cmd+Z, verify text is removed
- **Focus loss:** Start dictation, click away mid-processing, verify no injection
- **Concurrent audio:** Play music while dictating, verify ASR accuracy

### 11.4 User Testing Protocol

- 10 testers, diverse accents, 5-minute free dictation sessions
- Measure: corrections needed per 100 words, perceived latency rating (1-5),
  "would you use this daily?" (yes/no)
- Target: < 3 corrections per 100 words, latency rating >= 4, daily use >= 80%

---

## 12. Security & Privacy

### 12.1 Threat Model

Since this is a local-only tool with accessibility permissions, the attack
surface is:

| Threat | Mitigation |
|---|---|
| Audio data exfiltration | No network calls. Verify with Little Snitch / firewall rules. |
| Model supply chain attack | SHA256 checksum verification on all model downloads |
| Accessibility permission abuse | Minimal AX API usage. Only read focused element + inject. |
| Injection history leakage | SQLite DB encrypted at rest (SQLCipher optional) |
| Clipboard snooping | Clipboard contents restored within 100ms. Minimize exposure window. |

### 12.2 Privacy Guarantees

- **Zero network traffic** after model download. The app can be firewalled.
- **No telemetry** of any kind. No analytics, no crash reporting to external services.
- **Audio is never persisted** to disk. Processed in-memory ring buffer only.
- **Injection history** is local SQLite, user-deletable via settings.
- **Models are user-downloaded** from Hugging Face. We don't host or proxy.

---

## 13. Implementation Roadmap

### Phase 1: Core Pipeline (Weeks 1-3)

**Goal:** Hotkey → speak → raw text appears in a test app.

- [x] Tauri 2 skeleton with system tray and global hotkey
- [x] cpal audio capture → ~~ring buffer~~ mpsc channel → 16kHz resample
- [x] ~~Silero VAD~~ Energy-based VAD with Silero fallback (needs ONNX Runtime)
- [x] whisper.cpp integration via whisper-rs, single-shot transcription
- [x] Basic clipboard-paste text injection (~~enigo~~ osascript + arboard)
- [ ] Test in 3 apps: TextEdit, Chrome, VS Code (tested in terminal only)

**Deliverable:** Working Tauri app that transcribes speech and pastes text.
ASR latency: ~300ms (base model, CPU). ✅ Achieved.

### Phase 2: LLM Polish & Context (Weeks 4-6)

**Goal:** Polished, context-aware text with filler removal.

- [x] ~~llama.cpp integration via llama-cpp-rs~~ llama-server (out-of-process HTTP)
- [x] System prompt with filler removal + self-correction
- [x] Active app detection (via `osascript` — NSWorkspace frontmostApplication)
- [x] App category mapping → tone directive injection
- [ ] Streaming token injection (sentence-by-sentence) — currently batch
- [x] Voice command parser (basic: "new paragraph", "scratch that")

**Deliverable:** Full pipeline producing polished text. ✅ Achieved.
E2E latency: 1.0-2.0s (LLM polish is the bottleneck).

### Phase 3: UX Polish & Personalization (Weeks 7-9)

**Goal:** Polished, seamless dictation experience.

- [ ] Floating recording indicator (waveform animation)
- [ ] Settings UI: model selection, hotkey config, tone editor
- [ ] Personal dictionary (SQLite + UI editor + prompt injection)
- [ ] Voice snippets (trigger → expansion)
- [ ] Model download manager with progress bar + auto-selection
- [ ] Onboarding flow (permissions, model download, tutorial)
- [ ] Quick dictation (hold) + long dictation (toggle) modes
- [ ] Adaptive silence threshold

**Deliverable:** Feature-complete macOS app. Internal dogfooding begins.

### Phase 4: Hardening & Optimization (Weeks 10-12)

**Goal:** Production-ready quality and performance.

- [ ] GPU detection and auto-configuration (Metal/CUDA/DirectML)
- [ ] Model hot-swapping without restart
- [ ] Latency profiling and optimization (target < 800ms on M2+)
- [ ] Noise suppression (RNNoise) integration and toggle
- [ ] Whisper mode (low-volume speech support)
- [ ] Undo support (injection history stack + "scratch that")
- [ ] Correction learning pipeline (detect user edits → auto-add to dict)
- [ ] Comprehensive test suite (accuracy, latency, integration)
- [ ] Memory leak testing, long-session stability (8+ hours)

**Deliverable:** Beta-quality macOS app. External beta testing begins.

### Phase 5: Windows & Distribution (Weeks 13-16)

**Goal:** Cross-platform release.

- [ ] Windows text injection (UIAutomation + clipboard fallback)
- [ ] Windows GPU acceleration (CUDA + DirectML)
- [ ] Windows installer (.msi via Tauri bundler)
- [ ] macOS notarization and .dmg signing
- [ ] Auto-update mechanism (Tauri updater plugin)
- [ ] Documentation: README, user guide, contributing guide
- [ ] GitHub release with pre-built binaries

**Deliverable:** v1.0 release for macOS and Windows.

### Stretch Goals (Post v1.0)

- [ ] Linux support (X11/Wayland text injection via xdotool/wtype)
- [ ] Local LoRA fine-tuning on user corrections (overnight batch job)
- [ ] Code mode: variable name formatting (camelCase/snake_case detection)
- [ ] Multi-language code-switching within a single utterance
- [ ] Browser extension for enhanced web app context (Gmail compose, etc.)
- [ ] Apple SpeechAnalyzer framework integration (macOS 16+) as ASR alternative

---

## 14. Implementation Lessons Learned

> These are hard-won lessons from the actual build. Future contributors should
> read this section before making architectural changes.

### 14.1 The ggml Symbol Collision (Most Critical Bug)

**Symptom:** App crashes with SIGABRT or SIGSEGV immediately on launch.
Crash report shows `ggml_abort` called from `ggml_backend_metal_reg_device_get`
inside `whisper_init_with_params_no_state`.

**Root cause:** `whisper-rs-sys` and `llama-cpp-sys-2` both vendor their own
copy of ggml (the tensor library). On macOS, `llama-cpp-sys-2`'s cmake build
auto-detects Metal and compiles the Metal backend unconditionally — even if
you don't enable the `metal` cargo feature, and even if you set
`CMAKE_GGML_METAL=OFF` in build.rs (the cmake variable is checked too late).
When whisper's ggml initializes its backend registry, it finds llama's Metal
backend symbols instead of its own, calls into incompatible structures, and
aborts.

**What didn't work:**
- Disabling `metal` feature on both crates
- Setting `CMAKE_GGML_METAL=OFF` in build.rs
- Using `#[link_name]` or symbol renaming

**What worked:** Remove `llama-cpp-2` from the binary entirely. Run LLM
inference out-of-process via `llama-server`.

**Lesson:** Never link two crates that vendor different versions of the same
C library into one binary. Check with `nm binary | grep symbol_name` to
verify symbol origins.

### 14.2 macOS Audio Device Configuration

**Symptom:** `"The requested stream configuration is not supported by the device"`

**Root cause:** MacBook Pro microphone runs at 48kHz natively. Requesting
16kHz mono via cpal's `StreamConfig` fails because the hardware doesn't
support it.

**Fix:** Always use `device.default_input_config()` and resample in software.
Linear interpolation from 48kHz to 16kHz is cheap and accurate enough for
speech recognition.

### 14.3 enigo and macOS Thread Safety

**Symptom:** `dispatch_assert_queue_fail` crash in `TSMGetInputSourceProperty`

**Root cause:** enigo's macOS keyboard simulation uses InputSource APIs that
must run on the main thread (they call into the Text Services Manager which
asserts it's on the main dispatch queue). Our pipeline runs on a worker thread.

**Fix:** Replace enigo with `osascript -e 'tell application "System Events"
to keystroke "v" using command down'`. Works from any thread, requires
Accessibility permission.

### 14.4 cpal Stream is !Send on macOS

**Symptom:** Compilation error or runtime panic when holding a cpal `Stream`
across `.await` points on tokio's multi-threaded runtime.

**Fix:** The entire listening pipeline (audio capture + VAD + chunker) runs
on a dedicated `std::thread::spawn` with its own
`tokio::runtime::Builder::new_current_thread()`. The cpal Stream lives
entirely within this thread.

### 14.5 whisper.cpp Stack Size Requirements

**Symptom:** Stack overflow or segfault during model loading.

**Fix:** Use `std::thread::Builder::new().stack_size(64 * 1024 * 1024)` for
the model loading thread. whisper.cpp allocates large buffers on the stack
during initialization.

### 14.6 Whisper ASR Output Artifacts

**Symptom:** Whisper outputs `[no speech detected]`, `[BLANK_AUDIO]`, or
`(music)` for non-speech segments.

**Fix:** Filter any ASR output that starts with `[` or is empty before
passing to the polish step. Also enforce a minimum segment length of 4800
samples (0.3s) to avoid sending tiny noise bursts to ASR.

### 14.7 ort (ONNX Runtime) Dynamic Loading

**Symptom:** `ort` crate with `load-dynamic` feature panics at runtime if
`libonnxruntime.dylib` is not installed.

**Fix:** Wrap VAD initialization in `std::panic::catch_unwind`. Fall back to
energy-based RMS detection. The energy-based approach (RMS > 0.01 threshold
on 480-sample / 30ms frames) works adequately for normal speech volume.

### 14.8 llama-cli is Unsuitable for Subprocess Polish

**Symptom:** `llama-cli` hangs indefinitely, generating tokens forever.

**Root cause:** `llama-cli` in generation mode doesn't have clean stop
conditions for single-shot completion. It also loads the model from disk on
every invocation (~10-15s), making it unusable for real-time polish.

**Fix:** Use `llama-server` instead. It loads the model once, stays resident,
and exposes an OpenAI-compatible HTTP API with proper `max_tokens` and stop
conditions. Response time: 600-1400ms per polish call.

### 14.9 Build Configuration

**Required environment:**
```bash
MACOSX_DEPLOYMENT_TARGET=11.0  # Required for llama.cpp's std::filesystem
```

**Build command:**
```bash
cd openflow && MACOSX_DEPLOYMENT_TARGET=11.0 npm run tauri build
```

**External dependencies (not bundled):**
```bash
brew install llama.cpp     # Provides llama-server for LLM polish
brew install onnxruntime   # Optional: enables Silero VAD (otherwise energy-based fallback)
```

**Models directory:** `~/Library/Application Support/openflow/models/`
- `silero_vad.onnx` (2.2MB) — optional, for Silero VAD
- `ggml-base.bin` (141MB) — whisper base model (default)
- `ggml-small.bin` (465MB) — whisper small model (higher accuracy)
- `qwen2.5-3b-instruct-q4_k_m.gguf` (2.0GB) — LLM for polish
