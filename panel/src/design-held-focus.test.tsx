// Locked tests for #172 (design pass 2026-11-06): the open-elsewhere view's first focus is Wait.
import type { ReactElement, ReactNode } from "react";
import { isValidElement } from "react";
import { describe, expect, it } from "vitest";
import type { PendingChoice } from "./api.ts";
import statusHeld from "./fixtures/status-pending-held.json";
import { HeldView, Screen } from "./views.tsx";

function deepFreeze<T>(o: T): T {
  if (o && typeof o === "object" && !Object.isFrozen(o)) {
    Object.freeze(o);
    for (const v of Object.values(o)) deepFreeze(v);
  }
  return o;
}
/** Every `data-focus` id in document order (views are hook-free; Screen's children are walked, not Screen). */
function order(node: ReactNode, out: string[] = []): string[] {
  if (Array.isArray(node)) {
    for (const n of node) order(n, out);
    return out;
  }
  if (!isValidElement(node)) return out;
  const props = node.props as any;
  if (props["data-focus"]) out.push(props["data-focus"]);
  if (typeof node.type === "function" && node.type !== Screen) {
    return order((node.type as (p: unknown) => ReactNode)(props), out);
  }
  return order(props.children, out);
}

const held = deepFreeze(structuredClone(statusHeld.data.pending)) as PendingChoice;
const back = () => undefined;
const view = () =>
  HeldView({ game: "celeste", held: held.held!, error: null, notice: null, onAnswer: () => undefined, onBack: back }) as ReactElement<any>;

describe("open elsewhere: first focus (#172)", () => {
  it("renders inside Screen", () => {
    expect(view().type).toBe(Screen);
  });
  it("gives the first focus to Wait", () => {
    expect(view().props.initial).toBe("wait");
  });
  it("puts Wait before Play anyway", () => {
    const ids = order(view().props.children);
    expect(ids.indexOf("wait")).toBeGreaterThanOrEqual(0);
    expect(ids.indexOf("wait")).toBeLessThan(ids.indexOf("anyway"));
  });
  it("still goes back with B (Screen gets onBack)", () => {
    expect(view().props.onBack).toBe(back);
  });
});
