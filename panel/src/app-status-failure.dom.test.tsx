// @vitest-environment jsdom
// #250 item 13: a `status` that fails once must not hide the game's pending choice.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import type { Envelope } from "./api.ts";
import type { ApiArgs, ApiCommand, Bridge } from "./bridge.ts";
import { App } from "./App.tsx";
import { fixtureBridge } from "./fixtureBridge.ts";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const flush = async () => act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });

it("keeps a known pending choice when a later status call fails", async () => {
  const base = fixtureBridge();
  const failing = new Set<string>();
  const bridge: Bridge = {
    async call<T>(command: ApiCommand, args: ApiArgs): Promise<Envelope<T>> {
      if (command === "status" && failing.has(String(args.game))) {
        throw new Error("status failed");
      }
      return base.call<T>(command, args);
    },
  };
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  const click = async (selector: string) => {
    await act(async () => host.querySelector<HTMLElement>(selector)!.click());
    await flush();
  };
  await act(async () => root.render(<App bridge={bridge} />));
  await flush();
  await click('[data-focus="pending"]');
  const before = host.textContent ?? "";
  expect(before).toContain("hades");

  // Back to the games (this reloads them), now with every status failing.
  failing.add("hades");
  failing.add("celeste");
  await click('[data-focus="back"]');
  await click('[data-focus="pending"]');
  expect(host.textContent).toBe(before);
  act(() => root.unmount());
  host.remove();
});
