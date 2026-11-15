// One project's floor: its rooms, the faces standing in them, and the walk
// between rooms.
import { useLayoutEffect } from "./vendor/preact/hooks.mjs";
import { Icon, MARK_ICONS, Mark, Prose, ROOM, ROOMS, html, line } from "./marks.js";
import { Float, Pin } from "./pin.js";
import { faceKey, floats, pins } from "./stream.js";
import { forget, play } from "./walk.js";

/** What a floor whose stack did not read says in place of its rooms. */
const UNREADABLE = {
  title: "This project's processes could not be read",
  hint: "Run `muster projects` to see why. The other floors are not affected.",
};
/** The rooms that light their rim while anyone stands in them. */
const LIT_ROOMS = new Set([ROOM.failed, ROOM.starting, ROOM.restarting]);
/** Above this many attached clients, the badge shows the count. */
const COUNT_ABOVE = 1;
/** The line icon of the attached badge. */
const ATTACHED_ICON = line("user");
/** What a face's label adds while its state is not confirmed. */
const UNCONFIRMED_LABEL = ", not confirmed";

/** A process's face: its mark on a disc, the halo while it speaks, and its badges; dimmed while unconfirmed. */
export function Face({ process }) {
  const markIcon = process.mark ? MARK_ICONS[process.mark] : null;
  const attached = process.attached ?? 0;
  return html`<span
    class=${process.speaking ? "face speaking" : "face"}
    data-mark=${process.mark ?? undefined}
    data-unconfirmed=${process.unconfirmed ? "" : undefined}
  >
    <${Mark} icon=${process.icon} />
    ${attached > 0 &&
    html`<span class="attached" title=${`${attached} attached`}>
      <${Icon} name=${ATTACHED_ICON} />${attached > COUNT_ABOVE && html`<b>${attached}</b>`}
    </span>`}
    ${markIcon && html`<span class="state-mark"><${Icon} name=${markIcon} /></span>`}
  </span>`;
}

/** One face standing in its room, with any pin and floats on it. */
function Person({ project, process, room, onOpen }) {
  const key = faceKey(project, process.name);
  const pin = pins.value.get(key);
  const rising = floats.value.filter((float) => float.key === key);
  return html`<button
    type="button"
    class="person"
    data-name=${process.name}
    data-room=${process.room}
    aria-label=${`${process.name}, ${room.word.toLowerCase()}${process.unconfirmed ? UNCONFIRMED_LABEL : ""}`}
    onClick=${(event) => {
      event.stopPropagation();
      onOpen(process.name);
    }}
  >
    ${pin && html`<${Pin} key=${pin.id} pin=${pin.pin} />`}
    ${rising.map((float) => html`<${Float} key=${float.id} float=${float} />`)}
    <${Face} process=${process} />
    <span class="who">${process.name}</span>
  </button>`;
}

/** One room and everyone standing in it. */
function Room({ project, room, processes, onOpen }) {
  const lit = LIT_ROOMS.has(room) && processes.length > 0;
  return html`<section class=${lit ? "room lit" : "room"} data-room=${room.id}>
    <h2>${room.word} <span class="count">${processes.length || ""}</span></h2>
    <div class="well">
      ${processes.map(
        (process) =>
          html`<${Person} key=${process.name} project=${project} process=${process} room=${room} onOpen=${onOpen} />`,
      )}
    </div>
  </section>`;
}

/**
 * The floor of `floor`, each process in the room of its state. When its
 * stack did not read, a note says so: in place of the rooms, or above the
 * processes it last held, drawn unconfirmed.
 */
export function Floor({ floor, onOpen }) {
  useLayoutEffect(() => play(floor.name));
  useLayoutEffect(() => forget, []);
  const kept = floor.processes.length > 0;
  const note =
    floor.unreadable &&
    html`<div class=${kept ? "empty over-map" : "empty"} role="status" data-unreadable="">
      <b>${UNREADABLE.title}</b><span><${Prose} text=${UNREADABLE.hint} /></span>
    </div>`;
  if (floor.unreadable && !kept) return note;
  return html`${note || null}<div class="map">
    ${ROOMS.map(
      (room) =>
        html`<${Room}
          key=${room.id}
          project=${floor.name}
          room=${room}
          processes=${floor.processes.filter((process) => process.room === room.id)}
          onOpen=${onOpen}
        />`,
    )}
  </div>`;
}
