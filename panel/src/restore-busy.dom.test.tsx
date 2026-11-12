// @vitest-environment jsdom
// #249 S15: a restore in flight disables the Restore buttons, so a second press cannot run a
// second restore, and a later result replaces the earlier banner.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Envelope } from "./api.ts";
import type { ApiArgs, ApiCommand, Bridge } from "./bridge.ts";
import historyFx from "./fixtures/history.json";
import restoreFx from "./fixtures/restore.json";
import { App } from "./App.tsx";
import { fixtureBridge } from "./fixtureBridge.ts";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const SECOND = historyFx.data.entries[1]!.entry;
const THIRD = historyFx.data.entries[2]!.entry;

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const settle = async () => {
  for (let i = 0; i < 5; i++) await act(async () => { await new Promise((r) => setTimeout(r, 0)); });
};
const button = (focus: string) => host.querySelector<HTMLButtonElement>(`button[data-focus="${focus}"]`)!;
const click = async (el: HTMLElement) => { await act(async () => el.click()); };

type Pending = { resolve: (e: Envelope<unknown>) => void; reject: (e: Error) => void };

function deferredRestores() {
  const inner = fixtureBridge();
  const restores: Pending[] = [];
  const bridge: Bridge = {
    async call<T>(command: ApiCommand, args: ApiArgs): Promise<Envelope<T>> {
      if (command === "restore") {
        return new Promise((resolve, reject) => restores.push({ resolve: resolve as Pending["resolve"], reject }));
      }
      return inner.call<T>(command, args);
    },
  };
  return { bridge, restores };
}

async function openTimeline(bridge: Bridge) {
  await act(async () => { root.render(<App bridge={bridge} />); });
  await settle();
  await click(button("game:celeste"));
  await settle();
}

async function confirmRestore(entry: string) {
  await click(button(`restore:${entry}`));
  await click(button("confirm"));
}

describe("restore in flight (#249 S15)", () => {
  it("a second press while a restore runs does not run a second restore", async () => {
    const { bridge, restores } = deferredRestores();
    await openTimeline(bridge);
    await confirmRestore(SECOND);
    expect(restores).toHaveLength(1);
    expect(button(`restore:${SECOND}`).disabled).toBe(true);
    expect(button(`restore:${THIRD}`).disabled).toBe(true);
    await click(button(`restore:${THIRD}`));
    await click(button(`restore:${SECOND}`));
    expect(host.querySelector('[role="dialog"]')).toBeNull();
    expect(restores).toHaveLength(1);
    await act(async () => restores[0]!.resolve(restoreFx as Envelope<unknown>));
    await settle();
    expect(button(`restore:${SECOND}`).disabled).toBe(false);
  });

  it("a later error replaces the earlier success banner, and a later success clears the error", async () => {
    const { bridge, restores } = deferredRestores();
    await openTimeline(bridge);
    await confirmRestore(SECOND);
    await act(async () => restores[0]!.resolve(restoreFx as Envelope<unknown>));
    await settle();
    expect(host.querySelector(".banner-success, [class*=success]")).not.toBeNull();
    await confirmRestore(THIRD);
    // The old success is gone as soon as the next restore starts.
    expect(host.querySelector(".banner-success, [class*=success]")).toBeNull();
    await act(async () => restores[1]!.reject(new Error("the server said no")));
    await settle();
    expect(host.textContent).toContain("the server said no");
    expect(host.querySelector(".banner-success, [class*=success]")).toBeNull();
    await confirmRestore(THIRD);
    await act(async () => restores[2]!.resolve(restoreFx as Envelope<unknown>));
    await settle();
    expect(host.textContent).not.toContain("the server said no");
  });
});
