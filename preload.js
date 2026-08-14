'use strict'

const { contextBridge, ipcRenderer } = require('electron')

// 向渲染进程暴露一个最小化的、安全的 API
contextBridge.exposeInMainWorld('api', {
  getVersionInfo: () => ipcRenderer.invoke('get-version-info'),
  checkUpdate: () => ipcRenderer.invoke('check-update'),
  upgrade: (targetVersion) => ipcRenderer.invoke('upgrade-dsh', targetVersion),
  // 应用自身更新检测
  checkAppUpdate: () => ipcRenderer.invoke('check-app-update'),
  openAppRelease: (url) => ipcRenderer.invoke('open-app-release', url),
  // 订阅主进程事件
  onDshUrl: (cb) => ipcRenderer.on('dsh-url-updated', (_e, url) => cb(url)),
  onUpgradeProgress: (cb) => ipcRenderer.on('upgrade-progress', (_e, info) => cb(info)),
  onHidePanel: (cb) => ipcRenderer.on('hide-panel', () => cb())
})