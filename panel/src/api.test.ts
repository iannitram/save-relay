import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type {
  Action, AnswerData, Card, Counts, Decision, Envelope, ErrorBody, Held, HistoryData,
  HistoryEntry, PendingChoice, PictureData, RestoreData, Side, SignInPrompt, StatusData,
  SyncReport, WhereData, WhereGame, WhichData, SteamGame, SteamGamesData,
  InstallWrapApp, InstallWrapData, LaunchOptionData,
} from "./api.ts";

// JSON imports widen string unions to `string`, so a plain `satisfies` on a fixture cannot
// check an enum. Instead each type has a key table that `satisfies` makes exhaustive (a field
// added to the type without a row here fails `tsc`), and the test requires every fixture
// object to have exactly those keys (a field the types lack fails the test).
type Keys<T> = { [K in keyof Required<T>]: true };
const side = { device: true, time: true, revision: true } satisfies Keys<Side>;
const held = { device: true, since: true } satisfies Keys<Held>;
const card = { format: true, device: true, time: true, playtime_secs: true, place: true, picture: true } satisfies Keys<Card>;
const counts = { uploaded: true, downloaded: true, deleted: true } satisfies Keys<Counts>;
const pending = { format: true, game: true, kind: true, server: true, local: true, server_card: true, local_card: true, held: true, time: true, answer: true } satisfies Keys<PendingChoice>;
const entry = { entry: true, device: true, time: true, current: true, card: true } satisfies Keys<HistoryEntry>;
const whereGame = { game: true, held: true, server: true } satisfies Keys<WhereGame>;
const report = { game: true, action: true, revision: true, server: true, local: true, backup: true, counts: true, message: true, held: true, server_card: true, local_card: true } satisfies Keys<SyncReport>;
const status = { game: true, service: true, reachable: true, would: true, server: true, local: true, last_synced_revision: true, local_changed: true, message: true, held: true, pending: true } satisfies Keys<StatusData>;
const history = { game: true, current: true, entries: true } satisfies Keys<HistoryData>;
const restore = { game: true, entry: true, revision: true, new_entry: true, backup: true, counts: true, message: true } satisfies Keys<RestoreData>;
const where = { games: true } satisfies Keys<WhereData>;
const which = { app_id: true, game: true, name: true, saves_dir: true, include: true, source: true, message: true } satisfies Keys<WhichData>;
const steamGame = { app_id: true, name: true, steam_cloud: true, saves: true, wrapped: true } satisfies Keys<SteamGame>;
const steamGames = { games: true, unknown: true } satisfies Keys<SteamGamesData>;
const installWrapApp = { app_id: true, user_id: true, before: true, after: true } satisfies Keys<InstallWrapApp>;
const installWrap = { dry_run: true, changed: true, backups: true, message: true } satisfies Keys<InstallWrapData>;
const launchOption = { app_id: true, before: true, after: true } satisfies Keys<LaunchOptionData>;
const answer = { game: true, kind: true, answer: true, message: true } satisfies Keys<AnswerData>;
const picture = { path: true, bytes: true } satisfies Keys<PictureData>;
const prompt = { service: true, url: true, code: true, expires_at: true, message: true } satisfies Keys<SignInPrompt>;
const errorBody = { code: true, message: true } satisfies Keys<ErrorBody>;

const ACTIONS: Action[] = ["noop", "adopted", "pulled", "pushed", "offline", "deferred", "conflict", "held"];
const DECISIONS: Decision[] = ["noop", "adopt", "pull", "push", "conflict", "offline", "held"];

function exact(label: string, value: unknown, table: Record<string, true>) {
  expect(value, label).toBeTypeOf("object");
  expect(Object.keys(value as object).sort(), label).toEqual(Object.keys(table).sort());
}
const opt = (label: string, value: unknown, table: Record<string, true>) => {
  if (value !== null) exact(label, value, table);
};

const dir = new URL("./fixtures/", import.meta.url);
const fixtures = readdirSync(dir)
  .filter((f) => f.endsWith(".json"))
  .map((f) => [f.slice(0, -5), JSON.parse(readFileSync(new URL(f, dir), "utf8")) as Envelope<any>] as const);

function checkData(name: string, command: string, d: any) {
  const sides = (x: any) => {
    opt(name, x.server, side);
    opt(name, x.local, side);
  };
  switch (command) {
    case "pull":
    case "push":
      exact(name, d, report);
      sides(d);
      exact(name, d.counts, counts);
      expect(ACTIONS).toContain(d.action);
      opt(name, d.held, held);
      opt(name, d.server_card, card);
      opt(name, d.local_card, card);
      break;
    case "status":
      exact(name, d, status);
      sides(d);
      expect(DECISIONS).toContain(d.would);
      opt(name, d.held, held);
      if (d.pending !== null) {
        exact(name, d.pending, pending);
        expect(["conflict", "held"]).toContain(d.pending.kind);
        opt(name, d.pending.held, held);
        opt(name, d.pending.server_card, card);
        opt(name, d.pending.local_card, card);
      }
      break;
    case "history":
      exact(name, d, history);
      for (const e of d.entries) {
        exact(name, e, entry);
        opt(name, e.card, card);
      }
      break;
    case "restore": exact(name, d, restore); exact(name, d.counts, counts); break;
    case "where":
      exact(name, d, where);
      for (const g of d.games) { exact(name, g, whereGame); opt(name, g.held, held); opt(name, g.server, side); }
      break;
    case "which": exact(name, d, which); break;
    case "answer": exact(name, d, answer); break;
    case "picture": exact(name, d, picture); break;
    case "setup": exact(name, d, prompt); break;
    default: throw new Error(`${name}: no type for ${command}`);
  }
}

describe("fixtures", () => {
  it("keeps the M16 Steam data shapes complete", () => {
    expect([steamGame, steamGames, installWrapApp, installWrap, launchOption]).toHaveLength(5);
  });

  it("covers every command and outcome", () => {
    const names = fixtures.map(([n]) => n);
    for (const want of ["pull-pulled", "pull-conflict", "pull-held", "status-pending", "history", "where", "which", "answer", "picture", "error-usage", "error-auth", "error-server", "error-other"]) {
      expect(names).toContain(want);
    }
  });

  it.each(fixtures)("%s parses with its type", (name, envelope) => {
    if (!envelope.ok) {
      exact(name, envelope.error, errorBody);
      return;
    }
    checkData(name, envelope.command, envelope.data);
  });

  it("writes held and server_card as null, and an old entry's card as null", () => {
    const get = (n: string) => fixtures.find(([f]) => f === n)![1] as any;
    expect(get("pull-pulled").data.held).toBeNull();
    expect(get("pull-held").data.server_card).toBeNull();
    expect(get("pull-held").data.held).not.toBeNull();
    expect(get("history").data.entries.at(-1).card).toBeNull();
  });
});
