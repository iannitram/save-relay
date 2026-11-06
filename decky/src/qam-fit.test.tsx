// @vitest-environment jsdom
// Locked tests for #202 (design pass 2026-11-06): the Decky panel is 268 CSS px wide in the Quick
// Access menu, so the Decky-only CSS must replace the full-screen type scale and fix four layouts.
// The tests render the real plugin, read the stylesheet it injects (the <style> inside the panel),
// and check what the LAST rule for each `.save-relay-decky <selector>` sets, so a rule that is
// added but then overridden by a later rule fails. They hold wherever the fix lives in decky/src.
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { act, forwardRef, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { beforeAll, describe, expect, it, vi } from "vitest";

vi.mock("@decky/api", () => ({
  // run never answers: only the injected stylesheet is under test.
  callable: () => () => new Promise(() => {}),
  definePlugin: (factory: unknown) => factory,
}));
vi.mock("@decky/ui", () => ({
  PanelSection: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  staticClasses: { Title: "title" },
  GamepadButton: { OK: 1, CANCEL: 2, DIR_UP: 9, DIR_DOWN: 10, DIR_LEFT: 11, DIR_RIGHT: 12 },
  Focusable: forwardRef<HTMLDivElement, { children: ReactNode; className: string }>(
    ({ children, className }, ref) => <div className={className} ref={ref}>{children}</div>,
  ),
}));
import plugin from "./index.tsx";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

type Rule = { selectors: string[]; decls: Map<string, string> };
let rules: Rule[] = [];

/** A small parser for flat CSS (the panel stylesheet has no @media/@supports nesting). */
function parse(css: string): Rule[] {
  const out: Rule[] = [];
  const body = css.replace(/\/\*[\s\S]*?\*\//g, "");
  for (const m of body.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const head = m[1]!.trim();
    if (head.startsWith("@")) continue;
    const decls = new Map<string, string>();
    for (const d of m[2]!.split(";")) {
      const i = d.indexOf(":");
      if (i > 0) decls.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim().replace(/\s+/g, " "));
    }
    out.push({ selectors: head.split(",").map((s) => s.trim().replace(/\s+/g, " ")), decls });
  }
  return out;
}

/** Longhands a `font` / `padding` shorthand is not allowed to hide: tests ask for these directly. */
function value(selector: string, property: string): string | undefined {
  const full = `.save-relay-decky ${selector}`;
  let found: string | undefined;
  for (const r of rules) if (r.selectors.includes(full) && r.decls.has(property)) found = r.decls.get(property);
  return found;
}

const px = (v: string | undefined) => (v && /^-?\d+(\.\d+)?px$/.test(v) ? parseFloat(v) : NaN);
const norm = (v: string | undefined) => v?.replace(/\s*\/\s*/g, " / ");

beforeAll(async () => {
  const host = document.createElement("div");
  document.body.appendChild(host);
  await act(async () => createRoot(host).render(plugin().content!));
  const style = host.querySelector("style");
  expect(style).not.toBeNull();
  rules = parse(style!.textContent ?? "");
});

/** font-size / line-height / weight of a selector, as the last rule sets them. */
function type(selector: string, size: string, line: string, weight?: string) {
  expect(value(selector, "font-size")).toBe(size);
  expect(value(selector, "line-height")).toBe(line);
  if (weight) expect(value(selector, "font-weight")).toBe(weight);
}

describe("Decky type scale (Expectations 1-6)", () => {
  it("(1) view title h1, .title and .heading are 20px/26px bold", () => {
    for (const s of ["h1", ".title", ".heading"]) type(s, "20px", "26px", "700");
  });

  it("(2) body text .body, .body-lg, .big, .label and .row .name is 16px/22px", () => {
    for (const s of [".body", ".body-lg", ".big", ".label", ".row .name"]) type(s, "16px", "22px");
  });

  it("(3) .btn text is 16px/22px with min-height 40px", () => {
    type(".btn", "16px", "22px");
    expect(value(".btn", "min-height")).toBe("40px");
  });

  it("(4) meta text .meta, .row .meta, .badge, .qr-label and .address is 14px/20px", () => {
    for (const s of [".meta", ".row .meta", ".badge", ".qr-label", ".address"]) type(s, "14px", "20px");
  });

  it("(5) the sign-in .code is 28px mono", () => {
    expect(value(".code", "font-size")).toBe("28px");
    expect(value(".code", "line-height")).toBe("34px");
    // The font family stays the mono token (tokens.css), not overridden to a sans face.
    expect(value(".code", "font-family") ?? "var(--font-mono)").toContain("mono");
  });

  it("(6) the restore confirm: .dialog h2 is 20px/26px bold and .dialog p is 16px/22px", () => {
    type(".dialog h2", "20px", "26px", "700");
    type(".dialog p", "16px", "22px");
  });
});

describe("Decky layouts (Expectations 7-11)", () => {
  it("(7) .save stacks: column, children stretched", () => {
    expect(value(".save", "flex-direction")).toBe("column");
    expect(value(".save", "align-items")).toBe("stretch");
  });

  it("(8) .picture is full width with an automatic height from a 16 / 9 aspect ratio", () => {
    expect(value(".picture", "width")).toBe("100%");
    expect(value(".picture", "height")).toBe("auto");
    expect(norm(value(".picture", "aspect-ratio"))).toBe("16 / 9");
  });

  it("(9) .scrim padding is at most 8px and .dialog fits: width auto, max-width 100%, padding at most 16px", () => {
    expect(px(value(".scrim", "padding"))).toBeLessThanOrEqual(8);
    expect(value(".dialog", "width")).toBe("auto");
    expect(value(".dialog", "max-width")).toBe("100%");
    expect(px(value(".dialog", "padding"))).toBeLessThanOrEqual(16);
  });

  it("(10) .dialog-actions wraps", () => {
    expect(value(".dialog-actions", "flex-wrap")).toBe("wrap");
  });

  it("(11) .qr-box is at most the panel width and square, and .qr keeps its 240px cap", () => {
    expect(value(".qr-box", "width")).toBe("100%");
    expect(value(".qr-box", "height")).toBe("auto");
    expect(norm(value(".qr-box", "aspect-ratio"))).toBe("1 / 1");
    expect(value(".qr", "max-width")).toBe("240px");
  });

  it("(12) .row puts the status under the name: column", () => {
    expect(value(".row", "flex-direction")).toBe("column");
    expect(value(".row", "align-items")).toBe("flex-start");
  });
});

describe("Decky overrides that must stay, and the desktop panel stays as it is", () => {
  it("keeps the earlier overrides: .screen padding, .cards one column", () => {
    expect(value(".screen", "padding")).toBe("8px");
    expect(value(".cards", "grid-template-columns")).toBe("1fr");
  });

  it("panel/src/app.css and tokens.css are byte-identical to main 68babb8 (line endings ignored)", () => {
    const sha = (rel: string) =>
      createHash("sha256")
        .update(readFileSync(new URL(rel, import.meta.url), "utf8").replace(/\r\n/g, "\n"))
        .digest("hex");
    expect(sha("../../panel/src/app.css")).toBe("ee4e8931afafe0a2134f68c6651bbd45288cba2afd65db3bc9cef68d730f6266");
    expect(sha("../../panel/src/tokens.css")).toBe("86de0e9afdbd2678f926c684c466dd94c28f69c0a9e1effc36c2a112377e344d");
  });
});
