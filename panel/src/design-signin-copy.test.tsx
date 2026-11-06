// Locked tests for #179 (design pass 2026-11-06): the sign-in screen's words.
import { existsSync, readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { SignInPrompt } from "./api.ts";
import setupDropbox from "./fixtures/setup-dropbox.json";
import setupGdrive from "./fixtures/setup-gdrive.json";
import { SignInView } from "./views.tsx";

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
function deepFreeze<T>(o: T): T {
  if (o && typeof o === "object" && !Object.isFrozen(o)) {
    Object.freeze(o);
    for (const v of Object.values(o)) deepFreeze(v);
  }
  return o;
}
const gdrive = deepFreeze(structuredClone(setupGdrive.data)) as SignInPrompt;
const dropbox = deepFreeze(structuredClone(setupDropbox.data)) as SignInPrompt;
const noop = () => undefined;
const html = (prompt: SignInPrompt) =>
  renderToStaticMarkup(<SignInView prompt={prompt} error={null} onPick={noop} onBack={noop} />);

describe("sign-in copy (#179)", () => {
  it('says "Scan it, or open this address" as a label above the address', () => {
    expect(html(gdrive)).toMatch(/<p class="qr-label">Scan it, or open this address<\/p><p class="address">/);
    expect(html(gdrive)).not.toContain("Scan with a phone");
  });
  it('puts a "Type this code" label above the code, with no "Code:" prefix', () => {
    expect(html(gdrive)).toMatch(/<p class="qr-label">Type this code<\/p><p class="code">ABCD-EFGH<\/p>/);
    expect(html(gdrive)).not.toContain("Code:");
  });
  it("shows no code label when the service gives no code", () => {
    expect(html(dropbox)).not.toContain("Type this code");
  });
  it("keeps the service's message and the expiry wording", () => {
    expect(html(gdrive)).toContain(gdrive.message);
    expect(html(gdrive)).toMatch(/>Expires [^<]+\.<\/p>/);
  });
  it(".qr-label is a 20px bold muted line with no margin", () => {
    const d = decls(".qr-label");
    expect(d["font-size"]).toBe("20px");
    expect(d["font-weight"]).toBe("700");
    expect(d["color"]).toBe("var(--ink-muted)");
    expect(["0", "0px"]).toContain(d["margin"]);
  });
});
