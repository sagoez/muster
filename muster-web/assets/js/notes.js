// The shared notes panel: what agents left for each other on this floor.
import { AuthorMark, ago, authorTitle, html } from "./marks.js";
import { now } from "./stream.js";

/** The panel's title and what it says under it. */
const TITLE = "Shared notes";
const SUBTITLE = "what agents left for each other";
/** What the panel says when the floor has no notes. */
const NO_NOTES = "No notes on this floor yet.";

/** `notes`, newest first. */
export function newestFirst(notes) {
  return [...notes].sort((a, b) => b.at - a.at);
}

/** One note: who left it, its name, when, and its body as text. */
function Note({ note, time }) {
  return html`<article class="note">
    <span class="who" title=${note.label}><${AuthorMark} author=${note.author} /></span>
    <div>
      <div class="meta"><b>${authorTitle(note.author)}</b><span>${note.key}</span><span>${ago(note.at, time)}</span></div>
      <p>${note.body}</p>
    </div>
  </article>`;
}

/** The notes panel of a floor holding `notes`. */
export function Notes({ notes }) {
  const time = now.value;
  return html`<section class="notes open" id="notes" aria-label=${TITLE} onClick=${(event) => event.stopPropagation()}>
    <h3>${TITLE} <small>${SUBTITLE}</small></h3>
    ${notes.length
      ? newestFirst(notes).map((note) => html`<${Note} key=${note.key} note=${note} time=${time} />`)
      : html`<p class="none">${NO_NOTES}</p>`}
  </section>`;
}
