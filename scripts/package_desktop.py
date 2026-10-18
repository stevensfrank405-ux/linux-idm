#!/usr/bin/env python3
"""Bundle native desktop applications and companion extensions for the target OS."""
import argparse
import platform
import shutil
import subprocess
import sys
import tarfile
from pathlib import Path
from zipfile import ZipFile, ZIP_DEFLATED

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--platform', choices=['linux', 'windows'], default='windows' if sys.platform == 'win32' else 'linux')
parser.add_argument('--binary', type=Path)
parser.add_argument('--installer', action='store_true', help='Build Windows NSIS installer (makensis must be available)')
args = parser.parse_args()
version = '0.4.2'
output = root / 'dist'
stage = output / args.platform
stage.mkdir(parents=True, exist_ok=True)
binary = args.binary or root / 'target/release' / ('opendownload.exe' if args.platform == 'windows' else 'opendownload')
if not binary.is_file():
    raise SystemExit(f'Build the native executable first: {binary}')
subprocess.run([sys.executable, str(root / 'extension/package.py')], check=True)
name = 'OpenDownload.exe' if args.platform == 'windows' else 'opendownload'
shutil.copy2(binary, stage / name)
(stage / name).chmod(0o755)
for document in ('README.md', 'LICENSE'):
    shutil.copy2(root / document, stage / document)
companion = stage / 'extensions'
chromium = companion / 'chromium'
chromium.mkdir(parents=True, exist_ok=True)
for file in ('manifest.json', 'background.js', 'popup.js', 'popup.html', 'scanner.js'):
    shutil.copy2(root / 'extension' / file, chromium / file)
shutil.copy2(root / 'extension/README.md', companion / 'README.md')
shutil.copy2(output / 'opendownload-firefox.zip', companion)
tools = output / 'tools' / args.platform
if not all((tools / name).is_file() for name in (('yt-dlp.exe', 'node.exe', 'ffmpeg.exe', 'ffprobe.exe') if args.platform == 'windows' else ('yt-dlp', 'node', 'ffmpeg', 'ffprobe'))):
    raise SystemExit('Prepare bundled video tools first: python scripts/prepare_media.py --platform ' + args.platform)
shutil.copytree(tools, stage / 'tools', dirs_exist_ok=True)
if args.platform == 'windows':
    # GNU cross builds may depend on MinGW runtime DLLs; bundle only direct dependencies.
    objdump = shutil.which('x86_64-w64-mingw32-objdump')
    if objdump:
        import os, re
        imports = subprocess.check_output([objdump, '-p', str(binary)], text=True)
        runtime_names = set(re.findall(r'DLL Name: (lib(?:winpthread|gcc_s|stdc\+\+)[^\s]+)', imports, re.I))
        for dll in runtime_names:
            candidates = list(Path(os.environ.get('MINGW_ROOT', '/usr')).rglob(dll))
            if not candidates:
                raise SystemExit(f'Required runtime DLL not found: {dll}')
            shutil.copy2(candidates[0], stage / dll)
    archive = output / 'OpenDownload-Windows-x64.zip'
    with ZipFile(archive, 'w', ZIP_DEFLATED) as bundle:
        for file in stage.rglob('*'):
            if file.is_file(): bundle.write(file, Path('OpenDownload') / file.relative_to(stage))
    if args.installer:
        executable = shutil.which('makensis') or shutil.which('makensis.exe')
        if not executable: raise SystemExit('Install NSIS and put makensis on PATH first.')
        subprocess.run([executable, str(root / 'packaging/windows/installer.nsi')], check=True)
else:
    for file in ('opendownload.desktop', 'install.sh'):
        shutil.copy2(root / 'packaging/linux' / file, stage / file)
    shutil.copy2(root / 'assets/opendownload.svg', stage)
    (stage / 'install.sh').chmod(0o755)
    for path in stage.rglob('*'):
        path.chmod(0o755 if path.is_dir() or path.name in ('opendownload', 'install.sh', 'yt-dlp', 'node', 'ffmpeg', 'ffprobe') else 0o644)
    stage.chmod(0o755)
    archive = output / f'OpenDownload-Linux-{platform.machine()}.tar.gz'
    with tarfile.open(archive, 'w:gz') as bundle:
        bundle.add(stage, arcname='OpenDownload')
    if shutil.which('dpkg-deb'):
        deb = output / 'deb-root'
        if deb.exists(): shutil.rmtree(deb)
        for directory in ('DEBIAN', 'usr/bin', 'usr/share/applications', 'usr/share/icons/hicolor/scalable/apps', 'usr/share/opendownload'):
            (deb / directory).mkdir(parents=True, exist_ok=True)
        arch = {'x86_64': 'amd64', 'aarch64': 'arm64'}[platform.machine()]
        import re
        versions = re.findall(r'GLIBC_([0-9.]+)', subprocess.check_output(['objdump', '-T', str(binary)], text=True))
        glibc = max(versions, key=lambda v: tuple(map(int, v.split('.')))) if versions else '2.35'
        (deb / 'DEBIAN/control').write_text(f'Package: opendownload\nVersion: {version}\nArchitecture: {arch}\nMaintainer: OpenDownload contributors\nSection: net\nPriority: optional\nDepends: libc6 (>= {glibc}), libgcc-s1, libstdc++6, zlib1g, libx11-6, libxcursor1, libxi6, libxrandr2, libxkbcommon0, libxkbcommon-x11-0, libgl1, libegl1\nDescription: Free native desktop download manager with browser integration\n')
        shutil.copy2(stage / name, deb / 'usr/bin/opendownload')
        shutil.copy2(stage / 'opendownload.desktop', deb / 'usr/share/applications')
        shutil.copy2(stage / 'opendownload.svg', deb / 'usr/share/icons/hicolor/scalable/apps')
        shutil.copytree(companion, deb / 'usr/share/opendownload/extensions')
        shutil.copytree(tools, deb / 'usr/lib/opendownload/tools')
        for path in deb.rglob('*'):
            path.chmod(0o755 if path.is_dir() or path == deb / 'usr/bin/opendownload' or (path.parent == deb / 'usr/lib/opendownload/tools' and path.name in ('yt-dlp', 'node', 'ffmpeg', 'ffprobe')) else 0o644)
        deb.chmod(0o755)
        subprocess.run(['dpkg-deb', '--build', '--root-owner-group', str(deb), str(output / f'OpenDownload-{version}-{arch}.deb')], check=True)
print(archive)
