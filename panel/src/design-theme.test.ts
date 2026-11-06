// Locked tests for #177 (design pass 2026-11-06): the light theme follows the system setting.
import { existsSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { applyTheme, themeFor } from "./theme.ts";

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

describe("themeFor (#177)", () => {
  it("is light when the system prefers light", () => {
    expect(themeFor(true)).toBe("light");
  });
  it("is dark otherwise", () => {
    expect(themeFor(false)).toBe("dark");
  });
});

describe("applyTheme (#177)", () => {
  it("writes data-theme on the root it is given", () => {
    const root = { dataset: {} as Record<string, string | undefined> };
    applyTheme(root, true);
    expect(root.dataset.theme).toBe("light");
    applyTheme(root, false);
    expect(root.dataset.theme).toBe("dark");
  });
});

describe("main.tsx (#177)", () => {
  const main = readFileSync(new URL("./main.tsx", import.meta.url), "utf8");
  it("reads the system setting", () => {
    expect(main).toContain("(prefers-color-scheme: light)");
  });
  it("applies the theme to the document root and again when the setting changes", () => {
    expect(main).toMatch(/applyTheme\(\s*document\.documentElement/);
    expect(main).toMatch(/addEventListener\(\s*["']change["']/);
  });
});

describe("styles (#177)", () => {
  it("dark is the default color scheme", () => {
    expect(decls(":root")["color-scheme"]).toBe("dark");
  });
  it("the light theme switches the color scheme to light", () => {
    expect(decls('[data-theme="light"]')["color-scheme"]).toBe("light");
  });
  it("the light rule comes after :root in app.css, so it wins", () => {
    const app = css[css.length - 1]!;
    expect(app.lastIndexOf('[data-theme="light"]')).toBeGreaterThan(app.indexOf(":root"));
  });
  it("the light theme has its own surface, ink and focus tokens", () => {
    const d = decls('[data-theme="light"]');
    expect(d["--surface"]).toBe("#f5f3ee");
    expect(d["--ink"]).toBe("#12161c");
    expect(d["--focus"]).toBe("#a83800");
  });
});
