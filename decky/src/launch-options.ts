import type { Unregisterable } from "@decky/ui/dist/globals/steam-client/shared";

export interface SteamAppDetails {
  strLaunchOptions: string;
}

/** The two Steam client methods used by Add games, kept narrow for mocked regression tests. */
export interface SteamLaunchOptionsApi {
  RegisterForAppDetails?: (
    appId: number,
    callback: (details: SteamAppDetails) => void,
  ) => Unregisterable;
  SetAppLaunchOptions?: (appId: number, launchOptions: string) => void;
}

const WAIT_MS = 2_000;

function detailsUntil(
  apps: SteamLaunchOptionsApi,
  appId: number,
  accept: (launchOptions: string) => boolean,
  waitMs: number,
): Promise<string> {
  if (typeof apps.RegisterForAppDetails !== "function") {
    return Promise.reject(new Error("Steam app details are unavailable. Add games from Desktop Mode with Steam closed."));
  }

  return new Promise((resolve, reject) => {
    let subscription: Unregisterable | undefined;
    let settled = false;
    let unregistered = false;
    const timer = setTimeout(() => finish(new Error("Steam app details did not respond in time.")), waitMs);

    const cleanup = () => {
      clearTimeout(timer);
      if (subscription && !unregistered) {
        unregistered = true;
        try {
          subscription.unregister();
        } catch {
          // A stale Steam registration must not obscure the original result.
        }
      }
    };
    const finish = (error: Error | null, value?: string) => {
      if (settled) return;
      settled = true;
      cleanup();
      if (error) reject(error);
      else resolve(value ?? "");
    };

    try {
      subscription = apps.RegisterForAppDetails!(appId, (details) => {
        const current = details && typeof details.strLaunchOptions === "string"
          ? details.strLaunchOptions
          : null;
        if (current === null) return;
        if (accept(current)) finish(null, current);
      });
      // Steam may deliver cached details synchronously from RegisterForAppDetails.
      if (settled) cleanup();
    } catch (error) {
      finish(error instanceof Error ? error : new Error(String(error)));
    }
  });
}

/** Read Steam's current launch option with a bounded callback lifetime. */
export function readLaunchOption(
  apps: SteamLaunchOptionsApi,
  appId: number,
  waitMs = WAIT_MS,
): Promise<string> {
  return detailsUntil(apps, appId, () => true, waitMs);
}

/**
 * Ask Steam to set the launch option and wait for its app-details echo. The client setter returns
 * void, so callers only mark a row Added after the requested value is observed.
 */
export async function writeLaunchOption(
  apps: SteamLaunchOptionsApi,
  appId: number,
  text: string,
  waitMs = WAIT_MS,
): Promise<void> {
  if (
    typeof apps.SetAppLaunchOptions !== "function" ||
    typeof apps.RegisterForAppDetails !== "function"
  ) {
    throw new Error("Steam cannot set launch options in this session. Add games from Desktop Mode with Steam closed.");
  }
  apps.SetAppLaunchOptions(appId, text);
  const echoed = await detailsUntil(apps, appId, (current) => current === text, waitMs);
  if (echoed !== text) throw new Error("Steam did not confirm the launch option.");
}
