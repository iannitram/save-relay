// @vitest-environment jsdom
// Controller focus in a real DOM: views whose controls load late, and keys pressed while
// focus has fallen to <body>.
import { act, useEffect, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import setupGdrive from "./fixtures/setup-gdrive.json";
import type { SignInPrompt } from "./api.ts";
import { Screen, SignInView } from "./views.tsx";

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

const focused = () => (document.activeElement as HTMLElement | null)?.dataset?.focus ?? null;

function press(target: EventTarget, key: string) {
  act(() => {
    target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
  });
}

/** A view whose only control (Back) appears once an async load answers. */
function LateView(props: { load: Promise<void>; onBack: () => void }) {
  const [ready, setReady] = useState(false);
  useEffect(() => {
    props.load.then(() => setReady(true));
  }, [props.load]);
  return (
    <Screen title="Late" onBack={ready ? props.onBack : null}>
      <p>{ready ? "Loaded" : "Loading…"}</p>
    </Screen>
  );
}

/** SignIn as App.tsx wires it: picking a service swaps the picker for the QR view. */
function SignIn(props: { answer: Promise<SignInPrompt>; onBack: () => void }) {
  const [prompt, setPrompt] = useState<SignInPrompt | null>(null);
  return (
    <SignInView
      prompt={prompt}
      error={null}
      onPick={() => void props.answer.then(setPrompt)}
      onBack={() => (prompt ? setPrompt(null) : props.onBack())}
    />
  );
}

describe("a view whose controls appear after it mounts", () => {
  it("gets focus on the control once it appears, and Escape goes back", async () => {
    let answer!: () => void;
    const load = new Promise<void>((r) => (answer = r));
    const onBack = vi.fn();
    act(() => root.render(<LateView load={load} onBack={onBack} />));
    expect(focused()).toBeNull();
    await act(async () => answer());
    expect(focused()).toBe("back");
    press(document.activeElement!, "Escape");
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it("QR sign-in: focus lands on Back after the setup call answers", async () => {
    const onBack = vi.fn();
    const answer = Promise.resolve(setupGdrive.data as SignInPrompt);
    act(() => root.render(<SignIn answer={answer} onBack={onBack} />));
    expect(focused()).toBe("back");
    press(document.activeElement!, "ArrowDown");
    expect(focused()).toBe("gdrive");
    await act(async () => (document.activeElement as HTMLElement).click());
    expect(host.textContent).toContain("Sign in to Google Drive");
    expect(focused()).toBe("back");
    // Escape leaves the QR view for the picker, then the picker for the caller.
    press(document.activeElement!, "Escape");
    expect(host.textContent).not.toContain("Sign in to Google Drive");
    expect(focused()).not.toBeNull();
    press(document.activeElement!, "Escape");
    expect(onBack).toHaveBeenCalledTimes(1);
  });
});

describe("keys pressed while focus is on <body>", () => {
  it("Escape goes back from the current view", () => {
    const onBack = vi.fn();
    act(() => root.render(<Screen title="Plain" onBack={onBack}><p>Nothing</p></Screen>));
    act(() => (document.activeElement as HTMLElement).blur());
    expect(document.activeElement).toBe(document.body);
    press(document.body, "Escape");
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it("an arrow puts focus back into the view", () => {
    act(() => root.render(<SignInView prompt={null} error={null} onPick={() => {}} onBack={() => {}} />));
    act(() => (document.activeElement as HTMLElement).blur());
    press(document.body, "ArrowDown");
    expect(focused()).toBe("back");
  });
});

describe("the focus ring", () => {
  it("shows on the control focused at first load, before any key press", () => {
    act(() => root.render(<SignInView prompt={null} error={null} onPick={() => {}} onBack={() => {}} />));
    const el = document.activeElement as HTMLElement;
    expect(el.dataset.focus).toBe("back");
    expect(el.hasAttribute("data-focused")).toBe(true);
  });

  it("follows focus, and a mouse click does not show it", () => {
    act(() => root.render(<SignInView prompt={null} error={null} onPick={() => {}} onBack={() => {}} />));
    press(document.activeElement!, "ArrowDown");
    const gdrive = host.querySelector<HTMLElement>('[data-focus="gdrive"]')!;
    expect(gdrive.hasAttribute("data-focused")).toBe(true);
    expect(host.querySelectorAll("[data-focused]")).toHaveLength(1);
    const dropbox = host.querySelector<HTMLElement>('[data-focus="dropbox"]')!;
    act(() => {
      dropbox.dispatchEvent(new Event("pointerdown", { bubbles: true }));
      dropbox.focus();
    });
    expect(host.querySelectorAll("[data-focused]")).toHaveLength(0);
    press(dropbox, "ArrowUp");
    expect(focused()).toBe("gdrive");
    expect(gdrive.hasAttribute("data-focused")).toBe(true);
  });
});
