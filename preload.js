'use strict'

const { contextBridge, ipcRenderer } = require('electron')

// 向渲染进程暴露一个最小化的、安全的 API
contextBridge.exposeInMainWorld('api', {
  getVersionInfo: () => ipcRenderer.invoke('get-version-info'),
  checkUpdate: () => ipcRenderer.invoke('check-update'),
  upgrade: (targetVersion) => ipcRenderer.invoke('upgrade-dsh', targetVersion),
  // 订阅主进程事件
  onDshUrl: (cb) => ipcRenderer.on('dsh-url-updated', (_e, url) => cb(url)),
  onUpgradeProgress: (cb) => ipcRenderer.on('upgrade-progress', (_e, info) => cb(info)),
  onHidePanel: (cb) => ipcRenderer.on('hide-panel', () => cb())
})