// The pins that drop on a face when the service logs an event of it, and the
// floats that rise off it.
import { Icon, html, line } from "./marks.js";

/** The line icon each pin shows. */
const PIN_ICONS = { ok: line("check"), again: line("refresh-cw"), fail: line("triangle-alert"), late: line("triangle-alert") };
/** What each pin says to a screen reader. */
const PIN_WORDS = { ok: "up", again: "restarting", fail: "failed", late: "not ready in time" };
/** The line icon a restart float shows. */
const RESTART_ICON = line("refresh-cw");
/** What marks a port number as one. */
const PORT_MARK = ":";

/** The pin `pin` dropped on a face. */
export function Pin({ pin }) {
  return html`<span class=${`pin ${pin}`} data-pin=${pin} role="img" aria-label=${PIN_WORDS[pin] ?? pin}>
    <${Icon} name=${PIN_ICONS[pin] ?? PIN_ICONS.ok} />
  </span>`;
}

/** A float rising off a face: the port that opened, or a restart. */
export function Float({ float }) {
  const port = float.float === "port";
  return html`<span
    class=${`float ${float.float}`}
    data-float=${float.float}
    style=${{ "--drift": `${float.drift}px` }}
    aria-hidden="true"
  >
    ${port ? `${PORT_MARK}${float.port ?? ""}` : html`<${Icon} name=${RESTART_ICON} />`}
  </span>`;
}
