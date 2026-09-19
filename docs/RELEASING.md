# Building and releasing binaries

The `Release binaries` workflow builds native CLI and TUI binaries on Linux
(x86-64 and ARM64) and macOS (Intel and Apple Silicon). Each archive contains both
binaries, docs/examples, vendored notices, and `BUILD-INFO.json` with the source
commit, compiler, target, and binary hashes. Each archive has a SHA-256 sidecar.
The build record also flags a dirty source checkout; published builds should come
from a clean, tagged commit.

PRs changing the release workflow or packaging script build and smoke-test the
archives. A manual workflow run does the same without creating a GitHub release.
These are preview build artifacts, not a published version.

## Prepare a release

1. Confirm the project license is chosen and recorded in `LICENSE` and Cargo
   metadata. The release job refuses to proceed without it.
2. Finish the intended milestone and review its remaining validation gaps.
3. Update the package version and lockfile, and move changelog entries into the
   versioned release section. Merge that PR after CI passes.
4. Create and push a matching `v<version>` tag on the tested commit.
5. The workflow builds and tests each archive, then creates a **draft** GitHub
   release. Review its assets, checksums, build information, and release notes.
6. Publish the draft only when the maintainer is ready. The workflow never
   publishes a release automatically.

Tags with a prerelease suffix (such as `v0.5.0-rc.1`) create prerelease drafts,
not stable releases. The first public-welcome build uses this route while the full
live-model regression is still pending.

The pipeline currently targets Ubuntu 22.04 runners for Linux builds and macOS 15
runners for macOS builds. CI smoke tests the extracted binary on each target;
compatibility with older operating systems is not implied. macOS binaries are
not signed or notarized yet. Do not advise users to disable Gatekeeper globally.

## Verify an archive

```sh
# Linux
sha256sum -c ting-<version>-<target>.tar.gz.sha256
# macOS
shasum -a 256 -c ting-<version>-<target>.tar.gz.sha256

tar -xzf ting-<version>-<target>.tar.gz
./ting-<version>-<target>/bin/ting --version
./ting-<version>-<target>/bin/ting demo
```

Inspect `BUILD-INFO.json` to match the archive to its source commit. A checksum
detects a changed download; it is not a signature or independent attestation.
