// A process's card: its facts, the tail of its log, and the command to run
// next. It sits beside the face it belongs to.
import { useEffect, useLayoutEffect, useRef, useState } from "./vendor/preact/hooks.mjs";
import { Face } from "./floor.js";
import { Icon, MARK_WORDS, ROOM, ago, counted, html, lasting, memory } from "./marks.js";
import { now } from "./stream.js";

/** The log route's path, before `<project>/<name>`. */
const LOG_PATH = "api/log/";
/** How often an open card reads its log tail again. */
const LOG_EVERY_MS = 1000;
/** The card's width, as the stylesheet sets it. */
const CARD_WIDTH = 330;
/** How far the card stands off its face. */
const CARD_GAP = 14;
/** How far the card keeps from the window's edges. */
const EDGE = 12;
/** How far the card rises above its face's top. */
const CARD_RISE = 10;
/** The card's highest top, clear of the title bar. */
const TOP_CLEARANCE = 60;
/** What the log shows while a process has printed nothing. */
const EMPTY_LOG = "No output yet";
/** The close button's line icon. */
const CLOSE_ICON = "x";

/** The line of `muster` the card suggests for `process`, as a lead-in and the command. */
function nextStep(process) {
  if (process.room === "failed") return ["See why with", `muster logs ${process.quoted}`];
  if (process.room === "off") return ["Start it with", `muster start ${process.quoted}`];
  return ["Watch it with", `muster attach ${process.quoted}`];
}

/** The facts the card lists for `process` at `time`, each as a label and its text. */
function facts(process, time) {
  const using = [];
  if (process.cpu != null) using.push(`${process.cpu.toFixed(1)}% CPU`);
  if (process.memory != null) using.push(memory(process.memory));
  const ended = process.ended
    ? process.ended.at != null
      ? `${process.ended.text}, ${ago(process.ended.at, time)}`
      : process.ended.text
    : null;
  return [
    ["Ports", process.ports.length ? process.ports.map((port) => `:${port}`).join(", ") : null],
    ["Up for", process.started != null ? lasting(time - process.started) : null],
    ["Using", using.length ? using.join(", ") : null],
    ["Restarts", process.restarts > 0 ? String(process.restarts) : null],
    ["Ended", ended],
    ["Attached", process.attached > 0 ? counted(process.attached, "client") : null],
  ].filter(([, text]) => text);
}

/** The last lines of `name`'s log in `project`, read now and every second; `null` until read. */
function useLogTail(project, name) {
  const [lines, setLines] = useState(null);
  useEffect(() => {
    setLines(null);
    const control = new AbortController();
    const url = LOG_PATH + encodeURIComponent(project) + "/" + encodeURIComponent(name);
    const read = () =>
      fetch(url, { signal: control.signal, cache: "no-store" })
        .then((response) => response.json())
        .then((answer) => {
          if (Array.isArray(answer.lines)) setLines(answer.lines);
        })
        .catch(() => {});
    read();
    const timer = setInterval(read, LOG_EVERY_MS);
    return () => {
      clearInterval(timer);
      control.abort();
    };
  }, [project, name]);
  return lines;
}

/** Places `card` beside the face of `name`, inside the window. */
function place(card, name) {
  const win = card.closest(".window");
  let face = null;
  for (const person of document.querySelectorAll(".map .person")) {
    if (person.dataset.name === name) face = person.querySelector(".face");
  }
  if (!win || !face) return;
  const bounds = win.getBoundingClientRect();
  const anchor = face.getBoundingClientRect();
  let left = anchor.right - bounds.left + CARD_GAP;
  if (left + CARD_WIDTH > bounds.width - EDGE) left = anchor.left - bounds.left - CARD_WIDTH - CARD_GAP;
  let top = anchor.top - bounds.top - CARD_RISE;
  top = Math.min(top, bounds.height - card.offsetHeight - EDGE);
  card.style.left = `${Math.max(EDGE, left)}px`;
  card.style.top = `${Math.max(TOP_CLEARANCE, top)}px`;
}

/** The card of `process` in `project`, closed by its button. */
export function Card({ project, process, onClose }) {
  const ref = useRef(null);
  const lines = useLogTail(project, process.name);
  const time = now.value;
  const room = ROOM[process.room] ?? ROOM.off;
  const [lead, command] = nextStep(process);
  const shown = facts(process, time);

  useLayoutEffect(() => {
    let frame = 0;
    const follow = () => {
      if (ref.current) place(ref.current, process.name);
      frame = requestAnimationFrame(follow);
    };
    follow();
    return () => cancelAnimationFrame(frame);
  }, [process.name]);

  return html`<aside class="desk open" ref=${ref} aria-live="polite" onClick=${(event) => event.stopPropagation()}>
    <header>
      <${Face} process=${process} />
      <div>
        <h3>${process.name}</h3>
        <p class="now" style=${{ color: room.color }}>${MARK_WORDS[process.mark] ?? room.word}</p>
      </div>
      <button type="button" class="close" aria-label="Close" onClick=${onClose}><${Icon} name=${CLOSE_ICON} /></button>
    </header>
    ${shown.length > 0 &&
    html`<dl>
      ${shown.map(([label, text]) => html`<dt key=${`${label}-t`}>${label}</dt><dd key=${`${label}-d`}>${text}</dd>`)}
    </dl>`}
    ${lines && html`<pre class=${lines.length ? "log" : "log empty"}>${lines.length ? lines.join("\n") : EMPTY_LOG}</pre>`}
    <p class="next">${lead} <code>${command}</code></p>
  </aside>`;
}
