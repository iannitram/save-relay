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
  ariaLabel?: string;
  inertBackground?: boolean;
  children: ReactNode;
}) {
  const { ref, onKeyDown } = useFocusScope(props.onBack, props.initial);
  return (
    <section className="screen" ref={ref} onKeyDown={onKeyDown} aria-label={props.ariaLabel ?? props.title}>
      <header {...(props.inertBackground ? { inert: "" } : {})}>
        {props.onBack && (
          <button
            data-focus={props.inertBackground ? undefined : "back"}
            tabIndex={props.inertBackground ? -1 : undefined}
            className="btn"
            onClick={props.onBack}
          >
            <svg className="icon" viewBox="0 0 24 24" aria-hidden="true" focusable="false"><path d="M20 12 H5" /><path d="M11 6 L5 12 L11 18" /></svg>Back
          </button>
        )}
        <h1>{props.title}</h1>
      </header>
      {props.children}
    </section>
  );
}

export type BannerTone = "info" | "warning" | "danger" | "success";

const bannerIcons: Record<BannerTone, ReactNode> = {
  info: <><circle cx="12" cy="12" r="9.5" /><path d="M12 11 V17" /><path d="M12 7.2 V7.3" /></>,
  warning: <><path d="M12 3 L22 20 H2 Z" /><path d="M12 10 V14" /><path d="M12 17.2 V17.3" /></>,
  danger: <><circle cx="12" cy="12" r="9.5" /><path d="m9 9 6 6m0-6-6 6" /></>,
  success: <><circle cx="12" cy="12" r="9.5" /><path d="m7.5 12.5 3 3 6-7" /></>,
};

export function Banner(props: { tone: BannerTone; children: ReactNode; title?: string; role?: "status" | "alert" }) {
  return (
    <div className={`banner banner-${props.tone}`} role={props.role ?? "status"}>
      <svg className="banner-icon" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
        {bannerIcons[props.tone]}
      </svg>
      <div>
        {props.title && <p className="banner-title">{props.title}</p>}
        <p className="banner-text">{props.children}</p>
      </div>
    </div>
  );
}

export function ErrorBanner(props: { message: string | null }) {
  if (!props.message) return null;
  return <Banner tone="danger" role="alert">{props.message}</Banner>;
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
              <span className="name label">{g.game}</span>
              <span className="meta">
                {g.held ? `Open on ${g.held.device} since ${when(g.held.since)}` : "Not open"}
                {g.server ? ` · saved by ${g.server.device}` : " · nothing saved yet"}
              </span>
              {g.pending && <span className="badge badge-warning">Choice waiting</span>}
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

export function ChoiceCard(props: {
  focus: "keep-server" | "keep-local";
  eyebrow: string;
  action: string;
  side: Side | null;
  card: Card | null;
  onPick: () => void;
}) {
  const { side, card } = props;
  const time = playtime(card?.playtime_secs ?? null);
  return (
    <button
      type="button"
      data-focus={props.focus}
      className="choice-card ds-choice-card"
      onClick={props.onPick}
    >
      <p className="choice-eyebrow ds-choice-eyebrow">{props.eyebrow}</p>
      <p className="choice-place ds-choice-place">{card?.place ?? "No place recorded"}</p>
      {side ? (
        <p className="choice-facts ds-choice-facts">
          {side.device} · {when(side.time)}
        </p>
      ) : (
        <p className="choice-facts ds-choice-facts">No saves.</p>
      )}
      {time && <p className="choice-facts ds-choice-facts">Played {time}</p>}
      {side?.revision !== null && side?.revision !== undefined && (
        <p className="choice-facts ds-choice-facts">Revision {side.revision}</p>
      )}
      <span className="choice-action ds-choice-action">
        <span className="btn btn-primary ds-btn ds-btn-primary">{props.action}</span>
      </span>
    </button>
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
      {props.notice && <Banner tone="success">{props.notice}</Banner>}
      <Banner tone="info">Both this machine and the server changed. Nothing is overwritten until you choose.</Banner>
      <div className="cards choice ds-choice" role="group">
        <ChoiceCard
          focus="keep-server"
          eyebrow="On the server"
          action="Keep server"
          side={choice.server}
          card={choice.server_card}
          onPick={() => props.onAnswer("keep_server")}
        />
        <ChoiceCard
          focus="keep-local"
          eyebrow="On this machine"
          action="Keep local"
          side={choice.local}
          card={choice.local_card}
          onPick={() => props.onAnswer("keep_local")}
        />
      </div>
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
      {props.notice && <Banner tone="success">{props.notice}</Banner>}
      <Banner tone="warning" title={`Open on ${props.held.device} since ${when(props.held.since)}`}>
        Playing anyway can lose the other machine's newest saves.
      </Banner>
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
              <span className="name label">{c.game}</span>
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
  initial?: string;
  onAsk: (entry: string) => void;
  onConfirm: () => void;
  onCancel: () => void;
  onBack: () => void;
}) {
  const { history, confirm } = props;
  const entry = confirm ? history.entries.find((e) => e.entry === confirm) : null;
  return (
    <Screen
      title={`${history.game}: history`}
      ariaLabel={confirm ? "Restore these saves?" : undefined}
      onBack={confirm ? props.onCancel : props.onBack}
      initial={confirm ? "cancel" : (props.initial ?? "back")}
      inertBackground={Boolean(confirm)}
    >
      <ErrorBanner message={props.error} />
      {props.notice && <Banner tone="success">{props.notice}</Banner>}
      <ul className="list" {...(confirm ? { inert: "" } : {})}>
        {history.entries.map((e) => {
          const src = e.card?.picture ? props.pictureUrl(history.game, e.entry, e.card.picture) : null;
          const time = playtime(e.card?.playtime_secs ?? null);
          return (
            <li key={e.entry}>
              <div className="save">
                <span className="picture">
                  {src ? (
                    <img src={src} alt="" />
                  ) : (
                    <svg className="icon" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
                      <rect x="3" y="4" width="18" height="16" rx="2" />
                      <path d="M3 17 L9 11 L13 15 L16 12 L21 17" />
                    </svg>
                  )}
                </span>
                <span className="save-body">
                  <p className="save-place body-lg">{e.card?.place ?? "No place recorded"}</p>
                  <p className="save-facts meta">
                    {e.device} · {when(e.time)}
                    {time ? ` · played ${time}` : ""}
                  </p>
                </span>
                <span className="save-side">
                  {e.current ? (
                    <span className="badge">
                      <svg className="icon" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
                        <circle cx="12" cy="12" r="9.5" />
                        <path d="M7.5 12.5 L10.5 15.5 L16.5 9" />
                      </svg>
                      Current
                    </span>
                  ) : (
                    <button
                      data-focus={confirm ? undefined : `restore:${e.entry}`}
                      tabIndex={confirm ? -1 : undefined}
                      className="btn"
                      onClick={() => props.onAsk(e.entry)}
                    >
                      Restore
                    </button>
                  )}
                </span>
              </div>
            </li>
          );
        })}
      </ul>
      {confirm && (
        <div className="scrim">
          <div className="dialog" role="dialog" aria-modal="true" aria-label="Restore these saves?">
            <h2>Restore these saves?</h2>
            <p className="dialog-entry" style={{ fontWeight: 700 }}>
              {entry?.card?.place ?? confirm} · {entry ? when(entry.time) : ""}
            </p>
            <p className="muted">
              Your saves on the server become a copy of this entry. Unsynced changes on this machine
              are kept as a backup first.
            </p>
            <div className="dialog-actions">
              <button data-focus="cancel" className="btn" onClick={props.onCancel}>
                Cancel
              </button>
              <button data-focus="confirm" className="btn btn-primary" onClick={props.onConfirm}>
                Restore
              </button>
            </div>
          </div>
        </div>
      )}
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
        <Banner tone="info">Nextcloud is set up on the command line, not here.</Banner>
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
          <p className="qr-label">Scan it, or open this address</p>
          <p className="address">{prompt.url}</p>
          {prompt.code && (
            <>
              <p className="qr-label">Type this code</p>
              <p className="code">{prompt.code}</p>
            </>
          )}
          <p className="meta">{prompt.message}</p>
          <p className="meta">Expires {when(prompt.expires_at)}.</p>
        </div>
      </div>
    </Screen>
  );
}
