// The panel: loads data over the Bridge and routes between the views (views.tsx). The Decky
// plugin renders this same component inside Game Mode.

import { useCallback, useEffect, useState, type Dispatch, type SetStateAction } from "react";
import type {
  Answer,
  HistoryData,
  InstallWrapData,
  LaunchOptionData,
  PendingChoice,
  SignInPrompt,
  StatusData,
  SteamGamesData,
  WhereData,
} from "./api.ts";
import { call, choose, PanelError, restore, signIn } from "./actions.ts";
import type { Bridge } from "./bridge.ts";
import {
  ConflictView,
  AddGamesView,
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
  | { view: "add-games" }
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
  const [addedOverrides, setAddedOverrides] = useState<Record<number, boolean>>({});

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
          onAddGames={() => go({ view: "add-games" })}
        />
      );
    case "add-games":
      return (
        <AddGamesRoute
          bridge={bridge}
          added={addedOverrides}
          setAdded={setAddedOverrides}
          onBack={() => home("add-games")}
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

function steamRunningMessage(error: unknown) {
  return error instanceof PanelError && error.code === "steam_running"
    ? "Close Steam, then press Add again."
    : message(error);
}

function AddGamesRoute(props: {
  bridge: Bridge;
  added: Readonly<Record<number, boolean>>;
  setAdded: Dispatch<SetStateAction<Record<number, boolean>>>;
  onBack: () => void;
}) {
  const [games, setGames] = useState<SteamGamesData["games"] | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [busyAppId, setBusyAppId] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async (refresh = false) => {
    setLoading(true);
    try {
      const data = await call<SteamGamesData>(props.bridge, "steam-games", refresh ? { refresh: true } : {});
      setGames(data.games);
      setError(null);
    } catch (e) {
      setError(message(e));
    } finally {
      setLoading(false);
    }
  }, [props.bridge]);

  useEffect(() => { void load(); }, [load]);

  const change = useCallback(async (appId: number, remove: boolean) => {
    if (props.bridge.getLaunchOption && props.bridge.setLaunchOption) {
      const current = await props.bridge.getLaunchOption(appId);
      const data = await call<LaunchOptionData>(props.bridge, "launch-option", {
        appid: appId,
        ...(remove ? { remove: true } : {}),
        current,
      });
      await props.bridge.setLaunchOption(appId, data.after ?? "");
    } else {
      const result = await call<InstallWrapData>(props.bridge, "install-wrap", {
        "app-ids": [appId],
        ...(remove ? { remove: true } : {}),
      });
      if (!result.changed.some((row) => row.app_id === appId)) {
        const skipped = result.skipped.find((row) => row.app_id === appId);
        throw new Error(skipped ? `${skipped.name}: ${skipped.reason}` : "Steam did not report a launch option change.");
      }
    }
    props.setAdded((previous) => ({ ...previous, [appId]: !remove }));
  }, [props.bridge, props.setAdded]);

  const act = useCallback(async (appId: number, remove: boolean) => {
    const gameName = games?.find((game) => game.app_id === appId)?.name ?? String(appId);
    setBusy(true);
    setBusyAppId(appId);
    setError(null);
    setNotice(null);
    try {
      await change(appId, remove);
      setNotice(`${remove ? "Removed" : "Added"} ${gameName}.`);
    } catch (e) {
      setError(steamRunningMessage(e));
    } finally {
      setBusy(false);
      setBusyAppId(null);
    }
  }, [change, games]);

  const addMany = useCallback(async (appIds: number[]) => {
    if (appIds.length === 0) return;
    setBusy(true);
    setBusyAppId(null);
    setError(null);
    setNotice(null);
    let succeeded = 0;
    const failures: string[] = [];
    if (!props.bridge.getLaunchOption || !props.bridge.setLaunchOption) {
      try {
        const result = await call<InstallWrapData>(props.bridge, "install-wrap", { "app-ids": appIds });
        const changedIds = result.changed.map((row) => row.app_id).filter((id) => appIds.includes(id));
        props.setAdded((previous) => ({ ...previous, ...Object.fromEntries(changedIds.map((id) => [id, true])) }));
        succeeded = changedIds.length;
        if (succeeded !== appIds.length) {
          failures.push(...result.skipped
            .filter((row) => appIds.includes(row.app_id))
            .map((row) => `${row.name}: ${row.reason}`));
          if (failures.length === 0) failures.push("Steam did not report a launch option change for every selected game.");
        }
      } catch (e) {
        failures.push(steamRunningMessage(e));
      }
    } else {
      for (const appId of appIds) {
        setBusyAppId(appId);
        const name = games?.find((game) => game.app_id === appId)?.name ?? String(appId);
        try {
          await change(appId, false);
          succeeded++;
        } catch (e) {
          failures.push(`${name}: ${steamRunningMessage(e)}`);
          if (e instanceof PanelError && e.code === "steam_running") break;
        }
      }
    }
    setBusy(false);
    setBusyAppId(null);
    if (failures.length) {
      setError(`${succeeded} of ${appIds.length} added. ${failures.join(" ")}`);
    } else {
      setNotice(`Added ${succeeded} game${succeeded === 1 ? "" : "s"} without Steam Cloud.`);
    }
  }, [change, games, props.bridge, props.setAdded]);

  return (
    <AddGamesView
      games={games}
      added={props.added}
      loading={loading}
      busy={busy}
      busyAppId={busyAppId}
      error={error}
      notice={notice}
      onRefresh={() => void load(true)}
      onAdd={(appId) => void act(appId, false)}
      onRemove={(appId) => void act(appId, true)}
      onAddMany={(appIds) => void addMany(appIds)}
      onBack={props.onBack}
    />
  );
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
