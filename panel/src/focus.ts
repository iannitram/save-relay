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
  return Array.from(root.querySelectorAll<HTMLElement>(CONTROLS));
}

/**
 * Put on a view's root: focus moves into the view when it appears (to `initial`, else the
 * first control), so focus is never left on something that is gone, and arrows/Escape work.
 */
export function useFocusScope(onBack: (() => void) | null, initial?: string) {
  const ref = useRef<HTMLElement>(null);
  useEffect(() => {
    const root = ref.current;
    if (!root) return;
    const all = controls(root);
    (all.find((el) => el.dataset.focus === initial) ?? all[0])?.focus();
  }, [initial]);
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    const root = ref.current;
    if (!root) return;
    const all = controls(root);
    const current = (document.activeElement as HTMLElement | null)?.dataset.focus ?? null;
    const move = nextFocus(
      all.map((el) => el.dataset.focus ?? ""),
      current,
      event.key,
    );
    if (move === null) return;
    event.preventDefault();
    if (move === "back") onBack?.();
    else all.find((el) => el.dataset.focus === move.focus)?.focus();
  };
  return { ref, onKeyDown };
}
