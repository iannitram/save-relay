// Locked tests for #167 (design pass 2026-11-06): spacing.
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
const px16 = ["16px", "var(--space-4)"];
const px24 = ["24px", "var(--space-5)"];

describe("spacing (#167)", () => {
  it("the screen keeps 32px top and bottom, 48px left and right", () => {
    expect(["32px 48px", "var(--space-6) var(--space-8)"]).toContain(decls(".screen")["padding"]);
  });
  it("the screen puts 24px between the header and what follows", () => {
    expect(px24).toContain(decls(".screen")["gap"]);
  });
  it("the header puts 24px between Back and the title", () => {
    expect(px24).toContain(decls("header")["gap"]);
  });
  it("rows in a list are 16px apart", () => {
    expect(px16).toContain(decls(".list")["gap"]);
  });
  it("footer buttons are 24px apart", () => {
    expect(px24).toContain(decls("footer")["gap"]);
  });
});

describe("spacing #167 leaves alone", () => {
  it("the .cards gap stays 24px", () => {
    expect(px24).toContain(decls(".cards")["gap"]);
  });
});
