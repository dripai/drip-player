# Drip Player

一个基于 Tauri v2 + Vue 3 构建的现代化本地与在线媒体播放器。

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Tauri](https://img.shields.io/badge/Tauri-v2-orange.svg)
![Vue](https://img.shields.io/badge/Vue-3-green.svg)
![Rust](https://img.shields.io/badge/Rust-Edition%202021-brown.svg)

## ✨ 主要特性

*   **混合播放引擎**: 结合 Rust 后端 (`rodio` + `ffmpeg`) 与前端 (`video.js`)，支持多种音频和视频格式。
*   **本地媒体管理**: 支持添加本地文件和文件夹，支持递归扫描，支持拖拽。
*   **在线媒体支持**:
    *   支持解析和播放网络 URL。
    *   集成 `yt-dlp` 支持 YouTube、Bilibili 等主流平台的视频解析与下载（需配置依赖）。
    *   自动读取浏览器 Cookie 以支持会员画质或受限内容。
*   **现代化 UI**:
    *   基于 Tailwind CSS 的精美界面。
    *   支持浅色/深色模式切换。
    *   自定义标题栏与流畅的窗口控制。
    *   紧凑的侧边栏播放列表与文件夹树视图。
*   **稳定可靠**:
    *   网络播放超时自动检测（5秒超时）。
    *   全局错误提示系统。
    *   Rust 提供的线程安全状态管理。

## 🛠️ 技术栈

*   **Frontend**: Vue 3, TypeScript, Tailwind CSS, Pinia, Vue Router, Lucide Icons.
*   **Backend**: Rust, Tauri v2, Rodio, Tokio, FFmpeg (Command line wrapper).
*   **Tools**: Vite, pnpm.

## ⚙️ 环境要求

在运行或编译本项目之前，请确保您的环境满足以下要求：

1.  **Node.js**: 建议 v18+
2.  **Rust**: 最新稳定版 (`rustup update`)
3.  **构建工具**: `pnpm` (`npm install -g pnpm`)
4.  **Microsoft Visual Studio C++ Build Tools** (Windows 用户)

### 外部依赖 (可选但推荐)

为了获得完整的在线播放和下载体验，建议在项目根目录下创建 `lib` 文件夹，并放入以下可执行文件：

*   **ffmpeg.exe**: 用于流媒体处理和格式转换。[下载 FFmpeg](https://ffmpeg.org/download.html)
*   **yt-dlp.exe**: 用于在线视频解析和下载。[下载 yt-dlp](https://github.com/yt-dlp/yt-dlp/releases)

> 目录结构示例:
> ```
> drip-player/
> ├── lib/
> │   ├── ffmpeg.exe
> │   └── yt-dlp.exe
> ├── src/
> ├── src-tauri/
> ...
> ```

## 🚀 快速开始

### 1. 克隆项目

```bash
git clone https://github.com/your-username/drip-player.git
cd drip-player
```

### 2. 安装依赖

```bash
pnpm install
```

### 3. 开发模式运行

```bash
pnpm tauri dev
```
这将同时启动前端 Vite 服务器和 Tauri 应用程序窗口。

## 📦 构建发布

构建生产环境版本（生成 `.exe` 或 `.msi` 安装包）：

```bash
pnpm tauri build
```
构建产物通常位于 `src-tauri/target/release/bundle/` 目录下。

## 📂 项目结构

```
drip-player/
├── doc/                 # 项目文档与设计说明
├── lib/                 # 外部依赖库 (ffmpeg, yt-dlp)
├── src/                 # 前端源代码 (Vue)
│   ├── components/      # Vue 组件
│   ├── store/           # Pinia 状态管理
│   ├── App.vue          # 根组件
│   └── main.ts          # 入口文件
├── src-tauri/           # 后端源代码 (Rust)
│   ├── src/
│   │   ├── handlers/    # Tauri 命令处理器
│   │   ├── models/      # 数据模型
│   │   ├── services/    # 核心业务逻辑 (Audio, Download, Resolver)
│   │   ├── utils/       # 工具函数
│   │   ├── lib.rs       # 库入口
│   │   └── main.rs      # 主程序入口
│   ├── capabilities/    # Tauri 权限配置
│   └── tauri.conf.json  # Tauri 配置文件
└── package.json         # 项目配置
```

## 📖 使用指南

### 播放控制
*   **双击**: 播放列表中的曲目进行播放。
*   **空格键**: 暂停/继续播放。
*   **底部控制栏**: 包含上一曲、下一曲、进度条拖拽、音量调节。

### 添加媒体
*   点击侧边栏底部的 **"Add Files"** 添加本地音乐/视频文件。
*   点击 **"Add Folder"** 添加整个文件夹。
*   点击 **"+"** 输入 URL 添加网络媒体。

### 在线下载
*   添加 Bilibili 或 YouTube 链接后，双击列表项。
*   如果配置了 `yt-dlp`，系统将自动尝试下载并在下载完成后播放。
*   下载状态会在列表中显示（转圈图标）。

## 🤝 贡献

欢迎提交 Issue 和 Pull Request！

## 📄 许可证

[MIT License](LICENSE)

## ⚠️ 免责声明

本项目（Drip Player）及其源代码仅供**个人学习、学术研究和技术交流**使用，**严禁将本项目或其衍生产品用于任何形式的商业用途**（包括但不限于付费分发、广告盈利、作为商业软件的一部分等）。

1.  **资源版权说明**：本项目不提供、存储或分发任何受版权保护的音频或视频资源。所有播放或下载的媒体内容均来源于互联网公开渠道或用户本地文件，版权归原作者、版权方或原平台所有。
2.  **合规使用义务**：用户在使用本项目（特别是集成的第三方工具如 `yt-dlp`）下载或播放网络内容时，必须严格遵守相关国家或地区的法律法规，以及目标网站的服务条款（Terms of Service）。
3.  **责任豁免**：任何因不当使用本项目而产生的法律纠纷、版权侵权或数据丢失等后果，均由使用者**自行承担全部责任**。项目开发者及贡献者不对用户的任何行为承担任何形式的连带责任。
4.  **侵权处理**：如果您是相关内容的版权所有者，且认为本项目侵犯了您的合法权益，请通过 Issue 或邮件联系我们，我们将尽快处理。

**下载、安装、编译或使用本项目即视为您已完全阅读、理解并同意本免责声明的所有条款。**
