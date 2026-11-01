// The panel's views (SPEC.md M15). Placeholder: the views (games, a pending choice, history
// with save cards, the QR sign-in) are built by their issues. The Decky plugin renders this
// same component inside Game Mode.

import type { Bridge } from "./bridge.ts";

export interface AppProps {
  bridge: Bridge;
}

export function App(_props: AppProps) {
  return (
    <main>
      <h1>Save Relay</h1>
      <p>The panel is not built yet.</p>
    </main>
  );
}
