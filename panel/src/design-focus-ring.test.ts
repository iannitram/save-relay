// Locked tests for #165 (design pass 2026-11-06): the focus ring.
import { existsSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

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
const fill = (d: Record<string, string>) => d["background-color"] ?? d["background"];

describe("focus ring (#165)", () => {
  for (const sel of [".btn:focus-visible", ".btn[data-focused]"]) {
    it(`${sel} draws a solid 6px ring in the focus token`, () => {
      expect(decls(sel)["outline"]).toBe("6px solid var(--focus)");
    });
    it(`${sel} sets the ring 4px off the control`, () => {
      expect(decls(sel)["outline-offset"]).toBe("4px");
    });
    it(`${sel} fills the control with surface-focus`, () => {
      expect(fill(decls(sel))).toBe("var(--surface-focus)");
    });
  }
  it("no focus rule still uses the old 5px accent ring", () => {
    expect(css.join("\n")).not.toMatch(/outline:\s*5px/);
  });
});

describe("focus ring, kept as asked (#165)", () => {
  const app = css[css.length - 1]!;
  it("keeps both selectors in the one rule", () => {
    const lists = [...app.matchAll(/([^{}]+)\{/g)].map((m) => m[1]!.split(",").map((s) => s.trim().replace(/\s+/g, " ")));
    expect(lists.some((l) => l.includes(".btn:focus-visible") && l.includes(".btn[data-focused]"))).toBe(true);
  });
  it("does not define --focus or --surface-focus in app.css (they come from tokens.css)", () => {
    expect(app).not.toMatch(/--focus\s*:/);
    expect(app).not.toMatch(/--surface-focus\s*:/);
  });
});
