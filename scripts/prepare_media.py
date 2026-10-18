#!/usr/bin/env python3
"""Fetch pinned video helpers, verify SHA256, and build a small LGPL FFmpeg.

No machine-wide installation. Source, licenses and exact build flags ship with
our FFmpeg executables so recipients can rebuild or replace them.
"""
import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import urllib.request
from zipfile import ZipFile

ROOT = Path(__file__).resolve().parents[1]
YT_VERSION = '2026.08.19'
NODE_VERSION = '22.23.3'
FF_VERSION = '8.1.3'
ASSETS = {
    'linux': [('https://github.com/yt-dlp/yt-dlp/releases/download/'+YT_VERSION+'/yt-dlp_linux', '58162f9bfdc27458ea47bfcb311cf47028f17d8154a8bf7d689861d46399230a', 'yt-dlp'),
              ('https://nodejs.org/dist/v'+NODE_VERSION+'/node-v'+NODE_VERSION+'-linux-x64.tar.xz', 'df450af89261115ef9f9e3830c3eeb2cc9213b63c720b1af623cb5dcbe2e02de', 'node')],
    'windows': [('https://github.com/yt-dlp/yt-dlp/releases/download/'+YT_VERSION+'/yt-dlp.exe', '66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a', 'yt-dlp.exe'),
                ('https://nodejs.org/dist/v'+NODE_VERSION+'/node-v'+NODE_VERSION+'-win-x64.zip', '2b0ff57b049cda1bbcea2240eec20467018713c1efe1f7360c2681859b90ed71', 'node.exe')],
}
FF_URL = 'https://github.com/FFmpeg/FFmpeg/archive/refs/tags/n'+FF_VERSION+'.tar.gz'
FF_SHA = '09f990289327d3ebfedf4fc15cae3f884a65dab7a5aac7bbb5b01ff83900efd9'
FLAGS = ['--disable-everything', '--disable-autodetect', '--disable-doc', '--disable-debug', '--disable-network', '--disable-x86asm', '--disable-shared', '--enable-static', '--disable-ffplay', '--enable-ffmpeg', '--enable-ffprobe', '--disable-avdevice', '--disable-swscale', '--disable-swresample', '--enable-protocol=file,pipe', '--enable-demuxer=mov,matroska,aac,mp3,ogg,mpegts,concat,flv', '--enable-muxer=mp4,matroska,webm,ogg,mov,mp3,adts', '--enable-parser=aac,h264,hevc,av1,vp8,vp9,opus,vorbis', '--enable-bsf=aac_adtstoasc,h264_mp4toannexb,hevc_mp4toannexb,av1_frame_merge']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--platform', choices=ASSETS, default='windows' if os.name=='nt' else 'linux')
    parser.add_argument('--cross-prefix', help='MinGW cross compiler prefix, for Linux-hosted Windows packaging')
    args = parser.parse_args()
    target = ROOT/'dist/tools'/args.platform
    cache = ROOT/'dist/tool-cache'
    target.mkdir(parents=True, exist_ok=True)
    cache.mkdir(parents=True, exist_ok=True)
    licenses = target/'licenses'
    licenses.mkdir(exist_ok=True)

    def fetch(url, checksum):
        path = cache/url.rsplit('/',1)[1]
        if not path.exists() or hashlib.sha256(path.read_bytes()).hexdigest()!=checksum:
            temporary = path.with_suffix(path.suffix+'.download')
            print('Fetching', url, flush=True)
            with urllib.request.urlopen(url, timeout=120) as source, temporary.open('wb') as output:
                shutil.copyfileobj(source, output)
            if hashlib.sha256(temporary.read_bytes()).hexdigest()!=checksum:
                temporary.unlink()
                raise SystemExit('SHA256 mismatch: '+url)
            temporary.replace(path)
        return path

    for url, checksum, name in ASSETS[args.platform]:
        path = fetch(url, checksum)
        if name.startswith('node'):
            if args.platform=='windows':
                with ZipFile(path) as archive:
                    member = next(n for n in archive.namelist() if n.endswith('/node.exe'))
                    (target/name).write_bytes(archive.read(member))
                    notice = next(n for n in archive.namelist() if n.endswith('/LICENSE'))
                    (licenses/'Node-LICENSE').write_bytes(archive.read(notice))
            else:
                with tarfile.open(path) as archive:
                    member = next(n for n in archive.getmembers() if n.name.endswith('/bin/node'))
                    (target/name).write_bytes(archive.extractfile(member).read())
                    notice = next(n for n in archive.getmembers() if n.name.endswith('/LICENSE'))
                    (licenses/'Node-LICENSE').write_bytes(archive.extractfile(notice).read())
        else:
            temporary = target/(name+'.new')
            shutil.copy2(path, temporary)
            temporary.replace(target/name)
        (target/name).chmod(0o755)
    notice = fetch('https://raw.githubusercontent.com/yt-dlp/yt-dlp/'+YT_VERSION+'/THIRD_PARTY_LICENSES.txt', '472aefe951c7db35e1657c1d13fd337140511ed6f2b329205105ad441c5a02b7')
    shutil.copy2(notice, licenses/'yt-dlp-THIRD_PARTY_LICENSES.txt')
    # yt-dlp's standalone distributions include their third-party notices and ejs.
    (licenses/'yt-dlp-NOTICE.txt').write_text('yt-dlp '+YT_VERSION+' standalone distribution. Includes Python, yt-dlp-ejs and dependencies.\nProject and corresponding source: https://github.com/yt-dlp/yt-dlp/tree/'+YT_VERSION+'\nLicense and bundled dependency notices are included in yt-dlp-THIRD_PARTY_LICENSES.txt.\n')
    source = fetch(FF_URL, FF_SHA)
    build = cache/('ffmpeg-'+args.platform)
    stamp = build/'opendownload-flags.txt'
    flags = FLAGS.copy()
    suffix = '.exe' if args.platform=='windows' else ''
    if args.platform=='windows':
        flags += ['--target-os=mingw32', '--arch=x86_64', '--extra-ldflags=-static']
        if args.cross_prefix:
            flags += ['--cross-prefix='+args.cross_prefix, '--enable-cross-compile']
            if shutil.which(args.cross_prefix+'gcc-posix'):
                flags += ['--cc='+args.cross_prefix+'gcc-posix']
    signature = '\n'.join([FF_SHA,*flags])
    if not stamp.exists() or stamp.read_text()!=signature:
        if build.exists(): shutil.rmtree(build)
        build.mkdir()
        with tarfile.open(source) as archive:
            for member in archive.getmembers():
                member.name = '/'.join(member.name.split('/')[1:])
                if member.name: archive.extract(member, build, filter='data')
        # Configure uses a POSIX shell; on Windows run this script in MSYS2 MINGW64.
        subprocess.run(['bash','./configure',*flags], cwd=build, check=True)
        stamp.write_text(signature)
    subprocess.run(['make','-j3'], cwd=build, check=True)
    for name in ('ffmpeg','ffprobe'):
        shutil.copy2(build/(name+suffix),target/(name+suffix))
        (target/(name+suffix)).chmod(0o755)
    shutil.copy2(source, licenses/f'FFmpeg-{FF_VERSION}-source.tar.gz')
    for name in ('COPYING.LGPLv2.1','COPYING.LGPLv3','LICENSE.md'):
        shutil.copy2(build/name, licenses/('FFmpeg-'+name))
    (licenses/'FFmpeg-build.txt').write_text('FFmpeg '+FF_VERSION+'\nSource: '+FF_URL+'\nSHA256: '+FF_SHA+'\nConfigure: ./configure '+' '.join(flags)+'\nBuild: make -j3\nUnmodified FFmpeg source; no external codec libraries, GPL or nonfree components enabled.\nThese tools can be replaced independently of OpenDownload.\n')
    print('Video helpers ready:', target)

if __name__=='__main__': main()
