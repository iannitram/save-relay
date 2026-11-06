// Locked tests for #176 (design pass 2026-11-06): the sign-in code as QrSignIn.
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

describe("QrSignIn markup (#176)", () => {
  it("wraps the QR svg in a qr-box", () => {
    expect(html(gdrive)).toMatch(/<div class="qr-box"><svg class="qr"/);
  });
  it("keeps the 4-module quiet zone inside the svg", () => {
    expect(html(gdrive)).toMatch(/<rect width="(\d+)" height="\1" fill="#fff"/);
    expect(html(gdrive)).toMatch(/d="M4 4h1v1h-1z/);
  });
  it("shows the address in its own address paragraph", () => {
    expect(html(gdrive)).toContain('<p class="address">https://www.google.com/device</p>');
    expect(html(dropbox)).toContain(`<p class="address">${dropbox.url.replace(/&/g, "&amp;")}</p>`);
  });
  it("shows the code in a code element and the expiry on its own meta line", () => {
    expect(html(gdrive)).toMatch(/class="code">ABCD-EFGH</);
    expect(html(gdrive)).toMatch(/<p class="meta">Expires [^<]+<\/p>/);
  });
  it("shows no code element when the service gives no code", () => {
    expect(html(dropbox)).not.toContain('class="code"');
  });
});

describe("QrSignIn styles (#176)", () => {
  it("the box is 320px square, white paper, a 16px quiet padding, the border edge", () => {
    const d = decls(".qr-box");
    expect(d["width"]).toBe("320px");
    expect(d["height"]).toBe("320px");
    expect(["16px", "var(--space-4)"]).toContain(d["padding"]);
    expect(d["background"] ?? d["background-color"]).toBe("var(--qr-paper)");
    expect(d["border"]).toBe("2px solid var(--border)");
    expect(["14px", "var(--radius-md)"]).toContain(d["border-radius"]);
    expect(d["box-sizing"]).toBe("border-box");
    expect(d["flex"]).toBe("none");
  });
  it("the svg fills the box", () => {
    const d = decls(".qr");
    expect(d["width"]).toBe("100%");
    expect(d["height"]).toBe("100%");
    expect(d["display"]).toBe("block");
  });
  it("the address is 20px mono and breaks anywhere", () => {
    const d = decls(".address");
    expect(d["font-family"]).toBe("var(--font-mono)");
    expect(d["font-size"]).toBe("20px");
    expect(d["word-break"]).toBe("break-all");
  });
  it("the code is 40px bold mono with 0.12em spacing", () => {
    const d = decls(".code");
    expect(d["font-family"]).toBe("var(--font-mono)");
    expect(d["font-size"]).toBe("40px");
    expect(d["font-weight"]).toBe("700");
    expect(d["letter-spacing"]).toBe("0.12em");
  });
});

describe("QrSignIn, kept and removed as asked (#176)", () => {
  it("keeps the service's message as a meta line", () => {
    expect(html(gdrive)).toContain(`<p class="meta">${gdrive.message}</p>`);
  });
  it("the address is medium weight (500)", () => {
    expect(decls(".address")["font-weight"]).toBe("500");
  });
  it("drops the .url and big classes from the address, and the .url rule", () => {
    expect(html(gdrive)).not.toMatch(/class="[^"]*\b(url|big)\b/);
    expect(decls(".url")).toEqual({});
  });
  it("leaves QrCode as it is: the svg keeps the qr class and no wrapper of its own", () => {
    expect(html(gdrive)).toMatch(/<div class="qr-box"><svg class="qr"[^>]*>[\s\S]*?<\/svg><\/div>/);
  });
});
