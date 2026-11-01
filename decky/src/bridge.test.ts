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
