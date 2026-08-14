# DeepSeek Harness Desktop

DeepSeek Harness（`dsh` web）的 **Electron 桌面封装**。将 `@deepseek-ai/dsh` 的 Web 界面封装为跨平台原生桌面应用，支持无边框窗口、内嵌 dsh 子进程、版本检测与一键升级，并通过 GitHub Actions 三平台自动打包。

## ✨ 特性

- **内嵌 dsh**：启动时自动拉起 `dsh web` 子进程，页面上方以 `<webview>` 嵌入其 Web 界面，操作体验与原生应用一致。
- **无边框窗口**：macOS 使用原生 `hiddenInset` 无边框（保留红黄绿按钮，页面延伸至顶部），Windows/Linux 使用系统标题栏。
- **版本管理与一键升级**：启动时自动检测 dsh 最新版，发现新版即显示升级按钮；升级面板展示应用/Electron/dsh 三方版本，支持一键升级并自动重启。
- **三平台打包**：内置 `electron-builder` 配置，支持 macOS（dmg/zip）、Windows（nsis/portable）、Linux（AppImage/deb），并配好 GitHub Actions 矩阵构建。

## 🚀 快速开始

### 环境要求

- [Node.js](https://nodejs.org/) ≥ 20
- [pnpm](https://pnpm.io/) ≥ 10

### 安装与运行

```bash
# 安装依赖
pnpm install

# 本地开发运行
pnpm start
```

### 打包

```bash
pnpm build          # 打包当前平台
pnpm build:mac      # 打包 macOS（dmg/zip）
pnpm build:win      # 打包 Windows（nsis/portable）
pnpm build:linux    # 打包 Linux（AppImage/deb）
```

产出物位于 `dist/` 目录。

## 📦 CI 自动打包

仓库内置 `.github/workflows/build.yml`，在以下情况自动为 **macOS(arm64/x64)、Windows、Linux** 四个目标并行打包并上传产物：

- 手动触发：仓库 Actions 页面点击 **Run workflow**
- 推送 `v*` 标签（如 `v0.1.0`）自动触发

## 🗂 项目结构

```
├── main.js        # Electron 主进程：拉起/管理 dsh 子进程、版本检测与升级、窗口创建
├── preload.js     # 预加载脚本：经 contextBridge 向渲染进程暴露安全 API
├── renderer.js    # 渲染进程：标题栏、版本面板、升级交互、webview 加载
├── index.html     # 应用主页面（标题栏 + webview + 版本/升级面板）
├── package.json   # 依赖与 electron-builder 打包配置
└── .github/
    └── workflows/
        └── build.yml   # 三平台矩阵打包 CI
```

## 🛠 技术实现要点

- **系统 Node 运行 dsh**：dsh 的原生模块（`node-pty` / `koffi`）按本机 Node 编译，若用 Electron 内置 Node（`ELECTRON_RUN_AS_NODE`）会因 ABI 不匹配加载失败，故通过 `npm_node_execpath` / `NODE` 环境变量解析系统 Node 来启动子进程。
- **子进程生命周期**：主进程负责拉起 `dsh web`、解析其打印的 `http://<ip>:<port>` 地址并通过 IPC 通知渲染进程设置 `webview`；窗口关闭 / 应用退出时自动终止子进程。
- **版本升级**：通过 IPC 调用 `pnpm install @deepseek-ai/dsh@<version>` 升级，流式回传进度，完成后自动重启 dsh。
- **安全**：启用 `contextBridge` + `sandbox`，渲染进程仅能访问 `preload.js` 暴露的最小化 API；外部链接一律交给系统浏览器打开。

## 📄 License

私有项目，请参阅仓库内相关许可说明。