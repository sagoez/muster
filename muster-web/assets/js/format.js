// The words and formats the page shows facts in: how long something lasted,
// how long ago it was, how much memory it holds, and how many there are.

/** Seconds in a minute, an hour and a day. */
const MINUTE = 60;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
/** Bytes in a KiB, and the steps up from it. */
const KIB = 1024;
const MEMORY_UNITS = ["KiB", "MiB", "GiB", "TiB"];
/** Below this many of a unit, memory shows one decimal. */
const DECIMAL_BELOW = 10;

/** How long `seconds` lasts, in its two largest units: `6 h 12 m`, `48 m`, `12 s`. */
export function lasting(seconds) {
  const s = Math.max(0, Math.floor(seconds));
  if (s < MINUTE) return `${s} s`;
  if (s < HOUR) return `${Math.floor(s / MINUTE)} m`;
  if (s < DAY) return `${Math.floor(s / HOUR)} h ${Math.floor((s % HOUR) / MINUTE)} m`;
  return `${Math.floor(s / DAY)} d ${Math.floor((s % DAY) / HOUR)} h`;
}

/** How long ago `at` was at `now`, both in epoch seconds: `just now`, `4 min ago`, `2 days ago`. */
export function ago(at, now) {
  const s = Math.max(0, Math.floor(now - at));
  if (s < MINUTE) return "just now";
  if (s < HOUR) return `${Math.floor(s / MINUTE)} min ago`;
  if (s < DAY) return `${Math.floor(s / HOUR)} h ago`;
  const days = Math.floor(s / DAY);
  return days === 1 ? "1 day ago" : `${days} days ago`;
}

/** `bytes` in the largest binary unit that keeps it at one or more. */
export function memory(bytes) {
  let value = bytes / KIB;
  let unit = 0;
  while (value >= KIB && unit < MEMORY_UNITS.length - 1) {
    value /= KIB;
    unit += 1;
  }
  const shown = value < DECIMAL_BELOW ? value.toFixed(1) : Math.round(value).toString();
  return `${shown} ${MEMORY_UNITS[unit]}`;
}

/** `count` of `noun`, with an `s` unless it is one. */
export function counted(count, noun) {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}
