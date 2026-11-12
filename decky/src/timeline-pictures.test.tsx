// @vitest-environment jsdom
import { act, forwardRef, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import where from "../../panel/src/fixtures/where.json";
import status from "../../panel/src/fixtures/status-idle.json";
import history from "../../panel/src/fixtures/history.json";
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

// #250 item 12: the Game Mode timeline renders without waiting for its pictures.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

it("renders the timeline while a picture is still loading, then shows it", async () => {
  host.run.mockImplementation(async (command: string, args: { game?: string }) => {
    const fixture = command === "where" ? where : command === "history" ? history
      : { ...status, data: { ...status.data, game: args.game } };
    return JSON.stringify(fixture);
  });
  let finish: (url: string) => void = () => {};
  host.picture.mockReturnValue(new Promise<string>((resolve) => { finish = resolve; }));
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  await act(async () => root.render(plugin().content!));
  const game = container.querySelector<HTMLElement>('[data-focus="game:celeste"]')!;
  act(() => game.focus());
  const event = new CustomEvent("vgp_onbuttondown", { detail: { button: 1 }, cancelable: true }) as GamepadEvent;
  await act(async () => host.button!(event));
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });

  expect(container.textContent).toContain("Chapter 7: The Summit");
  expect(container.querySelector("img")).toBeNull();

  await act(async () => finish("data:image/png;base64,aW1hZ2U="));
  expect(container.querySelector("img")?.getAttribute("src")).toBe("data:image/png;base64,aW1hZ2U=");
  act(() => root.unmount());
});
