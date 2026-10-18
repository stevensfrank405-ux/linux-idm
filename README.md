# OpenDownload Desktop

A free, MIT-licensed **native desktop download manager for Windows and Linux**, written in Rust. It opens its own application window and connects to a companion browser extension. No browser window or Python runtime is required for normal desktop use. This is an independent project, not Internet Download Manager or proprietary IDM source code.

## Install and open

Build artifacts contain the application and browser companion:

- **Windows 10/11 x64:** run `OpenDownload-Setup-x64.exe`. It installs for your user, adds Start-menu and desktop shortcuts, and provides an uninstaller. Alternatively, extract `OpenDownload-Windows-x64.zip` and double-click `OpenDownload.exe`.
- **Debian/Ubuntu x64:** install the `.deb` with your package manager, then launch **OpenDownload** from the application menu. For example, `sudo apt install ./OpenDownload-0.3.0-amd64.deb`.
- **Other Linux distributions:** extract `OpenDownload-Linux-x86_64.tar.gz` and double-click `opendownload`, or run the included `install.sh` for a per-user application-menu installation. The helper needs Python 3; the app itself does not. X11 or XWayland, OpenGL, and the libraries listed in the Debian control metadata are required.

The locally built Linux archive requires glibc 2.39+ (for example Ubuntu 24.04+ or Debian 13). The CI Linux build uses Ubuntu 22.04 for broader compatibility; artifacts reflect their own build host. The Debian package declares its actual minimum glibc version. Windows installers and extension archives are unsigned; no store listing or code-signing claim is made.

Source and build workflow: [GitHub repository](https://github.com/stevensfrank405-ux/linux-idm). GitHub Actions uploads Windows and Linux packages after successful jobs. Workflow configuration alone does not establish that a remote build passed.

## Desktop controls

Paste a direct HTTP(S) file URL and choose **Add download**, or send a link from the browser companion. Documents, archives, installers, images, audio, video, and other HTTP file types are accepted. A website page URL downloads HTML, rather than extracting a hidden video or file.

The native window includes progress, aggregate speed, status filters, pause/resume/retry, pause-all and resume-all, completed-entry removal, open-folder controls, settings, and extension pairing. Removing a completed entry keeps its downloaded file. Closing the app saves and pauses unfinished transfers; it does **not** continue in a background tray. Restart it to resume. Keyboard shortcuts: Ctrl+L for URL entry, Ctrl+B for browser pairing, and Ctrl+, for settings.

Settings let you choose 1–16 connections per file and a download folder. Connection changes apply to new/resumed transfers. Folder changes apply after restart, while existing downloads retain their original destination and partial data.

## Browser companion

Open **Browser companion** in the desktop app, then **Open extension folder**. Install the companion, copy the pairing code in the app, and paste it into the extension popup. Pairing is retained after restart. The app must be running for handoffs to work.

- Chromium/Chrome/Edge/Brave: enable Developer mode on the extensions page, choose **Load unpacked**, and select the bundled `chromium` folder.
- Firefox: use the bundled Firefox ZIP through `about:debugging` for temporary installation. Permanent installation requires Mozilla signing, which has not been done.

The extension supports link/image/audio/video context menus, scanning up to 200 direct URLs in the active page, manual URL input, and optional browser-download handoff. Capture is off by default. It pauses the browser transfer, queues the URL in the desktop app, and cancels the browser copy only after acceptance. If queuing fails, it resumes the browser transfer. Tiny transfers may already be complete and stay in the browser. If the app subsequently fails to download, disable capture and retry in the browser.

See [extension installation, permissions, and limitations](extension/README.md). Some managed browsers prohibit unpacked extensions; this project does not bypass administrator policy.

## Transfer engine and local data

The engine uses Tokio, reqwest, and rustls with TLS certificate verification. Files at least 1 MiB use four parallel ranges by default if the server provides a strong ETag and exact range responses. Each segment is checked before assembly. Unsupported ranges fall back to a single stream; changed source versions restart the parts safely. Existing destination files are not overwritten.

Downloads default to the operating system's Downloads folder under `OpenDownload`. Linux state defaults to `~/.local/state/opendownload/` (respecting XDG), and Windows state uses the user's local application-data directory. Queue, settings, and the private `queue.bridge-key` live there. URLs may contain private tokens: do not share queue/state files. Keep partial files to resume. Run one app instance per queue. The local browser bridge listens only on `127.0.0.1:8765`, uses a private pairing token, and rejects unapproved mutations. A second normal instance fails visibly rather than becoming a second writer.

This is an early desktop release, not complete IDM feature parity. Scheduling, bandwidth limits, a tray/background mode, cookie-authenticated browser sessions, blob URLs, DRM, HLS/DASH assembly, torrents, and video extraction are not implemented. Direct links can expire or require website credentials. No IDM speed-parity benchmark has been performed.

## Build from source

Install Rust with rustup from https://rustup.rs. The pinned toolchain is in `rust-toolchain.toml`. Linux needs a C compiler and the usual X11/OpenGL runtime libraries; Windows native builds need the Visual Studio C++ build tools and Windows SDK.

```sh
cargo build --release --locked
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
python3 -m unittest -v
node tests/extension_protocol.mjs
node tests/browser_smoke.mjs
xvfb-run -a env LIBGL_ALWAYS_SOFTWARE=1 python3 tests/desktop_smoke.py
python3 scripts/package_desktop.py --platform linux
```

Node tests need Node 22+; the browser smoke test needs Chromium. Native Linux UI testing needs Xvfb, xdotool, and ImageMagick if capturing screenshots. The download suite tests real transfers, parallel ranges, changed versions, fallback, persisted state, Windows-safe names, duplicate-instance protection, folder changes, and preserving user files. The native UI test enters a URL, verifies file bytes, opens dialogs, closes normally, and restores the queue and pairing on restart.

On Windows, CI builds natively with a static CRT, runs the download/extension protocol suites and `python tests/desktop_windows_smoke.py`, then packages using NSIS:

```powershell
python scripts/package_desktop.py --platform windows --installer
```

Local validation has run the native Linux GUI and download suite and cross-built the Windows GUI executable and NSIS installer. Native Windows execution remains for the Windows CI job or a Windows machine; it is not claimed as verified here. Full unpacked-extension loading was blocked by the onboarding machine's Chromium policy; protocol logic and the real Chromium page scanner were tested. Firefox operation remains unverified. Optional actual-extension tests are in `tests/browser_extension_smoke.mjs`.
