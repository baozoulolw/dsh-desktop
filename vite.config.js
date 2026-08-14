import { defineConfig } from 'vite'

// 纯静态前端(Tauri 标准布局):前端在 src/,产物输出到 dist/
export default defineConfig({
  root: 'src',
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
  build: {
    target: 'es2021',
    minify: 'esbuild',
    sourcemap: false,
    outDir: '../dist',
    emptyOutDir: true,
  },
})