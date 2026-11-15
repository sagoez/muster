// The marks, rooms and line icons the page draws. The colour tables mirror
// ProcessIcon and AgentMark; a Rust test reads this file's text and checks
// every entry against them.
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
const ICON_COLORS = {
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
const AUTHORS = {
  claude: { asset: "icons/brands/claude.svg", color: "#d97757", title: "Claude" },
  codex: { asset: "icons/lucide/bot.svg", color: "#10a37f", title: "Codex" },
  agent: { asset: "icons/lucide/bot.svg", color: "#a1a2a8", title: "Agent" },
  human: { asset: "icons/lucide/user.svg", color: "#f1f1f3", title: "You" },
};
/** The author an unknown kind is drawn as. */
const FALLBACK_AUTHOR = "agent";

/** Each room, in the order the floor lays them out, with its word, colour and counter icon. */
export const ROOMS = [
  { id: "running", word: "Running", color: "var(--run)", icon: line("circle-check") },
  { id: "failed", word: "Failed", color: "var(--fail)", icon: line("circle-x") },
  { id: "starting", word: "Starting", color: "var(--start)", icon: line("loader-circle") },
  { id: "restarting", word: "Restarting", color: "var(--spin)", icon: line("refresh-cw") },
  { id: "off", word: "Off", color: "var(--ink-3)", icon: line("circle-pause") },
];
/** Each room by its id. */
export const ROOM = Object.fromEntries(ROOMS.map((room) => [room.id, room]));

/** What a mark says of its process on the card, in place of its room's word. */
export const MARK_WORDS = { paused: "Paused", muted: "Unsupervised", quiet: "Stopping" };
/** The line icon a mark shows on the face. */
export const MARK_ICONS = { paused: line("pause"), muted: line("eye-off") };

/**
 * Names the Lucide line icon `name`, unchanged. Every line icon the page
 * draws is named through it, so the asset tests can find each one and check
 * it is bundled.
 */
export function line(name) {
  return name;
}

/** The absolute URL of the asset at `path` under this version's root. */
function assetUrl(path) {
  return ROOT + path;
}

/** A brand mark: the process icon `icon`'s logo masked in its colour. */
export function Mark({ icon }) {
  const slug = Object.hasOwn(ICON_COLORS, icon) ? icon : FALLBACK_ICON;
  return html`<span
    class="mark"
    style=${{ "--logo": `url("${assetUrl(BRANDS + slug + SVG)}")`, "--brand": ICON_COLORS[slug] }}
  ></span>`;
}

/** The author entry of kind `author`, an unknown kind drawn as the fallback. */
function authorOf(author) {
  return AUTHORS[Object.hasOwn(AUTHORS, author) ? author : FALLBACK_AUTHOR];
}

/** An author's mark, drawn in its colour. */
export function AuthorMark({ author }) {
  const known = authorOf(author);
  return html`<span
    class="mark"
    style=${{ "--logo": `url("${assetUrl(known.asset)}")`, "--brand": known.color }}
  ></span>`;
}

/** The title of an author kind. */
export function authorTitle(author) {
  return authorOf(author).title;
}

/** A Lucide line icon, masked in the current colour. */
export function Icon({ name }) {
  return html`<span class="icon" style=${{ "--icon": `url("${assetUrl(LINES + name + SVG)}")` }}></span>`;
}

/** `text` with each backticked span drawn as code. */
export function Prose({ text }) {
  return text.split("`").map((part, index) => (index % 2 ? html`<code>${part}</code>` : part));
}
