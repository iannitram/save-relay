// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import { App } from "./App.tsx";
import { fixtureBridge } from "./fixtureBridge.ts";
import type { ApiArgs, ApiCommand, Bridge } from "./bridge.ts";
import type { Envelope } from "./api.ts";
import restored from "./fixtures/restore.json";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

it("blocks double presses and new restores in flight, then replaces earlier result banners", async () => {
  const inner = fixtureBridge();
  let calls = 0;
  let finish!: (reply: Envelope<unknown>) => void;
  const bridge: Bridge = {
    call<T>(command: ApiCommand, args: ApiArgs) {
      if (command !== "restore") return inner.call<T>(command, args);
      calls++;
      return new Promise<Envelope<T>>((resolve) => { finish = (reply) => resolve(reply as Envelope<T>); });
    },
  };
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  const buttons = () => [...host.querySelectorAll<HTMLButtonElement>('button[data-focus^="restore:"]')];
  const click = async (selector: string) => act(async () => {
    const button = host.querySelector<HTMLButtonElement>(selector);
    expect(button).not.toBeNull();
    button!.click();
  });
  const confirm = async () => {
    await act(async () => { buttons()[0]!.click(); });
    await act(async () => {
      const button = host.querySelector<HTMLButtonElement>('button[data-focus="confirm"]')!;
      button.click();
      button.click();
    });
  };
  try {
    await act(async () => { root.render(<App bridge={bridge} />); });
    await click('button[data-focus="game:celeste"]');
    await confirm();
    expect(calls).toBe(1);
    expect(buttons().length).toBeGreaterThan(0);
    expect(buttons().every((b) => b.disabled)).toBe(true);
    await act(async () => { buttons()[0]!.click(); });
    expect(host.querySelector('[role="dialog"]')).toBeNull();
    expect(calls).toBe(1);
    await act(async () => { finish({ ...restored, ok: true }); });
    expect(buttons().every((b) => !b.disabled)).toBe(true);
    expect(host.querySelector(".banner-success")?.textContent).toContain(restored.data.message);
    await confirm();
    expect(host.querySelector(".banner-success")).toBeNull();
    await act(async () => { finish({ ok: false, command: "restore", error: { code: "server", message: "Later restore failed" } }); });
    expect(host.querySelector(".banner-success")).toBeNull();
    expect(host.querySelector(".banner-danger")?.textContent).toContain("Later restore failed");
    expect(buttons().every((b) => !b.disabled)).toBe(true);
    await confirm();
    expect(host.querySelector(".banner-danger")).toBeNull();
    await act(async () => { finish({ ...restored, ok: true }); });
    expect(host.querySelector(".banner-success")?.textContent).toContain(restored.data.message);
    expect(host.querySelector(".banner-danger")).toBeNull();
  } finally {
    act(() => root.unmount());
    host.remove();
  }
});
