// The Decky plugin's build: @decky/rollup bundles src/index.tsx into dist/index.js and leaves
// React and @decky/ui to Decky Loader, which supplies Steam's own copies at run time.
import deckyPlugin from "@decky/rollup";

export default deckyPlugin();
