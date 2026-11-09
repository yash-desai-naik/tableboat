import { execSync } from "node:child_process";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

/** Stamped into the app at build time (shown in About): git commit, whether it had local edits, date. */
function buildInfo() {
  const git = (args: string) => {
    try {
      return execSync(`git ${args}`, { stdio: ["ignore", "pipe", "ignore"] }).toString().trim();
    } catch {
      return "";
    }
  };
  return {
    commit: git("rev-parse --short HEAD") || "unknown",
    dirty: git("status --porcelain") !== "",
    date: new Date().toISOString(),
  };
}

// Tauri expects a fixed dev port and serves the built files from ../dist.
export default defineConfig(({ command }) => ({
  plugins: [react()],
  clearScreen: false,
  define: {
    __APP_BUILD__: JSON.stringify({ ...buildInfo(), dev: command === "serve" }),
  },
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    // WebView2 is evergreen Chromium, so modern output is safe and smaller.
    target: "chrome120",
    sourcemap: false,
  },
}));
