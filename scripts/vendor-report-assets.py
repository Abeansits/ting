#!/usr/bin/env python3
"""Fetch pinned report assets from public npm and verify archive integrity."""
import base64
import hashlib
import io
import json
from pathlib import Path
import tarfile
import urllib.request

DEST = Path(__file__).resolve().parents[1] / 'vendor' / 'report'
PACKAGES = [
    ('marked', '15.0.12', 'sha512-8dD6FusOQSrpv9Z1rdNMdlSgQOIP880DHqnohobOmYLElGEqAL/JvxvuxZO16r4HtjTlfPRDC1hbvxC9dPN2nA==', {
        'package/marked.min.js': 'marked.min.js',
        'package/LICENSE.md': 'marked-LICENSE.md',
    }),
    ('dompurify', '3.4.15', 'sha512-EUBjM+B+lkDE41iE82DDSCfkoPGfXx8IxFxPMjNzm/Uk4xDet77rTN9wqlxlVg71kK7XGuUMv6wUxJUwwv+Xyw==', {
        'package/dist/purify.min.js': 'purify.min.js',
        'package/LICENSE': 'dompurify-LICENSE',
        'package/LICENSE-MPL': 'dompurify-LICENSE-MPL',
    }),
]

DEST.mkdir(parents=True, exist_ok=True)
manifest = []
for name, version, integrity, files in PACKAGES:
    url = f'https://registry.npmjs.org/{name}/-/{name}-{version}.tgz'
    with urllib.request.urlopen(url, timeout=30) as response:
        archive = response.read()
    actual = 'sha512-' + base64.b64encode(hashlib.sha512(archive).digest()).decode()
    if actual != integrity:
        raise RuntimeError(f'Integrity mismatch for {name}')
    entry = {'package': name, 'version': version, 'url': url, 'integrity': integrity, 'files': {}}
    with tarfile.open(fileobj=io.BytesIO(archive), mode='r:gz') as tar:
        for member, filename in files.items():
            content = tar.extractfile(member).read()
            if filename.endswith('.js') and b'</script' in content.lower():
                raise RuntimeError(f'{filename} is unsafe to embed in a script element')
            (DEST / filename).write_bytes(content)
            entry['files'][filename] = hashlib.sha256(content).hexdigest()
    manifest.append(entry)
(DEST / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
print('Vendored verified report assets and upstream licenses.')
