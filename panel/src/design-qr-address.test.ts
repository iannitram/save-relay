// Review test for #176: the address is the system's code-sm (20px mono, 28px line).
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

describe("QrSignIn address line height (#176)", () => {
  it("the address sets the code-sm line height of 28px", () => {
    expect(decls(".address")["line-height"]).toBe("28px");
  });
});
