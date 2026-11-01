// The desktop entry: `save-relay panel` opens `/?token=...`; the token comes with every call.

import "./app.css";
import { createRoot } from "react-dom/client";
import { App } from "./App.tsx";
import { httpBridge } from "./bridge.ts";

const token = new URLSearchParams(window.location.search).get("token") ?? "";
const bridge = httpBridge(window.location.origin, token, (url, init) => fetch(url, init));
const root = document.getElementById("root");
if (root) {
  createRoot(root).render(<App bridge={bridge} />);
}
