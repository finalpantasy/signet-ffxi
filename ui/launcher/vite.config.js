import { defineConfig } from "vite";

// Relative base: the native host serves dist/ from its own origin.
export default defineConfig({
  base: "./",
  build: { outDir: "dist", emptyOutDir: true, assetsInlineLimit: 0, chunkSizeWarningLimit: 900 },
  server: { host: "127.0.0.1", port: 5199, strictPort: true },
});
