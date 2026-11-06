// Plain text for times and playtime (UTC, so a screenshot reads the same everywhere).

export function when(time: string): string {
  return `${time.slice(0, 10)} ${time.slice(11, 16)} UTC`;
}

export function playtime(secs: number | null): string | null {
  if (secs === null) return null;
  const minutes = Math.round(secs / 60);
  const h = Math.floor(minutes / 60);
  return h > 0 ? `${h} h ${minutes % 60} min` : `${minutes} min`;
}
