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

    const { latest, is_outdated } = await invoke('check_update')
    $('pLatestVer').textContent = latest
    upgradeBtn.disabled = !is_outdated
    upgradeBtn.textContent = is_outdated ? `升级到 ${latest}` : '升级 dsh'
    // 结果提示
    status.className = 'status success'
    status.textContent = is_outdated ? `发现新版本 ${latest},可升级。` : '已是最新版本。'
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
  }
})

// ---------- 启动 dsh 并载入 iframe ----------
async function boot() {
  try {
    const url = await invoke('get_dsh_url')
    if (iframe.getAttribute('src') !== url) iframe.src = url
  } catch (err) {
    $('dshBadge').textContent = 'dsh 启动失败'
    console.error(err)
  }
}
boot()

// 升级/重启后 dsh URL 更新
listen('dsh-url-updated', ({ payload }) => {
  if (iframe.getAttribute('src') !== payload) iframe.src = payload
})