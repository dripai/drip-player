# Shadow Player

English | [简体中文](README.zh-CN.md)

Shadow Player is a Tauri 2, Vue 3, and Rust desktop media player for local course videos, audio materials, and online media links. It scans a configured media directory, downloads online media from sites such as YouTube and Bilibili, and chooses the playback path from media probing results.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Tauri](https://img.shields.io/badge/Tauri-2.x-orange.svg)](https://tauri.app/)
[![Vue](https://img.shields.io/badge/Vue-3.x-42b883.svg)](https://vuejs.org/)
[![Rust](https://img.shields.io/badge/Rust-2021-b7410e.svg)](https://www.rust-lang.org/)

![Shadow Player screenshot](shadow-player.png)

![Shadow Player usage screenshot](down.png)

## Features

- Local media library: choose and save a download directory in Settings → General to scan its media files and subfolders. Place local files in that directory and use the playlist refresh button.
- Playlist: saving a new directory replaces the entire list and stops the previous playback. The refresh button rescans the current directory for added or deleted files. An empty directory produces an empty list; scan failures preserve the previous directory and list. Existing files are neither moved nor deleted.
- Probe-first playback: use `ffprobe` to inspect the actual container and codecs before choosing a playback engine.
- Browser video playback: browser-compatible media plays through `video.js`.
- Remux cache: compatible H.264/AAC media can be remuxed losslessly into browser-friendly MP4, such as FLV to MP4.
- Audio backend: audio playback runs through the Rust backend and `rodio`, with FFmpeg processing when needed.
- External player support: videos that cannot be played through the browser or remux path can use MPV when bundled in `lib/`.
- Downloads window: analyze a URL, choose an available resolution, then start the download. The selected browser sign-in source is shared by analysis and downloading. MP3 conversion requires an explicit audio-only selection. Closing the window keeps downloads running; verified files enter the current playlist without changing playback. See [download behavior and validation](docs/downloads.md).
- Download recovery: tasks and quality choices persist in SQLite. Tasks awaiting a choice stay selectable; active tasks reopen as interrupted and resume manually using the current settings directory. An unavailable resolution requires a new choice. Changing directories stops active downloads; existing files stay in place.
- Subtitle discovery: automatically scans sibling `.srt`, `.vtt`, `.ass`, and `.ssa` files.
- Shadowing: subtitle timelines, sentence and AB repeat, masking, favorites, local recording and original-audio comparison. Bailian translation/transcription and iFlytek assessment are connected; live cloud validation requires your keys. See the [setup and verification notes](docs/learning-settings.md).
- Desktop experience: dark mode, custom title bar, playback controls, volume, playback rate, subtitles, and sidebar.
- Context menus share one theme-aware component. The page menu offers refresh and settings; playlist items support file renaming, membership removal and confirmed permanent media-file deletion. See [playlist file operations](docs/playlist-files.md).
- General settings: appearance (system, light, dark), interface language, and close to system tray, saved automatically.
- The settings window has a theme-aware custom title bar with dragging and an independent close button.
- SQLite storage for download tasks, playlists, media metadata and asset associations, appearance, language, play mode, and close-to-tray settings.
- Playback sessions use engine progress and completion; an obsolete download cannot replace a later playback request. MPV controls remain in its own window.

## Media Format Strategy

Shadow Player does not rely only on file extensions. Local files and cached files are probed first:

1. Browser-compatible containers/codecs, including H.264, HEVC and AV1 in MP4, use the built-in video player. HEVC decoding depends on the WebView and OS; a decoder failure is reported without forcing external MPV. See [Windows requirements](https://learn.microsoft.com/en-us/troubleshoot/microsoft-edge/development/video-playback-issues).
2. If the media can be remuxed losslessly into browser-compatible MP4, Shadow Player writes a local remux cache and plays that file.
3. If the file is audio-only, it is played by the Rust audio backend.
4. If the file is video that cannot use the browser path, and MPV is present in the bundled `lib/` directory, MPV is used.

Missing probe tools and probe failures are reported directly instead of selecting an engine from the extension. Downloads prefer H.264/AAC at the selected resolution and publish files directly into the configured directory; conflicting media/subtitle names receive a shared numeric suffix. See [download behavior](docs/downloads.md).

Common input formats:

- Audio: `mp3`, `wav`, `ogg`, `flac`, `m4a`, `aac`, `opus`
- Common browser video: `mp4`, `m4v`, `webm`
- Probed or external-player video: `mkv`, `avi`, `mov`, `flv`, `wmv`, `ts`, `m2ts`, `mpg`, `mpeg`, `3gp`

Release packages include the tools required for media probing and online media resolution.

## Requirements

- Node.js 24 LTS
- pnpm 11.6.0 (specified in `packageManager`)
- Rust stable toolchain, version 1.88 or later
- Tauri 2 build prerequisites for the current operating system
- Microsoft Visual Studio C++ Build Tools on Windows

See [dependency constraints and verification](docs/dependency-upgrade.md), including the WebView requirements for Tailwind 4 and the TypeScript 6 constraint.

Install pnpm:

```bash
npm install -g pnpm@11.6.0
```

## Download and Install

Download Windows and macOS installers from [GitHub Releases](https://github.com/dripai/drip-player/releases).

The app checks GitHub Releases for new versions on startup. When an update is available, the user can confirm to download and install it.

## Bundled Tools

The app uses only the `lib/` tool directory distributed with the application. During development and builds, the script downloads the tools required for the current platform into the project root `lib/`; during packaging, that directory is bundled with the app.

Windows example:

```text
drip-player/
├── lib/
│   ├── ffmpeg.exe
│   ├── ffprobe.exe
│   ├── ffplay.exe
│   └── yt-dlp.exe
```

Recommended tools:

- `ffmpeg` and `ffprobe`: media probing, duration detection, audio processing, and remux cache generation.
- `yt-dlp`: online media resolution and downloads.
- `mpv`: can be placed manually in `lib/` for video formats the browser path cannot handle.

Non-Windows systems use executable names without `.exe`. The application reads only the bundled `lib/` tool directory.

## Quick Start

Clone the repository:

```bash
git clone git@github.com:dripai/drip-player.git
cd drip-player
```

Install dependencies:

```bash
pnpm install --frozen-lockfile
```

Start the desktop app in development mode:

```bash
pnpm tauri dev
```

Build release packages:

```bash
pnpm tauri build
```

Frontend check:

```bash
pnpm build
```

Rust backend check:

```bash
cd src-tauri
cargo check --locked --all-targets
cargo test --locked --all-targets
```

## Project Structure

```text
drip-player/
├── public/                 # Frontend static assets
├── src/                    # Vue frontend
│   ├── components/          # Player, sidebar, file tree, dialogs
│   ├── store/               # Pinia player state
│   ├── utils/               # Frontend media helpers
│   ├── App.vue
│   └── main.ts
├── src-tauri/               # Rust/Tauri backend
│   ├── capabilities/         # Tauri permissions
│   ├── icons/                # App icons
│   ├── src/
│   │   ├── handlers/         # Tauri command handlers
│   │   ├── models/           # Media, assets, playlists, sessions, downloads, settings
│   │   ├── services/         # Playback, probing, remuxing, persistence, online resolution
│   │   └── main.rs
│   └── tauri.conf.json
├── package.json
├── pnpm-lock.yaml
├── README.md
└── README.zh-CN.md
```

## Runtime Data

The database is stored at `config/shadow-player.sqlite3` beside the executable. First launch starts with an empty playlist and default settings. Schema version 2 does not migrate older development databases or automatically delete their files. Removing playlist entries keeps media metadata and files. See [SQLite storage notes](docs/sqlite-storage.md) for reinitialization and [domain boundaries](docs/domain-model.md) for implementation details.

The following directories are ignored by Git:

- `lib/`: bundled tools downloaded during builds; local binary files are not committed.
- `cache/`: runtime cache.
- `config/`: SQLite database and runtime configuration.
- `downloads/`: downloaded media files.
- `doc/`: local design notes or private documentation.

The remux cache is cleaned automatically. Shadow Player removes old remux files by file age and total cache size.

## Online Media Notes

Online media features depend on `yt-dlp`. Some platforms or content may require browser cookies or a logged-in session. Shadow Player does not bypass platform restrictions; it invokes the user-configured local tools to resolve and download media.

## Contributing

Issues and pull requests are welcome.

Before submitting changes, run:

```bash
pnpm build
cd src-tauri
cargo check --locked --all-targets
cargo test --locked --all-targets
```

## Release

Before publishing a new version, configure these Tauri updater signing secrets in GitHub Actions:

- `TAURI_SIGNING_PRIVATE_KEY`: the generated updater private key content.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: the private key password.

Pushing a `v*.*.*` tag triggers GitHub Actions to build Windows and macOS packages and generate the signed updater artifacts.

## License

This project is licensed under the [MIT License](LICENSE).

## Disclaimer

Shadow Player does not provide, host, or distribute any media content. Users are responsible for ensuring that local files, online playback, and downloads comply with applicable laws, copyright rules, and the terms of service of the target platforms.
