# OpenDownload

A free, MIT-licensed download manager for Linux, with an asynchronous Rust engine, parallel file transfers, and a browser companion. Independent of Internet Download Manager; no proprietary source code is used.

## Build and run

Install Rust with rustup from https://rustup.rs, then:

```sh
cargo build --release --locked
./target/release/opendownload
```

Open `http://127.0.0.1:8765` in a browser on the same Linux machine. Paste a direct HTTP or HTTPS URL, or install the [browser companion](extension/README.md). Any file type is accepted: documents, archives, installers, images, audio, video, and other HTTP resources. Website page URLs download HTML rather than automatically finding a hidden file.

```sh
./target/release/opendownload --connections 8 --port 8765 --downloads /path/to/files --state /path/to/queue.json
```

`--connections` accepts 1–16 and defaults to 4. Set it to 1 for a single stream or servers with strict connection limits. The browser companion currently connects to the default port 8765 only.

Files default to `~/Downloads/OpenDownload`. Metadata defaults to `$XDG_STATE_HOME/opendownload/queue.json`, or `~/.local/state/opendownload/queue.json`. Metadata contains source URLs, which may include private access tokens; do not share it. Run only one instance per state file and download directory. Python and Node are used for tests and packaging, not by the application itself.

## Parallel transfers and recovery

Files of at least 1 MiB use multiple simultaneous byte-range requests when the server provides a strong ETag and valid range responses. Every range and validator is checked before the parts are assembled. Servers without these capabilities use one stream. Rejected or malformed segment ranges trigger a safe single-stream fallback; network failures remain visible for retry.

Each segment has its own partial file. Pause, retry, and application restart preserve completed bytes. Resume probes the source again: a changed ETag, length, or connection count restarts all segments to avoid combining different file versions. Existing destination files are never overwritten. Completed segment files are removed after assembly. Keep incomplete `.part` and `.segment-*` files if you want to resume.

The dashboard shows aggregate progress, speed, and the active connection count. Pause takes effect after the current chunk or network timeout (up to 30 seconds); wait for workers to stop before resuming. Rust and segmentation do not guarantee IDM speed parity; source limits and connection quality determine throughput, and no IDM benchmark has been performed.

## Browser integration

The companion supports Chromium/Chrome/Brave/Edge and provides a Firefox manifest:

- Right-click a link, image, audio, or video and choose **Download with OpenDownload**.
- Scan the current page for direct links and media URLs, then choose a resource.
- Paste a file URL into the extension popup.
- Optionally hand off new browser downloads. This is off by default. The extension pauses the browser transfer, queues its URL in the app, then cancels the browser copy. If queuing fails, it resumes the browser transfer. Tiny downloads may finish before they can be paused and remain in the browser.

Pair through the dashboard's **Browser pairing** button. Tokens are private and change when the app restarts. The extension talks only to the loopback app; it has no remote service. See [installation and permissions](extension/README.md).

This does not mean every object on every site is downloadable. Blob/data URLs, DRM, protected streaming, browser-only login sessions, HLS/DASH assembly, torrents, and video extraction are not supported. Cookies and browser authentication are not forwarded. Expired signed links need to be refreshed. An accepted handoff can still fail later at the source; check the app's queue and retry in the browser with capture disabled when necessary. Cross-origin frame resources and URLs absent from the page DOM are not discovered by scanning.

## Validate and package

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
python3 -m unittest -v
node tests/extension_protocol.mjs
node tests/browser_smoke.mjs       # requires Node 22+ and Chromium
python3 extension/package.py
python3 scripts/package_linux.py
```

The Python integration suite launches the real Rust executable and verifies concurrent ranges, content integrity, resumed offsets, changed-file detection, malformed/ignored ranges, weak validators, single-stream mode, errors, and file preservation. Extension protocol tests use mocked browser APIs against the real Rust app. The Chromium smoke test verifies the actual dashboard, responsive layout, and DOM scanner.

`node tests/browser_extension_smoke.mjs` additionally tests the actual unpacked Chromium extension where administrator policy permits it. The onboarding machine prohibits unpacked extensions, so full extension loading and Firefox operation have **not** been validated here. No tests bypass that administrator policy.

Archives are written to `dist/`. Browser archives are unsigned; Firefox requires signing for permanent installation. The Linux tarball is built for the current host architecture and libc, not every Linux distribution. GitHub CI builds the Linux binary and extension archives and uploads them as workflow artifacts; browser-store publication and a public release remain separate steps.
