import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { createHash } from "node:crypto";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { resolve, relative } from "node:path";

const root = resolve(import.meta.dirname, "web/console");
const output = resolve(import.meta.dirname, "dist");
const pages: { id: string; file: string }[] = JSON.parse(
  readFileSync(resolve(root, "pages.manifest.json"), "utf8"),
);

function releaseManifest(): Plugin {
  return {
    name: "hps-release-manifest",
    apply: "build",
    writeBundle() {
      const entries = readdirSync(output, {
        recursive: true,
        withFileTypes: true,
      })
        .filter((entry) => entry.isFile() && entry.name !== "web-manifest.json")
        .map((entry) =>
          relative(output, resolve(entry.parentPath, entry.name)).replaceAll(
            "\\",
            "/",
          ),
        )
        .sort();
      const assets = Object.fromEntries(
        entries.map((path) => [
          path,
          createHash("sha256")
            .update(readFileSync(resolve(output, path)))
            .digest("hex"),
        ]),
      );
      const web_build_id = createHash("sha256")
        .update(JSON.stringify(assets))
        .digest("hex");
      writeFileSync(
        resolve(output, "web-manifest.json"),
        `${JSON.stringify({ web_build_id, pages: pages.map((page) => page.file), assets }, null, 2)}\n`,
      );
    },
  };
}

export default defineConfig({
  root,
  base: "./",
  plugins: [react(), tailwindcss(), releaseManifest()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: process.env.TAURI_DEV_HOST || false,
    proxy: { "/api": "http://127.0.0.1:17900" },
    watch: { ignored: ["**/src-tauri/**", "**/target/**"] },
  },
  build: {
    outDir: output,
    emptyOutDir: true,
    rolldownOptions: {
      input: Object.fromEntries(
        pages.map((page) => [page.id, resolve(root, page.file)]),
      ),
    },
  },
});
