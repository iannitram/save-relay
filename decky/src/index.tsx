// The Game Mode shell: the shared panel owns every view and command through its Bridge.
import { callable, definePlugin } from "@decky/api";
import { Focusable, GamepadButton, PanelSection, staticClasses } from "@decky/ui";
import type { GamepadEvent } from "@decky/ui";
import { useMemo, useRef, useState } from "react";
import { App } from "panel/App";
import type { Envelope, HistoryData } from "panel/api";
import type { ApiArgs, ApiCommand, Bridge } from "panel/bridge";
import { deckyBridge, type RunBackend } from "./bridge.ts";
import { readLaunchOption, writeLaunchOption, type SteamLaunchOptionsApi } from "./launch-options.ts";
import { pressKey } from "../../panel/src/focus.ts";
import panelTokens from "../../panel/src/tokens.css?inline";
import panelCss from "../../panel/src/app.css?inline";
import atkinsonFont from "../../panel/src/fonts/AtkinsonHyperlegible-Regular.ttf?asset";
import jetBrainsFont from "../../panel/src/fonts/JetBrainsMono[wght].ttf?asset";

const run = callable<[command: ApiCommand, args: ApiArgs], string>("run");
const picture = callable<[game: string, entry: string], string | null>("picture");

function steamLaunchOptionsApi(): SteamLaunchOptionsApi | null {
  if (typeof SteamClient === "undefined" || !SteamClient.Apps) return null;
  return SteamClient.Apps;
}

// Reuse the panel stylesheet, scoped so its body/header/button rules never style Steam.
const scoped = (source: string) => source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/([^{}]+)\{/g, (_, selectors: string) =>
  selectors.trim() === ":root, [data-theme=\"dark\"]"
    ? ".save-relay-decky, .save-relay-decky[data-theme=\"dark\"]{"
    : selectors.split(",").map((selector: string) => {
    const s = selector.trim();
    if (s.startsWith("@")) return s;
    if (s === ":root") return ".save-relay-decky";
    if (s === "[data-theme=\"light\"]") return ".save-relay-decky[data-theme=\"light\"]";
    return s === "body" ? ".save-relay-decky" : `.save-relay-decky ${s}`;
  }).join(",") + "{",
);
const css = scoped(panelTokens + panelCss)
  .replace(/[^\s"')]*AtkinsonHyperlegible-Regular\.ttf/, atkinsonFont)
  .replace(/[^\s"')]*JetBrainsMono\[wght\]\.ttf/, jetBrainsFont) + `
.save-relay-decky .screen { min-height: 0; padding: 8px; }
.save-relay-decky header, .save-relay-decky footer, .save-relay-decky .row,
.save-relay-decky .entry, .save-relay-decky .signin { flex-wrap: wrap; gap: 12px; }
.save-relay-decky .btn { padding: 8px 12px; }
.save-relay-decky .row .name { min-width: 0; }
.save-relay-decky .cards { grid-template-columns: 1fr; }
.save-relay-decky .qr { width: 100%; max-width: 240px; height: auto; }
.save-relay-decky h1, .save-relay-decky .title, .save-relay-decky .heading { font-size: 20px; line-height: 26px; font-weight: 700; }
.save-relay-decky .body, .save-relay-decky .body-lg, .save-relay-decky .big, .save-relay-decky .label,
.save-relay-decky .row .name, .save-relay-decky .btn, .save-relay-decky .dialog p { font-size: 16px; line-height: 22px; }
.save-relay-decky .btn { min-height: 40px; }
.save-relay-decky .meta, .save-relay-decky .row .meta, .save-relay-decky .badge,
.save-relay-decky .qr-label, .save-relay-decky .address { font-size: 14px; line-height: 20px; }
.save-relay-decky .code { font-size: 28px; line-height: 34px; }
.save-relay-decky .dialog h2 { font-size: 20px; line-height: 26px; font-weight: 700; }
.save-relay-decky .save { flex-direction: column; align-items: stretch; }
.save-relay-decky .picture { width: 100%; height: auto; aspect-ratio: 16 / 9; }
.save-relay-decky .scrim { padding: 8px; }
.save-relay-decky .dialog { width: auto; max-width: 100%; padding: 12px; }
.save-relay-decky .dialog-actions { flex-wrap: wrap; }
.save-relay-decky .qr-box { width: 100%; height: auto; aspect-ratio: 1 / 1; }
.save-relay-decky .row { flex-direction: column; align-items: flex-start; }
`;

/** Translate Steam's controller events to the panel's existing DOM focus controls. */
function DeckyPanel({ run, picture }: {
  run: RunBackend;
  picture: (game: string, entry: string) => Promise<string | null>;
}) {
  const root = useRef<HTMLDivElement>(null);
  // Bumped when a picture arrives, so the timeline draws it then (it does not wait for them).
  const [, setArrived] = useState(0);
  const { bridge, pictures } = useMemo(() => {
    const cli = deckyBridge(run);
    const pictures = new Map<string, string>();
    const bridge: Bridge = {
      async call<T>(command: ApiCommand, args: ApiArgs): Promise<Envelope<T>> {
        const envelope = await cli.call<T>(command, args);
        if (command === "history" && envelope.ok) {
          const history = envelope.data as HistoryData;
          for (const e of history.entries.filter((e) => e.card?.picture)) {
            const key = JSON.stringify([history.game, e.entry]);
            void picture(history.game, e.entry).catch(() => null).then((url) => {
              if (url) pictures.set(key, url);
              else pictures.delete(key);
              setArrived((n) => n + 1);
            });
          }
        }
        return envelope;
      },
      async getLaunchOption(appId: number): Promise<string> {
        const apps = steamLaunchOptionsApi();
        if (!apps) throw new Error("Steam app details are unavailable in this session.");
        return readLaunchOption(apps, appId);
      },
      async setLaunchOption(appId: number, text: string): Promise<void> {
        const apps = steamLaunchOptionsApi();
        if (!apps) throw new Error("Steam cannot set launch options in this session. Add games from Desktop Mode with Steam closed.");
        await writeLaunchOption(apps, appId, text);
      },
    };
    return { bridge, pictures };
  }, [run, picture]);

  const button = (event: GamepadEvent) => {
    const keys: Partial<Record<GamepadButton, string>> = {
      [GamepadButton.DIR_UP]: "ArrowUp", [GamepadButton.DIR_DOWN]: "ArrowDown",
      [GamepadButton.DIR_LEFT]: "ArrowLeft", [GamepadButton.DIR_RIGHT]: "ArrowRight",
      [GamepadButton.CANCEL]: "Escape", [GamepadButton.OK]: "Enter",
    };
    const key = keys[event.detail.button];
    const container = root.current;
    if (!key || !container) return;
    event.preventDefault();
    event.stopPropagation();
    if (key === "Enter") {
      const active = container.ownerDocument.activeElement;
      const target = active instanceof HTMLElement && container.contains(active) && active.dataset.focus
        ? active : container.querySelector<HTMLElement>("[data-focus]:not([disabled])");
      target?.click();
    } else pressKey(key);
  };
  return (
    <Focusable className="save-relay-decky" ref={root} onButtonDown={button}>
      <style>{css}</style>
      <App bridge={bridge} pictureUrl={(game, entry) => pictures.get(JSON.stringify([game, entry])) ?? null} />
    </Focusable>
  );
}

export default definePlugin(() => ({
  name: "Save Relay",
  titleView: <div className={staticClasses.Title}>Save Relay</div>,
  content: <PanelSection><DeckyPanel run={run} picture={picture} /></PanelSection>,
  icon: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-label="Save Relay">
    <path d="M5 4h14v16H5zM8 4v6h8V4M8 20v-6h8v6" />
  </svg>,
}));
