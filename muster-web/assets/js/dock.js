// The dock: the notes chip, and how many of the floor stand in each room.
import { Icon, ROOMS, counted, html } from "./marks.js";
import { newestFirst } from "./notes.js";

/** The order the counters read in, the rooms that need a look first. */
const TALLY_ORDER = ["failed", "restarting", "starting", "running", "off"];
/** The chip's line icon. */
const NOTE_ICON = "sticky-note";
/** What the chip says with no notes on the floor. */
const NO_NOTES = "Nothing left yet";

/** The chip that opens the notes, showing the newest one. */
function NotesChip({ notes, open, onToggle }) {
  const newest = newestFirst(notes)[0];
  return html`<button
    type="button"
    class="chip"
    id="notes-chip"
    aria-expanded=${open ? "true" : "false"}
    aria-controls="notes"
    onClick=${(event) => {
      event.stopPropagation();
      onToggle();
    }}
  >
    <span class="note-icon"><${Icon} name=${NOTE_ICON} /></span>
    <p>Notes<span>${newest ? newest.body : NO_NOTES}</span></p>
    ${notes.length > 0 && html`<span class="more">${counted(notes.length, "note")}</span>`}
  </button>`;
}

/** One counter per room anyone stands in. */
function Tally({ processes }) {
  return html`<div class="tally">
    ${TALLY_ORDER.map((id) => {
      const room = ROOMS.find((known) => known.id === id);
      const count = processes.filter((process) => process.room === id).length;
      return (
        count > 0 &&
        html`<span
          class="count"
          key=${id}
          data-room=${id}
          title=${`${count} ${room.word.toLowerCase()}`}
          style=${{ "--state": room.color }}
        >
          <${Icon} name=${room.icon} /><b>${count}</b>
        </span>`
      );
    })}
  </div>`;
}

/** The dock under the floor. */
export function Dock({ processes, notes, notesOpen, onToggleNotes }) {
  return html`<div class="dock">
    <${NotesChip} notes=${notes} open=${notesOpen} onToggle=${onToggleNotes} />
    <${Tally} processes=${processes} />
  </div>`;
}
