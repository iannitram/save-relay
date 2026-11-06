// Controller focus: the d-pad is the arrow keys, A is Enter (a focused button's click), B is
// Escape. Every control carries `data-focus="<id>"`; one handler per view moves focus among
// the visible, enabled ones in document order, and Escape goes back.

import { useEffect, useRef } from "react";
import type { KeyboardEvent } from "react";

export type FocusMove = { focus: string } | "back" | null;

/** The pure rule: which control gets focus after `key`, among `order` (wraps around). */
export function nextFocus(order: string[], current: string | null, key: string): FocusMove {
  if (key === "Escape") return "back";
  if (order.length === 0) return null;
  const at = current === null ? -1 : order.indexOf(current);
  if (key === "ArrowDown" || key === "ArrowRight") {
    return { focus: order[(at + 1) % order.length]! };
  }
  if (key === "ArrowUp" || key === "ArrowLeft") {
    return { focus: order[(at <= 0 ? order.length : at) - 1]! };
  }
  if (key === "Home") return { focus: order[0]! };
  if (key === "End") return { focus: order[order.length - 1]! };
  return null;
}

const CONTROLS = "[data-focus]:not([hidden]):not([disabled]):not([aria-hidden=true])";

function controls(root: HTMLElement): HTMLElement[] {
  const modal = root.querySelector<HTMLElement>('[role="dialog"][aria-modal="true"]');
  const scope = modal ?? root;
  return Array.from(scope.querySelectorAll<HTMLElement>(CONTROLS));
}

/** Focus has fallen out of every control: it is on <body>, or nowhere. */
function focusLost(doc: Document): boolean {
  const active = doc.activeElement;
  return active === null || active === doc.body || !active.isConnected;
}

// The focus ring. `:focus-visible` is not applied to focus set by code before any key press,
// so the scope marks the focused control with `data-focused` instead (styled as the ring).
// A mouse click does not mark it; the next key press does.
let pointer = false;
const ringDocs = new WeakSet<Document>();

function mark(el: Element | null) {
  if (!(el instanceof HTMLElement) || el.dataset.focus === undefined) return;
  // A page without window focus moves activeElement but fires no focusout: clear old marks.
  for (const other of el.ownerDocument.querySelectorAll<HTMLElement>("[data-focused]")) {
    if (other !== el) delete other.dataset.focused;
  }
  el.dataset.focused = "";
}

/** Focus `el` from the scope (not from a click), showing the ring unless the mouse is in use. */
function focusControl(el: HTMLElement | undefined) {
  if (!el) return;
  el.focus();
  if (!pointer && el.ownerDocument.activeElement === el) mark(el);
}

function installRing(doc: Document) {
  if (ringDocs.has(doc)) return;
  ringDocs.add(doc);
  doc.addEventListener("pointerdown", () => (pointer = true), true);
  doc.addEventListener(
    "keydown",
    () => {
      pointer = false;
      mark(doc.activeElement);
    },
    true,
  );
  doc.addEventListener("focusin", (e) => !pointer && mark(e.target as Element), true);
  doc.addEventListener(
    "focusout",
    (e) => e.target instanceof HTMLElement && delete e.target.dataset.focused,
    true,
  );
}

// The views showing, newest last. A key pressed while focus is on <body> goes to the newest.
const scopes: { key: (key: string) => boolean }[] = [];
const fallbackDocs = new WeakSet<Document>();

function installFallback(doc: Document) {
  if (fallbackDocs.has(doc)) return;
  fallbackDocs.add(doc);
  doc.addEventListener("keydown", (event) => {
    if (event.defaultPrevented || !focusLost(doc)) return;
    const scope = scopes[scopes.length - 1];
    if (scope?.key(event.key)) event.preventDefault();
  });
}

/**
 * Put on a view's root: focus moves into the view when it appears (to `initial`, else the
 * first control), and back into it whenever it is lost (a control that loads late, or the
 * focused one went away), so arrows/Escape work. A key pressed with focus on <body> still
 * reaches the view.
 */
export function useFocusScope(onBack: (() => void) | null, initial?: string) {
  const ref = useRef<HTMLElement>(null);
  const latest = useRef({ onBack, initial });
  latest.current = { onBack, initial };
  const userMoved = useRef(false);

  const enter = (root: HTMLElement) => {
    const all = controls(root);
    focusControl(all.find((el) => el.dataset.focus === latest.current.initial) ?? all[0]);
  };

  /** Handles `key` for this view; true when it did something. */
  const key = (k: string): boolean => {
    const root = ref.current;
    if (!root) return false;
    const all = controls(root);
    const current = (root.ownerDocument.activeElement as HTMLElement | null)?.dataset?.focus ?? null;
    const move = nextFocus(
      all.map((el) => el.dataset.focus ?? ""),
      current,
      k,
    );
    if (move === null) return false;
    userMoved.current = true;
    if (move === "back") latest.current.onBack?.();
    else focusControl(all.find((el) => el.dataset.focus === move.focus));
    return true;
  };
  const keyRef = useRef(key);
  keyRef.current = key;

  // While the view shows: whenever focus is lost, it goes back in. Controls that
  // appear later (after an async load) or a focused control that goes away both count.
  useEffect(() => {
    const root = ref.current;
    if (!root) return;
    const doc = root.ownerDocument;
    installRing(doc);
    installFallback(doc);
    const markMoved = () => {
      userMoved.current = true;
    };
    doc.addEventListener("pointerdown", markMoved, true);
    doc.addEventListener("keydown", markMoved, true);
    const refocus = () => {
      const modal = root.querySelector<HTMLElement>('[role="dialog"][aria-modal="true"]');
      if (modal && !modal.contains(doc.activeElement)) enter(root);
      else if (focusLost(doc) || !userMoved.current) enter(root);
    };
    const observer = new MutationObserver(refocus);
    observer.observe(root, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["hidden", "disabled", "aria-hidden"],
    });
    const scope = { key: (k: string) => keyRef.current(k) };
    scopes.push(scope);
    return () => {
      doc.removeEventListener("pointerdown", markMoved, true);
      doc.removeEventListener("keydown", markMoved, true);
      observer.disconnect();
      scopes.splice(scopes.indexOf(scope), 1);
    };
  }, []);

  // When the view appears, or `initial` changes: focus goes in.
  useEffect(() => {
    userMoved.current = false;
    const root = ref.current;
    if (root) enter(root);
  }, [initial]);

  // After every render too, for changes the observer has not reported yet.
  useEffect(() => {
    const root = ref.current;
    if (root && (focusLost(root.ownerDocument) || !userMoved.current)) enter(root);
  });

  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    userMoved.current = true;
    const root = ref.current;
    if (root) {
      const modal = root.querySelector<HTMLElement>('[role="dialog"][aria-modal="true"]');
      if (modal && event.key === "Tab") {
        const modalControls = controls(root);
        if (modalControls.length > 0) {
          event.preventDefault();
          const at = root.ownerDocument.activeElement as HTMLElement | null;
          const idx = at ? modalControls.indexOf(at) : -1;
          if (event.shiftKey) {
            const nextIdx = idx <= 0 ? modalControls.length - 1 : idx - 1;
            focusControl(modalControls[nextIdx]);
          } else {
            const nextIdx = idx === -1 || idx >= modalControls.length - 1 ? 0 : idx + 1;
            focusControl(modalControls[nextIdx]);
          }
          return;
        }
      }
    }
    if (key(event.key)) event.preventDefault();
  };
  return { ref, onKeyDown };
}
