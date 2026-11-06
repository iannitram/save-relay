// @vitest-environment jsdom
// Locked tests for #201: in Game Mode, Steam's capture-phase keydown listener swallows ArrowUp,
// ArrowDown, ArrowLeft, ArrowRight, Escape and Enter before the panel sees them, so the D-pad and B
// must reach the focus scope without a DOM key event. The tests install such a listener on window
// and drive the plugin through the Focusable's onButtonDown, as Steam does.
import { act, forwardRef, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import where from "../../panel/src/fixtures/where.json";
import status from "../../panel/src/fixtures/status-idle.json";
import history from "../../panel/src/fixtures/history.json";
import { App } from "panel/App";
import { fixtureBridge } from "../../panel/src/fixtureBridge.ts";
import * as focusModule from "../../panel/src/focus.ts";
import type { GamepadEvent } from "@decky/ui";

const host = vi.hoisted(() => ({ run: vi.fn(), picture: vi.fn(), button: undefined as undefined | ((e: GamepadEvent) => void) }));
vi.mock("@decky/api", () => ({
  callable: (method: string) => method === "run" ? host.run : host.picture,
  definePlugin: (factory: unknown) => factory,
}));
vi.mock("@decky/ui", () => ({
  PanelSection: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  staticClasses: { Title: "title" },
  GamepadButton: { OK: 1, CANCEL: 2, DIR_UP: 9, DIR_DOWN: 10, DIR_LEFT: 11, DIR_RIGHT: 12 },
  Focusable: forwardRef<HTMLDivElement, { children: ReactNode; className: string; onButtonDown: (e: GamepadEvent) => void }>(
    ({ children, className, onButtonDown }, ref) => {
      host.button = onButtonDown;
      return <div className={className} ref={ref}>{children}</div>;
    },
  ),
}));
import plugin from "./index.tsx";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// The new export (named in the brief): sends a key to the newest focus scope, true when it acted.
// Read through the module namespace so this file compiles before the export exists.
const pressKey = (focusModule as unknown as { pressKey: (key: string) => boolean }).pressKey;

const OK = 1, CANCEL = 2, DIR_UP = 9, DIR_DOWN = 10;

function deepFreeze<T>(value: T): T {
  if (value !== null && typeof value === "object" && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const v of Object.values(value)) deepFreeze(v);
  }
  return value;
}
const fixtures = deepFreeze({ where, status, history });

const STEAM_KEYS = ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Escape", "Enter"];
let seenKeys: string[];
const steam = (event: Event) => {
  const key = (event as KeyboardEvent).key;
  seenKeys.push(key);
  if (STEAM_KEYS.includes(key)) {
    event.stopPropagation();
    event.preventDefault();
  }
};

let container: HTMLElement;
let root: Root;

beforeEach(() => {
  seenKeys = [];
  window.addEventListener("keydown", steam, true);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  host.run.mockImplementation(async (command: string, args: { game?: string }) => {
    const fixture = command === "where" ? fixtures.where : command === "history" ? fixtures.history
      : { ...fixtures.status, data: { ...fixtures.status.data, game: args.game } };
    return JSON.stringify(fixture);
  });
  host.picture.mockResolvedValue(null);
});

afterEach(() => {
  window.removeEventListener("keydown", steam, true);
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

const focused = () => (document.activeElement as HTMLElement | null)?.dataset?.focus ?? null;

async function settle() {
  for (let i = 0; i < 5; i++) {
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });
  }
}

async function press(button: number) {
  const event = new CustomEvent("vgp_onbuttondown", { detail: { button }, cancelable: true }) as GamepadEvent;
  await act(async () => host.button!(event));
  await settle();
}

async function mountPlugin() {
  await act(async () => root.render(plugin().content!));
  await settle();
}

describe("Game Mode buttons reach the focus scope though Steam swallows key events (#201)", () => {
  it("(1) DIR_DOWN moves focus from the first game to the next", async () => {
    await mountPlugin();
    expect(focused()).toBe("game:celeste");
    await press(DIR_DOWN);
    expect(focused()).toBe("game:hades");
  });

  it("(2) DIR_UP moves focus back to the previous game", async () => {
    await mountPlugin();
    await press(DIR_DOWN);
    expect(focused()).toBe("game:hades");
    await press(DIR_UP);
    expect(focused()).toBe("game:celeste");
  });

  it("(3) CANCEL on a game's history goes back to the games view, focus on that game", async () => {
    await mountPlugin();
    await press(OK);
    expect(container.querySelector('[aria-label="Games"]')).toBeNull();
    expect(container.textContent).toContain("Chapter 7: The Summit");
    await press(CANCEL);
    expect(container.querySelector('[aria-label="Games"]')).not.toBeNull();
    expect(focused()).toBe("game:celeste");
  });

  it("(4) OK still clicks the focused control", async () => {
    await mountPlugin();
    await press(DIR_DOWN);
    expect(focused()).toBe("game:hades");
    await press(OK);
    expect(host.run).toHaveBeenCalledWith("history", { game: "hades" });
  });

  it("(5) D-pad, CANCEL and OK dispatch no DOM key events at all", async () => {
    await mountPlugin();
    for (const b of [DIR_DOWN, DIR_UP, OK, CANCEL]) await press(b);
    expect(seenKeys).toEqual([]);
  });
});

describe("pressKey on the focus scope, with no Decky (#201)", () => {
  it("(6) moves focus down and up in a plain App render, and ArrowUp/Escape are handled", async () => {
    await act(async () => root.render(<App bridge={fixtureBridge()} />));
    await settle();
    expect(typeof pressKey).toBe("function");
    expect(focused()).toBe("game:celeste");
    let acted = false;
    await act(async () => {
      acted = pressKey("ArrowDown");
    });
    expect(acted).toBe(true);
    expect(focused()).toBe("game:hades");
    await act(async () => {
      pressKey("ArrowUp");
    });
    expect(focused()).toBe("game:celeste");
  });

  it("(7) Escape through pressKey goes back from a sub-view", async () => {
    await act(async () => root.render(<App bridge={fixtureBridge()} />));
    await settle();
    await act(async () => {
      container.querySelector<HTMLElement>('[data-focus="game:celeste"]')!.click();
    });
    await settle();
    expect(container.querySelector('[aria-label="Games"]')).toBeNull();
    await act(async () => {
      pressKey("Escape");
    });
    expect(container.querySelector('[aria-label="Games"]')).not.toBeNull();
  });

  it("(8) returns false for a key the scope does not handle, and when no view is showing", async () => {
    expect(pressKey("ArrowDown")).toBe(false);
    await act(async () => root.render(<App bridge={fixtureBridge()} />));
    await settle();
    expect(pressKey("q")).toBe(false);
    expect(focused()).toBe("game:celeste");
  });
});
