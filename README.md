# OpenFlow — Local-First AI Voice Dictation

A system-wide, always-on voice dictation app that runs 100% on-device. Press a hotkey, speak naturally, and polished, context-aware text appears at your cursor in any application — with no data leaving your machine.

## Features

- **Universal text injection** — works in any app with a text field
- **Filler removal & polish** — removes "um/uh/like"; fixes grammar & punctuation
- **Self-correction understanding** — "Meet at 2pm — no, 4pm" → outputs only the correction
- **Context-aware formatting** — detects active app, adjusts tone (email vs Slack vs code)
- **Sub-second perceived latency** — mic → ASR → LLM polish → clipboard paste in 1-2s
- **Push-to-talk + toggle modes** — quick burst or long dictation
- **Personal dictionary** — learns custom names, acronyms, jargon
- **Fully offline** — zero network calls, ever

## Tech Stack

- **Frontend:** SvelteKit + Vite
- **Backend:** Rust (Tauri v2)
- **ASR:** Local speech recognition engine
- **Polish:** On-device LLM for text refinement
- **Database:** SQLite (settings, dictionary, snippets, tones)

## Recommended IDE Setup

[VS Code](https://code.visualstudio.com/) + [Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## License

MIT
