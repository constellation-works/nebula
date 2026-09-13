import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Tauri drives this: `tauri dev` starts it on 1420 and `tauri build` bundles
// `dist/`. The port is fixed so `devUrl` in tauri.conf.json stays true.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["src/test/setup.ts"],
  },
});
