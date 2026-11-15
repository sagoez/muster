// The marks and line icons the page draws, and the words and formats it
// shows facts in. The colour tables mirror ProcessIcon and AgentMark; a Rust
// test checks every entry against them.
import { h } from "./vendor/preact/preact.mjs";
import htm from "./vendor/htm/htm.mjs";

export const html = htm.bind(h);

/** This version's asset root, absolute so a mask URL resolves the same from any stylesheet. */
const ROOT = new URL("../", import.meta.url).href;
/** Where the brand marks live under the asset root. */
const BRANDS = "icons/brands/";
/** Where the line icons live under the asset root. */
const LINES = "icons/lucide/";
/** What ends every icon's file name. */
const SVG = ".svg";

/** The colour each process icon is drawn in on the face disc, by its slug. */
export const ICON_COLORS = {
  rust: "#dea584",
  docker: "#2496ed",
  nodedotjs: "#5fa04e",
  python: "#5a9fd4",
  go: "#00add8",
  vite: "#9499ff",
  nextdotjs: "#f1f1f3",
  postgresql: "#6d8fe6",
  redis: "#ff4438",
  mysql: "#6fa3d4",
  mongodb: "#47a248",
  nginx: "#2fbf71",
  deno: "#f1f1f3",
  bun: "#fbf0df",
  ruby: "#e0473c",
  rubyonrails: "#e8473b",
  django: "#44b78b",
  php: "#8f93d6",
  elixir: "#a97fd8",
  dotnet: "#8f7fe8",
  vuedotjs: "#4fc08d",
  svelte: "#ff3e00",
  astro: "#d083ff",
  nuxt: "#00dc82",
  rabbitmq: "#ff6600",
  terminal: "#a1a2a8",
};
/** The icon a process with an unknown slug wears. */
const FALLBACK_ICON = "terminal";

/** The mark, colour and title of each note author, by its kind. */
export const AUTHORS = {
  claude: { asset: "icons/brands/claude.svg", color: "#d97757", title: "Claude" },
  codex: { asset: "icons/lucide/bot.svg", color: "#10a37f", title: "Codex" },
  agent: { asset: "icons/lucide/bot.svg", color: "#a1a2a8", title: "Agent" },
  human: { asset: "icons/lucide/user.svg", color: "#f1f1f3", title: "You" },
};
/** The author an unknown kind is drawn as. */
const FALLBACK_AUTHOR = "agent";

/** Each room, in the order the floor lays them out, with its word, colour and counter icon. */
export const ROOMS = [
  { id: "running", word: "Running", color: "var(--run)", icon: "circle-check" },
  { id: "failed", word: "Failed", color: "var(--fail)", icon: "circle-x" },
  { id: "starting", word: "Starting", color: "var(--start)", icon: "loader-circle" },
  { id: "restarting", word: "Restarting", color: "var(--spin)", icon: "refresh-cw" },
  { id: "off", word: "Off", color: "var(--ink-3)", icon: "circle-pause" },
];
/** Each room by its id. */
export const ROOM = Object.fromEntries(ROOMS.map((room) => [room.id, room]));

/** What a mark says of its process on the card, in place of its room's word. */
export const MARK_WORDS = { paused: "Paused", muted: "Unsupervised", quiet: "Stopping" };
/** The line icon a mark shows on the face. */
export const MARK_ICONS = { paused: "pause", muted: "eye-off" };

/** The absolute URL of the asset at `path` under this version's root. */
export function assetUrl(path) {
  return ROOT + path;
}

/** A brand mark: the process icon `icon`'s logo masked in its colour. */
export function Mark({ icon }) {
  const slug = icon in ICON_COLORS ? icon : FALLBACK_ICON;
  return html`<span
    class="mark"
    style=${{ "--logo": `url("${assetUrl(BRANDS + slug + SVG)}")`, "--brand": ICON_COLORS[slug] }}
  ></span>`;
}

/** An author's mark, drawn in its colour. */
export function AuthorMark({ author }) {
  const known = AUTHORS[author] ?? AUTHORS[FALLBACK_AUTHOR];
  return html`<span
    class="mark"
    style=${{ "--logo": `url("${assetUrl(known.asset)}")`, "--brand": known.color }}
  ></span>`;
}

/** The title of an author kind. */
export function authorTitle(author) {
  return (AUTHORS[author] ?? AUTHORS[FALLBACK_AUTHOR]).title;
}

/** A Lucide line icon, masked in the current colour. */
export function Icon({ name }) {
  return html`<span class="icon" style=${{ "--icon": `url("${assetUrl(LINES + name + SVG)}")` }}></span>`;
}

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

/** `text` with each backticked span drawn as code. */
export function Prose({ text }) {
  return text.split("`").map((part, index) => (index % 2 ? html`<code>${part}</code>` : part));
}
