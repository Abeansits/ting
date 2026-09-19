# Architecture and session files

Ting v0.4 ships a live dashboard so you can watch a deliberation unfold —
per-round syntheses arriving, per-metric scores updating each round, and
convergence climbing toward threshold — instead of tailing log files.

### What runs where

Turning on `--dashboard` activates four cooperating pieces:

1. **JSONL event log.** The Fire Keeper emits an append-only event stream
   to `~/.ting/sessions/<forum-id>/dashboard-events.jsonl` with a
   versioned envelope (`seq`, `forum_id`, `timestamp`, `type`, `payload`).
   Monotonic `seq` is the authoritative ordering key. The runtime
   emits `forum_started`, `round_started`, `participant_response`,
   `classifier_metrics`, `metric_scores`, `synthesis`, `convergence`,
   `forum_complete`, and `forum_failed`. `claims` and `alignment` remain reserved.
   JSON Schemas, a companion `dashboard-state.json` snapshot format,
   and reader/writer guarantees live in [`schemas/`](../schemas).

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
   disk (404 otherwise; clients replay the event log when there is no
   snapshot). The UI renders metric bars, a convergence gauge, and a
   synthesis progress indicator in pure CSS; no charting library. The Dissent Axis
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

A standalone terminal client lives under [`tui/`](../tui) for when a
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

No event log, classifier call, metric-scoring pass, or HTTP server is started.
Participant prompts and run outcomes are still saved for inspection and reliable
completion tracking.

## Directory Structure

```
~/.ting/sessions/<forum-id>/
  meta.toml
  run-status.json            # running/completed/failed outcome; runner PID
  dashboard-events.jsonl      # append-only event stream (with --dashboard)
  round-0/
    metrics.json              # classifier axes         (with --dashboard)
  round-1/
    prompt.md
    prompts/
      codex.md                # exact input with participant identity
      gemini.md
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

Completion is recorded only after synthesis, claims, dissent, and summary metadata
are written. Failed runs keep their partial artifacts for inspection, but `ting
result` does not present them as completed output. On Unix, a running record whose
process has exited is shown as interrupted. Older sessions without a run-status
record count as complete only when all four final artifacts exist.

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


See [execution and cost](EXECUTION.md) for model-call and permission details.
