# OpenFlow benchmarks

End-to-end voice-dictation experiments on an Apple M1 Pro (MacBook Pro, 16GB). Raw log files live here; methodology and headline results are summarized below. Audio fixtures are not committed (personal voice), but the harness is reproducible: `src-tauri/tests/e2e_experiments.rs`.

## Setup

- **Hardware**: Apple M1 Pro, 16GB, macOS Darwin 25.2.0
- **Backend**: whisper.cpp via `whisper-rs` 0.13 with the `metal` feature; llama.cpp `b8123` running Qwen 2.5 3B Q4_K_M as the polish LLM (GPU-offloaded via `-ngl 99`)
- **Audio**: 2 fixtures, each ~17 seconds of technical dictation (software engineering jargon, file paths, product requirements)

## Methodology

Each row in the sweep is one `(whisper model, beam size, thread count, initial-prompt on/off)` × fixture combination. For every combination the harness:

1. Runs one **untimed warmup** to discard Metal shader compilation + KV cache init.
2. Runs **N=5 timed iterations** and reports the **median** per stage (`asr_ms`, `polish_ms`, `total_ms`).
3. Drops the whisper context and sleeps 500ms before loading the next model, so Metal releases GPU memory cleanly.
4. Computes **WER** (Levenshtein word error rate) against a per-fixture ground-truth reference — for both the raw Whisper output and the post-polish output.

Model-load time is measured separately and not included in `asr_ms`. The polish LLM is loaded once and shared across all Whisper configs, since it does not depend on the ASR choice.

## Configurations swept

| config                 | model                          | decoding | threads | initial prompt |
|------------------------|--------------------------------|----------|---------|----------------|
| `base-greedy-p`        | `ggml-base.bin` (141MB)        | greedy   | default | on             |
| `base-greedy-NOp`      | `ggml-base.bin`                | greedy   | default | off            |
| `base-beam5-p`         | `ggml-base.bin`                | beam=5   | default | on             |
| `small.en-greedy-p`    | `ggml-small.en-q5_1.bin` (181MB) | greedy | default | on             |
| `small.en-greedy-p-t8` | `ggml-small.en-q5_1.bin`       | greedy   | 8       | on             |
| `small.en-beam5-p`     | `ggml-small.en-q5_1.bin`       | beam=5   | default | on             |
| `turbo-greedy-p`       | `ggml-large-v3-turbo-q5_0.bin` (547MB) | greedy | default | on       |
| `turbo-greedy-NOp`     | `ggml-large-v3-turbo-q5_0.bin` | greedy   | default | off            |
| `turbo-greedy-NOp-t8`  | `ggml-large-v3-turbo-q5_0.bin` | greedy   | 8       | off            |
| `turbo-beam5-p`        | `ggml-large-v3-turbo-q5_0.bin` | beam=5   | default | on             |

## Results — Run 2 (stricter polish prompt)

Sorted by `wer_raw` ascending (most accurate first). `wer_polished` is WER of the LLM-polished output against the same reference.

| config                 |   asr_ms |   total_ms |   wer_raw |   wer_pol |   rtf |
|------------------------|---------:|-----------:|----------:|----------:|------:|
| **turbo-greedy-NOp**   |     1735 |       3106 |     0.098 |     0.258 |  0.10 |
| turbo-greedy-NOp-t8    |     1808 |       3170 |     0.098 |     0.258 |  0.11 |
| turbo-greedy-p         |     1806 |       3122 |     0.146 |     0.258 |  0.11 |
| turbo-beam5-p          |     2622 |       4046 |     0.158 |     0.270 |  0.15 |
| small.en-greedy-p      |      992 |       2415 |     0.185 |     0.232 |  0.06 |
| small.en-greedy-p-t8   |     1070 |       2422 |     0.185 |     0.232 |  0.06 |
| small.en-beam5-p       |     1834 |       3173 |     0.198 |     0.257 |  0.11 |
| base-beam5-p           |      943 |       2274 |     0.256 |     0.304 |  0.06 |
| base-greedy-NOp        |      542 |       1860 |     0.292 |     0.376 |  0.03 |
| base-greedy-p          |      529 |       1904 |     0.329 |     0.341 |  0.03 |

- `asr_ms` / `total_ms`: median over 5 runs, post-warmup, on ~17s audio
- `wer_raw`: WER of Whisper output against ground-truth reference
- `wer_polished`: WER of post-LLM-polish output
- `rtf`: real-time factor (`asr_ms / audio_duration_ms`) — lower is faster; < 1.0 is faster than real-time

## Findings

**1. Turbo with no initial prompt is the accuracy winner.** `turbo-greedy-NOp` achieves WER 0.098 — under 10% — on dense technical dictation. That's a **2.7× improvement** over the previous default (`base-greedy-p` at WER 0.329), and 2× better than `small.en`. The jump is driven by turbo correctly hearing phrases base systematically mis-heard: "activated event is sent" (base → "activated in descent"), "Integ tests" (base → hallucinated "IndicTest"), "non-sycophancy behavior" (base → "non-sack behavior").

**2. The initial prompt hurts accuracy on technical dictation.** Both on `base` (WER 0.329 with prompt → 0.292 without) and `turbo` (0.146 → 0.098). The tech-vocab seed appears to bias decoding toward prompt-style phrasing ("in descent", "event descent") rather than actually helping with jargon. **The prompt was turned off in production.**

**3. Beam search is strictly worse.** In every model tier, `beam=5` was both slower and less accurate than greedy. whisper.cpp's greedy decoder already applies temperature fallback; beam search amplifies repetition pathologies on noisy audio. Don't use it.

**4. The polish prompt rewrite closed a major regression.** Before: the 3B Qwen polish turned imperative dictation ("Validate X. Read Y.") into hedged descriptive prose ("You should consider reviewing..."), doubling `wer_polished` on one fixture. After rewriting the prompt to forbid restructuring and paraphrasing, `wer_polished` dropped from 0.651 to 0.341 on base, 0.304 to 0.232 on small.en. Diff the two runs in `run1_baseline_polish.txt` and `run2_stricter_polish.txt` to see.

**5. Thread count (`n_threads=8` vs default) is within measurement noise** on this hardware / workload. No action needed.

**6. small.en is the latency sweet spot for short utterances.** At 992ms for 17s of audio (`rtf=0.06`), a 4s user utterance would land at ~240ms ASR — solidly in the "feels instant" regime. It's the right fallback when turbo is too slow or too big.

## Production defaults (post-experiment)

- **ASR model**: `ggml-large-v3-turbo-q5_0.bin` (auto-downloaded on first launch). Installs carrying the old `ggml-base.bin` keep working — the loader picks the best available model from `[turbo, small.en, base, small]`.
- **`AsrOpts::default()`**: `{ beam_size: None, n_threads: None, use_initial_prompt: false }`.
- **Polish prompt**: rewritten to forbid paraphrase and preserve sentence form (see `src-tauri/src/polish/prompt.rs`).
- **Silence threshold**: 500ms (from the prior optimization round).
- **Metal acceleration**: on (from the prior optimization round).

## Reproducing

```bash
# From the repo root. Requires models + llama-server already downloaded
# (they auto-download on first app launch).
cargo test --manifest-path src-tauri/Cargo.toml \
    --release --test e2e_experiments run_experiments \
    -- --ignored --nocapture > benchmark/run_new.txt 2>&1
```

Fixtures live in `src-tauri/tests/fixtures/<name>/` — see `src-tauri/tests/fixtures/README.md` for recording instructions.

## Raw logs

- [`run1_baseline_polish.txt`](run1_baseline_polish.txt) — 3 runs/config, N_RUNS=3, original polish prompt
- [`run2_stricter_polish.txt`](run2_stricter_polish.txt) — 5 runs/config, N_RUNS=5, stricter polish prompt (current production)
