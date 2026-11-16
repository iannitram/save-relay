import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { API_COMMANDS, httpBridge, TOKEN_HEADER, type ApiArgs, type ApiCommand } from "./bridge.ts";

describe("httpBridge", () => {
  it("includes the M16 API commands and accepts numeric app-id arrays", () => {
    const command: ApiCommand = "launch-option";
    const args: ApiArgs = { "app-ids": [504230, 504231], "no-steam-cloud": true };
    expect(API_COMMANDS).toContain(command);
    expect(API_COMMANDS).toContain("steam-games");
    expect(API_COMMANDS).toContain("install-wrap");
    expect(args["app-ids"]).toEqual([504230, 504231]);
  });

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

describe("httpBridge with a reply that is not JSON", () => {
  it("returns an error envelope that names the status, not 'Unexpected token'", async () => {
    const bridge = httpBridge("http://127.0.0.1:4000", "t", async () => ({
      status: 403,
      json: async () => {
        throw new SyntaxError("Unexpected token 'f', \"forbidden\" is not valid JSON");
      },
      text: async () => "forbidden",
    }));
    const envelope = await bridge.call("status", { game: "g" });
    expect(envelope.ok).toBe(false);
    if (envelope.ok) return;
    expect(envelope.command).toBe("status");
    expect(envelope.error.message).toContain("403");
    expect(envelope.error.message).not.toContain("Unexpected token");
  });
});

it("exposes only the shared panel commands and matches the Rust HTTP contract", () => {
  expect([...API_COMMANDS]).toEqual([
    "status", "history", "restore", "pull", "answer", "where", "setup",
    "steam-games", "install-wrap", "launch-option",
  ]);
  const rust = readFileSync(new URL("../../src/panel.rs", import.meta.url), "utf8");
  const list = rust.match(/pub const API_COMMANDS[^=]*= \[([\s\S]*?)\];/)![1]!;
  expect([...list.matchAll(/"([^"]+)"/g)].map((m) => m[1])).toEqual([...API_COMMANDS]);
});
