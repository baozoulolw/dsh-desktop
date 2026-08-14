'use strict'

const { app, BrowserWindow, dialog, shell, ipcMain } = require('electron')
const { spawn, execFile } = require('child_process')
const path = require('path')
const os = require('os')
const fs = require('fs')

const DSH_BIN = path.join(__dirname, 'node_modules', '@deepseek-ai', 'dsh', 'lib', 'bin.js')
const DSH_PKG = path.join(__dirname, 'node_modules', '@deepseek-ai', 'dsh', 'package.json')
const URL_RE = /dsh web: (http:\/\/\d+\.\d+\.\d+\.\d+:\d+)/

const STARTUP_TIMEOUT_MS = 30_000

let child = null // dsh 子进程
let booted = false // 是否已拿到 URL
let mainWindow = null

// 用系统 node 运行 dsh：dsh 的原生模块(node-pty/koffi)按本机 node 编译，
// 若用 Electron 内置 node(ELECTRON_RUN_AS_NODE)会因 ABI 不匹配而加载失败。
function resolveNodeBin() {
  // npm/pnpm 启动时注入的 node 绝对路径
  if (process.env.npm_node_execpath) return process.env.npm_node_execpath
  if (process.env.NODE) return process.env.NODE
  return 'node' // 回退：依赖 PATH
}

/** 启动 dsh web 子进程，返回一个 Promise<url>。 */
function startDsh() {
  return new Promise((resolve, reject) => {
    child = spawn(resolveNodeBin(), [DSH_BIN, '--profile', 'web', '--port', '0'], {
      cwd: os.homedir(), // workspace root 设为用户目录，避免污染项目目录
      env: { ...process.env },
      stdio: ['ignore', 'pipe', 'pipe']
    })

    booted = false
    const onData = (chunk) => {
      const text = chunk.toString()
      const m = text.match(URL_RE)
      if (m) {
        booted = true
        resolve(m[1])
      }
    }
    child.stdout.on('data', onData)
    child.stderr.on('data', onData)

    child.on('error', (err) => {
      if (!booted) reject(err)
    })

    child.on('exit', (code, signal) => {
      if (!booted) {
        reject(new Error(`dsh 进程提前退出 (code=${code}, signal=${signal})`))
      }
    })
  })
}

function killChild() {
  if (child) {
    try {
      child.kill('SIGTERM')
    } catch { /* 忽略 */ }
    child = null
  }
}

function createWindow() {
  mainWindow = new BrowserWindow({
    width: 1280,
    height: 820,
    title: 'DeepSeek Harness',
    // macOS 原生无边框：隐藏标题栏、保留红黄绿按钮、页面延伸到顶部
    titleBarStyle: process.platform === 'darwin' ? 'hiddenInset' : 'default',
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      webviewTag: true,
      sandbox: true
    }
  })

  mainWindow.webContents.setWindowOpenHandler(({ url: target }) => {
    // 外部链接走系统浏览器，不在应用内开新窗口
    shell.openExternal(target)
    return { action: 'deny' }
  })

  // 点击 webview 内部时，其 guest webContents 获得焦点，据此关闭版本面板
  mainWindow.webContents.on('did-attach-webview', (_e, guestContents) => {
    guestContents.on('focus', () => {
      mainWindow?.webContents.send('hide-panel')
    })
  })

  mainWindow.on('closed', () => {
    mainWindow = null
    killChild()
    app.quit()
  })

  mainWindow.loadFile('index.html')
}

// ---------- 版本与升级 ----------

function readDshVersion() {
  try {
    return JSON.parse(fs.readFileSync(DSH_PKG, 'utf8')).version
  } catch {
    return '未知'
  }
}

/** 用系统 node 的 fetch 查 npmmirror registry 拿 dsh 最新版。 */
function fetchLatestDshVersion() {
  return new Promise((resolve, reject) => {
    const script =
      "fetch('https://registry.npmmirror.com/@deepseek-ai/dsh')" +
      ".then(r=>r.json()).then(d=>process.stdout.write(d['dist-tags'].latest))" +
      ".catch(e=>{process.stderr.write(String(e));process.exit(1)})"
    const p = spawn(resolveNodeBin(), ['-e', script], { env: { ...process.env } })
    let out = ''
    const timer = setTimeout(() => {
      // 网络超时保护：避免查询永久卡住
      try { p.kill('SIGKILL') } catch { /* 忽略 */ }
      reject(new Error('查询超时（网络异常）'))
    }, 10_000)
    p.stdout.on('data', (c) => (out += c.toString()))
    p.on('error', (err) => { clearTimeout(timer); reject(err) })
    p.on('exit', (code) => {
      clearTimeout(timer)
      if (code === 0 && out.trim()) resolve(out.trim())
      else reject(new Error('查询最新版本失败'))
    })
  })
}

/** 在项目目录用包管理器升级 dsh 到指定版本，流式回调进度。 */
function upgradeDsh(targetVersion, onProgress) {
  return new Promise((resolve, reject) => {
    killChild() // 先停 dsh，避免 pnpm 改 node_modules 时仍在读写
    const pm = process.env.npm_config_user_agent?.includes('pnpm') ? 'pnpm' : 'npm' // 本项目用 pnpm，回退 npm
    const p = spawn(pm, ['install', `@deepseek-ai/dsh@${targetVersion}`], {
      cwd: __dirname,
      env: { ...process.env },
      stdio: ['ignore', 'pipe', 'pipe']
    })
    let stderr = ''
    p.stdout.on('data', (c) => onProgress(c.toString()))
    p.stderr.on('data', (c) => {
      stderr += c.toString()
      onProgress(c.toString())
    })
    p.on('error', reject)
    p.on('exit', (code) => {
      if (code === 0) resolve()
      else reject(new Error(`升级失败：${stderr.slice(-500)}`))
    })
  })
}

function registerIpc() {
  ipcMain.handle('get-version-info', () => ({
    appVersion: app.getVersion(),
    electronVersion: process.versions.electron,
    dshVersion: readDshVersion(),
    platform: process.platform
  }))

  ipcMain.handle('check-update', async () => {
    const current = readDshVersion()
    const latest = await fetchLatestDshVersion()
    return { current, latest, isOutdated: latest !== '未知' && latest !== current }
  })

  ipcMain.handle('upgrade-dsh', async (_e, targetVersion) => {
    try {
      let done = false
      mainWindow?.webContents.send('upgrade-progress', { phase: '开始升级…', done })
      await upgradeDsh(targetVersion, (line) => {
        mainWindow?.webContents.send('upgrade-progress', { phase: line.trim() || '…', done })
      })
      // 升级完成后重启 dsh
      const url = await startDsh()
      mainWindow?.webContents.send('dsh-url-updated', url)
      done = true
      mainWindow?.webContents.send('upgrade-progress', {
        phase: `升级完成，dsh 已重启到 ${url}`,
        done
      })
      return { ok: true, version: readDshVersion() }
    } catch (err) {
      return { ok: false, error: err.message }
    }
  })
}

app.on('will-quit', killChild)

app.whenReady().then(async () => {
  registerIpc()
  createWindow()

  // 启动 dsh 并拿到 URL 后通知 renderer 设置 webview
  try {
    const url = await Promise.race([
      startDsh(),
      new Promise((_, reject) =>
        setTimeout(() => reject(new Error('启动超时：30 秒内未获取到 dsh web 地址')), STARTUP_TIMEOUT_MS)
      )
    ])
    mainWindow?.webContents.send('dsh-url-updated', url)
  } catch (err) {
    dialog.showErrorBox('dsh 启动失败', err.message)
    killChild()
    app.quit()
  }
})

app.on('window-all-closed', () => {
  app.quit()
})