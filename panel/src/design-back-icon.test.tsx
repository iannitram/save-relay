// Locked tests for #178 (design pass 2026-11-06): the Back button's arrow icon.
import { existsSync, readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { Screen } from "./views.tsx";

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
const noop = () => undefined;
const withBack = () => renderToStaticMarkup(<Screen title="hades: history" onBack={noop}><p>x</p></Screen>);
const backButton = () => /<button data-focus="back"[^>]*>([\s\S]*?)<\/button>/.exec(withBack())?.[1] ?? "";

describe("Back icon (#178)", () => {
  it("draws an inline svg arrow before the word Back", () => {
    expect(backButton()).toMatch(/^<svg class="icon"[^>]*>[\s\S]*<\/svg>Back$/);
  });
  it("the svg is 24 units, hidden from screen readers and not focusable", () => {
    const svg = /<svg[^>]*>/.exec(backButton())?.[0] ?? "";
    expect(svg).toContain('viewBox="0 0 24 24"');
    expect(svg).toContain('aria-hidden="true"');
    expect(svg).toContain('focusable="false"');
  });
  it("the arrow is the system's two strokes", () => {
    expect(backButton()).toContain('<path d="M20 12 H5"></path>');
    expect(backButton()).toContain('<path d="M11 6 L5 12 L11 18"></path>');
  });
  it("has no text arrow glyph", () => {
    expect(withBack()).not.toContain("←");
  });
  it("shows no Back button when there is nowhere to go back to", () => {
    const html = renderToStaticMarkup(<Screen title="Games" onBack={null}><p>x</p></Screen>);
    expect(html).not.toContain('data-focus="back"');
    expect(html).not.toContain('class="icon"');
  });
  it(".icon is 28px, stroked in the text color, unfilled", () => {
    const d = decls(".icon");
    expect(d["width"]).toBe("28px");
    expect(d["height"]).toBe("28px");
    expect(d["flex"]).toBe("none");
    expect(d["fill"]).toBe("none");
    expect(d["stroke"]).toBe("currentColor");
    expect(d["stroke-width"]).toBe("2.5");
    expect(d["stroke-linecap"]).toBe("round");
    expect(d["stroke-linejoin"]).toBe("round");
  });
  it("buttons line the icon up with the label, 8px apart", () => {
    const d = decls(".btn");
    expect(d["display"]).toBe("inline-flex");
    expect(d["align-items"]).toBe("center");
    expect(["8px", "var(--space-2)"]).toContain(d["gap"]);
  });
});

describe("list rows after #178", () => {
  it(".row keeps display: flex after .btn becomes inline-flex", () => {
    expect(decls(".row")["display"]).toBe("flex");
    const app = css[css.length - 1]!;
    const last = (sel: string) => {
      let at = -1;
      for (const m of app.matchAll(/([^{}]+)\{/g)) if (m[1]!.split(",").map((s) => s.trim()).includes(sel)) at = m.index!;
      return at;
    };
    expect(last(".row")).toBeGreaterThan(last(".btn"));
  });
  it("the Back button keeps its data-focus id and the btn class", () => {
    expect(withBack()).toMatch(/<button data-focus="back" class="btn">/);
  });
});
