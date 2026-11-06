// Locked tests for #174 (design pass 2026-11-06): history entries become SaveCard.
import { existsSync, readFileSync } from "node:fs";
import type { ReactElement, ReactNode } from "react";
import { isValidElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { HistoryData } from "./api.ts";
import historyFx from "./fixtures/history.json";
import { Screen, TimelineView } from "./views.tsx";

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

/** The element with `data-focus=id` in an unrendered view (views are hook-free; Screen is not expanded). */
function find(node: ReactNode, id: string): ReactElement<any> | null {
  if (Array.isArray(node)) {
    for (const n of node) { const f = find(n, id); if (f) return f; }
    return null;
  }
  if (!isValidElement(node)) return null;
  const props = node.props as any;
  if (props["data-focus"] === id) return node as ReactElement<any>;
  if (typeof node.type === "function" && node.type !== Screen) {
    return find((node.type as (p: unknown) => ReactNode)(props), id);
  }
  return find(props.children, id);
}

const history = deepFreeze(structuredClone(historyFx.data)) as HistoryData;
const noop = () => undefined;
const mockPictureUrl = (_game: string, _entry: string, file: string) => `/mock-pic/${file}`;

const renderTimeline = (pictureUrl: (game: string, entry: string, file: string) => string | null = mockPictureUrl) =>
  renderToStaticMarkup(
    <TimelineView
      history={history}
      pictureUrl={pictureUrl}
      confirm={null}
      error={null}
      notice={null}
      onAsk={noop}
      onConfirm={noop}
      onCancel={noop}
      onBack={noop}
    />,
  );

describe("SaveCard styles (#174)", () => {
  it(".save card has 2px border, surface-raised background, and 20px radius", () => {
    const d = decls(".save");
    expect(d["border"]).toBe("2px solid var(--border)");
    expect(["20px", "var(--radius-lg)"]).toContain(d["border-radius"]);
    expect(d["background"]).toBe("var(--surface-raised)");
    expect(d["color"]).toBe("var(--ink)");
    expect(["24px", "var(--space-5)"]).toContain(d["gap"]);
  });

  it(".picture box is 192x108 with 8px radius and border-subtle edge", () => {
    const d = decls(".picture");
    expect(d["width"]).toBe("192px");
    expect(d["height"]).toBe("108px");
    expect(["8px", "var(--radius-sm)"]).toContain(d["border-radius"]);
    expect(d["border"]).toBe("2px solid var(--border-subtle)");
    expect(d["background"]).toBe("var(--surface-sunken)");
    expect(d["overflow"]).toBe("hidden");
  });

  it(".picture .icon is 40x40", () => {
    const d = decls(".picture .icon");
    expect(d["width"]).toBe("40px");
    expect(d["height"]).toBe("40px");
  });

  it(".picture img fits cover and fills 100%", () => {
    const d = decls(".picture img");
    expect(d["width"]).toBe("100%");
    expect(d["height"]).toBe("100%");
    expect(d["object-fit"]).toBe("cover");
  });
});

describe("SaveCard markup and picture box (#174)", () => {
  it("renders an img in the picture box when an entry has a picture", () => {
    const html = renderTimeline(mockPictureUrl);
    expect(html).toMatch(/<span class="picture"><img src="\/mock-pic\/picture\.jpg" alt=""\/><\/span>/);
  });

  it("renders a placeholder icon in the picture box when there is no picture", () => {
    const html = renderTimeline(() => null);
    expect(html).toMatch(
      /<span class="picture"><svg class="icon"[^>]*><rect x="3" y="4" width="18" height="16" rx="2"><\/rect><path d="M3 17 L9 11 L13 15 L16 12 L21 17"><\/path><\/svg><\/span>/,
    );
  });
});

describe("No place recorded and font tokens (#174)", () => {
  it('says "No place recorded" for an entry without a card place', () => {
    const html = renderTimeline();
    expect(html).toContain("No place recorded");
    expect(html).not.toContain("No card (older save)");
  });

  it("places place in body-lg and facts in meta", () => {
    const html = renderTimeline();
    expect(html).toContain('class="save-place body-lg"');
    expect(html).toContain('class="save-facts meta"');
  });
});

describe("Current badge (#174)", () => {
  it('renders the "Current" badge with the base info pill class="badge" and a check icon', () => {
    const html = renderTimeline();
    expect(html).toMatch(
      /<span class="badge"><svg class="icon"[^>]*><circle cx="12" cy="12" r="9\.5"><\/circle><path d="M7\.5 12\.5 L10\.5 15\.5 L16\.5 9"><\/path><\/svg>Current<\/span>/,
    );
  });

  it("does not render a Restore button on the current entry", () => {
    const html = renderTimeline();
    expect(html).not.toContain(`restore:${history.entries[0]!.entry}`);
  });
});

describe("Restore button (#174)", () => {
  it("uses the secondary button variant (btn) on non-current entries", () => {
    const calls: string[] = [];
    const view = (
      <TimelineView
        history={history}
        pictureUrl={mockPictureUrl}
        confirm={null}
        error={null}
        notice={null}
        onAsk={(e) => calls.push(e)}
        onConfirm={noop}
        onCancel={noop}
        onBack={noop}
      />
    );
    const nonCurrentEntry = history.entries[1]!.entry;
    const btn = find(view, `restore:${nonCurrentEntry}`)!;
    expect(btn).not.toBeNull();
    expect(btn.props.className).toBe("btn");
    btn.props.onClick();
    expect(calls).toEqual([nonCurrentEntry]);
  });
});

describe("First focus on history view (#174)", () => {
  const back = () => undefined;
  const view = () =>
    TimelineView({
      history,
      pictureUrl: mockPictureUrl,
      confirm: null,
      error: null,
      notice: null,
      onAsk: noop,
      onConfirm: noop,
      onCancel: noop,
      onBack: back,
    }) as ReactElement<any>;

  it("renders inside Screen", () => {
    expect(view().type).toBe(Screen);
  });

  it("gives the first focus to Back (Screen initial is back)", () => {
    expect(view().props.initial).toBe("back");
  });

  it("lets a caller's initial focus win over Back", () => {
    const target = `restore:${history.entries[1]!.entry}`;
    const el = TimelineView({
      history,
      pictureUrl: mockPictureUrl,
      confirm: null,
      error: null,
      notice: null,
      initial: target,
      onAsk: noop,
      onConfirm: noop,
      onCancel: noop,
      onBack: noop,
    }) as ReactElement<any>;
    expect(el.props.initial).toBe(target);
  });

  it("has Back as the first focus target before any Restore", () => {
    const html = renderTimeline();
    const tags = [...html.matchAll(/data-focus="([^"]+)"/g)].map((m) => m[1]);
    expect(tags[0]).toBe("back");
    expect(tags[1]).toBe(`restore:${history.entries[1]!.entry}`);
  });
});
