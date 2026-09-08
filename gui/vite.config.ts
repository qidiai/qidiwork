import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// Tauri 前端构建配置。dist 产物由 src-tauri 的 frontendDist 指向。
export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      // cargo 构建会写 src-tauri/target,vite 监视到会 EBUSY 崩溃
      ignored: ["**/src-tauri/**", "**/dist/**", "**/node_modules/**"],
    },
  },
  build: {
    // WebView2 = Chromium;105 覆盖 Win10 早期 Runtime,无需更低 target
    target: "chrome105",
    outDir: "dist",
    emptyOutDir: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
});
