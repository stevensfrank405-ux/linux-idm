# OpenDownload

A free, MIT-licensed download manager for Linux, with an asynchronous Rust engine and a local browser dashboard. Independent of Internet Download Manager; no proprietary source code is used.

## Build and run

Install Rust with rustup from https://rustup.rs, then:

```sh
cargo build --release --locked
./target/release/opendownload
```

Open `http://127.0.0.1:8765` in a browser on the same Linux machine. Paste a direct HTTP or HTTPS file link. No Python runtime is needed by the application. Python 3 is used only for integration tests.

```sh
./target/release/opendownload --port 8765 --downloads /path/to/files --state /path/to/queue.json
python3 -m unittest -v
```

Files default to `~/Downloads/OpenDownload`. Metadata defaults to `$XDG_STATE_HOME/opendownload/queue.json`, or `~/.local/state/opendownload/queue.json`. Queue metadata contains source URLs, which may include private access tokens; do not share it. Run only one instance per state file and download directory.

## Features

- Concurrent asynchronous HTTP(S) downloads using Tokio and reqwest, with certificate verification through rustls.
- Live transfer speed, progress, status filters, pause, resume, and retry.
- Persistent queue; interrupted downloads restore as paused after restart.
- Validator-protected range resume. Servers without resume support restart safely.
- Existing destination files are never overwritten.
- Local-only dashboard with host checks and a random per-process API token.

Pause takes effect after the current response chunk or network timeout (up to 30 seconds); resume once the transfer stops. Direct URLs must point to file content, not a website's download landing page. Signed URLs may expire and require a new download.

This first Rust release does not implement segmented acceleration, browser interception, scheduling, bandwidth limits, authenticated browser sessions, torrents, video extraction, or native Linux packaging. Rust alone does not guarantee faster network transfers; performance depends on the connection and source server. No IDM speed-parity benchmark has been performed.

## Validation

Build the release executable before running `python3 -m unittest -v`. Tests launch the Rust binary against a local HTTP fixture, check byte-for-byte file integrity, pause and restart persistence, servers ignoring ranges, failed downloads, URL validation, and API authorization.
