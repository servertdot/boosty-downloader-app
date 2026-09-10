<p align="center">
  <img src="public/app-icon.png" alt="Boosty Loader" width="160" />
</p>

<h1 align="center">Boosty Loader</h1>

<p align="center">
  A polished desktop UI for downloading content from Boosty.<br />
  Built on top of the
  <a href="https://github.com/Glitchy-Sheep/boosty-downloader/tree/main"><strong>boosty-downloader</strong></a>
  CLI — it still does all the heavy lifting.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Tauri-2-orange?style=flat-square" alt="Tauri 2" />
  <img src="https://img.shields.io/badge/React-19-61dafb?style=flat-square" alt="React 19" />
  <img src="https://img.shields.io/badge/Python-3.10%2B-3776ab?style=flat-square" alt="Python 3.10+" />
  <img src="https://img.shields.io/badge/powered%20by-boosty--downloader-brightgreen?style=flat-square" alt="powered by boosty-downloader" />
</p>

---

## Why this exists

The [boosty-downloader](https://github.com/Glitchy-Sheep/boosty-downloader/tree/main) CLI already downloads posts, videos, and files. **Boosty Loader** wraps it in a familiar window: settings, content filters, a live log, and a Stop button — no hand-editing configs in the terminal.

Made possible by the open-source project  
→ [Glitchy-Sheep/boosty-downloader](https://github.com/Glitchy-Sheep/boosty-downloader/tree/main)

## Features

- Save **Authorization** and **Cookie** locally
- Grab credentials via the built-in helper script
- Download **all posts** from a creator or a **single post** by URL
- Filters: posts, Boosty videos, external videos, files, audio
- Pick video quality and request delay
- Live log, stop mid-run, and resume sync later
- Isolated Python environment — your system packages stay untouched

## Download (macOS)

Grab a DMG from the [latest release](https://github.com/servertdot/boosty-downloader-app/releases/latest):

- **Apple Silicon** (M1/M2/M3/M4): `Boosty-Loader_*_aarch64.dmg`
- **Intel**: `Boosty-Loader_*_x64.dmg`

Open the DMG, drag **Boosty Loader** into Applications, then launch it.

The build is ad-hoc signed (not Apple-notarized). On first launch, use **right-click → Open** and confirm in the dialog. If macOS still says the app is damaged:

```bash
xattr -cr "/Applications/Boosty Loader.app"
```

Then open it again. On first launch inside the app, click **Install** to set up the isolated Python environment.

## Quick start (from source)

You need **Rust**, **Bun**, and **Python 3.10+**.

```bash
bun install
bun run tauri dev
```

On first launch, click **Install** at the top of the window. The app creates its own Python environment in the app data directory, installs `boosty-downloader` and an up-to-date `certifi` for HTTPS. System Python is left alone.

## Build

```bash
bun run tauri build
```

## Data & security

| | |
| --- | --- |
| Settings location | Standard app data directory (`config.yaml`) |
| Credential file mode (Unix) | `0600` |
| Networking | Credentials go only to the local `boosty-downloader`; there is no remote server |

Only download content you have lawful access to. [boosty-downloader](https://github.com/Glitchy-Sheep/boosty-downloader/tree/main) itself is MIT-licensed.

## Acknowledgments

Huge thanks to the authors and contributors of **[boosty-downloader](https://github.com/Glitchy-Sheep/boosty-downloader/tree/main)** — without that CLI, Boosty Loader wouldn’t exist.
