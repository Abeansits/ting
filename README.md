# Ting

**Independent proposals. Cross-examination. A recommendation that keeps the dissent.**

Ting runs structured discussions between AI models and humans. Each participant
proposes a position, critiques another, and revises its thinking. You get a saved
recommendation, the disagreements that remain, and a record of how the discussion
changed.

![Ting's illustrative demo: two rounds, three participants, and per-round metrics](docs/images/demo-dashboard.png)

*An actual screenshot of `ting demo`. Its participants and scores are hand-written
sample data, not live model evaluations.*

## Try it

**No compiler needed:** download a [v0.5.0-rc.2 preview archive](https://github.com/Abeansits/ting/releases/tag/v0.5.0-rc.2)
for Linux or macOS, verify its checksum, and run `bin/ting demo`.
See [binary installation](docs/INSTALL.md) for platform selection and commands.
The preview is available for testing; full live-model regression remains pending.

Build on **macOS or Linux** with [Rust 1.88+](https://rustup.rs/):

```sh
git clone https://github.com/Abeansits/ting.git
cd ting
cargo install --path . --locked
ting demo
```

The demo needs **no API keys, model CLIs, or paid calls**. It prints a sample
recommendation and dissent, then opens the dashboard at `http://127.0.0.1:3420`.
Press Ctrl+C to stop the server; the sample stays on disk for inspection.

Prefer files only? Run `ting demo --output ./my-sample --no-serve`. Existing
folders are never overwritten. Read the [sample output](examples/demo-output.md)
or edit the [sample source](examples/demo-forum.json).

## Run your own discussion

For a first live run, [install Claude Code and sign in](https://code.claude.com/docs/en/setup).
Ting uses it for synthesis and judging. This example also uses it for two
separately prompted participants, so you only need one model CLI:

```sh
ting doctor --participant claude

ting new "Should our small team add a message queue?" \
  --participant claude \
  --participant 'critic:command:cat {prompt_file} | claude -p -'
```

The two participants each call Claude once per round; synthesis and judging
make additional calls. They are not independent model providers. For different
models, replace `critic` with a `codex` or `gemini` participant after setting
up that CLI. Use `ting preset list` to inspect the built-in commands.
`doctor` checks executables, not authentication. A live run makes several paid
model calls and can take minutes. Review [execution, privacy, and cost](docs/EXECUTION.md)
before starting; participant commands inherit your environment and permissions.

After the run, use the forum ID printed by Ting:

```sh
ting list
ting status <forum-id>
ting result <forum-id>
ting result <forum-id> --html
ting serve <forum-id>
```

HTML reports are self-contained and work offline. Publishing is a separate,
explicit `--publish` action; review private context before sharing.

## What makes it useful

- **Structured disagreement:** proposals, assigned critiques, and informed revision.
- **Dissent survives convergence:** enough agreement to stop does not mean unanimity.
- **Bring your tools:** model CLIs, custom commands, local-model participants, or humans.
- **Inspect the evidence:** prompts, responses, claims, and outcomes are ordinary files.
- **Watch progress:** a browser dashboard and an optional Go TUI share the event log.

Ting is useful for architecture choices, planning, and decisions with real
tradeoffs. It is a standalone CLI, and deliberation is slower than a single
response. Agreement among models is not proof that their recommendation is right.

## Explore

- [CLI, presets, human participation, and evaluation](docs/CLI.md)
- [Architecture and session files](docs/ARCHITECTURE.md)
- [Resuming forums in 0.6 development builds](docs/RECOVERY.md)
- [Roadmap](ROADMAP.md) · [Changelog](CHANGELOG.md)
- [Contributing](CONTRIBUTING.md) · [Good first issues](https://github.com/Abeansits/ting/labels/good%20first%20issue)
- [Support](SUPPORT.md) · [Security reporting](SECURITY.md)

Small fixes, examples, and thoughtful critiques are welcome. The automated tests
use fake model commands, so you can contribute without paid model accounts.

## License

Ting is [MIT licensed](LICENSE). Bundled report libraries retain their
[upstream licenses and notices](vendor/report/README.md).
