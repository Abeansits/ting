# CLI reference

Use `ting --help` or `ting <command> --help` for the exact flags supported by your
installed version. See [execution and cost](EXECUTION.md) before live model calls.

## Demo and setup

```sh
ting demo                            # sample result + browser dashboard
ting demo --no-open --port 4000
ting demo --output ./sample --no-serve
ting doctor --participant codex --participant gemini
ting doctor --json
```

The demo is an explicitly illustrative, hand-written forum and never calls a
model. Doctor only inspects executable files. Custom commands and authentication
are not tested automatically; missing inspected binaries produce exit code 1.

## New forum

```sh
ting new "Should we adopt this architecture?" \
  --participant codex --participant gemini --participant human \
  --context notes.md --timeout 5m --max-rounds 2 --dashboard
```

The command blocks until the forum finishes or fails. Repeated `--participant`
flags select participants; names must be unique. Context is snapshotted once.
An existing context path is read as a file; other values become literal context.

| Flag | Behavior |
| --- | --- |
| `--timeout 5m` | Timeout for command participants and the manual response wait; default `5m` |
| `--max-rounds 2` | Hard ceiling on rounds; default `2`. Convergence can stop earlier, but disagreement never adds a round. |
| `--output-format review` | Produce a prioritized findings-oriented synthesis |
| `--dashboard` | Serve loopback dashboard, emit events, invent metrics, and score them |
| `--no-classifier` | Skip metric invention and scoring |
| `--no-metric-scoring` | Keep metric labels but skip per-round metric scoring |
| `--port 4000` | Dashboard port; default `3420` |
| `--no-open` | Do not open the browser automatically |

Resume is tracked in the [roadmap](../ROADMAP.md). A round ceiling limits iterations,
not provider tokens or dollars: each round can still make several model calls.

## Presets and custom participants

```sh
ting preset list
ting preset add mistral 'cat {prompt_file} | ollama run mistral'
ting new "Topic" --participant mistral --participant codex
ting preset remove mistral
```

Built-ins: `codex`, `gemini`, `claude`, `opencode`, `ollama`, and `human`.
`ting preset list` shows the configured commands. Presets saved in
`~/.ting/config.toml` override built-ins of the same name.

For a one-off command or two instances of one model, use distinct aliases:

```sh
ting new "Topic" \
  --participant 'optimist:command:cat {prompt_file} | ollama run mistral' \
  --participant 'skeptic:command:cat {prompt_file} | ollama run llama3'
```

Prompts go through stdin unless the command uses `{prompt_file}`. The same file
path is available as `$TING_PROMPT_FILE`. Commands run through `sh -c` from the
current working directory. Prompt text is not interpolated into the shell command.
The default Fire Keeper still requires Claude even when all participants use local
models. Personalized prompts explicitly name the participant and are saved under
`round-N/prompts/<name>.md`.

## Human participation

```sh
ting new "Topic" --participant human --participant codex
# From another terminal, read the personalized prompt path printed by Ting:
ting respond <forum-id>
# Or submit an existing file:
ting respond <forum-id> --round 2 --participant human --file response.md
```

`respond` chooses the latest round and the sole pending human when possible. With
multiple manual participants, specify a name. Without `--file`, it opens an editor;
empty responses are not submitted. Named manual participants use `name:manual`.

## Inspect and export

```sh
ting list
ting status <forum-id>
ting status <forum-id> --round 2
ting result <forum-id>
ting result <forum-id> --html
```

`result` requires a completed run. The HTML file is saved to `final/report.html`,
with its renderer and sanitizer embedded. Partial files remain inspectable after
a failed run, but they are not presented as a completed result.

```sh
ting result <forum-id> --html --publish
```

Publishing requires the separately installed `herenow` CLI. It sends the generated
report to that service. Read it first: reports can include attached context and
participant responses.

## Dashboard and terminal UI

```sh
ting serve <forum-id> --port 4000 --no-open
```

`serve` observes an existing forum without starting new rounds. The browser shows
round/participant progress, metric scores, convergence history, and synthesis
progress. Use `ting result` or an HTML export to read complete final content.

With Go 1.24+:

```sh
cd tui
go build -o ting-tui .
./ting-tui ~/.ting/sessions/<forum-id>
```

Keys: `q`/Ctrl+C/Esc quit, `r` reload snapshot, `?` help, arrows or `j`/`k` focus
rounds, `1`–`9` jump to a round, and `0` clear focus.

## Evaluation

```sh
ting eval "Should our small team add a message queue?" \
  --baseline claude --forum codex,gemini --judge claude --html
```

Evaluation compares a single-model response with a forum synthesis using a blind
A/B assignment. Defaults are a `10m` participant timeout and three requested
rounds. `--context` attaches the same input to the comparison. Without `--judge`,
Ting attempts to select a judge outside the participant set.

This is a model-judged comparison, not proof of better decisions. Inspect the
reasoning and the participant evidence. Evaluation files live in `~/.ting/evals/`;
the forum is linked from the evaluation directory on Unix. Evaluation reports also
embed their rendering assets for offline use.

## Configuration

`meta.toml` records the topic, participants, timing, synthesis model, and convergence
settings for a run. Normal `ting new` creates it automatically. Editing it during a
run does not reconfigure the in-memory execution.

The default convergence threshold is 7 on a 1–10 agreement scale, with a minimum
of two rounds for early stopping. The implemented protocol is `delphi-crossexam`
and the implemented convergence policy is `llm-judge`; other values are rejected
before a run starts. Timeouts must be positive and representable on the platform.

`quorum`, `late_policy`, and `max_prior_context` were previously written but never
implemented. They are retired and no longer emitted in new configuration. Old
generated defaults (`0`, `include_next`, and `4000`) remain readable for compatibility;
they do not enable a policy or token cap. Nondefault values must be removed before
execution. Historical sessions remain inspectable even with unsupported policies.
