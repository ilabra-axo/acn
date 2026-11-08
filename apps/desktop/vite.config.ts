import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { execSync } from "node:child_process";
import pkg from "./package.json" with { type: "json" };

const git = (args: string) => {
  try {
    return execSync(`git ${args}`, { encoding: "utf8" }).trim();
  } catch {
    return "unknown";
  }
};

const host = process.env.TAURI_DEV_HOST;

// https://v2.tauri.app/start/frontend/vite/
export default defineConfig({
  plugins: [svelte()],
  // Build metadata for presenter mode's version line (the Tauri app reports its own).
  define: {
    __NMESH_VERSION__: JSON.stringify(pkg.version),
    __NMESH_COMMIT__: JSON.stringify(git("rev-parse --short=9 HEAD")),
    __NMESH_BUILT_AT__: String(Math.floor(Date.now() / 1000)),
  },
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 5174 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: "es2022",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
