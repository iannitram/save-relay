import { describe, expect, it } from "vitest";
import { httpBridge, TOKEN_HEADER } from "./bridge.ts";

describe("httpBridge", () => {
  it("posts the args as JSON to /api/<command> with the token", async () => {
    const seen: { url: string; headers: Record<string, string>; body: string }[] = [];
    const bridge = httpBridge("http://127.0.0.1:4000", "t0k3n", async (url, init) => {
      seen.push({ url, headers: init.headers, body: init.body });
      return { json: async () => ({ ok: true, command: "status", data: { game: "celeste" } }) };
    });

    const envelope = await bridge.call<{ game: string }>("status", { game: "celeste" });

    expect(seen).toEqual([
      {
        url: "http://127.0.0.1:4000/api/status",
        headers: { "Content-Type": "application/json", [TOKEN_HEADER]: "t0k3n" },
        body: '{"game":"celeste"}',
      },
    ]);
    expect(envelope).toEqual({ ok: true, command: "status", data: { game: "celeste" } });
  });
});
