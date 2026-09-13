import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// @tauri-apps/cli drives this; fixed port so tauri.conf.json devUrl matches.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path.resolve(__dirname, "./src") } },
  clearScreen: false,
  server: {
    port: 1450,
    strictPort: true,
    watch: { ignored: ["**/coverage/**", "**/dist/**", "**/src-tauri/**"] },
  },
  build: { outDir: "dist", target: "es2021" },
});
