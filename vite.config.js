import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

/**
 * Manual chunking for large vendor deps to keep bundle growth manageable.
 * @param {string} id
 * @returns {string | undefined}
 */
function manualChunks(id) {
  if (!id.includes("node_modules")) return undefined;
  if (id.includes("/node_modules/@xterm/")) return "vendor-xterm";
  if (id.includes("/node_modules/prismjs/")) return "vendor-prism";
  if (id.includes("/node_modules/@tauri-apps/api/")) return "vendor-tauri-api";
  return undefined;
}

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [sveltekit(), tailwindcss()],

  // Vitest configuration
  test: {},

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  build: {
    rollupOptions: {
      output: {
        manualChunks,
      },
    },
  },
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. Ignore non-source directories to prevent Vite reloads on file edits.
      // Tailwind v4's oxide scanner puts .md files into the module graph via
      // addWatchFile(), so any markdown edit triggers a full-page reload.
      ignored: [
        "**/src-tauri/**",
        "**/*.md",
        "**/dev/**",
        "**/docs/**",
        "**/.claude/**",
        "**/.context/**",
        "**/target/**",
      ],
    },
  },
}));
