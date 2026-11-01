// The Game Mode bridge: each call goes to the Python backend (main.py `Plugin.run`), which
// runs `save-relay <command> ... --json` and hands back its one envelope line. The views are
// the panel's own (panel/src/App.tsx), so they see the same `Bridge` as on the desktop.

import type { Envelope } from "panel/api";
import type { ApiArgs, ApiCommand, Bridge } from "panel/bridge";

/** `callable("run")` from @decky/api, passed in so tests need no Decky. */
export type RunBackend = (command: ApiCommand, args: ApiArgs) => Promise<string>;

export function deckyBridge(run: RunBackend): Bridge {
  return {
    async call<T>(command: ApiCommand, args: ApiArgs): Promise<Envelope<T>> {
      return JSON.parse(await run(command, args)) as Envelope<T>;
    },
  };
}
