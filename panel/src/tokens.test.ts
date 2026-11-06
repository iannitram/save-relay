import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const tokens = readFileSync(new URL("./tokens.css", import.meta.url), "utf8");
const appCss = readFileSync(new URL("./app.css", import.meta.url), "utf8");

describe("shared design tokens and type scale", () => {
  it("provides the system type scale and theme tokens", () => {
    expect(tokens).toContain("--surface: #11151b;");
    expect(tokens).toContain("--font-sans: \"Atkinson Hyperlegible\"");
    expect(tokens).toContain("--font-mono: \"JetBrains Mono\"");
    expect(tokens).toContain(".body { font-family: var(--font-sans); font-size: 22px; line-height: 32px;");
    expect(tokens).toContain(".meta { font-family: var(--font-sans); font-size: 20px; line-height: 28px;");
    expect(tokens).toContain(".title { font-family: var(--font-sans); font-size: 40px; line-height: 48px;");
    expect(tokens).toContain(".heading { font-family: var(--font-sans); font-size: 30px; line-height: 38px;");
    expect(tokens).toContain(".code { font-family: var(--font-mono); font-size: 40px;");
  });

  it("uses named color tokens and keeps panel text at or above 20px", () => {
    expect(appCss).not.toMatch(/#[\da-f]{3,8}\b/i);
    const sizes = [...appCss.matchAll(/font-size:\s*(\d+)px/g)].map((match) => Number(match[1]));
    expect(sizes.length).toBeGreaterThan(0);
    expect(sizes.every((size) => size >= 20)).toBe(true);
    expect(appCss).toContain("font-size: 22px;\n  line-height: 32px;");
    expect(appCss).toContain("font-family: var(--font-mono)");
  });
});
