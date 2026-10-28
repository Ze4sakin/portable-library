import { defineConfig } from "vite";

/**
 * Tauri 约定：
 * - 固定端口 1420，与 tauri.conf.json 的 devUrl 对齐
 * - strictPort=true，端口被占用时直接失败，而不是静默换端口导致 Tauri 连不上
 * - 不为 Tauri 依赖做预打包优化（官方建议）
 */
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    target: "chrome105",
    minify: "esbuild",
    sourcemap: false,
  },
});
