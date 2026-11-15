// A process's card: its facts, the tail of its log, and the command to run
// next. It sits beside the face it belongs to.
import { useEffect, useLayoutEffect, useRef, useState } from "./vendor/preact/hooks.mjs";
import { Face } from "./floor.js";
import { ago, counted, lasting, memory } from "./format.js";
import { Icon, MARK_WORDS, ROOM, html, line } from "./marks.js";
import { now } from "./stream.js";
import { cssPixels } from "./theme.js";
import { faceNamed, onDrawn } from "./walk.js";

/** The log route's path, which takes the project and the process in its query. */
const LOG_PATH = "api/log";
/** How often an open card reads its log tail again. */
const LOG_EVERY_MS = 1000;
/** The stylesheet's width of the card. */
const CARD_WIDTH = "--card-width";
/** How far the card stands off its face. */
const CARD_GAP = 14;
/** How far the card keeps from the window's edges. */
const EDGE = 12;
/** How far the card rises above its face's top. */
const CARD_RISE = 10;
/** The card's highest top, clear of the title bar. */
const TOP_CLEARANCE = 60;
/** The window the card is placed in. */
const WINDOW_SELECTOR = ".window";
/** The disc of a face, which the card stands beside. */
const DISC_SELECTOR = ".face";
/** What the log shows while a process has printed nothing. */
const EMPTY_LOG = "No output yet";
/** What the card says of a process whose state nothing has confirmed. */
const UNCONFIRMED = { title: "Not confirmed", hint: "The service is not answering; last known state" };
/** The close button's line icon. */
const CLOSE_ICON = line("x");
/** What leads into the suggested command, by what it does. */
const NEXT_LEADS = {
  logs: "See why with",
  notices: "See why with",
  start: "Start it with",
  resume: "Resume it with",
  stop: "Stop it with",
  attach: "Watch it with",
};

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

/**
 * The last lines of `name`'s log in `project`; `null` until read. One read
 * is in flight at a time: the next starts a second after the last settled,
 * so a slow read never overtakes a newer one.
 */
function useLogTail(project, name) {
  const [lines, setLines] = useState(null);
  useEffect(() => {
    setLines(null);
    const control = new AbortController();
    const url = `${LOG_PATH}?project=${encodeURIComponent(project)}&name=${encodeURIComponent(name)}`;
    let timer = 0;
    const read = () =>
      fetch(url, { signal: control.signal, cache: "no-store" })
        .then((response) => response.json())
        .then((answer) => {
          if (!control.signal.aborted && Array.isArray(answer.lines)) setLines(answer.lines);
        })
        .catch(() => {})
        .finally(() => {
          if (!control.signal.aborted) timer = setTimeout(read, LOG_EVERY_MS);
        });
    read();
    return () => {
      clearTimeout(timer);
      control.abort();
    };
  }, [project, name]);
  return lines;
}

/** Whether focus has nowhere to be: on the page itself, or inside `card` as it goes. */
function focusLost(card) {
  const active = document.activeElement;
  return !active || active === document.body || Boolean(card?.contains(active));
}

/** Places `card` beside the face of `name`, inside the window. */
function place(card, name) {
  const win = card.closest(WINDOW_SELECTOR);
  const face = faceNamed(name)?.querySelector(DISC_SELECTOR);
  if (!win || !face) return;
  const width = cssPixels(CARD_WIDTH);
  const bounds = win.getBoundingClientRect();
  const anchor = face.getBoundingClientRect();
  let left = anchor.right - bounds.left + CARD_GAP;
  if (left + width > bounds.width - EDGE) left = anchor.left - bounds.left - width - CARD_GAP;
  let top = anchor.top - bounds.top - CARD_RISE;
  top = Math.min(top, bounds.height - card.offsetHeight - EDGE);
  card.style.left = `${Math.max(EDGE, left)}px`;
  card.style.top = `${Math.max(TOP_CLEARANCE, top)}px`;
}

/**
 * The card of `process` in `project`; closed by its button. Focus moves
 * into it as it opens and back to its face as it closes, unless the person
 * moved it elsewhere. It places itself as it draws, as the floor draws, and
 * as the window resizes, anything scrolls or it changes size. Its command is
 * the one the floor picked for the process's state, naming the project so
 * it acts on this floor from any folder.
 */
export function Card({ project, process, onClose }) {
  const ref = useRef(null);
  const closeRef = useRef(null);
  const lines = useLogTail(project, process.name);
  const time = now.value;
  const room = ROOM[process.room] ?? ROOM.off;
  const next = process.next;
  const shown = facts(process, time);

  useEffect(() => {
    const card = ref.current;
    closeRef.current?.focus({ preventScroll: true });
    return () => {
      if (focusLost(card)) faceNamed(process.name)?.focus({ preventScroll: true });
    };
  }, [process.name]);

  useLayoutEffect(() => {
    if (ref.current) place(ref.current, process.name);
  });

  useLayoutEffect(() => {
    const card = ref.current;
    if (!card) return undefined;
    const follow = () => place(card, process.name);
    const resized = new ResizeObserver(follow);
    resized.observe(card);
    const stopDrawn = onDrawn(follow);
    addEventListener("resize", follow);
    document.addEventListener("scroll", follow, { capture: true, passive: true });
    return () => {
      resized.disconnect();
      stopDrawn();
      removeEventListener("resize", follow);
      document.removeEventListener("scroll", follow, { capture: true });
    };
  }, [process.name]);

  return html`<aside class="desk open" ref=${ref} aria-label=${process.name} onClick=${(event) => event.stopPropagation()}>
    <header>
      <${Face} process=${process} />
      <div aria-live="polite">
        <h3>${process.name}</h3>
        <p class="now" style=${{ color: room.color }}>${MARK_WORDS[process.mark] ?? room.word}</p>
        ${process.unconfirmed &&
        html`<p class="unconfirmed" title=${UNCONFIRMED.hint}>${UNCONFIRMED.title}: ${UNCONFIRMED.hint}</p>`}
      </div>
      <button type="button" class="close" aria-label="Close" ref=${closeRef} onClick=${onClose}>
        <${Icon} name=${CLOSE_ICON} />
      </button>
    </header>
    ${shown.length > 0 &&
    html`<dl>
      ${shown.map(([label, text]) => html`<dt key=${`${label}-t`}>${label}</dt><dd key=${`${label}-d`}>${text}</dd>`)}
    </dl>`}
    ${lines && html`<pre class=${lines.length ? "log" : "log empty"}>${lines.length ? lines.join("\n") : EMPTY_LOG}</pre>`}
    ${next && html`<p class="next" data-verb=${next.verb}>${NEXT_LEADS[next.verb]} <code>${next.command}</code></p>`}
  </aside>`;
}
