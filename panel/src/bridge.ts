// How the views reach save-relay. The desktop page posts to `save-relay panel`'s own server
// (`httpBridge`); the Decky plugin calls its Python backend, which runs `save-relay --json`
// (decky/src/bridge.ts). Both answer the same envelope, so the views never know which.

import type { Envelope } from "./api.ts";

/** The commands `POST /api/<command>` accepts (src/panel.rs `API_COMMANDS`). */
export const API_COMMANDS = [
  "status",
  "history",
  "restore",
  "pull",
  "push",
  "answer",
  "where",
  "which",
  "picture",
  "setup",
  "steam-games",
  "install-wrap",
  "launch-option",
] as const;

export type ApiCommand = (typeof API_COMMANDS)[number];

/** The arguments of one command as a JSON object (src/panel.rs `api_args`). */
export type ApiArgs = Record<string, string | number | boolean | string[] | number[]>;

export interface Bridge {
  call<T>(command: ApiCommand, args: ApiArgs): Promise<Envelope<T>>;
  /** Present in Game Mode when Steam can report the current app launch option. */
  getLaunchOption?(appId: number): Promise<string>;
  /** Present in Game Mode when Steam can accept and confirm a launch option update. */
  setLaunchOption?(appId: number, text: string): Promise<void>;
}

/** The header that carries the panel's token (src/panel.rs `TOKEN_HEADER`). */
export const TOKEN_HEADER = "X-Save-Relay-Token";

type Fetch = (
  url: string,
  init: { method: string; headers: Record<string, string>; body: string },
) => Promise<{ json(): Promise<unknown> }>;

/** The desktop bridge: `POST <base>/api/<command>` with the token header and `args` as JSON. */
export function httpBridge(base: string, token: string, fetcher: Fetch): Bridge {
  return {
    async call<T>(command: ApiCommand, args: ApiArgs): Promise<Envelope<T>> {
      const response = await fetcher(`${base}/api/${command}`, {
        method: "POST",
        headers: { "Content-Type": "application/json", [TOKEN_HEADER]: token },
        body: JSON.stringify(args),
      });
      return (await response.json()) as Envelope<T>;
    },
  };
}
