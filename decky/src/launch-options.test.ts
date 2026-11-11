import { afterEach, describe, expect, it, vi } from "vitest";
import type { SteamAppDetails, SteamLaunchOptionsApi } from "./launch-options.ts";
import { readLaunchOption, writeLaunchOption } from "./launch-options.ts";

afterEach(() => vi.useRealTimers());

describe("Steam launch option callbacks", () => {
  it("reads an explicitly empty value and unregisters the app-details callback", async () => {
    const unregister = vi.fn();
    const apps: SteamLaunchOptionsApi = {
      RegisterForAppDetails: (_appId, callback) => {
        callback({ strLaunchOptions: "" });
        return { unregister };
      },
    };
    await expect(readLaunchOption(apps, 42)).resolves.toBe("");
    expect(unregister).toHaveBeenCalledOnce();
  });

  it("ignores missing or malformed launch-option data instead of treating it as empty", async () => {
    let callback!: (details: SteamAppDetails) => void;
    const unregister = vi.fn();
    const apps: SteamLaunchOptionsApi = {
      RegisterForAppDetails: (_appId, registered) => {
        callback = registered;
        queueMicrotask(() => {
          registered({} as SteamAppDetails);
          registered({ strLaunchOptions: "-novid %command%" });
        });
        return { unregister };
      },
    };
    await expect(readLaunchOption(apps, 42)).resolves.toBe("-novid %command%");
    expect(callback).toBeDefined();
    expect(unregister).toHaveBeenCalledOnce();
  });

  it("waits for the requested echo after the void setter", async () => {
    let current = "-novid %command%";
    const callbacks = new Set<(details: SteamAppDetails) => void>();
    const unregister = vi.fn();
    const apps: SteamLaunchOptionsApi = {
      RegisterForAppDetails: (_appId, callback) => {
        callbacks.add(callback);
        callback({ strLaunchOptions: current });
        return { unregister: () => { callbacks.delete(callback); unregister(); } };
      },
      SetAppLaunchOptions: vi.fn((_appId, text) => {
        queueMicrotask(() => {
          current = text;
          for (const callback of callbacks) callback({ strLaunchOptions: current });
        });
      }),
    };
    await expect(writeLaunchOption(apps, 42, "save-relay wrap -- -novid %command%")).resolves.toBeUndefined();
    expect(apps.SetAppLaunchOptions).toHaveBeenCalledWith(42, "save-relay wrap -- -novid %command%");
    expect(unregister).toHaveBeenCalledOnce();
  });

  it("times out and unregisters when Steam does not echo the requested value", async () => {
    vi.useFakeTimers();
    const unregister = vi.fn();
    const apps: SteamLaunchOptionsApi = {
      RegisterForAppDetails: (_appId, callback) => {
        callback({ strLaunchOptions: "old" });
        return { unregister };
      },
      SetAppLaunchOptions: vi.fn(),
    };
    const pending = writeLaunchOption(apps, 42, "new", 25);
    const failure = expect(pending).rejects.toThrow("Steam app details did not respond in time");
    await vi.advanceTimersByTimeAsync(25);
    await failure;
    expect(unregister).toHaveBeenCalledOnce();
  });

  it("does not write if either client method is unavailable", async () => {
    const set = vi.fn();
    await expect(writeLaunchOption({ SetAppLaunchOptions: set }, 42, "new")).rejects.toThrow("Desktop Mode");
    expect(set).not.toHaveBeenCalled();
  });
});
