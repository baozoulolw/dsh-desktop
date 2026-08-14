# DeepSeek Harness Desktop

DeepSeek Harness（`dsh` web）的 **Tauri 桌面封装**。将 `@deepseek-ai/dsh` 的 Web 界面封装为跨平台原生桌面应用，支持无边框窗口、内嵌 dsh 子进程、版本检测与一键升级，并通过 GitHub Actions 三平台自动打包。

## ✨ 特性

- **内嵌 dsh**：启动时自动拉起 `dsh web` 子进程，页面上方以 `<iframe>` 嵌入其 Web 界面，操作体验与原生应用一致。
- **无边框窗口**：macOS 使用原生 `Overlay` 无边框（保留红黄绿按钮，页面延伸至顶部），Windows/Linux 使用系统标题栏。
- **版本管理与一键升级**：启动时自动检测 dsh 最新版，发现新版即显示升级按钮；升级面板展示应用/Tauri/dsh 三方版本，支持一键升级并自动重启。
- **三平台打包**：基于 Tauri 打包，支持 macOS（dmg）、Windows（nsis/msi）、Linux（AppImage/deb），并配好 GitHub Actions 矩阵构建。

## 🚀 快速开始

### 环境要求

- [Node.js](https://nodejs.org/) ≥ 20
- [Rust](https://www.rust-lang.org/) stable（Tauri 后端编译）
- [pnpm](https://pnpm.io/) ≥ 9

### 安装与运行

```bash
# 安装依赖
pnpm install

# 本地开发运行(tauri dev,自动拉起 Vite 与 dsh 子进程)
pnpm dev
```

### 打包

```bash
pnpm build          # 打包当前平台
pnpm build:mac      # 打包 macOS(arm64)
```

产物位于 `src-tauri/target/release/bundle/` 目录。

## 📦 CI 自动打包

仓库内置 `.github/workflows/build.yml`，在以下情况自动为 **macOS(arm64)、Windows、Linux** 三个目标并行打包并上传产物：

- 手动触发：仓库 Actions 页面点击 **Run workflow**
- 推送 `v*` 标签（如 `v0.2.0`）自动触发，并发布到 GitHub Releases

## 🗂 项目结构

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

## 🛠 技术实现要点

- **dsh 引擎按用户目录安装**：dsh 不再随应用打包，而是装在每用户 runtime 目录（`app_data_dir/dsh-runtime`），首次启动用系统 npm 安装默认版本，升级就在该目录 `npm install` 新版本。
- **系统 Node 运行 dsh**：dsh 的原生模块（`node-pty` / `koffi`）按本机 Node 编译，必须用系统 node 而非内置运行时，否则 ABI 不匹配，故通过 `npm_node_execpath` / `NODE` 等环境变量解析系统 Node 来启动子进程。
- **子进程生命周期**：Rust 后端负责拉起 `dsh web`、解析其打印的 `http://<ip>:<port>` 地址并通过 `invoke` 返回前端设置 `iframe`；应用退出时自动终止子进程。
- **版本升级**：前端 `invoke('upgrade_dsh')` 在 runtime 目录执行 `npm install @deepseek-ai/dsh@<version>`，通过 `upgrade-progress` 事件流式回传进度，完成后自动重启 dsh。
- **安全**：采用 Tauri capabilities 权限模型，前端仅能调用显式注册的命令；外部链接一律交给系统浏览器打开。

## 📄 License

私有项目，请参阅仓库内相关许可说明。
