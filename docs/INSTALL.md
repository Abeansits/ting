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

These are complete examples for the most common systems. Each downloads the
archive and matching checksum from the [preview release](https://github.com/Abeansits/ting/releases/tag/v0.5.0-rc.2),
verifies it, and starts the no-account demo. Run one block in a new directory.

macOS Apple Silicon:

```sh
mkdir ting-preview && cd ting-preview
curl -fLO https://github.com/Abeansits/ting/releases/download/v0.5.0-rc.2/ting-0.5.0-rc.2-aarch64-apple-darwin.tar.gz
curl -fLO https://github.com/Abeansits/ting/releases/download/v0.5.0-rc.2/ting-0.5.0-rc.2-aarch64-apple-darwin.tar.gz.sha256
shasum -a 256 -c ting-0.5.0-rc.2-aarch64-apple-darwin.tar.gz.sha256
tar -xzf ting-0.5.0-rc.2-aarch64-apple-darwin.tar.gz
./ting-0.5.0-rc.2-aarch64-apple-darwin/bin/ting demo
```

Linux x86-64:

```sh
mkdir ting-preview && cd ting-preview
curl -fLO https://github.com/Abeansits/ting/releases/download/v0.5.0-rc.2/ting-0.5.0-rc.2-x86_64-unknown-linux-gnu.tar.gz
curl -fLO https://github.com/Abeansits/ting/releases/download/v0.5.0-rc.2/ting-0.5.0-rc.2-x86_64-unknown-linux-gnu.tar.gz.sha256
sha256sum -c ting-0.5.0-rc.2-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf ting-0.5.0-rc.2-x86_64-unknown-linux-gnu.tar.gz
./ting-0.5.0-rc.2-x86_64-unknown-linux-gnu/bin/ting demo
```

For macOS Intel or Linux ARM64, download the matching archive and `.sha256` file
from the release page using the target in the table above. Run `shasum -a 256 -c`
on macOS or `sha256sum -c` on Linux, extract the archive, then run `bin/ting demo`
inside the extracted directory.

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

The demo needs no model accounts. For the one-provider live example in the
[README](../README.md#run-your-own-discussion), [install and authenticate Claude Code](https://code.claude.com/docs/en/setup),
then run:

```sh
ting doctor --participant claude
```

Doctor checks binary files, not authentication. See [execution and cost](EXECUTION.md)
and the [CLI reference](CLI.md) before making model calls.
