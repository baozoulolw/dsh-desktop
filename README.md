# DeepSeek Harness Desktop

> [**中文**](#中文版) · [**English**](#english)

<div align="center">

[![platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-4D9BFE)](https://baozoulolw.github.io/dsh-desktop/)

</div>

---

## 中文版

DeepSeek Harness(`dsh` web)的 **Tauri 桌面封装**。将 `@deepseek-ai/dsh` 的 Web 界面封装为跨平台原生桌面应用,支持无边框窗口、内嵌 dsh 子进程、版本检测与一键升级,并通过 GitHub Actions 三平台自动打包。

### 📥 下载

- **👉 前往官网下载:** https://baozoulolw.github.io/dsh-desktop/
- 备选(GitHub Releases): https://github.com/baozoulolw/dsh-desktop/releases

官网包含 macOS / Windows / Linux 各平台的安装包与最新版本,进入页面点击右上角或首屏的「免费下载」即可获得对应平台的 .dmg / .exe / .AppImage 等产物。

### ✨ 特性

- **内嵌 dsh**:启动时自动拉起 `dsh web` 子进程,页面上方以 `<iframe>` 嵌入其 Web 界面,操作体验与原生应用一致。
- **无边框窗口**:macOS 使用原生 `Overlay` 无边框(保留红黄绿按钮,页面延伸至顶部),Windows/Linux 使用系统标题栏。
- **版本管理与一键升级**:启动时自动检测 dsh 最新版,发现新版即显示升级按钮;升级面板展示应用/Tauri/dsh 三方版本,支持一键升级并自动重启。
- **复用已有引擎 / 快速定位**:本机已装有 dsh 时直接复用,不重复安装——优先全局 npm 安装,其次本应用私有 runtime,最后回退 npx(本机常用 `npx @deepseek-ai/dsh`);「版本」面板还能一键在文件管理器(Finder)中打开引擎安装位置。
- **三平台打包**:基于 Tauri 打包,支持 macOS(dmg)、Windows(nsis/msi)、Linux(AppImage/deb),并配好 GitHub Actions 矩阵构建。

### 🚀 快速开始

**环境要求**

- [Node.js](https://nodejs.org/) ≥ 20
- [Rust](https://www.rust-lang.org/) stable(Tauri 后端编译)
- [pnpm](https://pnpm.io/) ≥ 9

**安装与运行**

```bash
# 安装依赖
pnpm install

# 本地开发运行(tauri dev,自动拉起 Vite 与 dsh 子进程)
pnpm dev
```

**打包**

```bash
pnpm build          # 打包当前平台
pnpm build:mac      # 打包 macOS(arm64)
```

产物位于 `src-tauri/target/release/bundle/` 目录。

### 📦 CI 自动打包

仓库内置 `.github/workflows/build.yml`,在以下情况自动为 **macOS(arm64)、Windows、Linux** 三个目标并行打包并上传产物:

- 手动触发:仓库 Actions 页面点击 **Run workflow**
- 推送 `v*` 标签(如 `v0.2.0`)自动触发,并发布到 GitHub Releases

### 🗂 项目结构

```
├── src/                 # 前端(Vite):index.html + renderer.js
├── src-tauri/
│   ├── src/lib.rs       # Tauri 后端:拉起/管理 dsh 子进程、版本检测与升级
│   ├── Cargo.toml       # Rust 依赖与产物配置
│   ├── tauri.conf.json  # Tauri 应用与打包配置
│   └── capabilities/    # 权限能力声明
├── vite.config.js       # Vite 构建配置(产物输出到 dist/)
├── package.json         # 前端依赖与脚本
└── .github/
    └── workflows/
        └── build.yml    # 三平台矩阵打包 CI
```

### 🛠 技术实现要点

- **引擎来源优先复用**:dsh 不随应用打包。启动时按「全局 npm 安装 > 本应用私有 runtime(`app_data_dir/dsh-runtime`)> npx 兜底」选择本机已装的一份直接复用,避免重复安装;未装时才提示安装(用系统 npm 装默认版本)。
- **系统 Node 运行 dsh**:dsh 的原生模块(`node-pty` / `koffi`)按本机 Node 编译,必须用系统 node 而非内置运行时,否则 ABI 不匹配,故通过 `npm_node_execpath` / `NODE` 等环境变量解析系统 Node 来启动子进程。
- **跨会话实例复用**:Rust 后端拉起 `dsh web`、解析其打印的 `http://<ip>:<port>` 地址并通过 `invoke` 返回前端设置 `iframe`;启动成功写入 `dsh-live.json`(pid+url),应用退出不再杀 dsh,下次启动探测到实例仍在就**直接复用**而非重复拉起。
- **版本升级**:前端 `invoke('upgrade_dsh')` 在 runtime 目录执行 `npm install @deepseek-ai/dsh@<version>`,通过 `upgrade-progress` 事件流式回传进度,完成后自动重启 dsh。
- **安全**:采用 Tauri capabilities 权限模型,前端仅能调用显式注册的命令;外部链接一律交给系统浏览器打开。

### 📄 License

私有项目,请参阅仓库内相关许可说明。

---

## English

DeepSeek Harness Desktop is a **Tauri desktop wrapper** around the DeepSeek Harness (`dsh` web) interface. It wraps the `@deepseek-ai/dsh` web UI into a cross-platform native desktop app with a frameless window, an embedded `dsh` subprocess, version detection with one-click upgrades, and three-platform packaging via GitHub Actions.

### 📥 Download

- **👉 Download from the official website:** https://baozoulolw.github.io/dsh-desktop/
- Alternative (GitHub Releases): https://github.com/baozoulolw/dsh-desktop/releases

The website hosts installers for macOS / Windows / Linux along with the latest version. Open the page and click **Free Download** on the top-right or in the hero section to get the `.dmg` / `.exe` / `.AppImage` and other artifacts for your platform.

### ✨ Features

- **Embedded dsh**: Automatically launches the `dsh web` subprocess and embeds its web UI via an `<iframe>` at the top of the page, giving a native app feel.
- **Frameless window**: macOS uses the native `Overlay` frameless style (keeping the traffic-light buttons, with the page extending to the top); Windows/Linux use the system title bar.
- **Version management & one-click upgrade**: Detects the latest dsh version on startup and shows an upgrade button when a new version is found. The upgrade panel shows the app/Tauri/dsh versions and supports one-click upgrade with auto-restart.
- **Reuse an existing engine / quick locate**: If a dsh engine already exists, reuse it instead of reinstalling — preferring a global npm install, then this app's private runtime, then an `npx` fallback (your `npx @deepseek-ai/dsh` workflow). The "Version" panel also offers a one-click button to reveal the engine's install location in your file manager (Finder).
- **Three-platform packaging**: Built with Tauri for macOS (dmg), Windows (nsis/msi), and Linux (AppImage/deb), with a GitHub Actions matrix build preconfigured.

### 🚀 Quick Start

**Requirements**

- [Node.js](https://nodejs.org/) ≥ 20
- [Rust](https://www.rust-lang.org/) stable (compiles the Tauri backend)
- [pnpm](https://pnpm.io/) ≥ 9

**Install & run**

```bash
# Install dependencies
pnpm install

# Run in development (tauri dev, automatically launches Vite and the dsh subprocess)
pnpm dev
```

**Build**

```bash
pnpm build          # build for the current platform
pnpm build:mac      # build for macOS (arm64)
```

Artifacts are placed under `src-tauri/target/release/bundle/`.

### 📦 CI Auto-build

The repo includes `.github/workflows/build.yml`, which builds and uploads artifacts for **macOS (arm64), Windows, Linux** in parallel in the following cases:

- Manual trigger: click **Run workflow** on the repo's Actions page
- Pushing a `v*` tag (e.g. `v0.2.0`) triggers it automatically and publishes to GitHub Releases

### 🗂 Project Structure

```
├── src/                 # Frontend (Vite): index.html + renderer.js
├── src-tauri/
│   ├── src/lib.rs       # Tauri backend: launches/manages dsh subprocess, version check & upgrade
│   ├── Cargo.toml       # Rust dependencies and artifact config
│   ├── tauri.conf.json  # Tauri app and packaging config
│   └── capabilities/    # Permission capability declarations
├── vite.config.js       # Vite build config (outputs to dist/)
├── package.json         # Frontend dependencies and scripts
└── .github/
    └── workflows/
        └── build.yml    # Three-platform matrix build CI
```

### 🛠 Implementation Notes

- **Engine sourcing prefers reuse**: dsh is not bundled with the app. On startup the app picks an already-installed copy — global npm install > this app's private runtime (`app_data_dir/dsh-runtime`) > an `npx` fallback — and reuses it instead of reinstalling. It only prompts to install (via the system npm) when none is present.
- **dsh runs on the system Node**: dsh's native modules (`node-pty` / `koffi`) are compiled against the local Node, so system Node must be used instead of a bundled runtime, or the ABI will mismatch. The system Node is resolved via env vars such as `npm_node_execpath` / `NODE` to launch the subprocess.
- **Cross-session instance reuse**: The Rust backend launches `dsh web`, parses the printed `http://<ip>:<port>` address, and returns it via `invoke` so the frontend sets the `iframe`. On a successful start it writes `dsh-live.json` (pid + url); the app no longer kills dsh on exit, so a running instance is **reused** (not relaunched) on the next start when it's still alive.
- **Version upgrade**: The frontend `invoke('upgrade_dsh')` runs `npm install @deepseek-ai/dsh@<version>` in the runtime directory, streams progress back via the `upgrade-progress` event, then auto-restarts dsh.
- **Security**: Uses the Tauri capabilities permission model so the frontend can only call explicitly registered commands; external links are opened by the system browser.

### 📄 License

Private project, see the relevant license notes in this repository.