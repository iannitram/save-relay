// Decky supplies React and @decky/ui at runtime. Only the shared views and assets bundle.
import { readFileSync } from "node:fs";
import deckyPlugin from "@decky/rollup";

export default deckyPlugin({
  plugins: [{
    name: "shared-panel-css",
    load(id) {
      if (id.endsWith(".css?inline")) {
        return `export default ${JSON.stringify(readFileSync(id.slice(0, -7), "utf8"))};`;
      }
    },
  }],
});
