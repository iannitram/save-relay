// The `--json` envelope and the `data` types the panel reads, mirroring src/api.rs (see
// docs/code-map.md, "API shape"). Field names are the JSON names; never rename one here
// without the Rust side.

export type Action =
  | "noop"
  | "adopted"
  | "pulled"
  | "pushed"
  | "offline"
  | "deferred"
  | "conflict"
  | "held";

export type Decision = "noop" | "adopt" | "pull" | "push" | "conflict" | "offline" | "held";

export interface Side {
  device: string;
  time: string;
  revision: number | null;
}

export interface Held {
  device: string;
  since: string;
}

export interface Card {
  format: number;
  device: string;
  time: string;
  playtime_secs: number | null;
  place: string | null;
  picture: string | null;
}

export interface Counts {
  uploaded: number;
  downloaded: number;
  deleted: number;
}

/** `data` of `pull` and `push` (Rust `SyncReport`: `SyncData` flattened plus three fields). */
export interface SyncReport {
  game: string;
  action: Action;
  revision: number | null;
  server: Side | null;
  local: Side | null;
  backup: string | null;
  counts: Counts;
  message: string;
  held: Held | null;
  server_card: Card | null;
  local_card: Card | null;
}

export type ChoiceKind = "conflict" | "held";

/** JSON form; the command line takes kebab case (`keep-server`). */
export type Answer = "keep_server" | "keep_local" | "play_anyway" | "wait";

export interface PendingChoice {
  format: number;
  game: string;
  kind: ChoiceKind;
  server: Side | null;
  local: Side | null;
  server_card: Card | null;
  local_card: Card | null;
  held: Held | null;
  time: string;
  answer: Answer | null;
}

export interface StatusData {
  game: string;
  service: string;
  reachable: boolean;
  would: Decision;
  server: Side | null;
  local: Side | null;
  last_synced_revision: number | null;
  local_changed: boolean;
  message: string;
  held: Held | null;
  pending: PendingChoice | null;
}

export interface HistoryEntry {
  entry: string;
  device: string;
  time: string;
  current: boolean;
  card: Card | null;
}

export interface HistoryData {
  game: string;
  current: string | null;
  entries: HistoryEntry[];
}

export interface WhereGame {
  game: string;
  held: Held | null;
  server: Side | null;
}

export interface WhereData {
  games: WhereGame[];
}

export interface ErrorBody {
  code: string;
  message: string;
}

/** One `--json` line: `{"ok":true,"command":..,"data":..}` or `{"ok":false,..,"error":..}`. */
export type Envelope<T> =
  | { ok: true; command: string; data: T }
  | { ok: false; command: string; error: ErrorBody };
