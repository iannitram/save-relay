// The desktop entry: `save-relay panel` opens `/?token=...`; the token comes with every call.
// With no token (`pnpm --dir panel dev`) or `?fixtures`, the views run on the fixtures.

import "./tokens.css";
import "./app.css";
import { createRoot } from "react-dom/client";
import { App } from "./App.tsx";
import { httpBridge } from "./bridge.ts";
import { fixtureBridge, fixturePicture } from "./fixtureBridge.ts";

const params = new URLSearchParams(window.location.search);
const token = params.get("token");
const fixtures = !token || params.has("fixtures");
const bridge = fixtures
  ? fixtureBridge()
  : httpBridge(window.location.origin, token, (url, init) => fetch(url, init));
const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <App bridge={bridge} pictureUrl={fixtures ? fixturePicture : undefined} token={token} />,
  );
}
