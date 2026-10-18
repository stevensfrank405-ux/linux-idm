#!/usr/bin/env python3
"""Create unsigned browser extension archives; signing/store publishing is separate."""
import json
from pathlib import Path
from zipfile import ZipFile, ZIP_DEFLATED

root = Path(__file__).resolve().parent
output = root.parent / 'dist'
output.mkdir(exist_ok=True)
for browser in ('chromium', 'firefox'):
    manifest = root / ('manifest.json' if browser == 'chromium' else 'manifest.firefox.json')
    json.loads(manifest.read_text())
    with ZipFile(output / f'opendownload-{browser}.zip', 'w', ZIP_DEFLATED) as archive:
        archive.writestr('manifest.json', manifest.read_bytes())
        for name in ('background.js', 'popup.js', 'popup.html', 'scanner.js'):
            archive.write(root / name, name)
    print(output / f'opendownload-{browser}.zip')
