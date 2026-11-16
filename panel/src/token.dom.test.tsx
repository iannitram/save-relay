// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";

const render = vi.fn();
vi.mock("react-dom/client", () => ({ createRoot: () => ({ render }) }));
afterEach(() => {
  document.body.innerHTML = "";
  document.querySelector('meta[name="save-relay-token"]')?.remove();
  window.history.replaceState(null, "", "/");
  vi.unstubAllGlobals();
  vi.resetModules();
});

it("reads the injected token, removes the legacy URL token, and uses the HTTP bridge", async () => {
  document.head.insertAdjacentHTML("beforeend", '<meta name="save-relay-token" content="injected-secret">');
  document.body.innerHTML = '<div id="root"></div>';
  window.history.replaceState(null, "", "/?token=old-secret&other=keep#here");
  const fetcher = vi.fn(async () => ({ text: async () => JSON.stringify({ ok: true }) }));
  vi.stubGlobal("fetch", fetcher);
  await import("./main.tsx");
  expect(window.location.search).toBe("?other=keep");
  expect(window.location.hash).toBe("#here");
  const app = render.mock.calls.at(-1)![0];
  expect(app.props.token).toBe("injected-secret");
  await app.props.bridge.call("status", { game: "g" });
  expect(fetcher).toHaveBeenCalledWith(`${window.location.origin}/api/status`, expect.objectContaining({
    headers: expect.objectContaining({ "X-Save-Relay-Token": "injected-secret" }),
  }));
});
