import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { nextFocus } from "./focus.ts";
import historyFx from "./fixtures/history.json";
import setupGdrive from "./fixtures/setup-gdrive.json";
import statusHeld from "./fixtures/status-pending-held.json";
import statusPending from "./fixtures/status-pending.json";
import type { HistoryData, PendingChoice } from "./api.ts";
import {
  ConflictView, GamesView, HeldView, PendingView, SignInView, TimelineView,
} from "./views.tsx";

const noop = () => undefined;
const conflict = statusPending.data.pending as PendingChoice;
const held = statusHeld.data.pending as PendingChoice;
const history = historyFx.data as HistoryData;
const e1 = history.entries[1]!.entry;

/** The data-focus ids of a view's controls, in order; fails if one is hidden or disabled. */
function order(view: React.ReactElement): string[] {
  const html = renderToStaticMarkup(view);
  const tags = html.match(/<[^>]*data-focus="[^"]*"[^>]*>/g) ?? [];
  for (const tag of tags) {
    expect(tag, "a focusable control is hidden or disabled").not.toMatch(/\bhidden\b|\bdisabled\b|aria-hidden/);
  }
  return tags.map((t) => /data-focus="([^"]*)"/.exec(t)![1]!);
}

const rows = [
  { game: "celeste", held: null, server: null, pending: held },
  { game: "hades", held: null, server: null, pending: conflict },
];
const timeline = (confirm: string | null) => (
  <TimelineView history={history} pictureUrl={() => null} confirm={confirm} error={null} notice={null}
    onAsk={noop} onConfirm={noop} onCancel={noop} onBack={noop} />
);

const views: [string, React.ReactElement, string[]][] = [
  ["games", <GamesView games={rows} error={null} onOpen={noop} onPending={noop} onSignIn={noop} />,
    ["game:celeste", "game:hades", "pending", "signin"]],
  ["pending", <PendingView choices={[held, conflict]} onOpen={noop} onBack={noop} />,
    ["back", "choice:celeste", "choice:hades"]],
  ["timeline", timeline(null), ["back", `restore:${e1}`, `restore:${history.entries[2]!.entry}`]],
  ["restore confirm", timeline(e1), ["back", "cancel", "confirm"]],
  ["conflict", <ConflictView choice={conflict} error={null} notice={null} onAnswer={noop} onBack={noop} />,
    ["back", "keep-server", "keep-local"]],
  ["held", <HeldView game="celeste" held={held.held!} error={null} notice={null} onAnswer={noop} onBack={noop} />,
    ["back", "wait", "anyway"]],
  ["sign-in picker", <SignInView prompt={null} error={null} onPick={noop} onBack={noop} />,
    ["back", "gdrive", "dropbox"]],
  ["sign-in QR", <SignInView prompt={setupGdrive.data} error={null} onPick={noop} onBack={noop} />, ["back"]],
];

describe("nextFocus", () => {
  const ids = ["a", "b", "c"];
  it("moves with the arrows and wraps", () => {
    expect(nextFocus(ids, "a", "ArrowDown")).toEqual({ focus: "b" });
    expect(nextFocus(ids, "c", "ArrowRight")).toEqual({ focus: "a" });
    expect(nextFocus(ids, "a", "ArrowUp")).toEqual({ focus: "c" });
    expect(nextFocus(ids, "b", "ArrowLeft")).toEqual({ focus: "a" });
    expect(nextFocus(ids, null, "ArrowDown")).toEqual({ focus: "a" });
    expect(nextFocus(ids, "gone", "ArrowUp")).toEqual({ focus: "c" });
  });
  it("Escape goes back; other keys are left alone", () => {
    expect(nextFocus(ids, "b", "Escape")).toBe("back");
    expect(nextFocus([], null, "Escape")).toBe("back");
    expect(nextFocus(ids, "b", "x")).toBeNull();
    expect(nextFocus([], null, "ArrowDown")).toBeNull();
  });
});

describe.each(views)("%s view", (_name, view, expected) => {
  it("has its controls in order, all visible and enabled", () => {
    expect(order(view)).toEqual(expected);
  });
  it("arrows reach every control in order, then wrap", () => {
    const ids = order(view);
    let at: string | null = null;
    const seen: string[] = [];
    for (let i = 0; i < ids.length; i++) {
      const move = nextFocus(ids, at, "ArrowDown");
      at = (move as { focus: string }).focus;
      seen.push(at);
    }
    expect(seen).toEqual(ids);
    expect(nextFocus(ids, at, "ArrowDown")).toEqual({ focus: ids[0] });
  });
});
