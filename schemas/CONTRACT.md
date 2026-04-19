# Ting v0.4 Dashboard Contract

Phase 1A ships the cross-language contract consumed by Track 2 (axum SSE
bridge) and Track 3 (Go Bubble Tea TUI). This document is the source of truth
for reader/writer guarantees; the JSON Schemas next to it (`dashboard-event.schema.json`,
`dashboard-state.schema.json`) are the wire-level spec. If a guarantee below
contradicts the code, fix the code.

## Files in a forum directory

| Path | Role | Writer | Readers |
|------|------|--------|---------|
| `dashboard-events.jsonl` | Append-only canonical event stream | Rust protocol (single writer) | axum tailer, Go TUI tailer |
| `dashboard-state.json` | Compacted snapshot of the events log up to `latest_seq` | Rust protocol | Same — used for `init` on SSE connect |

## Writer guarantees (Rust protocol)

**Event log (`dashboard-events.jsonl`):**
- Exactly one process writes. Multi-writer would need a lock file; that is
  out of scope for v0.4.
- Each event is one JSON object followed by `\n`. The writer uses `O_APPEND`
  and calls `sync_data()` after the line. A single `write_all` of a line
  under `PIPE_BUF` (4096 on Linux/macOS) is atomic under POSIX, so tailing
  readers see either zero or the full line — never a split.
- `seq` is monotonically increasing, starts at 1, and has no gaps under
  normal operation. On resume (fresh process), `next_seq` returns
  `max(existing_seq) + 1` so a restart continues where the crashed process
  left off.
- `timestamp` is informational; **`seq` is the authoritative ordering**.

**State snapshot (`dashboard-state.json`):**
- Written via `.json.tmp` + `rename`. Readers either see the file missing,
  the previous version, or the new version — never a torn read.
- Crash-durable: the temp file is fsynced before rename, and the containing
  directory is fsynced after rename. A successful `write_state` call means
  the bytes survive a power loss.
- `latest_seq` records the highest event seq folded into this snapshot.
  Consumers that replay must pick up at `latest_seq + 1` in the event log.

## Reader guarantees (axum, Go TUI, or any future consumer)

**Event log tailing:**
- Process lines in file order. Trailing data that does not end in `\n` is an
  in-flight write; hold it until a newline arrives.
- A well-formed event always has `version`, `seq`, `forum_id`, `timestamp`,
  `type`, and `payload`. Readers **MUST** validate these six fields.
- Unknown `type` values MUST be ignored with a warning, not fatally. This is
  what lets the Rust producer add new event types without breaking older
  consumers.
- Unknown payload keys MUST be ignored. Payload shapes are documented in the
  JSON Schema; the schema intentionally allows `additionalProperties` on
  payloads so producers can evolve.
- Malformed lines (invalid JSON, missing envelope fields) MUST be skipped
  with a warning; readers keep going.

**State snapshot:**
- If `dashboard-state.json` exists but fails to parse, fall back to replaying
  the event log from `seq=1`. A non-parseable snapshot is treated as missing.

## Versioning policy

Each event and each snapshot carries `version: u32`. The version lives at
the top level of each record (not only at the file level) so that mixed-
version logs from long-running or resumed forums remain legible.

- **v1 (current, Phase 1A):** the shape documented in the schemas.
- **Additive / minor changes** (no version bump): adding new `EventType`
  variants, adding optional fields to an existing payload, adding optional
  fields to the state snapshot.
- **Breaking / major changes** (version bump): renaming or removing an
  envelope field, changing the `seq` / `timestamp` type, changing the
  `type` discriminator.

When the contract goes to `version: 2`, readers MUST either upgrade or
refuse to interpret `v2` records — they **MUST NOT** silently ignore a
version bump.

## Cross-language notes

- `seq` is `u64` in Rust and `uint64` in Go. If a browser / JavaScript
  consumer ever tails the JSONL directly (instead of via the SSE bridge,
  which can re-encode), `seq` values above `Number.MAX_SAFE_INTEGER`
  (2⁵³ − 1 ≈ 9 × 10¹⁵) will lose precision. In practice a single forum
  will never emit that many events, so this is a documented accepted risk
  rather than a per-event encoding change.
- Timestamps are RFC 3339 UTC (`2026-04-19T13:55:00Z`). Go's
  `time.Parse(time.RFC3339Nano, …)` and JavaScript's `new Date(s)` both
  accept this.

## What Phase 1A does not ship

- `protocol.rs` does not yet emit events. Phase 1B wires this in.
- There is no event-log rotation; the log grows unbounded. For v0.4 this is
  acceptable — a forum has minutes-to-hours of events, not days.
- There is no `EventWriter` struct caching an open file handle. For Phase 1A
  the free-function `append_event` is enough; Phase 1B will introduce a
  writer with cached `seq` and open FD when the protocol starts emitting at
  per-event frequency.
