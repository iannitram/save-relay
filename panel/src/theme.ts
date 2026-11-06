export function themeFor(prefersLight: boolean): "light" | "dark" {
  return prefersLight ? "light" : "dark";
}

export function applyTheme(
  root: { dataset: Record<string, string | undefined> },
  prefersLight: boolean,
): void {
  root.dataset.theme = themeFor(prefersLight);
}
