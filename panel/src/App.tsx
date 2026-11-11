// The panel: loads data over the Bridge and routes between the views (views.tsx). The Decky
// plugin renders this same component inside Game Mode.

import { useCallback, useEffect, useState } from "react";
import type { HistoryData, PendingChoice, StatusData, WhereData, SignInPrompt, Answer } from "./api.ts";
import { call, choose, restore, signIn } from "./actions.ts";
import type { Bridge } from "./bridge.ts";
import {
  ConflictView,
  GamesView,
  HeldView,
  LoadingView,
  PendingView,
  SignInView,
  TimelineView,
  type GameRow,
} from "./views.tsx";

export interface AppProps {
  bridge: Bridge;
  /** Where an entry's picture is served; the default is the panel server's, with the token. */
  pictureUrl?: (game: string, entry: string, file: string) => string | null;
  /** The panel's token (the URL's `token`), which the default picture URL carries. */
  token?: string | null;
}

type Route =
  | { view: "games"; initial?: string }
  | { view: "pending" }
  | { view: "timeline"; game: string }
  | { view: "choice"; choice: PendingChoice; from: "games" | "pending" }
  | { view: "signin" };

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export function App({ bridge, pictureUrl, token }: AppProps) {
  const [route, setRoute] = useState<Route>({ view: "games" });
  const [rows, setRows] = useState<GameRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const { games } = await call<WhereData>(bridge, "where", {});
      const loaded = await Promise.all(
        games.map(async (g): Promise<GameRow> => {
          const status = await call<StatusData>(bridge, "status", { game: g.game }).catch(() => null);
          return { ...g, pending: status?.pending ?? null };
        }),
      );
      setRows(loaded);
      setError(null);
    } catch (e) {
      setError(message(e));
    }
  }, [bridge]);

  useEffect(() => {
    void load();
  }, [load]);

  const go = (next: Route) => {
    setError(null);
    setNotice(null);
    setRoute(next);
  };
  const home = (initial?: string) => {
    go({ view: "games", initial });
    void load();
  };

  switch (route.view) {
    case "games":
      return (
        <GamesView
          games={rows}
          error={error}
          initial={route.initial}
          onOpen={(game) => go({ view: "timeline", game })}
          onPending={() => go({ view: "pending" })}
          onSignIn={() => go({ view: "signin" })}
        />
      );
    case "pending":
      return (
        <PendingView
          choices={rows.flatMap((r) => (r.pending ? [r.pending] : []))}
          onOpen={(choice) => go({ view: "choice", choice, from: "pending" })}
          onBack={() => home("pending")}
        />
      );
    case "choice": {
      const { choice } = route;
      const back = () => (route.from === "pending" ? go({ view: "pending" }) : home(`game:${choice.game}`));
      const answer = (a: Answer) =>
        choose(bridge, { game: choice.game, pending: true }, a).then(
          (r) => {
            setError(null);
            setNotice(r.message);
            void load();
          },
          (e) => setError(message(e)),
        );
      return choice.kind === "conflict" ? (
        <ConflictView choice={choice} error={error} notice={notice} onAnswer={answer} onBack={back} />
      ) : (
        <HeldView
          game={choice.game}
          held={choice.held ?? { device: "another machine", since: choice.time }}
          error={error}
          notice={notice}
          onAnswer={answer}
          onBack={back}
        />
      );
    }
    case "timeline":
      return (
        <Timeline
          key={route.game}
          bridge={bridge}
          game={route.game}
          pictureUrl={pictureUrl ?? panelPicture(token)}
          onBack={() => home(`game:${route.game}`)}
        />
      );
    case "signin":
      return <SignIn bridge={bridge} onBack={() => home("signin")} />;
  }
}

/**
 * An entry's picture on the panel server: `/picture/<game>/<entry>/<file>`, each segment
 * percent-encoded, with the token as `?token=` (an `<img>` cannot send the header; src/panel.rs
 * `route`). Without a token the path alone, which the server refuses.
 */
export function panelPicture(token: string | null | undefined) {
  return (game: string, entry: string, file: string): string => {
    const path = `/picture/${encodeURIComponent(game)}/${encodeURIComponent(entry)}/${encodeURIComponent(file)}`;
    return token ? `${path}?token=${encodeURIComponent(token)}` : path;
  };
}

function Timeline(props: {
  bridge: Bridge;
  game: string;
  pictureUrl: (game: string, entry: string, file: string) => string | null;
  onBack: () => void;
}) {
  const [history, setHistory] = useState<HistoryData | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);
  const [opener, setOpener] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const { bridge, game } = props;

  const load = useCallback(
    () =>
      call<HistoryData>(bridge, "history", { game }).then(setHistory, (e) => setError(message(e))),
    [bridge, game],
  );
  useEffect(() => {
    void load();
  }, [load]);

  if (!history) {
    return <LoadingView title={`${game}: history`} error={error} onBack={props.onBack} />;
  }
  return (
    <TimelineView
      history={history}
      pictureUrl={props.pictureUrl}
      confirm={confirm}
      error={error}
      notice={notice}
      initial={opener ? `restore:${opener}` : undefined}
      onAsk={(entry) => {
        setError(null);
        setConfirm(entry);
        setOpener(entry);
      }}
      onCancel={() => setConfirm(null)}
      onConfirm={() => {
        const entry = confirm;
        setConfirm(null);
        if (entry)
          restore(bridge, game, entry).then(
            (r) => {
              setError(null);
              setNotice(r.message);
              return load();
            },
            (e) => setError(message(e)),
          );
      }}
      onBack={props.onBack}
    />
  );
}

function SignIn(props: { bridge: Bridge; onBack: () => void }) {
  const [prompt, setPrompt] = useState<SignInPrompt | null>(null);
  const [error, setError] = useState<string | null>(null);
  return (
    <SignInView
      prompt={prompt}
      error={error}
      onPick={(service) =>
        signIn(props.bridge, service).then(setPrompt, (e) => setError(message(e)))
      }
      onBack={() => (prompt ? setPrompt(null) : props.onBack())}
    />
  );
}
