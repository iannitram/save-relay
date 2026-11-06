// Plain text for times and playtime.

export interface WhenClock {
  now: Date;
  timeZone: string;
}

let defaultClock: WhenClock | null = null;

export function setWhenClock(clock: WhenClock): void {
  defaultClock = clock;
}

export function when(time: string, clock?: WhenClock): string {
  const activeClock = clock ?? defaultClock ?? {
    now: new Date(),
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
  };
  const instant = new Date(time);
  const dateParts = (date: Date) =>
    new Intl.DateTimeFormat("en-US", {
      timeZone: activeClock.timeZone,
      year: "numeric",
      month: "short",
      day: "numeric",
    }).formatToParts(date);
  const parts = (date: Date) => Object.fromEntries(dateParts(date).map(({ type, value }) => [type, value]));
  const shown = parts(instant);
  const today = parts(activeClock.now);
  const timeText = new Intl.DateTimeFormat("en-US", {
    timeZone: activeClock.timeZone,
    hour: "numeric",
    minute: "2-digit",
    hour12: true,
  }).format(instant).toLowerCase().replace(" ", "");
  return shown.year === today.year && shown.month === today.month && shown.day === today.day
    ? timeText
    : `${shown.month} ${shown.day}, ${timeText}`;
}

export function playtime(secs: number | null): string | null {
  if (secs === null) return null;
  const minutes = Math.round(secs / 60);
  const h = Math.floor(minutes / 60);
  return h > 0 ? `${h} h ${minutes % 60} min` : `${minutes} min`;
}
