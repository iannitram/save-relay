// Locked tests for #166 (design pass 2026-11-06): button size, edge and the primary / destructive variants.
import { existsSync, readFileSync } from "node:fs";
import type { ReactElement, ReactNode } from "react";
import { isValidElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { HistoryData, PendingChoice } from "./api.ts";
import historyFx from "./fixtures/history.json";
import statusHeld from "./fixtures/status-pending-held.json";
import statusPending from "./fixtures/status-pending.json";
import whereFx from "./fixtures/where.json";
import { ConflictView, GamesView, HeldView, PendingView, Screen, SignInView, TimelineView, type GameRow } from "./views.tsx";

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
/** Where the last rule naming `selector` starts in app.css. */
const at = (selector: string) => {
  const app = css[css.length - 1]!;
  let last = -1;
  for (const m of app.matchAll(/([^{}]+)\{/g)) {
    if (m[1]!.split(",").map((s) => s.trim().replace(/\s+/g, " ")).includes(selector)) last = m.index!;
  }
  return last;
};

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
const classes = (el: ReactElement<any> | null) => String(el?.props.className ?? "").split(/\s+/);

const held = deepFreeze(structuredClone(statusHeld.data.pending)) as PendingChoice;
const history = deepFreeze(structuredClone(historyFx.data)) as HistoryData;
const conflict = deepFreeze(structuredClone(statusPending.data.pending)) as PendingChoice;
const noop = () => undefined;

describe("button base (#166)", () => {
  const btn = () => decls(".btn");
  it("is at least 64px tall", () => {
    expect(["64px", "var(--control-height)"]).toContain(btn()["min-height"]);
  });
  it("has 32px side padding and none top or bottom", () => {
    expect(["0 32px", "0px 32px", "0 var(--space-6)"]).toContain(btn()["padding"]);
  });
  it("has a 2px edge in the border token", () => {
    expect(btn()["border"]).toBe("2px solid var(--border)");
  });
  it("has a 14px radius", () => {
    expect(["14px", "var(--radius-md)"]).toContain(btn()["border-radius"]);
  });
  it("sits on surface-raised in ink", () => {
    expect(fill(btn())).toBe("var(--surface-raised)");
    expect(btn()["color"]).toBe("var(--ink)");
  });
});

describe("variants (#166)", () => {
  it(".btn-primary fills with accent, on-accent text, accent edge", () => {
    const d = decls(".btn-primary");
    expect(fill(d)).toBe("var(--accent)");
    expect(d["color"]).toBe("var(--on-accent)");
    expect(d["border-color"]).toBe("var(--accent)");
  });
  it(".btn-danger fills with danger, on-danger text, danger edge", () => {
    const d = decls(".btn-danger");
    expect(fill(d)).toBe("var(--danger)");
    expect(d["color"]).toBe("var(--on-danger)");
    expect(d["border-color"]).toBe("var(--danger)");
  });
  it("primary and destructive keep their fill when focused, after the plain focus rule", () => {
    for (const [cls, token] of [[".btn-primary", "var(--accent)"], [".btn-danger", "var(--danger)"]] as const) {
      for (const state of [":focus-visible", "[data-focused]"]) {
        expect(fill(decls(cls + state)), cls + state).toBe(token);
        expect(at(cls + state), cls + state).toBeGreaterThan(at(".btn" + state));
      }
    }
  });
});

describe("which button is which (#166)", () => {
  it("open elsewhere: Wait is primary, Play anyway is destructive", () => {
    const view = <HeldView game="celeste" held={held.held!} error={null} notice={null} onAnswer={noop} onBack={noop} />;
    expect(classes(find(view, "wait"))).toEqual(expect.arrayContaining(["btn", "btn-primary"]));
    expect(classes(find(view, "anyway"))).toEqual(expect.arrayContaining(["btn", "btn-danger"]));
    expect(classes(find(view, "anyway"))).not.toContain("btn-primary");
  });
  it("restore confirm: Restore is primary, Cancel is plain", () => {
    const view = (
      <TimelineView history={history} pictureUrl={() => null} confirm={history.entries[1]!.entry} error={null}
        notice={null} onAsk={noop} onConfirm={noop} onCancel={noop} onBack={noop} />
    );
    expect(classes(find(view, "confirm"))).toEqual(expect.arrayContaining(["btn", "btn-primary"]));
    expect(classes(find(view, "cancel"))).not.toContain("btn-primary");
    expect(classes(find(view, "cancel"))).not.toContain("btn-danger");
  });
});

describe("button base, kept as asked (#166)", () => {
  const btn = () => decls(".btn");
  it("keeps font: inherit", () => {
    expect(btn()["font"]).toBe("inherit");
  });
  it("keeps the 22px label size", () => {
    expect(btn()["font-size"]).toBe("22px");
  });
  it("keeps the pointer cursor", () => {
    expect(btn()["cursor"]).toBe("pointer");
  });
  it("sets a bold label (font-weight 700)", () => {
    expect(btn()["font-weight"]).toBe("700");
  });
});

describe("what #166 leaves alone", () => {
  it("Cancel keeps exactly the plain btn class", () => {
    const view = (
      <TimelineView history={history} pictureUrl={() => null} confirm={history.entries[1]!.entry} error={null}
        notice={null} onAsk={noop} onConfirm={noop} onCancel={noop} onBack={noop} />
    );
    expect(classes(find(view, "cancel"))).toEqual(["btn"]);
  });
  it("Keep server and Keep local are not made destructive (#173 rebuilds them)", () => {
    const view = <ConflictView choice={conflict} error={null} notice={null} onAnswer={noop} onBack={noop} />;
    expect(find(view, "keep-server")).not.toBeNull();
    expect(classes(find(view, "keep-server"))).not.toContain("btn-danger");
    expect(classes(find(view, "keep-local"))).not.toContain("btn-danger");
  });
  it("keeps the data-focus ids, words and handlers of Wait and Play anyway", () => {
    const answers: string[] = [];
    const view = <HeldView game="celeste" held={held.held!} error={null} notice={null} onAnswer={(a) => { answers.push(a); }} onBack={noop} />;
    const wait = find(view, "wait")!;
    const anyway = find(view, "anyway")!;
    expect(wait.props.children).toBe("Wait");
    expect(anyway.props.children).toBe("Play anyway");
    wait.props.onClick();
    anyway.props.onClick();
    expect(answers).toEqual(["wait", "play_anyway"]);
  });
  it("keeps the data-focus ids, words and handlers of Cancel and Restore", () => {
    const calls: string[] = [];
    const view = (
      <TimelineView history={history} pictureUrl={() => null} confirm={history.entries[1]!.entry} error={null}
        notice={null} onAsk={noop} onConfirm={() => { calls.push("confirm"); }} onCancel={() => { calls.push("cancel"); }} onBack={noop} />
    );
    const cancel = find(view, "cancel")!;
    const restore = find(view, "confirm")!;
    expect(cancel.props.children).toBe("Cancel");
    expect(restore.props.children).toBe("Restore");
    cancel.props.onClick();
    restore.props.onClick();
    expect(calls).toEqual(["cancel", "confirm"]);
  });
  it("does not define the button tokens in app.css (they come from tokens.css)", () => {
    const app = css[css.length - 1]!;
    for (const t of ["--border", "--surface-raised", "--ink", "--on-accent", "--danger", "--on-danger"]) {
      expect(app, t).not.toMatch(new RegExp(t + "\\s*:"));
    }
  });
});

describe("every other button keeps its class (#166)", () => {
  /** data-focus id -> class list, for every button in the rendered markup. */
  const buttons = (html: string) =>
    Object.fromEntries([...html.matchAll(/<button data-focus="([^"]+)" class="([^"]*)"/g)].map((m) => [m[1]!, m[2]!.split(/\s+/)]));
  const rows = deepFreeze(whereFx.data.games.map((g) => ({ ...g, pending: g.game === "hades" ? conflict : null }))) as GameRow[];
  const games = buttons(renderToStaticMarkup(<GamesView games={rows} error={null} onOpen={noop} onPending={noop} onSignIn={noop} />));
  const pending = buttons(renderToStaticMarkup(<PendingView choices={[conflict]} onOpen={noop} onBack={noop} />));
  const timeline = buttons(renderToStaticMarkup(
    <TimelineView history={history} pictureUrl={() => null} confirm={null} error={null} notice={null} onAsk={noop} onConfirm={noop} onCancel={noop} onBack={noop} />,
  ));
  const signIn = buttons(renderToStaticMarkup(<SignInView prompt={null} error={null} onPick={noop} onBack={noop} />));
  it("Back, Pending, Sign in, Google Drive and Dropbox stay exactly btn", () => {
    for (const [id, got] of [["back", pending.back], ["pending", games.pending], ["signin", games.signin], ["gdrive", signIn.gdrive], ["dropbox", signIn.dropbox]] as const) {
      expect(got, id).toEqual(["btn"]);
    }
  });
  it("each history entry's Restore stays exactly btn", () => {
    const ids = Object.keys(timeline).filter((id) => id.startsWith("restore:"));
    expect(ids.length).toBe(history.entries.filter((e) => !e.current).length);
    expect(ids.length).toBeGreaterThan(0);
    for (const id of ids) expect(timeline[id], id).toEqual(["btn"]);
  });
  it("game and choice rows stay exactly btn row", () => {
    const ids = [...Object.keys(games).filter((id) => id.startsWith("game:")), ...Object.keys(pending).filter((id) => id.startsWith("choice:"))];
    expect(ids.length).toBe(rows.length + 1);
    for (const id of ids) expect((games[id] ?? pending[id]), id).toEqual(["btn", "row"]);
  });
});
