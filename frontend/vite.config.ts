import { defineConfig } from "vite";

export default defineConfig({
  root: ".",
  publicDir: "public",
  build: {
    outDir: "dist",
    assetsDir: "assets",
    assetsInlineLimit: 0,
  },
  server: {
    port: 5173,
    proxy: {
      // Proxy API requests to Rust server in development
      "/api": {
        target: "http://localhost:3000",
        changeOrigin: true,
      },
      // Proxy WASM files to Rust server (which serves from CLI assets)
      "/wasm": {
        target: "http://localhost:3000",
        changeOrigin: true,
      },
    },
  },
  optimizeDeps: {
    exclude: ["vibe_graph_viz"],
  },
});
