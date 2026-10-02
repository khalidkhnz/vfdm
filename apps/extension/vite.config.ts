import { defineConfig } from "vite";
import { resolve } from "node:path";

// Three entries: the MV3 service worker plus two HTML pages. Stable file names
// so manifest.json can reference them without hashing.
export default defineConfig({
  publicDir: "public",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
    minify: false,
    sourcemap: false,
    rollupOptions: {
      input: {
        background: resolve(__dirname, "src/background.ts"),
        options: resolve(__dirname, "options.html"),
        popup: resolve(__dirname, "popup.html"),
      },
      output: {
        entryFileNames: "[name].js",
        chunkFileNames: "chunks/[name].js",
        assetFileNames: "assets/[name][extname]",
      },
    },
  },
});
