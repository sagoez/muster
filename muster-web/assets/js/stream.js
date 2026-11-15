// The live stream and the store it fills: one EventSource, each message
// applied once and in order, a snapshot replacing everything.
import { batch, signal } from "./vendor/signals/signals.mjs";
import { cssMillis } from "./theme.js";

/** The stream's path, relative so it stays below the token's. */
const EVENTS_PATH = "events";
/**
 * Every kind of message the floor sends, each its own SSE event. Exported
 * for the demo recorder, which reads the same stream.
 */
export const KINDS = ["snapshot", "projects", "process", "pin", "float", "speaking", "notes"];
/** The kinds that can move a face, which the walk measures before. */
const MOVING_KINDS = new Set(["snapshot", "projects", "process"]);
/** The stylesheet's time a pin stays before it is gone, lift included. */
const PIN_LIFE = "--pin-life";
/** The stylesheet's time a float rises before it is gone. */
const FLOAT_LIFE = "--float-life";
/** How far a float drifts sideways at most, either way, in pixels. */
const FLOAT_DRIFT_PX = 12;
/** How long the page waits to open a new stream after the browser gave one up. */
const REOPEN_MS = 2000;
/** How often the clock the page counts uptimes and ages by ticks. */
const TICK_MS = 1000;
/** Milliseconds in a second. */
const MS_PER_SECOND = 1000;

/** How the view's sources stand, `null` until the first snapshot. */
export const health = signal(null);
/** The project holding the folder the view was started in. */
export const here = signal(null);
/** Every project's floor, in the store's order. */
export const projects = signal([]);
/**
 * Each project's notes, by project name. A `Map`, so a project named after
 * an `Object.prototype` key reads as itself and never as that key.
 */
export const notes = signal(new Map());
/** The pin up on each face, by `faceKey`: `{ pin, id }`. */
export const pins = signal(new Map());
/** The floats rising off faces: `{ id, key, float, port, drift }`. */
export const floats = signal([]);
/** Whether the stream is open. */
export const live = signal(false);
/** The time now, in epoch seconds, ticking each second. */
export const now = signal(Date.now() / MS_PER_SECOND);

/** What runs before a message that can move a face is applied. */
const beforeMoves = new Set();
/** The sequence number of the last message applied. */
let lastSeq = 0;
/** A counter that names each pin and float. */
let serial = 0;

/** The key of process `name` of `project` in the pin and float stores. */
export function faceKey(project, name) {
  return JSON.stringify([project, name]);
}

/** Runs `measure` before each message that can move a face, with the message. */
export function onBeforeMove(measure) {
  beforeMoves.add(measure);
}

/** The floors with `change` applied to process `name` of `project`, adding it when absent. */
function withProcess(project, name, change) {
  return projects.value.map((floor) => {
    if (floor.name !== project) return floor;
    const at = floor.processes.findIndex((process) => process.name === name);
    const processes = floor.processes.slice();
    if (at < 0) {
      const added = change(null);
      if (added) processes.push(added);
    } else {
      processes[at] = change(processes[at]);
    }
    return { ...floor, processes };
  });
}

/** `kept`'s notes of only the projects `floors` lists, so a removed project's go with it. */
function notesOf(kept, floors) {
  const listed = new Set(floors.map((floor) => floor.name));
  return new Map([...kept].filter(([project]) => listed.has(project)));
}

/** Drops `pin` on a face, replacing any pin up on it, until it lifts. */
function dropPin(key, pin) {
  const id = ++serial;
  const next = new Map(pins.value);
  next.set(key, { pin, id });
  pins.value = next;
  setTimeout(() => {
    if (pins.value.get(key)?.id !== id) return;
    const left = new Map(pins.value);
    left.delete(key);
    pins.value = left;
  }, cssMillis(PIN_LIFE));
}

/** Sends a float rising off a face, gone once it has risen. */
function raiseFloat(key, float, port) {
  const id = ++serial;
  const drift = Math.round((Math.random() * 2 - 1) * FLOAT_DRIFT_PX);
  floats.value = [...floats.value, { id, key, float, port, drift }];
  setTimeout(() => {
    floats.value = floats.value.filter((risen) => risen.id !== id);
  }, cssMillis(FLOAT_LIFE));
}

/** Applies one message, unless its sequence number is not above the last applied. */
function apply(message) {
  if (typeof message?.seq !== "number" || message.seq <= lastSeq) return;
  lastSeq = message.seq;
  if (MOVING_KINDS.has(message.kind)) for (const measure of beforeMoves) measure(message);
  switch (message.kind) {
    case "snapshot":
      batch(() => {
        health.value = message.health;
        here.value = message.here;
        projects.value = message.projects;
        notes.value = new Map(Object.entries(message.notes));
        pins.value = new Map();
        floats.value = [];
      });
      break;
    case "projects":
      batch(() => {
        here.value = message.here;
        projects.value = message.projects;
        notes.value = notesOf(notes.value, message.projects);
      });
      break;
    case "process":
      projects.value = withProcess(message.project, message.process.name, () => message.process);
      break;
    case "speaking":
      projects.value = withProcess(message.project, message.name, (process) =>
        process ? { ...process, speaking: message.on } : null,
      );
      break;
    case "pin":
      dropPin(faceKey(message.project, message.name), message.pin);
      break;
    case "float":
      raiseFloat(faceKey(message.project, message.name), message.float, message.port);
      break;
    case "notes":
      notes.value = new Map(notes.value).set(message.project, message.notes);
      break;
    default:
      break;
  }
}

/** Opens the stream, and opens it again whenever the browser gives it up. */
function connect() {
  const source = new EventSource(EVENTS_PATH);
  source.onopen = () => {
    live.value = true;
  };
  source.onerror = () => {
    live.value = false;
    if (source.readyState === EventSource.CLOSED) setTimeout(connect, REOPEN_MS);
  };
  for (const kind of KINDS) {
    source.addEventListener(kind, (event) => {
      let message;
      try {
        message = JSON.parse(event.data);
      } catch {
        return;
      }
      apply(message);
    });
  }
}

/** Starts the stream and the clock. */
export function start() {
  connect();
  setInterval(() => {
    now.value = Date.now() / MS_PER_SECOND;
  }, TICK_MS);
}
