#!/usr/bin/env python3
"""Package the already-built host binary without modifying the checkout."""
import platform
import subprocess
import sys
import tarfile
from pathlib import Path

root = Path(__file__).resolve().parents[1]
binary = root / 'target/release/opendownload'
if not binary.is_file():
    raise SystemExit('Run cargo build --release --locked first.')
subprocess.run([sys.executable, str(root / 'extension/package.py')], check=True)
output = root / 'dist'
archive_path = output / f'opendownload-linux-{platform.machine()}.tar.gz'
with tarfile.open(archive_path, 'w:gz') as archive:
    archive.add(binary, arcname='opendownload/opendownload')
    for name in ('README.md', 'LICENSE'):
        archive.add(root / name, arcname=f'opendownload/{name}')
    archive.add(root / 'extension/README.md', arcname='opendownload/extension/README.md')
    for browser in ('chromium', 'firefox'):
        name = f'opendownload-{browser}.zip'
        archive.add(output / name, arcname=f'opendownload/extension/{name}')
print(archive_path)
