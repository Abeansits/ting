# Execution, privacy, and cost

## Before a live run

Run `ting doctor --participant codex --participant gemini` to check executable
availability. It does not run presets, authenticate accounts, contact providers,
or spend model credits. Custom shell commands are reported for manual inspection.
`--json` produces a machine-readable report; exit code 1 means an inspected
executable is missing. A successful check does not prove authentication or model
access. `ting demo` works without those prerequisites.

Use `ting preset list` to see the actual configured participant commands. User
presets in `~/.ting/config.toml` override built-ins. For the commands captured by a
particular run, inspect its `meta.toml`; personalized prompts are saved under
`round-N/prompts/<participant>.md`.

## Where commands run

Ting launches participant commands through `sh -c` from the directory where you
started Ting. Commands inherit your environment and operating-system permissions.
The default internal Claude call invokes `claude` directly. Ting does not add a
sandbox around either path. Each provider CLI controls its own tool permissions;
some presets enable automatic agent actions. Run from a suitable working
directory and review the configured command before using an unfamiliar preset.

Prompts are delivered through stdin or a temporary prompt file, according to the
command template. The temporary file is removed when the invocation finishes.
Session files remain on disk until you remove them.
Recovery checkpoints and archived artifacts also contain forum content; handle
them with the same care as the original prompts and responses.

## What leaves your machine

A provider-backed participant receives the forum topic, attached context, and
the prompt for its round. Later rounds include other participants' positions and
synthesis. Internal synthesis and evaluation calls also receive relevant forum
content. `--context` reads a file if the path exists; otherwise the value is
treated as literal text. Check spelling before relying on an attached file.

Local-model participants do not make the whole run local: the default Fire Keeper
still uses Claude for internal generation and judging. Manual participants also
do not remove that dependency. Provider CLIs may have their own tools and data
handling behavior; consult their settings for the account you use.

The dashboard binds to `127.0.0.1` and has no authentication. It is intended for
your own machine. `ting result --html --publish` explicitly invokes `herenow` to
publish a report, which includes participant responses and potentially attached
context. Review the report before publishing it. Don't commit private session
directories or raw credentials to this repository.

Exported HTML embeds its renderer and sanitizer, so it works without CDN access.
The report blocks automatic network resources and omits images/media from model
Markdown. Links remain clickable; following one is an explicit browser action.

## What adds model calls

Participants are called once per round. Each round also generates a synthesis
and claims, scores participant alignment, and evaluates convergence once the
minimum round count is reached. Final dissent is generated even after convergence.

With `--dashboard`, Ting additionally invents metrics once and scores them each
round. `--no-classifier` disables both metric invention and metric scoring;
`--no-metric-scoring` keeps the metrics but skips their round scores. Dashboard
transport itself makes no model calls.

Convergence and alignment failures retry once. More participants, larger prompts,
additional rounds, and retries can all increase usage. Pricing and allowances
come from your provider accounts; Ting currently does not promise a dollar-cost
estimate. `--max-rounds` is a hard ceiling on iterations: severe disagreement does
not add extra rounds. The final summary distinguishes convergence from an exhausted
round budget.
