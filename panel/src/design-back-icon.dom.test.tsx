// @vitest-environment jsdom
// Locked tests for #178 (design pass 2026-11-06): the Back button still goes back after the icon change.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { Screen } from "./views.tsx";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe("Back button handler (#178)", () => {
  it("pressing Back calls onBack", () => {
    let backs = 0;
    act(() => root.render(<Screen title="hades: history" onBack={() => { backs += 1; }}><p>x</p></Screen>));
    const back = host.querySelector<HTMLButtonElement>('button[data-focus="back"]')!;
    expect(back.className).toBe("btn");
    act(() => back.click());
    expect(backs).toBe(1);
  });
});
