# 依赖升级说明

2026-09-10：直接依赖已升级到当前稳定版本，TypeScript 保留在 Vue 类型检查所需的 6.x。完整版本以 `package.json`、`src-tauri/Cargo.toml` 和两份锁文件为准。

## 当前选择与约束

| 组件 | 当前版本与接入方式 |
| --- | --- |
| Tauri | Rust 2.11.5 / JS API 2.11.1；两端保持 2.11，dialog、updater、process 插件两端版本一致 |
| Vue 工具链 | Vue 3.5.42、Pinia 4.0.3、vue-i18n 11.4.10、VueUse 14.4.0、vue-tsc 3.3.11 |
| 构建与样式 | Vite 8.2.2、Tailwind 4.3.3，使用 `@tailwindcss/vite`；主题由 `.dark` 类控制 |
| 视频 | Video.js 8.24.0，由 Vue 组件直接创建和销毁播放器 |
| 音频 | Rodio 0.22.2，使用 `Player::get_pos()` 和浮点 PCM |
| HTTP | Reqwest 0.13.5 / Warp 0.4.3，使用 `warp::reply::stream` 转发响应体和 Range 相关响应头 |
| 数据库 | rusqlite 0.40.2；仍使用随包 SQLite，数据库结构不因本次依赖升级改变 |

- 开发与 CI 使用 Node.js 24 LTS、pnpm 11.6.0；Rust 最低声明为 1.88，本机使用 1.96.0。安装时使用 `pnpm install --frozen-lockfile`。
- CI 的 `pnpm/action-setup@v4` 使用 `standalone: true` 安装带 Node 的 pnpm，避免其 Node 20 动作运行时与 pnpm 11 的 Node 22.13+ 要求冲突；应用构建使用单独配置的 Node 24。[动作官方参数](https://github.com/pnpm/action-setup/blob/v4/action.yml)
- TypeScript 使用 6.0.3。TS 7.0.2 没有 Vue 类型检查所需的程序化 JS API；Vue 工具链仍依赖 6.x API。本项目只有一条 `vue-tsc` 检查路径，暂不引入双编译器。[官方说明](https://github.com/vuejs/language-tools/pull/6123)
- Tailwind 4 要求 Safari 16.4+、Chrome 111+ 或 Firefox 128+。桌面 WebView 也必须具备对应 CSS 能力；旧系统 WebView 不在本次验证范围内。[官方升级指南](https://tailwindcss.com/docs/upgrade-guide)
- Reqwest 显式保留 `native-tls`、HTTP/2、流式响应和系统代理功能；0.13 默认 TLS 已改变，因此未直接使用新版默认功能集。[官方变更](https://github.com/seanmonstar/reqwest/releases/tag/v0.13.0)
- Tauri 核心两端按次版本同步，插件按完整版本同步；补齐 `tauri-plugin-process` 注册，供更新后重启调用。[版本同步规则](https://v2.tauri.app/develop/updating-dependencies/#sync-npm-packages-and-cargo-crates-versions)、[Process 插件](https://v2.tauri.app/plugin/process/)

已移除 `@videojs-player/vue`、弃用的 `lucide-vue-next`（改用 `@lucide/vue`）、未使用的 FS/Shell 直接依赖与权限、旧 PostCSS/Autoprefixer 配置及未使用的 Rust 直接依赖。FS 仍作为 dialog 的间接依赖存在。`cargo machete` 未发现未使用的直接依赖。

## 验证结果

Windows 本机已通过：

- 锁文件安装、前端类型检查与生产构建。
- Rust 全目标编译和 Clippy（`-D warnings`）。
- 4 项默认 Rust 测试，包括二进制 HTTP 206 / Range 转发和上游断开报错。
- 单独运行的静音设备测试：WAV 与 FFmpeg 解码的 Opus，覆盖播放进度、暂停、保留暂停状态的定位和自然结束。测试使用临时文件和当前音频输出设备，不代表听感或所有设备验证。
- Edge 浏览器检查：视频播放、暂停、定位、结束、VTT 字幕实际显示、旧播放器释放、明暗主题和语言切换。使用 Tauri 官方 IPC mock，不代表原生窗口、更新安装和跨窗口事件已通过实测。

常规检查：

```sh
pnpm build
cargo check --locked --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path src-tauri/Cargo.toml --all-targets
```

音频设备测试默认忽略；本机准备好 `lib/ffmpeg`、`lib/ffprobe` 和输出设备后可单独运行：

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml muted_audio_play_pause_seek_and_complete -- --ignored --nocapture
```

远端 CI、macOS、签名发布包、在线下载、数据库与跨窗口同步尚未进行本次运行验证。前端构建仍提示主 JS 包超过 500 kB；当前约 916 kB（gzip 约 280 kB），后续可按实际加载表现决定是否拆分。
