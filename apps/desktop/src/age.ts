// "3m", "2h", "5d": how long ago a capture was, from its `at` stamp.

/** nebula-core's stamp before 0.2.0: `YYYY-MM-DDTHH:MM`, with no offset. */
const LEGACY = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/;
/** RFC 3339 with its offset, which is what core stamps and lists now. */
const RFC3339 = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$/i;

/**
 * The instant a stamp names: RFC 3339 as written, and a legacy stamp as local
 * time, which is how core reads it. `null` when it is neither.
 */
export function parseStamp(at: string): Date | null {
  const legacy = LEGACY.exec(at);
  if (legacy) {
    const [, y, mo, d, h, mi] = legacy;
    return new Date(Number(y), Number(mo) - 1, Number(d), Number(h), Number(mi));
  }
  if (!RFC3339.test(at)) return null;
  const then = new Date(at.toUpperCase());
  return Number.isNaN(then.getTime()) ? null : then;
}

/** A short relative age, or the raw stamp when it does not parse. */
export function formatAge(at: string, now: number = Date.now()): string {
  const then = parseStamp(at);
  if (!then) return at;
  const minutes = Math.max(0, Math.floor((now - then.getTime()) / 60_000));
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (days < 14) return `${days}d`;
  const weeks = Math.floor(days / 7);
  if (weeks < 9) return `${weeks}w`;
  return `${Math.floor(days / 30)}mo`;
}

/**
 * Match core's inbox threshold: fourteen calendar dates since capture. Core
 * counts from the date the stamp was taken on, as its own offset has it, which
 * is the stamp's first ten characters in either form.
 */
export function isStale(at: string, now: number = Date.now()): boolean {
  if (!parseStamp(at)) return false;
  const [y, mo, d] = at.slice(0, 10).split("-").map(Number);
  const today = new Date(now);
  const date = Date.UTC(today.getFullYear(), today.getMonth(), today.getDate());
  return (date - Date.UTC(y, mo - 1, d)) / 86_400_000 >= 14;
}
