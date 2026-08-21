import { defineConfig, Plugin, PluginOption } from "vite";
import react from "@vitejs/plugin-react";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

/**
 * Dev-server bridge that serves the live core bearer token at
 * `/__agpeer_token`. The frontend fetches it so the WebUI always uses the same
 * token the running core accepts, even if the process was started without a
 * VITE_AGPEER_TOKEN env var or the token was regenerated.
 *
 * Token source (first match): AGPEER_TOKEN_FILE env, else ../../run/data/token
 * (the file the agpeer core reads from `[server].data_dir`).
 */
function agpeerTokenBridge(): Plugin {
  return {
    name: "agpeer-token-bridge",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        if (req.url !== "/__agpeer_token") {
          next();
          return;
        }
        const file =
          process.env.AGPEER_TOKEN_FILE || resolve(process.cwd(), "../../run/data/token");
        try {
          const token = readFileSync(file, "utf8").trim();
          res.statusCode = 200;
          res.setHeader("Content-Type", "text/plain");
          res.setHeader("Cache-Control", "no-store");
          res.end(token);
        } catch {
          res.statusCode = 404;
          res.end("not found");
        }
      });
    },
  };
}

// Tauri expects a fixed dev host/port and strict port.
export default defineConfig({
  plugins: [react(), agpeerTokenBridge() as PluginOption],
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port: 5173,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
