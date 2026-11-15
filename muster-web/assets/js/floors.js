// The side list of every project's floor, each in miniature.
import { Mark, html } from "./marks.js";

/** The rooms each miniature's three cells hold, and the class of each cell. */
const CELLS = [
  { rooms: ["running"], className: "" },
  { rooms: ["failed", "restarting", "starting"], className: "busy" },
  { rooms: ["off"], className: "off" },
];
/** The rooms a floor's status line counts, first match wins, and the word for each. */
const STATUS = [
  { room: "failed", word: "failed", className: "fail" },
  { room: "running", word: "running", className: "" },
  { room: "starting", word: "starting", className: "" },
  { room: "restarting", word: "restarting", className: "" },
];
/** What a floor with nothing on says. */
const ALL_OFF = "All off";

/** The status line of `floor`: the count of its most telling room. */
function Status({ floor }) {
  for (const { room, word, className } of STATUS) {
    const count = floor.processes.filter((process) => process.room === room).length;
    if (count) return html`<small class=${className}>${count} ${word}</small>`;
  }
  return html`<small>${ALL_OFF}</small>`;
}

/** One project's floor in miniature, a button that opens it. */
function Other({ floor, current, onPick }) {
  const failed = floor.processes.some((process) => process.room === "failed");
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
