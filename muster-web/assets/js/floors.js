// The side list of every project's floor, each in miniature.
import { Mark, ROOM, html } from "./marks.js";

/** The rooms each miniature's three cells hold, and the class of each cell. */
const CELLS = [
  { rooms: [ROOM.running.id], className: "" },
  { rooms: [ROOM.failed.id, ROOM.restarting.id, ROOM.starting.id], className: "busy" },
  { rooms: [ROOM.off.id], className: "off" },
];
/** The rooms a floor's status line counts, first match wins. */
const STATUS_ROOMS = [ROOM.failed, ROOM.running, ROOM.starting, ROOM.restarting];
/** The class the status line takes while its count is of the failed. */
const FAILED_CLASS = "fail";
/** What a floor with nothing on says. */
const ALL_OFF = "All off";
/** What a floor whose stack did not read says. */
const UNREADABLE = "Unreadable";

/** Whether any of `floor`'s processes stands in `room`. */
function anyIn(floor, room) {
  return floor.processes.some((process) => process.room === room.id);
}

/** The status line of `floor`: the count of its most telling room, or that its stack did not read. */
function Status({ floor }) {
  if (floor.unreadable) return html`<small class=${FAILED_CLASS}>${UNREADABLE}</small>`;
  for (const room of STATUS_ROOMS) {
    const count = floor.processes.filter((process) => process.room === room.id).length;
    if (count) {
      const className = room === ROOM.failed ? FAILED_CLASS : "";
      return html`<small class=${className}>${count} ${room.word.toLowerCase()}</small>`;
    }
  }
  return html`<small>${ALL_OFF}</small>`;
}

/** One project's floor in miniature, a button that opens it. */
function Other({ floor, current, onPick }) {
  const failed = anyIn(floor, ROOM.failed);
  return html`<button
    type="button"
    class=${current ? "other here" : "other"}
    data-project=${floor.name}
    aria-current=${current ? "page" : undefined}
    onClick=${() => onPick(floor.name)}
  >
    <div class="top">
      <b><${Mark} icon=${floor.icon} />${floor.name}</b><${Status} floor=${floor} />
    </div>
    <div class="mini">
      ${CELLS.map(({ rooms, className }) => {
        const seats = floor.processes.filter((process) => rooms.includes(process.room));
        const classes = [className, className === "busy" && failed ? "failed" : ""].filter(Boolean).join(" ");
        return html`<div class=${classes}>
          ${seats.map((process) => html`<span class="seat" key=${process.name}><${Mark} icon=${process.icon} /></span>`)}
        </div>`;
      })}
    </div>
  </button>`;
}

/** Every project's floor, the current one marked. */
export function Floors({ projects, current, onPick }) {
  return html`<nav class="side" aria-label="Projects">
    ${projects.map(
      (floor) =>
        html`<${Other} key=${floor.name} floor=${floor} current=${floor.name === current} onPick=${onPick} />`,
    )}
  </nav>`;
}
