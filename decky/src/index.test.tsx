// @vitest-environment jsdom
import { act, forwardRef, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import where from "../../panel/src/fixtures/where.json";
import status from "../../panel/src/fixtures/status-idle.json";
import history from "../../panel/src/fixtures/history.json";
import setup from "../../panel/src/fixtures/setup-gdrive.json";
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
const container = document.createElement("div");
document.body.appendChild(container);
let root = createRoot(container);
afterEach(() => {
  act(() => root.unmount());
  container.replaceChildren();
  root = createRoot(container);
  vi.clearAllMocks();
});

async function press(button: number) {
  const event = new CustomEvent("vgp_onbuttondown", { detail: { button }, cancelable: true }) as GamepadEvent;
  await act(async () => host.button!(event));
  expect(event.defaultPrevented).toBe(true);
}

it("loads the real panel through callable(run), navigates by controller, and reads pictures", async () => {
  host.run.mockImplementation(async (command: string, args: { game?: string }) => {
    const fixture = command === "where" ? where : command === "history" ? history
      : command === "setup" ? setup : { ...status, data: { ...status.data, game: args.game } };
    return JSON.stringify(fixture);
  });
  host.picture.mockResolvedValue("data:image/png;base64,aW1hZ2U=");
  const entry = plugin();
  expect(entry.name).toBe("Save Relay");
  expect(entry.icon.type).toBe("svg");
  await act(async () => root.render(entry.content!));
  expect(container.textContent).toContain("celeste");
  expect(container.textContent).toContain("hades");
  expect(host.run).toHaveBeenCalledWith("where", {});
  expect(host.run).toHaveBeenCalledWith("status", { game: "celeste" });
  // First game -> timeline -> back; the same App and focus scope as the desktop.
  const game = container.querySelector<HTMLElement>('[data-focus="game:celeste"]')!;
  act(() => game.focus());
  await press(1);
  expect(host.run).toHaveBeenCalledWith("history", { game: "celeste" });
  expect(container.textContent).toContain("Chapter 7: The Summit");
  expect(host.picture).toHaveBeenCalledWith("celeste", history.data.entries[0].entry);
  expect(container.querySelector("img")?.getAttribute("src")).toBe("data:image/png;base64,aW1hZ2U=");
  await press(2);
  expect(container.querySelector('[aria-label="Games"]')).not.toBeNull();
  expect((document.activeElement as HTMLElement).dataset.focus).toBe("game:celeste");
  await press(10);
  expect((document.activeElement as HTMLElement).dataset.focus).toBe("game:hades");
  await press(9);
  expect((document.activeElement as HTMLElement).dataset.focus).toBe("game:celeste");
  const signIn = container.querySelector<HTMLElement>('[data-focus="signin"]')!;
  act(() => signIn.focus());
  await press(1);
  await press(10); // Back -> Google Drive
  await press(1);
  expect(host.run).toHaveBeenCalledWith("setup", { service: "gdrive", "no-browser": true });
  expect(container.querySelector('svg[role="img"]')).not.toBeNull();
  expect(container.querySelector("style")?.textContent).not.toMatch(/(?:^|\})\s*(?:body|header|:root)\s*\{/);
  const scopedCss = container.querySelector("style")?.textContent ?? "";
  expect(scopedCss).toContain(".save-relay-decky[data-theme=\"dark\"]");
  expect(scopedCss).toContain("--surface: #11151b;");
  expect(scopedCss).toContain("font-size: 40px");
  expect(scopedCss).toContain("data:font/ttf;base64,");
  expect(scopedCss).not.toContain("font-size: 18px");
});

it("shows backend errors in the shared games view", async () => {
  host.run.mockResolvedValue('{"ok":false,"command":"where","error":{"code":"io","message":"save-relay binary not found"}}');
  await act(async () => root.render(plugin().content!));
  expect(container.querySelector('[role="alert"]')?.textContent).toBe("save-relay binary not found");
});

it("locks Game Mode style floors at >= 14px font size and >= 40px min-height (#202: the Quick Access scale)", async () => {
  await act(async () => root.render(plugin().content!));
  const scopedCss = container.querySelector("style")?.textContent ?? "";
  const fontSizes = [...scopedCss.matchAll(/font-size:\s*(\d+)px/g)].map((match) => Number(match[1]));
  expect(fontSizes.length).toBeGreaterThan(0);
  expect(fontSizes.every((size) => size >= 14)).toBe(true);

  const minHeights = [...scopedCss.matchAll(/min-height:\s*(\d+)px/g)].map((match) => Number(match[1]));
  expect(minHeights.length).toBeGreaterThan(0);
  expect(minHeights.every((height) => height >= 40)).toBe(true);
});
