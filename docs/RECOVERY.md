# Resuming a forum

This feature is in the `0.6.0-dev` source build, after the `v0.5.0-rc.2` preview. It requires a
forum created with resumable execution records; older sessions and the illustrative
demo remain readable but cannot be safely resumed automatically.

```sh
ting resume <forum-id>
ting resume <forum-id> --dashboard --no-open
```

Resume executes the commands saved in `meta.toml`. Only resume forums whose
commands you trust. `status` and `result` inspect files without executing them.

## What is reused

Ting records successful operations under `.checkpoints/` beside their output
files. Each record contains an input fingerprint, the output and its checksum,
and whether the output file was materialized. Checkpoints cover prompts,
participant responses, synthesis, claims, metric invention/scoring, alignment,
convergence, and final dissent generation.

A matching checkpoint skips the provider call. A missing output file is restored
from the recorded result. `run-options.json` preserves the original classifier
and scoring selections. `--dashboard` can enable event logging/serving during
resume without adding classifier or scoring calls that the original run omitted.

The runner holds an operating-system file lock. Two Ting processes cannot execute
the same forum concurrently. A killed process releases that lock; new checkpointed
forums use it to detect interruption instead of trusting a potentially reused PID.

## When inputs change

Saved prompts, participant responses, syntheses, claims, and metric definitions
can be edited. Resume adopts those edits when their own generating inputs still
match, then invalidates dependent work. Changed rubric descriptions/scales trigger
new metric scoring; changed responses trigger dependent generation and judgments.
Derived score files are restored/recomputed rather than trusted as manual overrides.

If a participant's prompt changes, its old answer no longer satisfies that prompt.
Command participants run again; old manual answers are archived and a new response
is requested. Use `ting respond --file ...` for atomic manual submissions. Direct
file writers must also publish complete files atomically rather than stream into a
response file while Ting is watching it.

Superseded artifacts are retained under `.checkpoints/obsolete/`. If a resumed
discussion finishes earlier, unused later rounds are archived so they cannot
appear in the new report. Old HTML exports are also archived; regenerate them with
`ting result --html` after the new attempt finishes.

The event log retains attempt history. A new `forum_started` marks a new attempt;
the browser replays the latest attempt, and browser/TUI reducers clear old rounds
and scores at that boundary. Refresh an already-ended browser stream or run
`ting serve` again to observe a new attempt.

## Limits

- A provider call can be repeated if the process dies after the provider responds
  but before the result reaches the durable journal. Recovery is not an
  exactly-once guarantee for external APIs.
- Checkpoints compare recorded inputs and requested model/command settings. They
  do not freeze a provider's implementation, environment, or the contents of an
  external command script. Pin models and keep scripts stable when that matters.
- An invalid checkpoint checksum or unsupported checkpoint format is an error,
  not permission to invent or silently trust a result.
- Final results are available only after successful finalization. A failed attempt
  keeps its evidence, but `ting result` does not present it as complete.
- Graceful whole-forum cancellation remains separate roadmap work.
