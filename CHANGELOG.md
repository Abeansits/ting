# Changelog

All notable changes to Ting will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] - 2026-04-21

First release of the live dashboard stack: an append-only JSONL event log
emitted by the Fire Keeper, an axum-backed HTML dashboard, and a standalone
Go TUI — all reading the same on-disk contract so a forum's state can be
replayed or observed without coupling the renderer to the orchestrator.

### Added

- **Dashboard event contract** (#1) — append-only `dashboard-events.jsonl` per
  forum with a versioned envelope (`version`, `seq`, `forum_id`, `timestamp`,
  `type`, `payload`). Event types: `forum_started`, `round_started`,
  `participant_response`, `synthesis`, `claims`, `alignment`,
  `classifier_metrics`, `metric_scores`, `convergence`, `forum_complete`.
  Monotonic `seq` is the authoritative ordering key; `timestamp` is
  informational. Writes are `O_APPEND` + `\n`-terminated + `sync_data()`,
  atomic per-line under `PIPE_BUF`. Companion `dashboard-state.json`
  snapshot is written via temp-file + rename with directory fsync.
  JSON Schema (Draft 2020-12) for both the event envelope and the
  state snapshot lives under `schemas/`, alongside `CONTRACT.md` covering
  reader/writer guarantees, versioning, and cross-language notes.

- **Pre-round Fire Keeper classifier** (#2) — before round 1, when
  `--dashboard` is on, the Fire Keeper generates 5–10 question-specific
  metrics plus a mandatory `dissent_axis`, writes `round-0/metrics.json`,
  and emits a `classifier_metrics` event. Resume re-validates the on-disk
  file and backfills a missing event if a prior run crashed between the
  write and the log append. `--no-classifier` opts out.

- **Per-round metric scoring** (#3) — after each round's responses land, the
  Fire Keeper scores every classifier metric in a single batched pass,
  writes `round-N/metric-scores.json`, and emits a `metric_scores` event.
  Resume semantics mirror the classifier (file re-validated; event
  backfilled if missing). Scoring failures are warn-and-continue so a
  classifier hiccup cannot abort the forum. `--no-metric-scoring` opts out.

- **Dashboard HTTP server** (#4, #5, #6) — `axum` 0.8 router bound to
  loopback only, default port `3420`:
  - `GET /` serves the dashboard HTML shell.
  - `GET /api/state` returns the compacted snapshot.
  - `GET /api/events` is an SSE stream: `event: init` (snapshot) →
    `event: update` (full log replay) → live events deduped by `seq` →
    `event: ping` every 15s → clean close on `forum_complete`.
  - Tailer uses `notify` + cursor-based incremental reads, buffers
    partial trailing lines, recovers from truncation, and catches up on
    events written before the watcher armed.
  - HTML/CSS/JS live under `src/static/` and are `include_str!`-ed.
    Metric bars, convergence SVG gauge, and status pills animate in pure
    CSS. The Dissent Axis is always pinned to the top of the metrics
    panel. Static routes set `X-Content-Type-Options: nosniff`.

- **`ting new --dashboard`** (#7) — co-schedules the HTTP server alongside
  the forum on a `spawn_blocking` task, with structured-concurrency
  shutdown (forum-error-wins reconcile). New flags: `--port <N>`,
  `--no-open`. When stderr is a TTY the dashboard URL auto-opens in the
  browser; the child process is reaped in a detached thread so it never
  becomes a zombie.

- **`ting serve <forum-id>`** (#7) — serves the dashboard against an
  existing forum directory without running a new round. Detects completed
  vs in-progress. Graceful Ctrl+C with a 5s force-exit fallback so a
  long-lived SSE client cannot hang the process. Same `--port` and
  `--no-open` flags.

- **Go TUI** (#8, #9) — a standalone terminal client under `tui/`, built
  on Bubble Tea + Lip Gloss + fsnotify. Reads `dashboard-state.json` for
  the initial frame, tails `dashboard-events.jsonl` for live updates; no
  HTTP coupling to the Rust binary. Renders a header with status +
  elapsed + seq, topic and participant chips, a rounds table with a
  progress bar, per-axis metric bars with round-indexed sparklines (the
  Dissent Axis always in reverse-video), a convergence panel with chained
  history, and a focusable synthesis list. Keys: `q`/Ctrl-C/Esc quit,
  `r` reload, `?` help, `↑`/`↓` or `j`/`k` focus rounds, `1`–`9` jump,
  `0` clear.

- **CI** (#8) — new `Go TUI` GitHub Actions workflow running `gofmt`,
  `go vet`, and `go test -race -count=1`, guarding the new module from
  day one.

### Changed

- **`ting new` CLI** (#2, #3, #7) — new flags: `--dashboard`,
  `--no-classifier`, `--no-metric-scoring`, `--port <N>`, `--no-open`.
  Without `--dashboard`, behavior is bit-for-bit identical to v0.3: no
  new disk side effects, no event log, no classifier call.
- **Startup banner** — now reports `v0.4.0` (driven by
  `CARGO_PKG_VERSION`).

### Fixed

- **Dashboard lifecycle events** (#10) — `run_forum` now emits the
  `synthesis`, `convergence`, and `forum_complete` events that Phase 2C
  built widgets for but no earlier phase wired. Without this, the
  dashboard's Latest Synthesis and Convergence gauge rendered empty on a
  live run. Emission is gated on a new `RunOptions::emit_events` flag
  tracking `--dashboard` (independent of `--no-classifier`). Additional
  fixes folded in: non-finite judge scores are filtered in
  `parse_judge_response` (serde_json panics on NaN/inf), and the
  `ForumCompletePayload.rounds_used` schema constraint was relaxed to
  `minimum: 0` so zero-round degenerate runs still close the stream
  cleanly.

## [0.3.0] - 2026-03-29

Initial public release.
