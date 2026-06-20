# Drip Player

English | [简体中文](README.zh-CN.md)

Drip Player is a Tauri 2, Vue 3, and Rust desktop media player for local course videos, audio materials, and online media links. It can import files or folders, resolve online media from sites such as YouTube and Bilibili, and choose the playback path from media probing results.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Tauri](https://img.shields.io/badge/Tauri-2.x-orange.svg)](https://tauri.app/)
[![Vue](https://img.shields.io/badge/Vue-3.x-42b883.svg)](https://vuejs.org/)
[![Rust](https://img.shields.io/badge/Rust-2021-b7410e.svg)](https://www.rust-lang.org/)

![Drip Player screenshot](drip-player.png)

![Drip Player usage screenshot](down.png)

## Features

- Local media library: import a single file or recursively import an entire folder.
- Playlist and folder tree: browse imported media by folder and double-click to play.
- Probe-first playback: use `ffprobe` to inspect the actual container and codecs before choosing a playback engine.
- Browser video playback: browser-compatible media plays through `video.js`.
- Remux cache: compatible H.264/AAC media can be remuxed losslessly into browser-friendly MP4, such as FLV to MP4.
- Audio backend: audio playback runs through the Rust backend and `rodio`, with FFmpeg processing when needed.
- External player support: videos that cannot be played through the browser or remux path can use MPV when bundled in `lib/`.
- Online media: resolve and download online videos through `yt-dlp`.
- Subtitle discovery: automatically scans sibling `.srt`, `.vtt`, `.ass`, and `.ssa` files.
- Desktop experience: dark mode, custom title bar, playback controls, volume, playback rate, subtitles, and sidebar.

## Media Format Strategy

Drip Player does not rely only on file extensions. Local files and cached files are probed first:

1. If the container and codecs are browser-compatible, the built-in video player is used directly.
2. If the media can be remuxed losslessly into browser-compatible MP4, Drip Player writes a local remux cache and plays that file.
3. If the file is audio-only, it is played by the Rust audio backend.
4. If the file is video that cannot use the browser path, and MPV is present in the bundled `lib/` directory, MPV is used.

Common input formats:

- Audio: `mp3`, `wav`, `ogg`, `flac`, `m4a`, `aac`, `opus`
- Common browser video: `mp4`, `m4v`, `webm`
- Probed or external-player video: `mkv`, `avi`, `mov`, `flv`, `wmv`, `ts`, `m2ts`, `mpg`, `mpeg`, `3gp`

Release packages include the tools required for media probing and online media resolution.

## Requirements

- Node.js 18+
- pnpm
- Rust stable toolchain
- Tauri 2 build prerequisites for the current operating system
- Microsoft Visual Studio C++ Build Tools on Windows

Install pnpm:

```bash
npm install -g pnpm
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
pnpm install
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
cargo check
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
│   │   ├── models/           # Playlist and player models
│   │   ├── services/         # Playback, probing, remuxing, persistence, online resolution
│   │   └── main.rs
│   └── tauri.conf.json
├── package.json
├── pnpm-lock.yaml
├── README.md
└── README.zh-CN.md
```

## Runtime Data

The following directories are ignored by Git:

- `lib/`: bundled tools downloaded during builds; local binary files are not committed.
- `cache/`: runtime cache.
- `downloads/`: downloaded media files.
- `doc/`: local design notes or private documentation.

The remux cache is cleaned automatically. Drip Player removes old remux files by file age and total cache size.

## Online Media Notes

Online media features depend on `yt-dlp`. Some platforms or content may require browser cookies or a logged-in session. Drip Player does not bypass platform restrictions; it invokes the user-configured local tools to resolve and download media.

## Contributing

Issues and pull requests are welcome.

Before submitting changes, run:

```bash
pnpm build
cd src-tauri
cargo check
```

## Release

Before publishing a new version, configure these Tauri updater signing secrets in GitHub Actions:

- `TAURI_SIGNING_PRIVATE_KEY`: the generated updater private key content.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: the private key password.

Pushing a `v*.*.*` tag triggers GitHub Actions to build Windows and macOS packages and generate the signed updater artifacts.

## License

This project is licensed under the [MIT License](LICENSE).

## Disclaimer

Drip Player does not provide, host, or distribute any media content. Users are responsible for ensuring that local files, online playback, and downloads comply with applicable laws, copyright rules, and the terms of service of the target platforms.
