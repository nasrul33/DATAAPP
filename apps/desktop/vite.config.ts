import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { env } from "node:process";
import { defineConfig } from "vite";

const tauriDevHost = env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: tauriDevHost ?? false,
    ...(tauriDevHost
      ? { hmr: {
          protocol: "ws",
          host: tauriDevHost,
          port: 1421,
        } }
      : {}),
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: env.TAURI_ENV_PLATFORM === "macos" ? "safari13" : "chrome105",
    minify: env.TAURI_ENV_DEBUG ? false : "esbuild",
    sourcemap: Boolean(env.TAURI_ENV_DEBUG),
  },
});
