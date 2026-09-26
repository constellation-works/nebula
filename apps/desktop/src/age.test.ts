import { describe, expect, it } from "vitest";
import { formatAge, isStale, parseStamp } from "./age";

describe("parseStamp", () => {
  it("reads a legacy stamp as local time", () => {
    expect(parseStamp("2026-09-13T11:30")?.getTime()).toBe(new Date(2026, 8, 13, 11, 30).getTime());
    expect(parseStamp("yesterday")).toBeNull();
  });

  it("reads an RFC 3339 stamp at its own offset", () => {
    expect(parseStamp("2026-09-13T11:30:05+02:00")?.getTime()).toBe(Date.UTC(2026, 8, 13, 9, 30, 5));
    expect(parseStamp("2026-09-13T11:30:05-05:30")?.getTime()).toBe(Date.UTC(2026, 8, 13, 17, 0, 5));
    expect(parseStamp("2026-09-13T11:30:05Z")?.getTime()).toBe(Date.UTC(2026, 8, 13, 11, 30, 5));
    expect(parseStamp("2026-09-13T11:30:05")).toBeNull();
    expect(parseStamp("2026-09-13T11:30:05+0200")).toBeNull();
    expect(parseStamp("2026-19-13T11:30:05Z")).toBeNull();
  });
});

describe("formatAge", () => {
  const now = new Date(2026, 8, 13, 12, 0).getTime();

  it("shortens to the largest unit that fits", () => {
    expect(formatAge("2026-09-13T12:00", now)).toBe("now");
    expect(formatAge("2026-09-13T11:35", now)).toBe("25m");
    expect(formatAge("2026-09-13T09:00", now)).toBe("3h");
    expect(formatAge("2026-09-10T12:00", now)).toBe("3d");
    expect(formatAge("2026-08-20T12:00", now)).toBe("3w");
    expect(formatAge("2026-03-01T12:00", now)).toBe("6mo");
  });

  it("counts an RFC 3339 stamp from the instant it names", () => {
    const utcNoon = Date.UTC(2026, 8, 13, 12, 0);
    expect(formatAge("2026-09-13T13:35:00+02:00", utcNoon)).toBe("25m");
    expect(formatAge("2026-09-13T04:00:00-05:00", utcNoon)).toBe("3h");
    expect(formatAge("2026-09-10T12:00:00Z", utcNoon)).toBe("3d");
  });

  it("falls back to the raw text when the stamp is odd", () => {
    expect(formatAge("soon", now)).toBe("soon");
  });
});

describe("isStale", () => {
  const now = new Date(2026, 8, 27, 0, 1).getTime();

  it("marks captures from fourteen calendar dates ago, even before their capture time", () => {
    expect(isStale("2026-09-13T23:59", now)).toBe(true);
    expect(isStale("2026-09-14T00:00", now)).toBe(false);
    expect(isStale("invalid", now)).toBe(false);
  });

  it("counts an RFC 3339 stamp from the date its own offset gives it", () => {
    expect(isStale("2026-09-13T23:59:00-05:00", now)).toBe(true);
    expect(isStale("2026-09-14T00:00:00+14:00", now)).toBe(false);
    expect(isStale("2026-09-13T12:00:00Z", now)).toBe(true);
    expect(isStale("2026-09-13T12:00:00", now)).toBe(false);
  });
});
