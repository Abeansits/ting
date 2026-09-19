#!/usr/bin/env python3
"""Package already-built native binaries, docs, notices, and build provenance."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--target', required=True, choices=[
        'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu',
        'x86_64-apple-darwin', 'aarch64-apple-darwin',
    ])
    parser.add_argument('--output', type=Path, default=Path('dist'))
    parser.add_argument('--tui-binary', type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=root))
    package = next(item for item in metadata['packages'] if item['name'] == 'ting')
    version = package['version']
    binary = root / 'target' / args.target / 'release' / 'ting'
    if not binary.is_file():
        parser.error(f'Build {binary} before packaging')
    if args.tui_binary and not args.tui_binary.is_file():
        parser.error(f'Missing TUI binary: {args.tui_binary}')
    args.output.mkdir(parents=True, exist_ok=True)
    basename = f'ting-{version}-{args.target}'
    archive = args.output / f'{basename}.tar.gz'
    binaries = {'bin/ting': binary}
    if args.tui_binary:
        binaries['bin/ting-tui'] = args.tui_binary
    info = {
        'version': version,
        'target': args.target,
        'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
        'dirty': bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=root, text=True).strip()),
        'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
        'license': package.get('license'),
        'sha256': {name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in binaries.items()},
    }
    with tarfile.open(archive, 'w:gz') as tar:
        for name, path in binaries.items():
            tar.add(path, arcname=f'{basename}/{name}')
        for name in ['README.md', 'ROADMAP.md', 'CONTRIBUTING.md', 'SECURITY.md', 'SUPPORT.md', 'CHANGELOG.md', 'LICENSE']:
            path = root / name
            if path.is_file():
                tar.add(path, arcname=f'{basename}/{name}')
        for name in ['docs', 'examples', 'schemas', 'vendor/report']:
            tar.add(root / name, arcname=f'{basename}/{name}')
        content = (json.dumps(info, indent=2) + '\n').encode()
        entry = tarfile.TarInfo(f'{basename}/BUILD-INFO.json')
        entry.size = len(content)
        entry.mode = 0o644
        tar.addfile(entry, io.BytesIO(content))
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(archive.suffix + '.sha256').write_text(f'{checksum}  {archive.name}\n')
    print(archive)


if __name__ == '__main__':
    main()
