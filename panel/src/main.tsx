// The desktop entry: `save-relay panel` opens `/?token=...`; the token comes with every call.
// With no token (`pnpm --dir panel dev`) or `?fixtures`, the views run on the fixtures.

/// <reference types="vite/client" />
import "./tokens.css";
import "./app.css";
import { createRoot } from "react-dom/client";
import { App } from "./App.tsx";
import { httpBridge } from "./bridge.ts";
import { applyTheme } from "./theme.ts";

const colorScheme = typeof window !== "undefined" && typeof window.matchMedia === "function"
  ? window.matchMedia("(prefers-color-scheme: light)")
  : undefined;
applyTheme(document.documentElement, Boolean(colorScheme?.matches));
const onSystemTheme = (event: MediaQueryListEvent) => applyTheme(document.documentElement, event.matches);
if (typeof colorScheme?.addEventListener === "function") {
  colorScheme.addEventListener("change", onSystemTheme);
} else {
  // Older WebKit has only the deprecated addListener; without either, the theme stays as set above.
  colorScheme?.addListener?.(onSystemTheme);
}

const params = new URLSearchParams(window.location.search);
const token = params.get("token");
// The fixtures are for `pnpm dev` only: `import.meta.env.DEV` is false in the release build, so
// the bundler leaves the fixture bridge out and a token-less page just fails its calls.
const root = document.getElementById("root");
if (root && import.meta.env.DEV && (!token || params.has("fixtures"))) {
  void import("./fixtureBridge.ts").then(({ fixtureBridge, fixturePicture }) => {
    createRoot(root).render(
      <App bridge={fixtureBridge()} pictureUrl={fixturePicture} token={token} />,
    );
  });
} else if (root) {
  const bridge = httpBridge(window.location.origin, token ?? "", (url, init) => fetch(url, init));
  createRoot(root).render(<App bridge={bridge} token={token} />);
}
