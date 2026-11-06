// A Bridge that answers from panel/src/fixtures, so the views run (and are screenshotted)
// without a server: `pnpm --dir panel dev` uses it when the page has no token.

import answer from "./fixtures/answer.json";
import history from "./fixtures/history.json";
import pullConflict from "./fixtures/pull-conflict.json";
import pullPulled from "./fixtures/pull-pulled.json";
import restore from "./fixtures/restore.json";
import setupDropbox from "./fixtures/setup-dropbox.json";
import setupGdrive from "./fixtures/setup-gdrive.json";
import statusHeld from "./fixtures/status-pending-held.json";
import statusIdle from "./fixtures/status-idle.json";
import statusPending from "./fixtures/status-pending.json";
import where from "./fixtures/where.json";
import type { Envelope } from "./api.ts";
import type { ApiArgs, ApiCommand, Bridge } from "./bridge.ts";
import { setWhenClock } from "./format.ts";

const fixtureClock = { now: new Date("2026-11-01T00:30:00Z"), timeZone: "UTC" };

function pick(command: ApiCommand, args: ApiArgs): unknown {
  switch (command) {
    case "where":
      return where;
    case "status":
      if (args.game === "hades") return statusPending;
      if (args.game === "celeste") return statusHeld;
      return { ...statusIdle, data: { ...statusIdle.data, game: args.game } };
    case "history":
      return { ...history, data: { ...history.data, game: args.game } };
    case "restore":
      return restore;
    case "pull":
    case "push":
      return args.resolve || args.anyway ? pullPulled : pullConflict;
    case "answer":
      return answer;
    case "setup":
      return args.service === "dropbox" ? setupDropbox : setupGdrive;
    default:
      return {
        ok: false,
        command,
        error: { code: "usage", message: `${command} has no fixture` },
      };
  }
}

export function fixtureBridge(): Bridge {
  setWhenClock(fixtureClock);
  return {
    async call<T>(command: ApiCommand, args: ApiArgs): Promise<Envelope<T>> {
      return pick(command, args) as Envelope<T>;
    },
  };
}

/** A stand-in picture (an SVG) for entries with a card picture, on fixtures only. */
export function fixturePicture(): string {
  const svg =
    "<svg xmlns='http://www.w3.org/2000/svg' width='160' height='90'><rect width='160' height='90' fill='#3b5b8c'/><path d='M0 90 L50 40 L90 70 L120 30 L160 90Z' fill='#9ec1e8'/></svg>";
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}
