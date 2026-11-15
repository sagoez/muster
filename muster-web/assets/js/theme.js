// The times the stylesheet owns, read from its custom properties so
// the page's scripts never keep a second copy of them.

/** Milliseconds in a second, for a time the stylesheet writes in seconds. */
const MS_PER_SECOND = 1000;
/** The units a time custom property may be written in. */
const SECONDS = "s";
const MILLISECONDS = "ms";

/** Each custom property read so far, by name, as the stylesheet wrote it. */
const read = new Map();

/** The custom property `name` as the root element's stylesheet sets it, once it is set. */
function property(name) {
  if (!read.has(name)) {
    const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    if (!value) return "";
    read.set(name, value);
  }
  return read.get(name);
}

/** The time custom property `name`, in milliseconds; `0` before the stylesheet sets it. */
export function cssMillis(name) {
  const value = property(name);
  const number = Number.parseFloat(value) || 0;
  if (value.endsWith(MILLISECONDS)) return number;
  return value.endsWith(SECONDS) ? number * MS_PER_SECOND : number;
}
