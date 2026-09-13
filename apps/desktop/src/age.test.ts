import { describe, expect, it } from "vitest";
import { formatAge, parseStamp } from "./age";

describe("formatAge", () => {
  const now = new Date(2026, 8, 13, 12, 0).getTime();

  it("reads the core stamp as local time", () => {
    expect(parseStamp("2026-09-13T11:30")?.getTime()).toBe(new Date(2026, 8, 13, 11, 30).getTime());
    expect(parseStamp("yesterday")).toBeNull();
  });

  it("shortens to the largest unit that fits", () => {
    expect(formatAge("2026-09-13T12:00", now)).toBe("now");
    expect(formatAge("2026-09-13T11:35", now)).toBe("25m");
    expect(formatAge("2026-09-13T09:00", now)).toBe("3h");
    expect(formatAge("2026-09-10T12:00", now)).toBe("3d");
    expect(formatAge("2026-08-20T12:00", now)).toBe("3w");
    expect(formatAge("2026-03-01T12:00", now)).toBe("6mo");
  });

  it("falls back to the raw text when the stamp is odd", () => {
    expect(formatAge("soon", now)).toBe("soon");
  });
});
