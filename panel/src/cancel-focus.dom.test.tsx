// @vitest-environment jsdom
// Locked tests for #197 (design pass 2026-11-06): after the restore confirm is cancelled (Cancel,
// or Escape, which is B on the controller: focus.ts maps B to Escape), focus goes back to the
// Restore button of the entry that opened the confirm, not to Back. The tests drive the whole
// App over a fixture bridge, so they hold wherever the fix lives.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { ApiArgs, ApiCommand, Bridge } from "./bridge.ts";
import type { Envelope } from "./api.ts";
import historyFx from "./fixtures/history.json";
import { App } from "./App.tsx";
import { fixtureBridge } from "./fixtureBridge.ts";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function deepFreeze<T>(value: T): T {
  if (value !== null && typeof value === "object" && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const v of Object.values(value)) deepFreeze(v);
  }
  return value;
}

/** The history envelope with `index` as its only current entry (a current entry has no Restore). */
function historyEnvelope(index: number) {
  const copy = structuredClone(historyFx) as typeof historyFx;
  copy.data.entries.forEach((e, i) => {
    e.current = i === index;
  });
  copy.data.current = copy.data.entries[index]!.entry;
  return deepFreeze(copy);
}

// history.json: entries[0] is current (no Restore); entries[1] and entries[2] have Restore buttons.
const SECOND = historyFx.data.entries[1]!.entry; // 20261030T210500Z-amd-handheld
const FIRST_NOT_CURRENT = historyFx.data.entries[0]!.entry; // 20261031T195100Z-steam-deck, opened when entries[2] is current

type Call = { command: string; args: ApiArgs };

/** The fixture bridge, with `history` answering with `current` as the current entry. */
function bridgeWith(current: number, calls: Call[]): Bridge {
  const inner = fixtureBridge();
  const envelope = historyEnvelope(current);
  return {
    async call<T>(command: ApiCommand, args: ApiArgs): Promise<Envelope<T>> {
      calls.push({ command, args });
      if (command === "history") {
        return { ...envelope, data: { ...envelope.data, game: args.game } } as unknown as Envelope<T>;
      }
      return inner.call<T>(command, args);
    },
  };
}

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

const focused = () => (document.activeElement as HTMLElement | null)?.dataset?.focus ?? null;

function button(focus: string): HTMLButtonElement {
  const el = host.querySelector<HTMLButtonElement>(`button[data-focus="${focus}"]`);
  if (!el) throw new Error(`no control data-focus="${focus}"`);
  return el;
}

function restoreOf(entry: string): HTMLButtonElement {
  return button(`restore:${entry}`);
}

function dialog(): HTMLElement | null {
  return host.querySelector<HTMLElement>('[role="dialog"]');
}

/** Lets the fixture bridge's promises and the App's loads finish. */
async function settle() {
  for (let i = 0; i < 5; i++) {
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });
  }
}

async function press(target: EventTarget, key: string) {
  await act(async () => {
    target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
  });
}

/** Opens celeste's history from the games view, then the confirm from `entry`'s Restore, as clicks do. */
async function openConfirm(bridge: Bridge, entry: string) {
  await act(async () => {
    root.render(<App bridge={bridge} />);
  });
  await settle();
  await act(async () => {
    button("game:celeste").click();
  });
  await settle();
  await act(async () => {
    restoreOf(entry).click();
  });
  expect(dialog()).not.toBeNull();
  expect(focused()).toBe("cancel");
}

describe("cancelling the restore confirm returns focus to its Restore (#197)", () => {
  it("(a) Cancel click from the second entry's Restore leaves focus on restore:<entry>", async () => {
    await openConfirm(bridgeWith(0, []), SECOND);
    await act(async () => {
      button("cancel").click();
    });
    expect(dialog()).toBeNull();
    expect(focused()).toBe(`restore:${SECOND}`);
    expect(document.activeElement).toBe(restoreOf(SECOND));
  });

  it("(b) Escape from the second entry's Restore leaves focus on restore:<entry>", async () => {
    await openConfirm(bridgeWith(0, []), SECOND);
    await press(document.activeElement!, "Escape");
    expect(dialog()).toBeNull();
    expect(focused()).toBe(`restore:${SECOND}`);
    expect(document.activeElement).toBe(restoreOf(SECOND));
  });

  it("(d) opened from the first entry, Cancel click returns focus to that entry's Restore", async () => {
    await openConfirm(bridgeWith(2, []), FIRST_NOT_CURRENT);
    await act(async () => {
      button("cancel").click();
    });
    expect(dialog()).toBeNull();
    expect(focused()).toBe(`restore:${FIRST_NOT_CURRENT}`);
    expect(document.activeElement).toBe(restoreOf(FIRST_NOT_CURRENT));
  });

  it("(d) opened from the first entry, Escape returns focus to that entry's Restore", async () => {
    await openConfirm(bridgeWith(2, []), FIRST_NOT_CURRENT);
    await press(document.activeElement!, "Escape");
    expect(dialog()).toBeNull();
    expect(focused()).toBe(`restore:${FIRST_NOT_CURRENT}`);
    expect(document.activeElement).toBe(restoreOf(FIRST_NOT_CURRENT));
  });

  it("(e) confirming still sends the restore for the entry that opened the confirm", async () => {
    const calls: Call[] = [];
    await openConfirm(bridgeWith(0, calls), SECOND);
    await act(async () => {
      button("confirm").click();
    });
    await settle();
    expect(calls.filter((c) => c.command === "restore")).toEqual([
      { command: "restore", args: { game: "celeste", entry: SECOND } },
    ]);
  });
});
