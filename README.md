# OpenDownload Desktop

A free, MIT-licensed **native desktop download manager for Windows and Linux**, written in Rust. It opens its own application window and connects to a companion browser extension. No browser window or separately installed Python runtime is required for normal desktop use. This is an independent project, not Internet Download Manager or proprietary IDM source code.

## Install and open

Version 0.4 adds video page extraction and separate native progress windows. Build artifacts contain the application, browser companion, yt-dlp, Node.js, and a small FFmpeg/ffprobe build:

- **Windows 10/11 x64:** run `OpenDownload-Setup-x64.exe`. It installs for your user, adds Start-menu and desktop shortcuts, and provides an uninstaller. Alternatively, extract `OpenDownload-Windows-x64.zip` and double-click `OpenDownload.exe`.
- **Debian/Ubuntu x64:** install the `.deb` with your package manager, then launch **OpenDownload** from the application menu. For example, `sudo apt install ./OpenDownload-0.4.2-amd64.deb`.
- **Other Linux distributions:** extract `OpenDownload-Linux-x86_64.tar.gz` and double-click `opendownload`, or run the included `install.sh` for a per-user application-menu installation. The helper needs Python 3; the app itself does not. X11 or XWayland, OpenGL 2.0+, and the libraries listed in the Debian control metadata are required.

The locally built Linux archive requires glibc 2.39+ (for example Ubuntu 24.04+ or Debian 13). The CI Linux build uses Ubuntu 22.04 for broader compatibility; artifacts reflect their own build host. The Debian package declares its actual minimum glibc version. Windows installers and extension archives are unsigned; no store listing or code-signing claim is made.

Source and build workflow: [GitHub repository](https://github.com/stevensfrank405-ux/linux-idm). GitHub Actions uploads Windows and Linux packages after successful jobs. Workflow configuration alone does not establish that a remote build passed.

## Desktop controls

Paste a file URL or supported video page link (including YouTube) and choose **Add URL**. Auto mode sends known video sites, DASH/HLS manifests, and HTML responses to yt-dlp. Video mode forces extraction. File mode saves the URL as a file, including HTML when you explicitly want it. Unsupported pages fail with the extractor's error instead of silently saving HTML.

The compact manager has a toolbar, categories, and a file table. Each new download, including extension handoffs, opens a **separate native progress window** with status, transferred bytes, speed, remaining time, pause/resume, and open-file/folder controls. Double-click a row or choose **Show progress** to reopen it. Closing a progress window keeps its transfer running; closing the manager pauses unfinished jobs.

Video downloads select the best available video and audio, download fragments concurrently, and merge separate tracks using FFmpeg. Merged output uses Matroska (`.mkv`); single-stream downloads retain their source container. Playlists are limited to one item, and live broadcasts are excluded. Partial video downloads remain in a hidden per-job folder for resuming.

The native manager includes aggregate speed, status filters, pause/resume/retry, pause-all and resume-all, completed-entry removal, open-folder controls, settings, and extension pairing. Removing a completed entry keeps its downloaded file. Closing the app saves and pauses unfinished transfers; it does **not** continue in a background tray. Restart it to resume. Keyboard shortcuts: Ctrl+L for URL entry, Ctrl+B for browser pairing, and Ctrl+, for settings.

Settings let you choose 1–16 connections per file and a download folder. Connection changes apply to new/resumed transfers. Folder changes apply after restart, while existing downloads retain their original destination and partial data.

## Browser companion

Choose **Browser** in the desktop toolbar, then **Open extension folder**. Install the companion, copy the pairing code in the app, and paste it into the extension popup. Pairing is retained after restart. The app must be running for handoffs to work.

- Chromium/Chrome/Edge/Brave: enable Developer mode on the extensions page, choose **Load unpacked**, and select the bundled `chromium` folder.
- Firefox: use the bundled Firefox ZIP through `about:debugging` for temporary installation. Permanent installation requires Mozilla signing, which has not been done.

The extension supports page/link/image/audio/video context menus, **Download video from this page**, scanning up to 200 URLs in the active page, manual URL input, and optional browser-download handoff. Capture is off by default. It pauses the browser transfer, queues the URL in the desktop app, and cancels the browser copy only after acceptance. If queuing fails, it resumes the browser transfer. Tiny transfers may already be complete and stay in the browser. If the app subsequently fails to download, disable capture and retry in the browser.

See [extension installation, permissions, and limitations](extension/README.md). Some managed browsers prohibit unpacked extensions; this project does not bypass administrator policy.

## Transfer engine and local data

The engine uses Tokio, reqwest, and rustls with TLS certificate verification. Files at least 1 MiB use four parallel ranges by default if the server provides a strong ETag and exact range responses. Each segment is checked before assembly. Unsupported ranges fall back to a single stream; changed source versions restart the parts safely. Existing destination files are not overwritten.

Downloads default to the operating system's Downloads folder under `OpenDownload`. Linux state defaults to `~/.local/state/opendownload/` (respecting XDG), and Windows state uses the user's local application-data directory. Queue, settings, and the private `queue.bridge-key` live there. URLs may contain private tokens: do not share queue/state files. Keep partial files to resume. Run one app instance per queue. The local browser bridge listens only on `127.0.0.1:8765`, uses a private pairing token, and rejects unapproved mutations. A second normal instance fails visibly rather than becoming a second writer.

This is an early desktop release, not complete IDM feature parity. Scheduling, bandwidth limits, a tray/background mode, cookie-authenticated browser sessions, blob URLs, DRM, and torrents are not implemented. Video extraction depends on yt-dlp and website changes; private, age-restricted, region-blocked, or login-required videos may fail with a website-specific error. Direct links can expire or require website credentials. No IDM speed-parity benchmark has been performed.

## Build from source

Install Rust with rustup from https://rustup.rs. The pinned toolchain is in `rust-toolchain.toml`. Linux needs a C compiler and the usual X11/OpenGL runtime libraries; Windows native builds need the Visual Studio C++ build tools and Windows SDK. Windows uses DirectX 12 through wgpu, with WARP software rendering available; it does not require an OpenGL driver. Set `OPENDOWNLOAD_SOFTWARE_RENDERING=1` to explicitly select a compatible CPU adapter.

```sh
cargo build --release --locked
python3 scripts/prepare_media.py --platform linux
export OPENDOWNLOAD_TOOLS_DIR="$PWD/dist/tools/linux"
./target/release/opendownload          # Linux desktop
# Windows: target\release\opendownload.exe
```

An explicit headless mode retains the local dashboard for servers and automated tests:

```sh
./target/release/opendownload --headless --connections 4 --downloads /absolute/folder --state /absolute/state/queue.json
```

The `--port` option changes the local bridge port, but the companion currently requires 8765. A desktop display is required when `--headless` is absent. The bridge key persists; delete it while the app is stopped to generate a new key and require re-pairing.

## Validation and packaging

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
python3 scripts/prepare_media.py --platform linux
export OPENDOWNLOAD_TOOLS_DIR="$PWD/dist/tools/linux"
python3 -m unittest -v
node tests/extension_protocol.mjs
node tests/browser_smoke.mjs
xvfb-run -a env LIBGL_ALWAYS_SOFTWARE=1 python3 tests/desktop_smoke.py
python3 scripts/package_desktop.py --platform linux
```

Node tests need Node 22+; the browser smoke test needs Chromium. Native Linux UI testing needs Xvfb, xdotool, and ImageMagick if capturing screenshots. The download suite tests real transfers, parallel ranges, changed versions, fallback, persisted state, Windows-safe names, duplicate-instance protection, folder changes, and preserving user files. The native UI tests verify a separate OS progress window, closing that window without stopping the app, file bytes, graceful app shutdown, and queue restoration. The media suite runs real yt-dlp and FFmpeg against a local HTML/DASH fixture, verifies both audio/video tracks, pause/resume, unsupported-page errors, and explicit HTML file mode.

On Windows, CI builds natively with a static CRT, runs the download/extension protocol suites and `python tests/desktop_windows_smoke.py`, then packages using NSIS:

```powershell
# Run the helper build in MSYS2 MINGW64 with GCC and make available:
python scripts/prepare_media.py --platform windows
$env:OPENDOWNLOAD_TOOLS_DIR = "$PWD/dist/tools/windows"
python scripts/package_desktop.py --platform windows --installer
```

Local validation has run the native Linux GUI and download suite and cross-built the Windows GUI executable and NSIS installer. The previous 0.3 release passed its native Windows CI smoke test. Version 0.4 adds Windows popup and real media integration checks; consult its workflow run for their current result. Full unpacked-extension loading was blocked by the onboarding machine's Chromium policy; protocol logic and the real Chromium page scanner were tested. Firefox operation remains unverified. Optional actual-extension tests are in `tests/browser_extension_smoke.mjs`.

## Video helpers and verification limits

The Rust application and extension are MIT-licensed. Bundled helpers retain their own licenses: yt-dlp standalone distributions include GPLv3+ components, Node.js includes MIT and dependency notices, and the minimal unmodified FFmpeg build is LGPL 2.1-or-later with no external codecs or GPL/nonfree options enabled. License texts, the complete FFmpeg source archive, and exact build flags are included under `tools/licenses`. Helpers are independent executables and can be replaced. `scripts/prepare_media.py` pins download versions and SHA256 hashes; it builds FFmpeg from source and never installs machine-wide tools. A developer build can use `OPENDOWNLOAD_TOOLS_DIR` or helpers from PATH.

The cloud environment blocks YouTube and the official IDM installer host with HTTP 403 before requests reach those sites. Local HTML extraction and real DASH audio/video download and merging are tested; a successful live YouTube transfer and analysis of IDM's proprietary executable are **not** claimed. REA (`morluto/rea`, CLI 6.7.0, source commit `23f984b`) was used for static analysis of our extension; without the IDM executable it cannot establish IDM's internal implementation. No private IDM source or assets are included.
