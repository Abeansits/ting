# Installing Ting

## Preview binaries

The `v0.5.0-rc.2` release candidate is intended for trying the new workflow and
running the later full regression. See its release notes for validation and known
limits. Choose the archive matching your operating system and CPU:

| Platform | Archive target |
| --- | --- |
| Linux x86-64 | `x86_64-unknown-linux-gnu` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` |
| macOS Intel | `x86_64-apple-darwin` |
| macOS Apple Silicon | `aarch64-apple-darwin` |

Download the archive and matching `.sha256` file from the
[preview release](https://github.com/Abeansits/ting/releases/tag/v0.5.0-rc.2). From their
download directory, verify the checksum:

```sh
# Linux
sha256sum -c ting-0.5.0-rc.2-<target>.tar.gz.sha256
# macOS
shasum -a 256 -c ting-0.5.0-rc.2-<target>.tar.gz.sha256
```

Then extract and run:

```sh
tar -xzf ting-0.5.0-rc.2-<target>.tar.gz
cd ting-0.5.0-rc.2-<target>
./bin/ting --version
./bin/ting demo
```

The archive includes the optional `bin/ting-tui`, docs, examples, upstream notices,
and build provenance. Copy `bin/ting` to a directory already on your PATH if you
want to invoke it as `ting`. No Rust or Go installation is needed for these binaries.

Linux archives are built and smoke-tested on Ubuntu 22.04. macOS archives are built
and smoke-tested on macOS 15; they are not signed or notarized. Compatibility with
older operating systems has not been established. Native Windows binaries are not
currently provided; source builds in WSL are an option to try.

## From source

Install [Rust 1.88 or newer](https://rustup.rs/), then:

```sh
git clone https://github.com/Abeansits/ting.git
cd ting
cargo install --path . --locked
ting demo
```

Cargo normally installs to `~/.cargo/bin`. Ensure that directory is on PATH if your
shell cannot find `ting`. To install to a different prefix, use
`cargo install --path . --locked --root /your/prefix` and invoke its `bin/ting`.

Building from `main` can include unreleased changes. To build the preview's exact
source, check out `v0.5.0-rc.2` before running the install command.

## Live-run prerequisites

The demo needs no model accounts. For live discussions, install/authenticate Claude
Code plus the participant CLIs you want to use, then run:

```sh
ting doctor --participant codex --participant gemini
```

Doctor checks binary files, not authentication. See [execution and cost](EXECUTION.md)
and the [CLI reference](CLI.md) before making model calls.
