/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const page = (name: string) => decodeURIComponent(new URL(`./${name}`, import.meta.url).pathname);

// Tauri expects a fixed dev port (tauri.conf.json `devUrl`).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/target/**", "**/crates/**"] },
  },
  build: {
    target: "safari16",
    outDir: "dist",
    // Two pages: the main window and the quick-capture window (src/quick).
    rolldownOptions: {
      input: {
        main: page("index.html"),
        quick: page("quick.html"),
      },
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
