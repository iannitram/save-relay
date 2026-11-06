import { describe, expect, it } from "vitest";
import { playtime, setWhenClock, when, type WhenClock } from "./format.ts";

describe("when", () => {
  const clock: WhenClock = { now: new Date("2026-10-31T19:12:00Z"), timeZone: "UTC" };

  it("formats today as h:mmpm with no whitespace", () => {
    const formatted = when("2026-10-31T19:12:00Z", clock);
    expect(formatted).toBe("7:12pm");
    expect(formatted).not.toMatch(/\s/);
  });

  it("formats another day as Mon D, h:mmpm", () => {
    expect(when("2026-10-30T19:12:00Z", clock)).toBe("Oct 30, 7:12pm");
  });

  it("formats a date from another year as Mon D, h:mmpm", () => {
    expect(when("2025-12-31T19:12:00Z", clock)).toBe("Dec 31, 7:12pm");
  });

  it("formats noon as 12:00pm", () => {
    expect(when("2026-10-31T12:00:00Z", clock)).toBe("12:00pm");
  });

  it("formats midnight as 12:00am", () => {
    expect(when("2026-10-31T00:00:00Z", clock)).toBe("12:00am");
  });

  it("handles time zone change where Nov 1 00:30Z is still today on Oct 31 in Los Angeles", () => {
    expect(when("2026-11-01T00:30:00Z", clock)).toBe("Nov 1, 12:30am");
    expect(when("2026-11-01T00:30:00Z", { ...clock, timeZone: "America/Los_Angeles" })).toBe("5:30pm");
  });

  it("returns unparseable times unchanged", () => {
    expect(when("garbage", clock)).toBe("garbage");
    expect(when("not-a-date")).toBe("not-a-date");
  });

  it("uses default clock set by setWhenClock and allows resetting", () => {
    setWhenClock(clock);
    expect(when("2026-10-31T19:12:00Z")).toBe("7:12pm");
    setWhenClock(null);
  });
});

describe("playtime", () => {
  it("returns null for null seconds", () => {
    expect(playtime(null)).toBeNull();
  });

  it("formats minutes only", () => {
    expect(playtime(0)).toBe("0 min");
    expect(playtime(120)).toBe("2 min");
  });

  it("formats hours and minutes", () => {
    expect(playtime(3600)).toBe("1 h 0 min");
    expect(playtime(4320)).toBe("1 h 12 min");
  });
});
