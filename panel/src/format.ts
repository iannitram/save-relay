// Plain text for times and playtime.

export interface WhenClock {
  now: Date;
  timeZone: string;
}

let defaultClock: WhenClock | null = null;

export function setWhenClock(clock: WhenClock | null): void {
  defaultClock = clock;
}

export function when(time: string, clock?: WhenClock): string {
  const instant = new Date(time);
  if (Number.isNaN(instant.getTime())) {
    return time;
  }
  const activeClock = clock ?? defaultClock ?? {
    now: new Date(),
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
  };
  const dateParts = (date: Date) =>
    new Intl.DateTimeFormat("en-US", {
      timeZone: activeClock.timeZone,
      year: "numeric",
      month: "short",
      day: "numeric",
    }).formatToParts(date);
  const parts = (date: Date) =>
    Object.fromEntries(dateParts(date).map(({ type, value }) => [type, value]));
  const shown = parts(instant);
  const today = parts(activeClock.now);

  const timeParts = Object.fromEntries(
    new Intl.DateTimeFormat("en-US", {
      timeZone: activeClock.timeZone,
      hour: "numeric",
      minute: "2-digit",
      hour12: true,
    })
      .formatToParts(instant)
      .map(({ type, value }) => [type, value]),
  );
  const period = (timeParts.dayPeriod ?? "").toLowerCase();
  const timeText = `${timeParts.hour}:${timeParts.minute}${period}`;

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
