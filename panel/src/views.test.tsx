import { readFileSync } from "node:fs";
import type { ReactElement, ReactNode } from "react";
import { isValidElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { setWhenClock } from "./format.ts";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { choose, restore, signIn } from "./actions.ts";
import type { ApiArgs, ApiCommand, Bridge } from "./bridge.ts";
import answerFx from "./fixtures/answer.json";
import errorAuth from "./fixtures/error-auth.json";
import historyFx from "./fixtures/history.json";
import pullConflict from "./fixtures/pull-conflict.json";
import pullPulled from "./fixtures/pull-pulled.json";
import restoreFx from "./fixtures/restore.json";
import setupGdrive from "./fixtures/setup-gdrive.json";
import statusHeld from "./fixtures/status-pending-held.json";
import statusPending from "./fixtures/status-pending.json";
import whereFx from "./fixtures/where.json";
import type { Answer, Envelope, HistoryData, PendingChoice } from "./api.ts";
import {
  Banner, ConflictView, ErrorBanner, GamesView, HeldView, PendingView, QrCode, Screen, SignInView, TimelineView,
  type GameRow,
} from "./views.tsx";

/** A Bridge over fixtures that records its calls. */
function fake(reply: unknown) {
  const calls: [ApiCommand, ApiArgs][] = [];
  const bridge: Bridge = {
    async call<T>(command: ApiCommand, args: ApiArgs) {
      calls.push([command, args]);
      return reply as Envelope<T>;
    },
  };
  return { bridge, calls };
}

/** Find the element with `data-focus=id` in an unrendered view and click it. */
function find(node: ReactNode, id: string): ReactElement<any> | null {
  if (Array.isArray(node)) {
    for (const n of node) { const f = find(n, id); if (f) return f; }
    return null;
  }
  if (!isValidElement(node)) return null;
  const props = node.props as any;
  if (props["data-focus"] === id) return node as ReactElement<any>;
  // Views and their parts are hook-free functions: expand them. Screen (the hook) is not.
  if (typeof node.type === "function" && node.type !== Screen) {
    return find((node.type as (p: unknown) => ReactNode)(props), id);
  }
  return find(props.children, id);
}
function press(view: ReactElement, id: string) {
  const el = find(view, id);
  expect(el, id).not.toBeNull();
  el!.props.onClick();
}

/** The markup inside every banner of one tone: the words a tone wraps, and nothing after the banner. */
function wrapped(html: string, tone: string): string {
  return [...html.matchAll(new RegExp(`class="banner banner-${tone}"[^>]*>([\\s\\S]*?)</div></div>`, "g"))]
    .map((m) => m[1])
    .join("");
}

const conflict = statusPending.data.pending as PendingChoice;
const held = statusHeld.data.pending as PendingChoice;
const history = historyFx.data as HistoryData;
const noop = () => undefined;

beforeAll(() => {
  setWhenClock({ now: new Date("2026-11-01T00:30:00Z"), timeZone: "UTC" });
});

afterAll(() => {
  setWhenClock(null);
});

describe("games list", () => {
  it("shows a held game and a pending one", () => {
    const rows: GameRow[] = whereFx.data.games.map((g) => ({
      ...g,
      pending: g.game === "hades" ? conflict : null,
    })) as GameRow[];
    const html = renderToStaticMarkup(
      <GamesView games={rows} error={null} onOpen={noop} onPending={noop} onSignIn={noop} />,
    );
    expect(html).toContain('<span class="name label">celeste</span>');
    expect(html).toContain("Open on steam-deck since Oct 31, 7:12pm");
    expect(html).toContain("Choice waiting");
    expect(html).toContain('class="badge badge-warning"');
    expect(html).toContain("Pending choices (1)");
    expect([...html.matchAll(/data-focus="([^"]+)"/g)][0]?.[1]).toBe("game:celeste");
  });
});

describe("pending choices list", () => {
  it("renders game names and metadata as ListRow label and meta", () => {
    const html = renderToStaticMarkup(
      <PendingView choices={[held]} onOpen={noop} onBack={noop} />,
    );
    expect(html).toContain('<span class="name label">celeste</span>');
    expect(html).toContain('<span class="meta">Open on steam-deck');
  });
});

describe("banners", () => {
  it("renders each tone as an icon and text, never a button", () => {
    for (const tone of ["info", "warning", "danger", "success"] as const) {
      const html = renderToStaticMarkup(<Banner tone={tone}>Message</Banner>);
      expect(html).toContain(`banner-${tone}`);
      expect(html).toContain('aria-hidden="true"');
      expect(html).toContain("Message");
      expect(html).not.toContain("<button");
    }
  });

  it("uses danger alerts for errors, success for notices, and info for explanatory text", () => {
    const error = renderToStaticMarkup(<ErrorBanner message="Could not save" />);
    expect(error).toContain('class="banner banner-danger" role="alert"');
    expect(wrapped(error, "danger")).toContain("Could not save");
    const conflictHtml = renderToStaticMarkup(
      <ConflictView choice={conflict} error={null} notice="Saved" onAnswer={noop} onBack={noop} />,
    );
    expect(wrapped(conflictHtml, "success")).toContain("Saved");
    expect(wrapped(conflictHtml, "info")).toContain("Nothing is overwritten until you choose.");
  });

  it("uses the warning banner for held saves and info for Nextcloud", () => {
    const heldHtml = renderToStaticMarkup(
      <HeldView game="celeste" held={held.held!} error={null} notice="Saved" onAnswer={noop} onBack={noop} />,
    );
    expect(wrapped(heldHtml, "warning")).toContain("Open on steam-deck since");
    expect(wrapped(heldHtml, "warning")).toContain("Playing anyway can lose the other machine&#x27;s newest saves.");
    expect(wrapped(heldHtml, "success")).toContain("Saved");
    const signInHtml = renderToStaticMarkup(
      <SignInView prompt={null} error={null} onPick={noop} onBack={noop} />,
    );
    expect(wrapped(signInHtml, "info")).toContain("Nextcloud is set up on the command line, not here.");
  });

  it("uses the success banner for notices on timeline history", () => {
    const timelineHtml = renderToStaticMarkup(
      <TimelineView history={history} pictureUrl={() => null} confirm={null} error={null} notice="Restored" onAsk={noop} onConfirm={noop} onCancel={noop} onBack={noop} />,
    );
    expect(wrapped(timelineHtml, "success")).toContain("Restored");
  });

  it("styles each tone's border and the icon's 2.5 stroke in app.css", () => {
    const css = readFileSync(new URL("./app.css", import.meta.url), "utf8");
    expect(css).toMatch(/\.banner-icon\s*\{[^}]*stroke-width:\s*2\.5;/);
    for (const tone of ["info", "warning", "danger", "success"]) {
      expect(css).toMatch(new RegExp(`\\.banner-${tone}\\s*\\{[^}]*border-color:`));
    }
  });
});

describe("conflict view", () => {
  it("shows two cards and answers keep-server / keep-local", async () => {
    const answers: Answer[] = [];
    const view = <ConflictView choice={conflict} error={null} notice={null} onAnswer={(a) => answers.push(a)} onBack={noop} />;
    const html = renderToStaticMarkup(view);
    expect(html).toContain("Chapter 7: The Summit");
    expect(html).toContain("On the server");
    expect(html).toContain("On this machine");
    press(view, "keep-server");
    press(view, "keep-local");
    expect(answers).toEqual(["keep_server", "keep_local"]);

    const { bridge, calls } = fake(answerFx);
    await choose(bridge, { game: "hades", pending: true }, "keep_server");
    await choose(bridge, { game: "hades", pending: true }, "keep_local");
    expect(calls).toEqual([
      ["answer", { game: "hades", answer: "keep-server" }],
      ["answer", { game: "hades", answer: "keep-local" }],
    ]);
  });
});

describe("held view", () => {
  it("offers Wait and Play anyway; a held pull answers with pull --anyway", async () => {
    const answers: Answer[] = [];
    const view = <HeldView game="celeste" held={held.held!} error={null} notice={null} onAnswer={(a) => answers.push(a)} onBack={noop} />;
    expect(renderToStaticMarkup(view)).toContain("Open on steam-deck since Oct 31, 7:12pm");
    press(view, "wait");
    press(view, "anyway");
    expect(answers).toEqual(["wait", "play_anyway"]);

    const { bridge, calls } = fake({ ...pullPulled });
    await choose(bridge, { game: "celeste", pending: false }, "play_anyway");
    expect(calls).toEqual([["pull", { game: "celeste", anyway: true }]]);
    const pend = fake(answerFx);
    await choose(pend.bridge, { game: "celeste", pending: true }, "play_anyway");
    expect(pend.calls).toEqual([["answer", { game: "celeste", answer: "play-anyway" }]]);
  });
});

describe("restore", () => {
  const entry = history.entries[1]!.entry;
  const view = (confirm: string | null, calls: string[]) => (
    <TimelineView history={history} pictureUrl={() => null} confirm={confirm} error={null} notice={null}
      onAsk={(e) => calls.push(`ask ${e}`)} onConfirm={() => calls.push("confirm")} onCancel={() => calls.push("cancel")} onBack={noop} />
  );

  it("asks before it calls restore", async () => {
    const ui: string[] = [];
    press(view(null, ui), `restore:${entry}`);
    expect(ui).toEqual([`ask ${entry}`]);
    const { bridge, calls } = fake(restoreFx);
    expect(calls).toEqual([]); // asking called nothing
    const html = renderToStaticMarkup(view(entry, ui));
    expect(html).toContain("Restore these saves?");
    press(view(entry, ui), "confirm");
    press(view(entry, ui), "cancel");
    expect(ui.slice(1)).toEqual(["confirm", "cancel"]);
    await restore(bridge, "celeste", entry);
    expect(calls).toEqual([["restore", { game: "celeste", entry }]]);
  });

  it("lists cards newest first, an old entry without one, and no Restore on the current", () => {
    const html = renderToStaticMarkup(view(null, []));
    expect(html.indexOf("Chapter 7")).toBeLessThan(html.indexOf("Chapter 6"));
    expect(html).toContain("No card (older save)");
    expect(html).toContain("Played 1 h 12 min".replace("Played", "played"));
    expect(html).toContain("Current");
    expect(html).not.toContain(`restore:${history.entries[0]!.entry}`);
  });
});

describe("sign-in", () => {
  it("renders a QR code for the URL and shows the code", () => {
    const prompt = setupGdrive.data;
    const html = renderToStaticMarkup(<SignInView prompt={prompt} error={null} onPick={noop} onBack={noop} />);
    expect(html).toContain("<svg");
    expect(html).toContain(`QR code for ${prompt.url}`);
    expect(html).toContain("ABCD-EFGH");
    expect(renderToStaticMarkup(<QrCode text="https://example.com" />)).toMatch(/<path d="M\d/);
  });

  it("has no Nextcloud password field", () => {
    const html = renderToStaticMarkup(<SignInView prompt={null} error={null} onPick={noop} onBack={noop} />);
    expect(html).not.toContain("<input");
    expect(html).toContain("Google Drive");
    expect(html).toContain("Dropbox");
  });

  it("asks setup --no-browser", async () => {
    const { bridge, calls } = fake(setupGdrive);
    expect((await signIn(bridge, "gdrive")).code).toBe("ABCD-EFGH");
    expect(calls).toEqual([["setup", { service: "gdrive", "no-browser": true }]]);
  });
});

describe("errors", () => {
  it("shows an error envelope's message", async () => {
    const { bridge } = fake(errorAuth);
    const err = await choose(bridge, { game: "g", pending: true }, "wait").catch((e) => e);
    expect(err.code).toBe("reauth_needed");
    expect(renderToStaticMarkup(<ErrorBanner message={err.message} />)).toContain("sign in again");
  });

  it("keeps a conflict pull fixture usable", () => {
    expect(pullConflict.data.action).toBe("conflict");
  });
});
