# Ting

A multi-agent deliberation tool where any LLM, CLI tool, or human can participate in structured, multi-turn discussions using the filesystem as a shared medium. Ting orchestrates a modified Delphi protocol — independent proposals, adversarial cross-examination, informed revision — then synthesizes agreement and preserves dissent as a first-class output.

> **Why "Ting"?** From Old Swedish *ting* — an open-air assembly where free people gathered to settle disputes, make laws, and render judgments. The tradition dates back over a thousand years across Scandinavia. Fittingly, Ting named itself: three AI models deliberated on what this tool should be called, and *ting* is what they converged on.

## Who This Is For

**Ting is for you if:**
- You use multiple AI models and want better decisions than any single model gives
- You want structured disagreement, not just "ask Claude" — cross-examination surfaces blind spots
- You make architecture, planning, or strategy decisions regularly and want to stress-test your thinking
- You want a record of *why* a decision was made, including the dissenting views

**Ting is NOT for:**
- Simple Q&A where one model is enough — Ting is overkill for "fix this bug"
- Real-time chat — deliberation takes minutes, not seconds
- People who want a framework or SDK — this is a standalone CLI tool
- Consensus-seeking — Ting preserves dissent as a first-class output, not a failure mode

## Prerequisites

- **Rust** (1.85+, edition 2024)
- **Claude Code** (`claude` CLI) — required for synthesis generation and convergence evaluation (fire keeper internals). Also available as a participant preset, but not required as one
- At least one participant CLI installed and authenticated: `codex`, `gemini`, `opencode`, or just use `human` for manual participation
- Optional: `herenow` CLI for publishing HTML reports via `--publish`

## Quick Start

```bash
# Build
cargo build --release

# Run a 3-model deliberation with the live dashboard
# (opens http://127.0.0.1:3420 in your browser)
ting new "Should we use Pipecat or Vapi for voice?" \
  --participant codex \
  --participant gemini \
  --participant claude \
  --dashboard

# Or run without the dashboard (behaves identically to v0.3)
ting new "Should we use Pipecat or Vapi for voice?" \
  --participant codex --participant gemini --participant claude

# Check progress
ting status <forum-id>

# View result
ting result <forum-id>

# Generate HTML report
ting result --html <forum-id>

# Publish report to the web
ting result --html --publish <forum-id>

# Re-open the dashboard against an existing forum (in-progress or done)
ting serve <forum-id>
```

### What You'll See

```
  ████████╗██╗███╗   ██╗ ██████╗
  ╚══██╔══╝██║████╗  ██║██╔════╝
     ██║   ██║██╔██╗ ██║██║  ███╗
     ██║   ██║██║╚██╗██║██║   ██║
     ██║   ██║██║ ╚████║╚██████╔╝
     ╚═╝   ╚═╝╚═╝  ╚═══╝ ╚═════╝
  v0.4.0  Structured deliberation between AI models

  Forum  ting-2026-03-27-a1b2c3d4
  Topic  Should we use Pipecat or Vapi for voice?
  With   codex, gemini, claude
  Rules  5 rounds, 5m timeout

=== Round 1 (proposal) ===
  Wrote round-1/prompt.md
  Invoking participant: codex
  Invoking participant: gemini
  Invoking participant: claude
  Collected 3/3 responses
  Generating synthesis...
  Generating claims...

=== Round 2 (cross-examination) ===
  Wrote round-2/prompt.md
  Invoking participant: codex
  Invoking participant: gemini
  Invoking participant: claude
  Collected 3/3 responses
  Generating synthesis...
  Generating claims...
  Evaluating convergence...
  CONVERGED (score: 8.0): Strong agreement on core architecture...

=== Final output written to ~/.ting/sessions/ting-2026-03-27-a1b2c3d4/final/ ===
```

## Protocol

```
 Round 1: PROPOSAL (blind)
 Each participant independently proposes their position.
         |
         v
 Round 2: CROSS-EXAMINATION (adversarial)
 Each participant critiques an assigned other's position,
 then defends or revises their own.
         |
         v
 Round 3+: REVISION (informed)
 Participants revise their positions given all prior context.
         |
         v
 CONVERGENCE CHECK (LLM judge, score 1-10)
   >= threshold --> final/synthesis.md + final/claims.toml
   < threshold  --> another round (up to max_rounds)
                    final/dissent.md preserves disagreements
```

Dissent is not failure — it's the most valuable output when models genuinely disagree.

## CLI Reference

### `ting new`

```bash
ting new "Your question or topic" \
  --participant codex \
  --participant gemini \
  --participant human \
  --timeout 5m \
  --max-rounds 5 \
  --context notes.md    # attach supplementary material
```

Creates a forum and runs the full deliberation (blocking). The `--context` flag accepts a file path or inline text that gets included in every round's prompt. Context is snapshotted at creation time (not re-read each round) for reproducibility.

### `ting status <forum-id>`

Shows round-by-round progress with who has/hasn't responded:

```bash
ting status <forum-id>

# View a specific round's full responses
ting status <forum-id> --round 2
```

### `ting list`

Lists all forums with status and topic.

### `ting result <forum-id>`

Prints the final synthesis and dissent to terminal. Add `--html` to generate a self-contained HTML report. Add `--publish` to push it to the web via here.now.

### `ting serve <forum-id>`

Serves the dashboard against an existing forum directory — in-progress or
completed — without running a new round.

```bash
ting serve <forum-id>              # default port 3420, auto-opens browser
ting serve <forum-id> --port 4000 --no-open
```

Ctrl+C triggers a graceful shutdown; a second Ctrl+C (or a 5s stall from a
long-lived SSE client) forces exit so the process never hangs.

### `ting respond <forum-id>`

For human participants — submit a response from another terminal while the forum is running.
Round, participant name, and input method are all auto-detected:

```bash
# Simplest: auto-detects round + participant, opens $EDITOR
ting respond <forum-id>

# Explicit: specify round, name, and file
ting respond <forum-id> -r 2 -n human -f my-response.md
```

## Participant Types

### Presets (built-in)

| Preset     | Command                                  | Input Method |
|------------|------------------------------------------|--------------|
| `codex`    | `codex exec --full-auto -`               | stdin        |
| `gemini`   | `cat {prompt_file} \| gemini -p ' '`     | file pipe    |
| `claude`   | `cat {prompt_file} \| claude -p -`       | file pipe    |
| `opencode` | `opencode run`                           | stdin        |
| `ollama`   | `cat {prompt_file} \| ollama run llama3` | file pipe    |
| `human`    | (manual — writes files directly)         | filesystem   |

```bash
ting new "topic" --participant codex --participant gemini
```

### Custom Presets

Save reusable presets with `ting preset`:

```bash
# Add a custom preset
ting preset add mistral "cat {prompt_file} | ollama run mistral"

# List all presets (built-in + custom)
ting preset list

# Use it
ting new "topic" --participant mistral --participant codex

# Remove it
ting preset remove mistral
```

Custom presets are stored in `~/.ting/config.toml` and override built-ins of the same name.

### Custom Commands (inline)

```bash
ting new "topic" \
  --participant "llama:command:cat {prompt_file} | ollama run llama3" \
  --participant "gpt:command:cat {prompt_file} | openai-cli chat"
```

The prompt is delivered to commands via:
1. **stdin** — piped directly (safest)
2. **`{prompt_file}`** — replaced with a temp file path in the command
3. **`$TING_PROMPT_FILE`** — env var pointing to the same temp file

### Human / Manual

```bash
ting new "topic" --participant human --participant codex
```

When the fire keeper needs a human response, it prints instructions:
```
  ✓ claude responded (1,203 words)
  ✓ codex responded (987 words)

  ⏳ Waiting for YOU (human)

    Read others' responses:  ting status <id> --round 1
    Write your response:     ting respond <id>
    Or edit directly:        ~/.ting/sessions/<id>/round-1/human.md

  Watching for your file... (timeout in 4m30s)
```

### Other Models

Any CLI that reads from stdin or a file can participate. Examples:

```bash
# Cursor (editor, no CLI agent mode — use via custom command if they add one)
# Pi (no public CLI — use via API wrapper)

# Any ollama model
ting preset add deepseek "cat {prompt_file} | ollama run deepseek-r1"
```

## Configuration

Forums are configured via `meta.toml`, generated automatically by `ting new`:

```toml
[forum]
id = "ting-2026-03-27-001"
topic = "Should we use Pipecat or Vapi?"
created = "2026-03-27T00:30:00Z"
max_rounds = 5
protocol = "delphi-crossexam"
context = "Optional supplementary material..."

[participants]
names = ["codex", "gemini"]

[participants.codex]
type = "command"
command = "codex exec --full-auto -"

[participants.gemini]
type = "command"
command = "gemini -p \" \""

[timing]
round_timeout = "5m"
participant_timeout = "2m"

[convergence]
policy = "llm-judge"
judge_model = "claude-opus"
threshold = 7
min_rounds = 2

[synthesis]
model = "claude-opus"
```

## Dashboard

Ting v0.4 ships a live dashboard so you can watch a deliberation unfold —
per-round syntheses arriving, per-metric scores updating each round, and
convergence climbing toward threshold — instead of tailing log files.

### What runs where

Turning on `--dashboard` activates four cooperating pieces:

1. **JSONL event log.** The Fire Keeper emits an append-only event stream
   to `~/.ting/sessions/<forum-id>/dashboard-events.jsonl` with a
   versioned envelope (`seq`, `forum_id`, `timestamp`, `type`, `payload`).
   Monotonic `seq` is the authoritative ordering key. The v0.4 runtime
   emits five event types: `classifier_metrics`, `metric_scores`,
   `synthesis`, `convergence`, and `forum_complete`; five more
   (`forum_started`, `round_started`, `participant_response`, `claims`,
   `alignment`) are reserved by the contract for a future phase.
   JSON Schemas, a companion `dashboard-state.json` snapshot format,
   and reader/writer guarantees live in [`schemas/`](./schemas).

2. **Pre-round classifier.** Before round 1, the Fire Keeper generates
   5–10 question-specific metrics plus a mandatory Dissent Axis, written
   to `round-0/metrics.json`. These are the axes the dashboard animates
   across rounds. Opt out with `--no-classifier`.

3. **Per-round metric scoring.** After each round's responses and
   synthesis land, the Fire Keeper scores every classifier metric in a
   single batched pass and emits a `metric_scores` event. Scoring
   failures warn-and-continue; they never abort the forum. Opt out with
   `--no-metric-scoring`.

4. **HTML dashboard (axum).** A small `axum` server binds to loopback
   (default port `3420`), serves the dashboard shell at `GET /`, and an
   SSE stream at `GET /api/events` that replays the log and then
   forwards live events. A compacted snapshot is also available at
   `GET /api/state` when a `dashboard-state.json` snapshot exists on
   disk (404 otherwise; clients can always seed from the SSE `init`
   frame). The UI renders metric bars, a convergence gauge, and a
   synthesis feed in pure CSS; no charting library. The Dissent Axis
   is always pinned to the top of the metrics panel.

### Running it

```bash
# Forum + dashboard, auto-opens the browser
ting new "topic" --participant codex --participant gemini --dashboard

# Non-default port, no browser auto-open
ting new "topic" --participant codex --dashboard --port 4000 --no-open

# Turn off the Fire Keeper axes (dashboard still works; metrics panel
# stays empty since no classifier_metrics event is emitted)
ting new "topic" --participant codex --dashboard --no-classifier

# Re-open the dashboard against an existing forum (in-progress or done)
ting serve <forum-id>
```

The server binds to `127.0.0.1` only. There is no authentication and no
remote exposure knob — if you want to share a run, use `ting result --html
--publish <forum-id>` for the post-hoc report.

### Go TUI

A standalone terminal client lives under [`tui/`](./tui) for when a
browser isn't convenient. It reads the same filesystem contract
(`dashboard-state.json` + `dashboard-events.jsonl`) directly, with no
HTTP dependency on the Rust server. Build and run:

```bash
cd tui
go build -o ting-tui .
./ting-tui ~/.ting/sessions/<forum-id>
```

Keys: `q` / Ctrl-C / Esc quit, `r` reload snapshot, `?` help,
`↑`/`↓` or `j`/`k` focus rounds, `1`–`9` jump to a round, `0` clear.

### Without `--dashboard`

Behavior is bit-for-bit identical to v0.3: no event log, no classifier
call, no server, no added disk state. Upgrade safely without opting in.

## Directory Structure

```
~/.ting/sessions/<forum-id>/
  meta.toml
  dashboard-events.jsonl      # append-only event stream (with --dashboard)
  round-0/
    metrics.json              # classifier axes         (with --dashboard)
  round-1/
    prompt.md
    codex.md
    gemini.md
    synthesis.md
    claims.toml
    metric-scores.json        # per-round scores        (with --dashboard)
  round-2/
    ...
  final/
    synthesis.md
    claims.toml
    dissent.md
    meta-summary.toml
    report.html               # with --html flag
```

## Architecture

```
Participants (any CLI, LLM, or human)
        |  write responses
        v
   Filesystem Substrate
   sessions/<id>/round-N/*.md
        |  watch (notify)
        v
    Fire Keeper (this binary)
    - Orchestrates rounds
    - Generates synthesis (via claude CLI)
    - Evaluates convergence (LLM judge)
    - Writes final output
```

---

<p align="center">Built on 🌍 with ❤️</p>
