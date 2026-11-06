// The panel's views: plain components over props, so they run the same in the browser panel
// and in Game Mode. They hold no state of their own; App.tsx loads data and routes. Every
// control is a <button data-focus="id"> (controller focus, see focus.ts); none hides on hover.

import QRCode from "qrcode";
import type { ReactNode } from "react";
import type {
  Answer,
  Card,
  Held,
  HistoryData,
  PendingChoice,
  Side,
  SignInPrompt,
} from "./api.ts";
import { useFocusScope } from "./focus.ts";
import { playtime, when } from "./format.ts";

/** A view's root: the heading, the focus scope (arrows, Escape) and the focus ring's home. */
export function Screen(props: {
  title: string;
  onBack: (() => void) | null;
  initial?: string;
  children: ReactNode;
}) {
  const { ref, onKeyDown } = useFocusScope(props.onBack, props.initial);
  return (
    <section className="screen" ref={ref} onKeyDown={onKeyDown} aria-label={props.title}>
      <header>
        {props.onBack && (
          <button data-focus="back" className="btn" onClick={props.onBack}>
            <svg className="icon" viewBox="0 0 24 24" aria-hidden="true" focusable="false"><path d="M20 12 H5" /><path d="M11 6 L5 12 L11 18" /></svg>Back
          </button>
        )}
        <h1>{props.title}</h1>
      </header>
      {props.children}
    </section>
  );
}

export function ErrorBanner(props: { message: string | null }) {
  if (!props.message) return null;
  return (
    <p className="error" role="alert">
      {props.message}
    </p>
  );
}

export function LoadingView(props: { title: string; error: string | null; onBack: () => void }) {
  return (
    <Screen title={props.title} onBack={props.onBack}>
      <ErrorBanner message={props.error} />
      {!props.error && <p className="meta">Loading…</p>}
    </Screen>
  );
}

export interface GameRow {
  game: string;
  held: Held | null;
  server: Side | null;
  pending: PendingChoice | null;
}

export function GamesView(props: {
  games: GameRow[];
  error: string | null;
  initial?: string;
  onOpen: (game: string) => void;
  onPending: () => void;
  onSignIn: () => void;
}) {
  const waiting = props.games.filter((g) => g.pending).length;
  return (
    <Screen title="Games" onBack={null} initial={props.initial}>
      <ErrorBanner message={props.error} />
      <ul className="list">
        {props.games.map((g) => (
          <li key={g.game}>
            <button
              data-focus={`game:${g.game}`}
              className="btn row"
              onClick={() => props.onOpen(g.game)}
            >
              <span className="name">{g.game}</span>
              <span className="meta">
                {g.held ? `Open on ${g.held.device} since ${when(g.held.since)}` : "Not open"}
                {g.server ? ` · saved by ${g.server.device}` : " · nothing saved yet"}
              </span>
              {g.pending && <span className="badge">Choice waiting</span>}
            </button>
          </li>
        ))}
        {props.games.length === 0 && !props.error && <li className="meta">No games yet.</li>}
      </ul>
      <footer>
        <button data-focus="pending" className="btn" onClick={props.onPending}>
          Pending choices ({waiting})
        </button>
        <button data-focus="signin" className="btn" onClick={props.onSignIn}>
          Sign in…
        </button>
      </footer>
    </Screen>
  );
}

export function CardView(props: { title: string; side: Side | null; card: Card | null }) {
  const { side, card } = props;
  const time = playtime(card?.playtime_secs ?? null);
  return (
    <div className="card">
      <h2>{props.title}</h2>
      {side ? (
        <>
          <p className="big">{card?.place ?? "No place recorded"}</p>
          <p>
            {side.device} · {when(side.time)}
          </p>
          {time && <p>Played {time}</p>}
          {side.revision !== null && <p className="meta">Revision {side.revision}</p>}
        </>
      ) : (
        <p>No saves.</p>
      )}
    </div>
  );
}

export function ConflictView(props: {
  choice: PendingChoice;
  error: string | null;
  notice: string | null;
  onAnswer: (answer: Answer) => void;
  onBack: () => void;
}) {
  const { choice } = props;
  return (
    <Screen title={`${choice.game}: two versions`} onBack={props.onBack}>
      <ErrorBanner message={props.error} />
      {props.notice && <p className="notice">{props.notice}</p>}
      <p>Both this machine and the server changed. Nothing is overwritten until you choose.</p>
      <div className="cards">
        <CardView title="On the server" side={choice.server} card={choice.server_card} />
        <CardView title="On this machine" side={choice.local} card={choice.local_card} />
      </div>
      <footer>
        <button
          data-focus="keep-server"
          className="btn"
          onClick={() => props.onAnswer("keep_server")}
        >
          Keep server
        </button>
        <button
          data-focus="keep-local"
          className="btn"
          onClick={() => props.onAnswer("keep_local")}
        >
          Keep local
        </button>
      </footer>
    </Screen>
  );
}

export function HeldView(props: {
  game: string;
  held: Held;
  error: string | null;
  notice: string | null;
  onAnswer: (answer: Answer) => void;
  onBack: () => void;
}) {
  return (
    <Screen title={`${props.game} is open elsewhere`} onBack={props.onBack} initial="wait">
      <ErrorBanner message={props.error} />
      {props.notice && <p className="notice">{props.notice}</p>}
      <p className="big">
        Open on {props.held.device} since {when(props.held.since)}
      </p>
      <p>Playing anyway can lose the other machine's newest saves.</p>
      <footer>
        <button data-focus="wait" className="btn btn-primary" onClick={() => props.onAnswer("wait")}>
          Wait
        </button>
        <button data-focus="anyway" className="btn btn-danger" onClick={() => props.onAnswer("play_anyway")}>
          Play anyway
        </button>
      </footer>
    </Screen>
  );
}

export function PendingView(props: {
  choices: PendingChoice[];
  onOpen: (choice: PendingChoice) => void;
  onBack: () => void;
}) {
  return (
    <Screen title="Pending choices" onBack={props.onBack}>
      <ul className="list">
        {props.choices.map((c) => (
          <li key={c.game}>
            <button
              data-focus={`choice:${c.game}`}
              className="btn row"
              onClick={() => props.onOpen(c)}
            >
              <span className="name">{c.game}</span>
              <span className="meta">
                {c.kind === "conflict"
                  ? "Both sides changed"
                  : `Open on ${c.held?.device ?? "another machine"}`}
                {" · "}
                {when(c.time)}
              </span>
            </button>
          </li>
        ))}
        {props.choices.length === 0 && <li className="meta">Nothing is waiting for you.</li>}
      </ul>
    </Screen>
  );
}

export function TimelineView(props: {
  history: HistoryData;
  pictureUrl: (game: string, entry: string, file: string) => string | null;
  /** The entry waiting for its confirm, if any. */
  confirm: string | null;
  error: string | null;
  notice: string | null;
  onAsk: (entry: string) => void;
  onConfirm: () => void;
  onCancel: () => void;
  onBack: () => void;
}) {
  const { history, confirm } = props;
  if (confirm) {
    const entry = history.entries.find((e) => e.entry === confirm);
    return (
      <Screen title="Restore these saves?" onBack={props.onCancel} initial="cancel">
        <ErrorBanner message={props.error} />
        <p className="big">
          {entry?.card?.place ?? confirm} · {entry ? when(entry.time) : ""}
        </p>
        <p>
          Your saves on the server become a copy of this entry. Unsynced changes on this machine
          are kept as a backup first.
        </p>
        <footer>
          <button data-focus="cancel" className="btn" onClick={props.onCancel}>
            Cancel
          </button>
          <button data-focus="confirm" className="btn btn-primary" onClick={props.onConfirm}>
            Restore
          </button>
        </footer>
      </Screen>
    );
  }
  return (
    <Screen title={`${history.game}: history`} onBack={props.onBack}>
      <ErrorBanner message={props.error} />
      {props.notice && <p className="notice">{props.notice}</p>}
      <ul className="list">
        {history.entries.map((e) => {
          const src = e.card?.picture ? props.pictureUrl(history.game, e.entry, e.card.picture) : null;
          const time = playtime(e.card?.playtime_secs ?? null);
          return (
            <li key={e.entry} className="entry">
              {src ? (
                <img className="picture" src={src} alt="" />
              ) : (
                <div className="picture none" aria-hidden="true" />
              )}
              <div className="text">
                <p className="big">{e.card?.place ?? "No card (older save)"}</p>
                <p>
                  {e.device} · {when(e.time)}
                  {time ? ` · played ${time}` : ""}
                </p>
              </div>
              {e.current ? (
                <span className="badge">Current</span>
              ) : (
                <button
                  data-focus={`restore:${e.entry}`}
                  className="btn"
                  onClick={() => props.onAsk(e.entry)}
                >
                  Restore
                </button>
              )}
            </li>
          );
        })}
      </ul>
    </Screen>
  );
}

/** The QR code as an inline SVG (sync, so it renders on the server and in tests too). */
export function QrCode(props: { text: string }) {
  const qr = QRCode.create(props.text, { errorCorrectionLevel: "M" });
  const n = qr.modules.size;
  const quiet = 4;
  let d = "";
  for (let y = 0; y < n; y++) {
    for (let x = 0; x < n; x++) {
      if (qr.modules.get(x, y)) d += `M${x + quiet} ${y + quiet}h1v1h-1z`;
    }
  }
  const size = n + quiet * 2;
  return (
    <svg
      className="qr"
      role="img"
      aria-label={`QR code for ${props.text}`}
      viewBox={`0 0 ${size} ${size}`}
      shapeRendering="crispEdges"
    >
      <rect width={size} height={size} fill="#fff" />
      <path d={d} fill="#000" />
    </svg>
  );
}

export function SignInView(props: {
  prompt: SignInPrompt | null;
  error: string | null;
  onPick: (service: "gdrive" | "dropbox") => void;
  onBack: () => void;
}) {
  const { prompt } = props;
  if (!prompt) {
    return (
      <Screen title="Sign in" onBack={props.onBack}>
        <ErrorBanner message={props.error} />
        <p>Nextcloud is set up on the command line, not here.</p>
        <footer>
          <button data-focus="gdrive" className="btn" onClick={() => props.onPick("gdrive")}>
            Google Drive
          </button>
          <button data-focus="dropbox" className="btn" onClick={() => props.onPick("dropbox")}>
            Dropbox
          </button>
        </footer>
      </Screen>
    );
  }
  return (
    <Screen title={`Sign in to ${prompt.service === "gdrive" ? "Google Drive" : "Dropbox"}`} onBack={props.onBack}>
      <ErrorBanner message={props.error} />
      <div className="signin">
        <div className="qr-box">
          <QrCode text={prompt.url} />
        </div>
        <div>
          <p>Scan with a phone, or open:</p>
          <p className="address">{prompt.url}</p>
          {prompt.code && (
            <p>
              Code: <strong className="code">{prompt.code}</strong>
            </p>
          )}
          <p className="meta">{prompt.message}</p>
          <p className="meta">Expires {when(prompt.expires_at)}.</p>
        </div>
      </div>
    </Screen>
  );
}
