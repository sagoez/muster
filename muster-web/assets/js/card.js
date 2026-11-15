// A process's card: its facts, the tail of its log, and the command to run
// next. It sits beside the face it belongs to.
import { useEffect, useLayoutEffect, useRef, useState } from "./vendor/preact/hooks.mjs";
import { Face } from "./floor.js";
import { Icon, MARK_WORDS, ROOM, ago, counted, html, line, lasting, memory } from "./marks.js";
import { now } from "./stream.js";
import { onDrawn } from "./walk.js";

/** The log route's path, which takes the project and the process in its query. */
const LOG_PATH = "api/log";
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
const CLOSE_ICON = line("x");

/**
 * The line of `muster` the card suggests for `process` of the project
 * quoted `project`, as a lead-in and the command. It names the project, so
 * it acts on this floor from any folder.
 */
function nextStep(project, process) {
  const command = (verb) => `muster -p ${project} ${verb} ${process.quoted}`;
  if (process.room === "failed") return ["See why with", command("logs")];
  if (process.room === "off") return ["Start it with", command("start")];
  return ["Watch it with", command("attach")];
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
  const win = card.closest(".window");
  const face = faceButton(name)?.querySelector(".face");
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

/** The face of `name` on the floor, if it stands there. */
function faceButton(name) {
  for (const person of document.querySelectorAll(".map .person")) {
    if (person.dataset.name === name) return person;
  }
  return null;
}

/**
 * The card of `process` in `project`, whose name quoted for a shell is
 * `quotedProject`; closed by its button. Focus moves into it as it opens and
 * back to its face as it closes, unless the person moved it elsewhere. It
 * places itself as it draws, as the floor draws, and as the window resizes,
 * anything scrolls or it changes size.
 */
export function Card({ project, quotedProject, process, onClose }) {
  const ref = useRef(null);
  const closeRef = useRef(null);
  const lines = useLogTail(project, process.name);
  const time = now.value;
  const room = ROOM[process.room] ?? ROOM.off;
  const [lead, command] = nextStep(quotedProject, process);
  const shown = facts(process, time);

  useEffect(() => {
    const card = ref.current;
    closeRef.current?.focus({ preventScroll: true });
    return () => {
      if (focusLost(card)) faceButton(process.name)?.focus({ preventScroll: true });
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
    <p class="next">${lead} <code>${command}</code></p>
  </aside>`;
}
