// The Game Mode shell: the shared panel owns every view and command through its Bridge.
import { callable, definePlugin } from "@decky/api";
import { Focusable, GamepadButton, PanelSection, staticClasses } from "@decky/ui";
import type { GamepadEvent } from "@decky/ui";
import { useMemo, useRef } from "react";
import { App } from "panel/App";
import type { Envelope, HistoryData } from "panel/api";
import type { ApiArgs, ApiCommand, Bridge } from "panel/bridge";
import { deckyBridge, type RunBackend } from "./bridge.ts";
import panelCss from "../../panel/src/app.css?inline";

const run = callable<[command: ApiCommand, args: ApiArgs], string>("run");
const picture = callable<[game: string, entry: string], string | null>("picture");

// Reuse the panel stylesheet, scoped so its body/header/button rules never style Steam.
const css = panelCss.replace(/\/\*[\s\S]*?\*\//g, "").replace(/([^{}]+)\{/g, (_, selectors: string) =>
  selectors.split(",").map((s) => {
    s = s.trim();
    return s === ":root" || s === "body" ? ".save-relay-decky" : `.save-relay-decky ${s}`;
  }).join(",") + "{",
) + `
.save-relay-decky .screen { min-height: 0; padding: 8px; font-size: 16px; }
.save-relay-decky header, .save-relay-decky footer, .save-relay-decky .row,
.save-relay-decky .entry, .save-relay-decky .signin { flex-wrap: wrap; gap: 12px; }
.save-relay-decky h1 { font-size: 24px; }
.save-relay-decky .big, .save-relay-decky h2 { font-size: 20px; }
.save-relay-decky .btn { font-size: 18px; min-height: 48px; padding: 8px 12px; }
.save-relay-decky .row .name { min-width: 0; }
.save-relay-decky .cards { grid-template-columns: 1fr; }
.save-relay-decky .qr { width: 100%; max-width: 240px; height: auto; }
`;

/** Translate Steam's controller events to the panel's existing DOM focus controls. */
function DeckyPanel({ run, picture }: {
  run: RunBackend;
  picture: (game: string, entry: string) => Promise<string | null>;
}) {
  const root = useRef<HTMLDivElement>(null);
  const { bridge, pictures } = useMemo(() => {
    const cli = deckyBridge(run);
    const pictures = new Map<string, string>();
    const bridge: Bridge = {
      async call<T>(command: ApiCommand, args: ApiArgs): Promise<Envelope<T>> {
        const envelope = await cli.call<T>(command, args);
        if (command === "history" && envelope.ok) {
          const history = envelope.data as HistoryData;
          await Promise.all(history.entries.filter((e) => e.card?.picture).map(async (e) => {
            const key = JSON.stringify([history.game, e.entry]);
            const url = await picture(history.game, e.entry).catch(() => null);
            if (url) pictures.set(key, url);
            else pictures.delete(key);
          }));
        }
        return envelope;
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
    const active = container.ownerDocument.activeElement;
    const target = active instanceof HTMLElement && container.contains(active) && active.dataset.focus
      ? active : container.querySelector<HTMLElement>("[data-focus]:not([disabled])");
    if (!target) return;
    event.preventDefault();
    event.stopPropagation();
    if (key === "Enter") target.click();
    else {
      target.focus();
      target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
    }
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
