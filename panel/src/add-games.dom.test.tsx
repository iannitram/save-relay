// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Envelope, InstallWrapData, SteamGamesData } from "./api.ts";
import type { ApiArgs, ApiCommand, Bridge } from "./bridge.ts";
import { App } from "./App.tsx";
import { fixtureBridge } from "./fixtureBridge.ts";
import steamGames from "./fixtures/steam-games.json";
import { AddGamesView } from "./views.tsx";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

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

const flush = async () => act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
const click = async (selector: string) => {
  await act(async () => host.querySelector<HTMLElement>(selector)!.click());
  await flush();
};
const press = (key: string) => act(() => {
  document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
});

async function openAddGames(call: Bridge["call"]) {
  act(() => root.render(<App bridge={{ call }} />));
  await flush();
  await click('[data-focus="add-games"]');
  await flush();
}

describe("Add Games", () => {
  it("keeps D-pad focus in order and Escape cancels the confirmation", () => {
    let backed = false;
    const onBack = () => { backed = true; };
    act(() => root.render(
      <AddGamesView
        games={(steamGames.data as SteamGamesData).games}
        added={{}}
        loading={false}
        busy={false}
        busyAppId={null}
        error={null}
        notice={null}
        onRefresh={() => {}}
        onAdd={() => {}}
        onRemove={() => {}}
        onAddMany={() => {}}
        onBack={onBack}
      />,
    ));
    expect((document.activeElement as HTMLElement).dataset.focus).toBe("back");
    press("ArrowDown");
    expect((document.activeElement as HTMLElement).dataset.focus).toBe("add-all");
    press("ArrowDown");
    expect((document.activeElement as HTMLElement).dataset.focus).toBe("steam-game:42");
    act(() => host.querySelector<HTMLElement>('[data-focus="add-all"]')!.click());
    expect((document.activeElement as HTMLElement).dataset.focus).toBe("bulk-cancel");
    press("Escape");
    expect(host.querySelector('[role="dialog"]')).toBeNull();
    expect(backed).toBe(false);
  });

  it("confirms only eligible IDs and allows a known-save unknown-cloud game individually", async () => {
    const fixture = fixtureBridge();
    const calls: Array<[ApiCommand, ApiArgs]> = [];
    const call: Bridge["call"] = async <T,>(command: ApiCommand, args: ApiArgs): Promise<Envelope<T>> => {
      calls.push([command, args]);
      return fixture.call<T>(command, args);
    };
    await openAddGames(call);

    expect(host.textContent).toContain("No Steam Cloud");
    expect(host.textContent).toContain("Saves folder not known");
    expect(host.querySelector<HTMLElement>('[data-focus="steam-game:44"]')?.hasAttribute("disabled")).toBe(true);

    await click('[data-focus="add-all"]');
    expect(host.querySelector('[role="dialog"]')?.textContent).toContain("Add 1 game?");
    await click('[data-focus="bulk-confirm"]');
    expect(calls.filter(([command]) => command === "install-wrap").at(-1)?.[1]).toEqual({ "app-ids": [42] });
    expect(host.textContent).toContain("Added 1 game without Steam Cloud.");

    await click('[data-focus="steam-game:43"]');
    expect(calls.filter(([command]) => command === "install-wrap").at(-1)?.[1]).toEqual({ "app-ids": [43] });
    expect(host.textContent).toContain("Added Unknown Game.");
  });

  it("shows the Steam-running recovery copy", async () => {
    const fixture = fixtureBridge();
    const call: Bridge["call"] = async <T,>(command: ApiCommand, args: ApiArgs): Promise<Envelope<T>> => {
      if (command === "install-wrap") {
        return { ok: false, command, error: { code: "steam_running", message: "Steam is running" } } as Envelope<T>;
      }
      return fixture.call<T>(command, args);
    };
    await openAddGames(call);
    await click('[data-focus="steam-game:42"]');
    expect(host.textContent).toContain("Close Steam, then press Add again.");
    expect(host.querySelector('[data-focus="steam-game:42"]')?.textContent).not.toContain("Added");
  });

  it("marks only IDs reported as changed when a browser bulk result is partial", async () => {
    const fixture = fixtureBridge();
    const call: Bridge["call"] = async <T,>(command: ApiCommand, args: ApiArgs): Promise<Envelope<T>> => {
      if (command === "steam-games") {
        const data = await fixture.call<SteamGamesData>(command, args);
        if (!data.ok) return data as Envelope<T>;
        return {
          ...data,
          data: {
            ...data.data,
            games: [...data.data.games, { app_id: 45, name: "Second No Cloud Game", steam_cloud: "no", saves: "ludusavi", wrapped: false }],
          },
        } as Envelope<T>;
      }
      if (command === "install-wrap") {
        const partial: InstallWrapData = {
          dry_run: false,
          changed: [{ app_id: 42, user_id: "123", before: null, after: "save-relay wrap -- %command%" }],
          skipped: [{ app_id: 45, name: "Second No Cloud Game", reason: "no_saves_entry" }],
          backups: [],
          message: "wrapped 1 launch option(s)",
        };
        return { ok: true, command, data: partial } as Envelope<T>;
      }
      return fixture.call<T>(command, args);
    };
    await openAddGames(call);
    await click('[data-focus="add-all"]');
    expect(host.querySelector('[role="dialog"]')?.textContent).toContain("Add 2 games?");
    await click('[data-focus="bulk-confirm"]');
    expect(host.querySelector('[data-focus="steam-game:42"]')?.textContent).toContain("Added");
    expect(host.querySelector('[data-focus="steam-game:45"]')?.textContent).not.toContain("Added");
    expect(host.textContent).toContain("1 of 2 added. Second No Cloud Game: no_saves_entry");
  });
});
