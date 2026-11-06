// @vitest-environment jsdom
// Locked tests for #175 (design pass 2026-11-06): Restore confirm becomes a ConfirmDialog over history.
import { existsSync, readFileSync } from "node:fs";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { HistoryData } from "./api.ts";
import historyFx from "./fixtures/history.json";
import { TimelineView } from "./views.tsx";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const files = ["./tokens.css", "./app.css"].map((f) => new URL(f, import.meta.url)).filter((u) => existsSync(u));
const css = Object.freeze(files.map((u) => readFileSync(u, "utf8").replace(/\/\*[\s\S]*?\*\//g, "")));

/** Every declaration of the rules whose selector list holds `selector` (tokens.css, then app.css; later wins). */
function decls(selector: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const text of css) {
    for (const m of text.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
      if (!m[1]!.split(",").map((s) => s.trim().replace(/\s+/g, " ")).includes(selector)) continue;
      for (const d of m[2]!.split(";")) {
        const i = d.indexOf(":");
        if (i > 0) out[d.slice(0, i).trim()] = d.slice(i + 1).trim().replace(/\s+/g, " ");
      }
    }
  }
  return out;
}

const history = Object.freeze(structuredClone(historyFx.data)) as HistoryData;
const e1 = history.entries[1]!.entry;
const noop = () => undefined;

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

function pressKey(target: EventTarget, key: string, shiftKey = false) {
  act(() => {
    target.dispatchEvent(new KeyboardEvent("keydown", { key, shiftKey, bubbles: true, cancelable: true }));
  });
}

describe("ConfirmDialog styles (#175)", () => {
  it(".scrim covers screen with var(--scrim) background and var(--space-8) padding", () => {
    const d = decls(".scrim");
    expect(d["position"]).toBe("absolute");
    expect(d["inset"]).toBe("0");
    expect(d["background"]).toBe("var(--scrim)");
    expect(d["display"]).toBe("flex");
    expect(d["align-items"]).toBe("center");
    expect(d["justify-content"]).toBe("center");
    expect(["48px", "var(--space-8)"]).toContain(d["padding"]);
    expect(d["z-index"]).toBe("10");
  });

  it(".dialog is 760px wide, raised surface, 20px radius, 2px border, shadow-dialog", () => {
    const d = decls(".dialog");
    expect(d["width"]).toBe("760px");
    expect(d["max-width"]).toBe("100%");
    expect(["32px", "var(--space-6)"]).toContain(d["padding"]);
    expect(d["background"]).toBe("var(--surface-raised)");
    expect(d["color"]).toBe("var(--ink)");
    expect(d["border"]).toBe("2px solid var(--border)");
    expect(["20px", "var(--radius-lg)"]).toContain(d["border-radius"]);
    expect(d["box-shadow"]).toBe("var(--shadow-dialog)");
    expect(["12px", "var(--space-3)"]).toContain(d["gap"]);
  });

  it(".dialog h2 is 30px bold in ink", () => {
    const d = decls(".dialog h2");
    expect(d["font-size"]).toBe("30px");
    expect(d["line-height"]).toBe("38px");
    expect(d["font-weight"]).toBe("700");
    expect(d["color"]).toBe("var(--ink)");
  });

  it(".dialog-actions separates buttons by 24px (var(--space-5))", () => {
    const d = decls(".dialog-actions");
    expect(d["display"]).toBe("flex");
    expect(["24px", "var(--space-5)"]).toContain(d["gap"]);
  });

  it("does not use hardcoded hex colors or font sizes under 20px in dialog rules", () => {
    const app = css[css.length - 1]!;
    const dialogSection = app.slice(app.indexOf(".scrim"));
    expect(dialogSection).not.toMatch(/#[0-9a-fA-F]{3,8}/);
    const fontSizes = [...dialogSection.matchAll(/font-size:\s*(\d+)px/g)].map((m) => Number(m[1]));
    for (const size of fontSizes) {
      expect(size).toBeGreaterThanOrEqual(20);
    }
  });
});

describe("ConfirmDialog structure and focus (#175)", () => {
  it("renders the dialog role and aria-modal, with the list still rendered beneath", () => {
    act(() => {
      root.render(
        <TimelineView
          history={history}
          pictureUrl={() => null}
          confirm={e1}
          error={null}
          notice={null}
          onAsk={noop}
          onConfirm={noop}
          onCancel={noop}
          onBack={noop}
        />,
      );
    });

    const dialog = host.querySelector<HTMLElement>('[role="dialog"]')!;
    expect(dialog).not.toBeNull();
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(dialog.getAttribute("aria-label")).toBe("Restore these saves?");
    expect(dialog.parentElement?.classList.contains("scrim")).toBe(true);

    const list = host.querySelector<HTMLUListElement>("ul.list")!;
    expect(list).not.toBeNull();
    const cards = list.querySelectorAll(".save");
    expect(cards.length).toBe(history.entries.length);
  });

  it("has Cancel first and Restore second, with Restore primary and 24px apart", () => {
    act(() => {
      root.render(
        <TimelineView
          history={history}
          pictureUrl={() => null}
          confirm={e1}
          error={null}
          notice={null}
          onAsk={noop}
          onConfirm={noop}
          onCancel={noop}
          onBack={noop}
        />,
      );
    });

    const dialog = host.querySelector<HTMLElement>('[role="dialog"]')!;
    const actions = dialog.querySelector<HTMLElement>(".dialog-actions")!;
    expect(actions).not.toBeNull();
    const buttons = Array.from(actions.querySelectorAll<HTMLButtonElement>("button"));
    expect(buttons).toHaveLength(2);

    expect(buttons[0].dataset.focus).toBe("cancel");
    expect(buttons[0].textContent).toBe("Cancel");
    expect(buttons[0].classList.contains("btn")).toBe(true);
    expect(buttons[0].classList.contains("btn-primary")).toBe(false);

    expect(buttons[1].dataset.focus).toBe("confirm");
    expect(buttons[1].textContent).toBe("Restore");
    expect(buttons[1].classList.contains("btn")).toBe(true);
    expect(buttons[1].classList.contains("btn-primary")).toBe(true);
  });

  it("Cancel has the first focus when dialog opens", () => {
    act(() => {
      root.render(
        <TimelineView
          history={history}
          pictureUrl={() => null}
          confirm={e1}
          error={null}
          notice={null}
          onAsk={noop}
          onConfirm={noop}
          onCancel={noop}
          onBack={noop}
        />,
      );
    });

    const dialog = host.querySelector<HTMLElement>('[role="dialog"]')!;
    const cancel = dialog.querySelector<HTMLButtonElement>('button[data-focus="cancel"]')!;
    expect(document.activeElement).toBe(cancel);
    expect(cancel.hasAttribute("data-focused")).toBe(true);
  });

  it("focus cannot leave the dialog via Tab or D-pad", () => {
    act(() => {
      root.render(
        <TimelineView
          history={history}
          pictureUrl={() => null}
          confirm={e1}
          error={null}
          notice={null}
          onAsk={noop}
          onConfirm={noop}
          onCancel={noop}
          onBack={noop}
        />,
      );
    });

    const dialog = host.querySelector<HTMLElement>('[role="dialog"]')!;
    const cancel = dialog.querySelector<HTMLButtonElement>('button[data-focus="cancel"]')!;
    const restore = dialog.querySelector<HTMLButtonElement>('button[data-focus="confirm"]')!;

    expect(document.activeElement).toBe(cancel);

    // Tab moves to Restore
    pressKey(cancel, "Tab");
    expect(document.activeElement).toBe(restore);

    // Tab wraps to Cancel
    pressKey(restore, "Tab");
    expect(document.activeElement).toBe(cancel);

    // Shift+Tab wraps to Restore
    pressKey(cancel, "Tab", true);
    expect(document.activeElement).toBe(restore);

    // ArrowRight / ArrowDown moves to Cancel
    pressKey(restore, "ArrowRight");
    expect(document.activeElement).toBe(cancel);

    pressKey(cancel, "ArrowDown");
    expect(document.activeElement).toBe(restore);

    // ArrowLeft / ArrowUp moves to Cancel
    pressKey(restore, "ArrowUp");
    expect(document.activeElement).toBe(cancel);

    pressKey(cancel, "ArrowLeft");
    expect(document.activeElement).toBe(restore);

    // Back button and list restore buttons under scrim are never focused
    const back = host.querySelector<HTMLButtonElement>('header button');
    expect(document.activeElement).not.toBe(back);
  });

  it("Escape calls onCancel", () => {
    let cancelled = 0;
    act(() => {
      root.render(
        <TimelineView
          history={history}
          pictureUrl={() => null}
          confirm={e1}
          error={null}
          notice={null}
          onAsk={noop}
          onConfirm={noop}
          onCancel={() => { cancelled += 1; }}
          onBack={noop}
        />,
      );
    });

    const cancel = host.querySelector<HTMLButtonElement>('button[data-focus="cancel"]')!;
    pressKey(cancel, "Escape");
    expect(cancelled).toBe(1);
  });

  it("Restore calls onConfirm", () => {
    let confirmed = 0;
    act(() => {
      root.render(
        <TimelineView
          history={history}
          pictureUrl={() => null}
          confirm={e1}
          error={null}
          notice={null}
          onAsk={noop}
          onConfirm={() => { confirmed += 1; }}
          onCancel={noop}
          onBack={noop}
        />,
      );
    });

    const restoreBtn = host.querySelector<HTMLButtonElement>('button[data-focus="confirm"]')!;
    act(() => restoreBtn.click());
    expect(confirmed).toBe(1);
  });
});
