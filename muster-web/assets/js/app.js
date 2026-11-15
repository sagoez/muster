// The page's entry module: it puts the floor, the side list, the dock, the
// card and the notes together over the live store.
import { render } from "./vendor/preact/preact.mjs";
import { useEffect } from "./vendor/preact/hooks.mjs";
import { computed, signal } from "./vendor/signals/signals.mjs";
import { Card } from "./card.js";
import { Dock } from "./dock.js";
import { Floor } from "./floor.js";
import { Floors } from "./floors.js";
import { Icon, Prose, html } from "./marks.js";
import { Notes } from "./notes.js";
import { health, here, live, notes, onBeforeMove, projects, start } from "./stream.js";
import { measure, settle } from "./walk.js";

/** What a hash naming a floor starts with. */
const HASH_PREFIX = "#/";
/** The health in which no banner shows. */
const HEALTHY = "running";
/** The healths whose banner says why the floor cannot be shown at all. */
const NO_FLOOR = new Set(["no-state", "newer-schema", "unreadable"]);
/** Each health's banner: its title and the hint under it. */
const BANNERS = {
  absent: {
    title: "The muster service is not running",
    hint: "Run `muster up` in a project to start it. This page keeps watching.",
  },
  starting: { title: "The muster service is starting" },
  "other-version": {
    title: "Another version of muster's service is running",
    hint: "Run any muster command to replace it.",
  },
  "no-state": { title: "muster has no state yet", hint: "Add a process with `muster add` to start." },
  "newer-schema": { title: "A newer muster wrote this database", hint: "Update muster to view it." },
  unreadable: { title: "muster's database could not be read" },
};
/** What the floor shows with no project registered. */
const NO_PROJECTS = { title: "No projects yet", hint: "Add a process with `muster add`." };
/** The key that closes the card and the notes. */
const ESCAPE = "Escape";
/** What the title chip says while the stream is open, and while it reconnects. */
const LIVE = "Live";
const RECONNECTING = "Reconnecting";
/** The title chip's line icon. */
const FOLDER_ICON = "folder-open";

/** The URL's hash, kept as it changes. */
const hash = signal(location.hash);
/** The open card's process, `{ project, name }`, or none. */
const selected = signal(null);
/** Whether the notes panel is open. */
const notesOpen = signal(false);

/** The project the hash names, if it names one. */
function hashedProject() {
  if (!hash.value.startsWith(HASH_PREFIX)) return null;
  try {
    return decodeURIComponent(hash.value.slice(HASH_PREFIX.length));
  } catch {
    return null;
  }
}

/** The floor on show: the hash's, else the one the view started in, else the first with anything not off, else the first. */
const current = computed(() => {
  const floors = projects.value;
  const named = (name) => floors.find((floor) => floor.name === name);
  return (
    named(hashedProject()) ??
    named(here.value) ??
    floors.find((floor) => floor.processes.some((process) => process.room !== "off")) ??
    floors[0] ??
    null
  );
});

/** Shows the floor of `name`, through the hash so it survives a reload. */
function pick(name) {
  selected.value = null;
  location.hash = HASH_PREFIX + encodeURIComponent(name);
}

/** A banner: a title and an optional hint, marked with the health it is for. */
function Banner({ banner, health: state }) {
  return html`<div class="banner" data-health=${state} role="status">
    <b>${banner.title}</b>
    ${banner.hint && html`<span><${Prose} text=${banner.hint} /></span>`}
  </div>`;
}

/** The title bar: the wordmark, any banner, and the chip of the floor on show. */
function Titlebar({ floor, banner }) {
  return html`<div class="titlebar">
    <div class="wordmark">
      <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true">
        <circle cx="10" cy="10" r="8" />
        <circle cx="10" cy="10" r="4.5" />
        <circle cx="10" cy="10" r="1.2" fill="currentColor" />
      </svg>
      muster
    </div>
    ${banner}
    ${floor &&
    html`<div class="floor-name">
      <span class="folder"><${Icon} name=${FOLDER_ICON} /></span>
      <b>${floor.name}</b><span class="path">${floor.path}</span>
      <span class="sep"></span>
      <span class=${live.value ? "live" : "live lost"}><i></i>${live.value ? LIVE : RECONNECTING}</span>
    </div>`}
  </div>`;
}

/** The whole page. */
function App() {
  const floor = current.value;
  const state = health.value;
  const floorNotes = floor ? (notes.value[floor.name] ?? []) : [];
  const chosen = selected.value;
  const open =
    floor && chosen && chosen.project === floor.name
      ? floor.processes.find((process) => process.name === chosen.name)
      : null;

  useEffect(() => {
    if (chosen && !open) selected.value = null;
  });

  useEffect(() => {
    settle();
  }, [floor?.name]);

  const known = state && state !== HEALTHY ? BANNERS[state] : null;
  const banner = known && html`<${Banner} banner=${known} health=${state} />`;
  const noFloor = NO_FLOOR.has(state);
  const showFloor = floor && !noFloor;
  const empty = state && !noFloor && projects.value.length === 0;

  return html`<div class="window">
    <${Titlebar} floor=${showFloor ? floor : null} banner=${noFloor ? null : banner} />
    <div class="body">
      <div class="floor">
        ${showFloor &&
        html`<${Floor}
          floor=${floor}
          onOpen=${(name) => {
            notesOpen.value = false;
            selected.value = { project: floor.name, name };
          }}
        />`}
        ${empty &&
        html`<div class="empty" role="status">
          <b>${NO_PROJECTS.title}</b><span><${Prose} text=${NO_PROJECTS.hint} /></span>
        </div>`}
        ${noFloor && banner}
      </div>
      <${Floors} projects=${projects.value} current=${floor?.name} onPick=${pick} />
    </div>
    <${Dock}
      processes=${showFloor ? floor.processes : []}
      notes=${floorNotes}
      notesOpen=${notesOpen.value}
      onToggleNotes=${() => {
        selected.value = null;
        notesOpen.value = !notesOpen.value;
      }}
    />
    ${notesOpen.value && html`<${Notes} notes=${floorNotes} />`}
    ${open &&
    html`<${Card}
      key=${open.name}
      project=${floor.name}
      quotedProject=${floor.quoted}
      process=${open}
      onClose=${() => {
        selected.value = null;
      }}
    />`}
  </div>`;
}

/** Closes the card and the notes. */
function closeAll() {
  selected.value = null;
  notesOpen.value = false;
}

addEventListener("hashchange", () => {
  hash.value = location.hash;
});
document.addEventListener("click", closeAll);
document.addEventListener("keydown", (event) => {
  if (event.key === ESCAPE) closeAll();
});
onBeforeMove(() => {
  if (current.value) measure(current.value.name);
});
start();
render(html`<${App} />`, document.getElementById("app"));
