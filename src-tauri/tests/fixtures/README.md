# e2e voice fixtures

These fixtures drive `cargo test --release --test e2e_voice -- --ignored --nocapture`.

**Audio files are gitignored by default** (see `.gitignore`) — your voice is personal data. If you want to commit fixtures for CI, remove them from `.gitignore` explicitly.

## Layout

```
tests/fixtures/
├── record.sh                       # recording helper
├── short_sentence/
│   ├── audio.wav                   # 16kHz mono 16-bit PCM
│   └── expect.txt                  # substring assertions
├── tech_jargon/
│   ├── audio.wav
│   └── expect.txt
├── chunker_pauses/                 # used by e2e_chunker_segments test
│   ├── audio.wav                   # e.g. "one. <pause 1s> two. <pause 1s> three."
│   └── expect_segments.txt         # just an integer, e.g. 3
...
```

## Recording

```bash
./record.sh short_sentence 4        # records 4 seconds
./record.sh tech_jargon 8
./record.sh with_fillers 6
./record.sh long_paragraph 20
./record.sh chunker_pauses 10       # speak 3 short phrases with ~1s pauses
echo 3 > chunker_pauses/expect_segments.txt
```

`record.sh` uses `ffmpeg` + macOS AVFoundation. Override the mic with `AVFOUNDATION_INDEX=:1 ./record.sh ...`. Run it once with no args to see a list of input device indices.

## Suggested fixture set (starter)

| name               | what to say                                                                 | expect.txt hints                |
|--------------------|-----------------------------------------------------------------------------|---------------------------------|
| `short_sentence`   | "The quick brown fox jumps over the lazy dog."                              | `brown fox`, `lazy dog`         |
| `tech_jargon`      | "Deploy the gRPC service to Kubernetes with a PostgreSQL backend."          | `gRPC`, `Kubernetes`, `PostgreSQL` |
| `with_fillers`     | "Um, so basically, I think we should uh deploy the new version."            | `deploy`, `!um`, `!uh`          |
| `long_paragraph`   | ~20s of natural speech about a project                                      | 2–3 distinctive nouns           |
| `chunker_pauses`   | "First sentence." *(1s silence)* "Second sentence." *(1s)* "Third."         | `expect_segments.txt` → `3`     |

## expect.txt format

```
# Lines starting with # are comments (ignored).
# Substrings that MUST appear in the polished output (case-insensitive):
Kubernetes
gRPC

# Substrings that MUST NOT appear (prefix with !):
!um
!uh
```

The test matches lowercased substrings, so don't worry about case.

## Running

```bash
# All fixtures + assertions
cargo test --release --test e2e_voice e2e_all_fixtures -- --ignored --nocapture

# Per-stage timing benchmark (first fixture, 5 runs with warmup)
cargo test --release --test e2e_voice e2e_benchmark -- --ignored --nocapture

# Chunker segment test
cargo test --release --test e2e_voice e2e_chunker_segments -- --ignored --nocapture
```

`--release` matters — debug builds run Whisper ~5x slower. `--nocapture` lets the structured timing lines through; otherwise Rust swallows them on success.

## Reading the output

Each fixture prints one line like:

```
E2E fixture=tech_jargon asr_ms=187 polish_ms=412 total_ms=599 words=12
     -> Deploy the gRPC service to Kubernetes with a PostgreSQL backend.
```

Grep for `E2E fixture=` or `BENCH` to track per-stage latency over time.
