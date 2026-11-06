import { readFileSync } from "node:fs";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [{
    name: "shared-panel-assets",
    enforce: "pre",
    load(id) {
      if (id.endsWith(".ttf?asset")) {
        const base64 = readFileSync(id.slice(0, -6)).toString("base64");
        return `export default ${JSON.stringify(`data:font/ttf;base64,${base64}`)};`;
      }
    },
  }],
  test: {
    environment: "node",
    css: true,
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
  },
});
