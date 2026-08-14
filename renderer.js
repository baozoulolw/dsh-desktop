'use strict'

const $ = (id) => document.getElementById(id)
const webview = $('dsh')
// 注意：api 由 preload 经 contextBridge 注入为全局变量，不能再用 const 重新声明

// ---------- 标题栏：版本徽章 + 平台适配 ----------
// macOS 原生红黄绿按钮在左上角，标题栏需左侧留位；Windows 用系统标题栏，无需留位
api.getVersionInfo().then((info) => {
  $('dshBadge').textContent = `dsh ${info.dshVersion}`
  if (info.platform !== 'darwin') $('titlebar').style.paddingLeft = '10px'
})

// ---------- 启动时自动检查更新，有更新才显示更新按钮 ----------
const updateBtn = $('updateBtn')

async function autoCheckUpdate() {
  try {
    const { latest, isOutdated } = await api.checkUpdate()
    if (isOutdated) {
      updateBtn.textContent = `升级到 ${latest}`
      updateBtn.style.display = 'inline-flex'
    }
  } catch {
    // 检查失败静默：不打扰启动，用户可从版本面板手动检查
  }
}
autoCheckUpdate()

// ---------- 应用自身更新检测（DeepSeek Harness 本体，仅提示 + 跳转下载） ----------
let appLatestUrl = '' // 最新版下载页地址
const appDownloadBtn = $('pAppDownloadBtn')
const appUpdateDot = $('appUpdateDot')

async function checkAppUpdate() {
  try {
    const info = await api.checkAppUpdate()
    if (info.ok === false) throw new Error(info.error)
    $('pAppLatest').textContent = info.latest || '—'
    appLatestUrl = info.url || ''
    const hasNew = !!info.isOutdated
    appDownloadBtn.style.display = hasNew ? 'inline-flex' : 'none'
    appUpdateDot.style.display = hasNew ? 'block' : 'none'
  } catch {
    // 失败静默：不打扰，面板里显示"查询失败"
    $('pAppLatest').textContent = '查询失败'
  }
}
appDownloadBtn.addEventListener('click', () => {
  if (appLatestUrl) api.openAppRelease(appLatestUrl)
})
// 启动时后台自动检查一次
checkAppUpdate()

updateBtn.addEventListener('click', () => {
  panel.classList.add('open')
  refreshPanel()
})

// ---------- 版本/升级面板 ----------
const panel = $('panel')
// 点击版本按钮：已打开则关闭，否则打开并刷新
$('versionBtn').addEventListener('click', () => {
  if (panel.classList.contains('open')) {
    panel.classList.remove('open')
  } else {
    panel.classList.add('open')
    refreshPanel()
  }
})
// 点击面板外部关闭（排除 versionBtn / updateBtn 两个标题栏按钮）
document.addEventListener('click', (e) => {
  if (!panel.contains(e.target) && e.target !== $('versionBtn') && e.target !== $('updateBtn')) {
    panel.classList.remove('open')
  }
})
// 点击 webview(独立渲染进程)内部时，webview 获得焦点，据此关闭面板
webview.addEventListener('focus', () => panel.classList.remove('open'))
// 主窗口整体失焦(切到其他应用)时也关闭面板
api.onHidePanel(() => panel.classList.remove('open'))

async function refreshPanel() {
  const status = $('pStatus')
  const refreshBtn = $('pRefreshBtn')
  const upgradeBtn = $('pUpgradeBtn')
  // 应用自身更新状态独立刷新（不阻塞下方 dsh 检查）
  checkAppUpdate()
  // 进入 loading 状态
  refreshBtn.disabled = true
  refreshBtn.textContent = '检查中…'
  status.className = 'status loading'
  status.textContent = '正在检查最新版本…'
  try {
    const info = await api.getVersionInfo()
    $('pAppVer').textContent = info.appVersion
    $('pEleVer').textContent = info.electronVersion
    $('pCurVer').textContent = info.dshVersion

    const { latest, isOutdated } = await api.checkUpdate()
    $('pLatestVer').textContent = latest
    upgradeBtn.disabled = !isOutdated
    upgradeBtn.textContent = isOutdated ? `升级到 ${latest}` : '升级 dsh'
    // 结果提示（按钮文案不再重复"已是最新"）
    status.className = 'status success'
    status.textContent = isOutdated ? `发现新版本 ${latest}，可升级。` : '已是最新版本。'
  } catch (err) {
    $('pLatestVer').textContent = '查询失败'
    upgradeBtn.disabled = true
    status.className = 'status error'
    status.textContent = `检查失败：${err.message || '网络或服务异常'}`
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
  const res = await api.upgrade(latest)
  if (!res.ok) {
    prog.textContent += `\n[错误] ${res.error}`
    $('pUpgradeBtn').disabled = false
    $('pRefreshBtn').disabled = false
  }
})

// 升级进度
api.onUpgradeProgress(({ phase, done }) => {
  $('pProgress').textContent += phase + '\n'
  $('pProgress').scrollTop = $('pProgress').scrollHeight
  if (done) {
    $('pUpgradeBtn').disabled = true
    $('pRefreshBtn').disabled = false
  }
})

// ---------- webview 加载 dsh ----------
api.onDshUrl((url) => {
  if (webview.getURL() !== url) webview.src = url
})