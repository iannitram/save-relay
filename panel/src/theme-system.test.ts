// @vitest-environment jsdom
// The system-theme wiring in main.tsx (#177 review fix): matchMedia missing, a MediaQueryList
// with addEventListener, one with only the older addListener, and one with neither.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type Listener = (event: { matches: boolean }) => void;

function fakeMediaQueryList(matches: boolean, api: "both" | "addListener" | "none") {
  const listeners: Listener[] = [];
  const mql: Record<string, unknown> = { matches, media: "(prefers-color-scheme: light)" };
  if (api === "both") mql.addEventListener = (_type: string, l: Listener) => listeners.push(l);
  if (api === "both" || api === "addListener") mql.addListener = (l: Listener) => listeners.push(l);
  return { mql, fire: (m: boolean) => listeners.forEach((l) => l({ matches: m })) };
}

function stubMatchMedia(mql: unknown) {
  Object.defineProperty(window, "matchMedia", { value: vi.fn(() => mql), configurable: true, writable: true });
}

/** Import main.tsx fresh; with no #root element it only wires the theme and renders nothing. */
async function loadMain() {
  vi.resetModules();
  await import("./main.tsx");
}

beforeEach(() => {
  document.documentElement.removeAttribute("data-theme");
});

afterEach(() => {
  delete (window as { matchMedia?: unknown }).matchMedia;
});

describe("theme when window.matchMedia is missing (#177)", () => {
  it("stays dark and does not throw", async () => {
    expect(window.matchMedia).toBeUndefined();
    await expect(loadMain()).resolves.toBeUndefined();
    expect(document.documentElement.dataset.theme).toBe("dark");
  });
});

describe("theme when the MediaQueryList has addEventListener (#177)", () => {
  it("starts light when the system prefers light, and follows a change", async () => {
    const q = fakeMediaQueryList(true, "both");
    stubMatchMedia(q.mql);
    await loadMain();
    expect(document.documentElement.dataset.theme).toBe("light");
    q.fire(false);
    expect(document.documentElement.dataset.theme).toBe("dark");
  });
});

describe("theme when the MediaQueryList has only addListener (#177)", () => {
  it("subscribes with addListener and follows a change", async () => {
    const q = fakeMediaQueryList(false, "addListener");
    stubMatchMedia(q.mql);
    await loadMain();
    expect(document.documentElement.dataset.theme).toBe("dark");
    q.fire(true);
    expect(document.documentElement.dataset.theme).toBe("light");
  });
});

describe("theme when the MediaQueryList has neither listener method (#177)", () => {
  it("applies the current setting, skips live updates and does not throw", async () => {
    const q = fakeMediaQueryList(true, "none");
    stubMatchMedia(q.mql);
    await expect(loadMain()).resolves.toBeUndefined();
    expect(document.documentElement.dataset.theme).toBe("light");
  });
});
