import { describe, expect, it } from "vitest";
import { deckyBridge } from "./bridge.ts";

describe("deckyBridge", () => {
  it("passes the command to the backend and parses its envelope line", async () => {
    const calls: unknown[] = [];
    const bridge = deckyBridge(async (command, args) => {
      calls.push([command, args]);
      return '{"ok":true,"command":"where","data":{"games":[]}}';
    });

    const envelope = await bridge.call<{ games: unknown[] }>("where", {});

    expect(calls).toEqual([["where", {}]]);
    expect(envelope).toEqual({ ok: true, command: "where", data: { games: [] } });
  });
});

describe("invalid backend output", () => {
  it.each(["not JSON", "", "{}\n{}"])("rejects a bad or empty envelope line: %j", async (line) => {
    await expect(deckyBridge(async () => line).call("where", {})).rejects.toThrow();
  });

  it("preserves a CLI error and future fields", async () => {
    const envelope = { ok: false, command: "status", error: { code: "io", message: "Unavailable" }, future: 42 };
    const bridge = deckyBridge(async () => JSON.stringify(envelope));
    expect(await bridge.call("status", { game: "g; echo unsafe" })).toEqual(envelope);
  });

  it("propagates a backend transport failure for the panel to show", async () => {
    const bridge = deckyBridge(async () => { throw new Error("Decky disconnected"); });
    await expect(bridge.call("where", {})).rejects.toThrow("Decky disconnected");
  });
});
