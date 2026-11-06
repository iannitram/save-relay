// The desktop panel's build. Fixed output names (index.html, app.js, app.css): `save-relay
// panel` embeds exactly these with the Rust `panel-dist` feature (src/panel.rs).
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  base: "./",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    cssCodeSplit: false,
    // The two fonts go into app.css as data URLs: `save-relay panel` serves only the three files
    // above, so a separate font file would never reach the browser (and its token-less request
    // would be refused).
    assetsInlineLimit: 1_000_000,
    rollupOptions: {
      output: {
        entryFileNames: "app.js",
        chunkFileNames: "app-[name].js",
        assetFileNames: "app.[ext]",
        inlineDynamicImports: true,
      },
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
