// "3m", "2h", "5d": how long ago a capture was, from its `at` stamp.

/** Parse nebula-core's `YYYY-MM-DDTHH:MM` stamp as local time. */
export function parseStamp(at: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/.exec(at);
  if (!m) return null;
  const [, y, mo, d, h, mi] = m;
  return new Date(Number(y), Number(mo) - 1, Number(d), Number(h), Number(mi));
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
