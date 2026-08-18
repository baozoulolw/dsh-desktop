'use strict'

// 官方 Tauri v2 API(vite 打包,显式 import,不依赖全局 __TAURI__)
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

const $ = (id) => document.getElementById(id)
const iframe = $('dsh')

// ---------- 标题栏:版本徽章 + 平台适配 ----------
// macOS 原生红黄绿按钮在左上角,标题栏需左侧留位(CSS 已设 92px);其他平台用系统标题栏,无需留位。
// 注意:Tauri(Rust std::env::consts::OS)在 macOS 返回 "macos",而非 Electron/Node 的 "darwin",两者都要认。
invoke('get_version_info').then((info) => {
  $('dshBadge').textContent = `dsh ${info.dsh_version}`
  const isMac = info.platform === 'macos' || info.platform === 'darwin'
  if (!isMac) $('titlebar').style.paddingLeft = '10px'
}).catch(() => {})

// 刷新徽章为当前安装的引擎版本(安装/升级重载后调用)
async function refreshBadge() {
  try {
    const info = await invoke('get_version_info')
    $('dshBadge').textContent = `dsh ${info.dsh_version}`
  } catch {}
}

// ---------- 启动时自动检查更新,有更新才显示更新按钮 ----------
const updateBtn = $('updateBtn')

async function autoCheckUpdate() {
  try {
    const { latest, is_outdated } = await invoke('check_update')
    if (is_outdated) {
      updateBtn.textContent = `升级到 ${latest}`
      updateBtn.style.display = 'inline-flex'
    }
  } catch {
    // 检查失败静默:不打扰启动,用户可从版本面板手动检查
  }
}
autoCheckUpdate()

// ---------- 应用自身更新检测(DeepSeek Harness 本体,仅提示 + 跳转下载) ----------
let appLatestUrl = '' // 最新版下载页地址
const appDownloadBtn = $('pAppDownloadBtn')
const appUpdateDot = $('appUpdateDot')

async function checkAppUpdate() {
  try {
    const info = await invoke('check_app_update')
    if (info.ok === false) throw new Error(info.error || '查询失败')
    $('pAppLatest').textContent = info.latest || '—'
    appLatestUrl = info.url || ''
    const hasNew = !!info.is_outdated
    appDownloadBtn.style.display = hasNew ? 'inline-flex' : 'none'
    appUpdateDot.style.display = hasNew ? 'block' : 'none'
  } catch {
    // 失败静默:不打扰,面板里显示"查询失败"
    $('pAppLatest').textContent = '查询失败'
  }
}
appDownloadBtn.addEventListener('click', () => {
  if (appLatestUrl) invoke('open_external', { url: appLatestUrl })
})
// 启动时后台自动检查一次
checkAppUpdate()

updateBtn.addEventListener('click', () => {
  panel.classList.add('open')
  refreshPanel()
})

// ---------- 版本/升级面板 ----------
const panel = $('panel')
// 点击版本按钮:已打开则关闭,否则打开并刷新
$('versionBtn').addEventListener('click', () => {
  if (panel.classList.contains('open')) {
    panel.classList.remove('open')
  } else {
    panel.classList.add('open')
    refreshPanel()
  }
})
// 点击面板外部关闭(排除 versionBtn / updateBtn 两个标题栏按钮)
document.addEventListener('click', (e) => {
  if (!panel.contains(e.target) && e.target !== $('versionBtn') && e.target !== $('updateBtn')) {
    panel.classList.remove('open')
  }
})
// 主窗口整体失焦(切到其他应用)时关闭面板
window.addEventListener('blur', () => panel.classList.remove('open'))

async function refreshPanel() {
  const status = $('pStatus')
  const refreshBtn = $('pRefreshBtn')
  const upgradeBtn = $('pUpgradeBtn')
  // 应用自身更新状态独立刷新(不阻塞下方 dsh 检查)
  checkAppUpdate()
  // 进入 loading 状态
  refreshBtn.disabled = true
  refreshBtn.textContent = '检查中…'
  status.className = 'status loading'
  status.textContent = '正在检查最新版本…'
  try {
    const info = await invoke('get_version_info')
    $('pAppVer').textContent = info.app_version
    $('pEleVer').textContent = info.engine_version
    $('pCurVer').textContent = info.dsh_version
    renderEngineInfo(info)
    const notInstalled = info.dsh_version === '未知'

    const { latest, is_outdated } = await invoke('check_update')
    if (notInstalled) {
      // 引擎未安装:不显示最新版,也不提示升级
      $('pLatestVer').textContent = '—'
      upgradeBtn.disabled = true
      upgradeBtn.textContent = '升级 dsh'
      status.className = 'status error'
      status.textContent = '引擎尚未安装,请先在主界面点击"安装引擎"。'
    } else {
      $('pLatestVer').textContent = latest
      upgradeBtn.disabled = !is_outdated
      upgradeBtn.textContent = is_outdated ? `升级到 ${latest}` : '升级 dsh'
      status.className = 'status success'
      status.textContent = is_outdated ? `发现新版本 ${latest},可升级。` : '已是最新版本。'
    }
  } catch (err) {
    $('pLatestVer').textContent = '查询失败'
    upgradeBtn.disabled = true
    status.className = 'status error'
    status.textContent = `检查失败:${err || '网络或服务异常'}`
  } finally {
    refreshBtn.disabled = false
    refreshBtn.textContent = '检查更新'
  }
}

// 渲染引擎来源 / 安装位置,并切换"打开"按钮的语义(在文件管理器里打开本机目录)。
function renderEngineInfo(info) {
  const src = $('pEngSource')
  const addr = $('pEngAddress')
  const btn = $('pEngineBtn')
  const { engine_source: source, engine_address: address } = info
  switch (source) {
    case 'global':
      src.textContent = '全局 npm'
      addr.textContent = address
      btn.textContent = '打开安装位置'
      break
    case 'app':
      src.textContent = '本应用'
      addr.textContent = address
      btn.textContent = '打开安装位置'
      break
    case 'npx':
      src.textContent = 'npx (本机)'
      addr.textContent = address
      btn.textContent = '打开安装位置'
      break
    case 'none':
    default:
      src.textContent = '未安装'
      addr.textContent = ''
      btn.textContent = '去安装 dsh'
      break
  }
  btn.style.display = 'inline-flex'
}
// "快捷跳转":在文件管理器里显示引擎安装目录;未安装则去安装页,均由后端 decide。
$('pEngineBtn').addEventListener('click', () => {
  invoke('reveal_engine').catch(() => {})
})

$('pRefreshBtn').addEventListener('click', refreshPanel)

$('pUpgradeBtn').addEventListener('click', async () => {
  const latest = $('pLatestVer').textContent
  if (!latest || latest === '—' || latest === '查询失败') return
  const prog = $('pProgress')
  prog.style.display = 'block'
  prog.textContent = ''
  $('pUpgradeBtn').disabled = true
  $('pRefreshBtn').disabled = true
  try {
    await invoke('upgrade_dsh', { targetVersion: latest })
  } catch (err) {
    prog.textContent += `\n[错误] ${err}`
    $('pUpgradeBtn').disabled = false
    $('pRefreshBtn').disabled = false
  }
})

// 升级进度
listen('upgrade-progress', ({ payload }) => {
  $('pProgress').textContent += payload.phase + '\n'
  $('pProgress').scrollTop = $('pProgress').scrollHeight
  if (payload.done) {
    $('pUpgradeBtn').disabled = true
    $('pRefreshBtn').disabled = false
    // 升级完成后刷新左上角版本徽章,并重新检查以隐藏标题栏红
    // 色"升级到 X"按钮(boot() 不会在升级后调用,两者都要手动补)。
    refreshBadge()
    autoCheckUpdate()
  }
})

// ---------- 启动失败提示层 ----------
const bootError = $('bootError')
const errTitle = $('errTitle')
const errMsg = $('errMsg')
const errInstallBtn = $('errInstallBtn')
const errEngInstallBtn = $('errEngInstallBtn')
const errProgress = $('errProgress')
const errRetryBtn = $('errRetryBtn')

function showBootError({ title, message, offerNode = false, offerEngine = false }) {
  errTitle.textContent = title
  errMsg.textContent = message
  errMsg.style.display = 'block'
  errProgress.style.display = 'none'
  errInstallBtn.style.display = offerNode ? 'inline-flex' : 'none'
  errEngInstallBtn.style.display = offerEngine ? 'inline-flex' : 'none'
  errEngInstallBtn.disabled = false
  errRetryBtn.disabled = false
  bootError.style.display = 'flex'
}
function hideBootError() {
  bootError.style.display = 'none'
}
// 引导安装 Node.js(引擎依赖 Node 才能安装与运行)
errInstallBtn.addEventListener('click', () => {
  invoke('open_external', { url: 'https://nodejs.org/' })
})
// 安装引擎:窗口内实时显示 npm 安装进度,装完自动 boot
async function installEngine() {
  errEngInstallBtn.disabled = true
  errRetryBtn.disabled = true
  errMsg.style.display = 'none'
  errProgress.style.display = 'block'
  errProgress.textContent = ''
  try {
    await invoke('install_dsh')
  } catch (err) {
    $('dshBadge').textContent = '安装失败'
    errProgress.textContent += `\n[错误] ${err}`
    errEngInstallBtn.disabled = false
    errRetryBtn.disabled = false
  }
}
errEngInstallBtn.addEventListener('click', installEngine)
// 安装进度(实时回传;done 后自动启动引擎并载入)
listen('install-progress', ({ payload }) => {
  errProgress.textContent += (payload.phase || '') + '\n'
  errProgress.scrollTop = errProgress.scrollHeight
  if (payload.done) boot()
})
// 重试启动
errRetryBtn.addEventListener('click', () => {
  boot()
})

// ---------- 启动 dsh 并载入 iframe ----------
async function boot() {
  hideBootError()
  try {
    const res = await invoke('get_dsh_url')
    if (res.status === 'ready') {
      if (iframe.getAttribute('src') !== res.url) iframe.src = res.url
      refreshBadge()
    } else if (res.status === 'node_missing') {
      $('dshBadge').textContent = 'dsh 未安装'
      showBootError({
        title: '未检测到 Node.js',
        message: 'dsh 引擎依赖 Node.js 来安装和运行,但当前系统没有检测到 Node.js。\n请先前往 Node.js 官网下载安装,安装后再回本应用点击"安装引擎"。',
        offerNode: true,
      })
    } else if (res.status === 'not_installed') {
      $('dshBadge').textContent = 'dsh 未安装'
      showBootError({
        title: 'dsh 引擎未安装',
        message: '首次使用需要先安装 dsh 引擎(约几十 MB)。点击下方"安装引擎",安装进度会在窗口内实时显示,完成后自动启动。\n· 也可以点右上角「版本」面板里的「去安装 dsh」跳到官方安装页。',
        offerEngine: true,
      })
    } else {
      $('dshBadge').textContent = 'dsh 启动失败'
      showBootError({
        title: 'dsh 启动失败',
        message: res.message || '未知错误',
      })
    }
  } catch (err) {
    $('dshBadge').textContent = 'dsh 启动失败'
    console.error(err)
    showBootError({
      title: '启动失败',
      message: String(err),
      offerNode: String(err).includes('node') || String(err).includes('Node'),
    })
  }
}
boot()

// 升级/重启后 dsh URL 更新
listen('dsh-url-updated', ({ payload }) => {
  if (iframe.getAttribute('src') !== payload) iframe.src = payload
})