// The edit decisions, all read off timeline.json: where the body starts and
// ends, where the camera looks at each moment, which caption is up, and how
// far the terminal is lifted. Every time is in page milliseconds, counted from
// the page footage's first frame. The composition and soundtrack.mjs both read
// them here, so a cue always lands with its picture.
import { Easing, interpolate } from "remotion";
import { FADE_FRAMES, FPS, HEIGHT, INTRO_FRAMES, MS_PER_SECOND, OUTRO_FRAMES, WIDTH, framesOf } from "./theme.js";

/** Frame pixels per page CSS pixel at zoom 1: the page's 1280 wide fills the 1920 frame. */
export const PAGE_SCALE = 1.5;
/** The tightest the camera goes: past it the 1280x800 page would stop looking sharp. */
const ZOOM_MAX = 1.6;
/** The widest it goes: the whole page in the frame's height. */
const ZOOM_MIN = 0.9;
/** The whole page, centred. */
const OVERVIEW = { x: 640, y: 400, z: ZOOM_MIN };
/** The part of the frame a framed box goes in: clear of the terminal at bottom left and the captions below. */
const CLEAR = { left: 720, top: 80, right: 1860, bottom: 860 };
/** The opening shot's zoom at most: close on the stopped faces, but the room still reads as one. */
const OPENING_ZOOM_MAX = 1.3;
/** Room left around a framed box, in CSS pixels. */
const FRAME_PAD = 28;
/** How tight the camera follows a walking face. */
const FOLLOW_ZOOM = 1.35;
/** How far either side of a moment the followed face's place is averaged, so the camera does not jitter. */
const FOLLOW_SPAN_MS = 300;
/** How the camera moves between shots. */
const MOVE = Easing.bezier(0.65, 0, 0.35, 1);
/** How much of the terminal's first second is shown before the first command lands. */
const LEAD_IN_MS = 150;
/** How long a caption takes to come and go, and how long it stays. */
const CAPTION_FADE_MS = 350;
const CAPTION_HOLD_MS = 2900;
/** How long the terminal stays lifted after a command is entered, and how long it eases up and back. */
const LIFT_HOLD_MS = 700;
const LIFT_EASE_MS = 260;
/** Roughly how long the tape takes per typed character, so the lift starts as typing does. */
const TYPING_MS_PER_CHAR = 45;
/** The longest a lift starts before its command is entered. */
const LIFT_LEAD_MAX_MS = 1400;

/** The page time of the body's first frame and its length, in frames. */
export function bodyWindow(timeline) {
  const start = timeline.terminal.startMs + LEAD_IN_MS;
  const end = timeline.terminal.endMs;
  return { start, frames: Math.round(((end - start) / MS_PER_SECOND) * FPS) };
}

/** The page time `frame` frames into the body. */
export function pageTimeAt(timeline, frame) {
  return bodyWindow(timeline).start + (frame / FPS) * MS_PER_SECOND;
}

/** The soundtrack's file in the public folder, which soundtrack.mjs writes and the composition plays. */
export const SOUNDTRACK = "soundtrack.wav";

/** The video frame the body starts on: under the intro's fade. */
export const BODY_FROM = INTRO_FRAMES - FADE_FRAMES;

/** The video frame the outro starts on, over the body's last frames. */
export function outroFrom(timeline) {
  return BODY_FROM + bodyWindow(timeline).frames - FADE_FRAMES;
}

/** The video's length in frames for `timeline`. */
export function totalFrames(timeline) {
  return outroFrom(timeline) + OUTRO_FRAMES;
}

/** The video frame on which page time `at` first shows: the body plays the page footage from its frame at the body's start. */
export function videoFrameOf(timeline, at) {
  return BODY_FROM + Math.ceil((at / MS_PER_SECOND) * FPS) - framesOf(bodyWindow(timeline).start);
}

/** The command line that starts with `prefix`; the recording is incomplete without it. */
function command(timeline, prefix) {
  const found = timeline.commands.find((candidate) => candidate.text.startsWith(prefix));
  if (!found) throw new Error(`the timeline has no command starting with ${prefix}`);
  return found;
}

/** When the page recorder did `name`. */
function action(timeline, name) {
  const found = timeline.actions.find((candidate) => candidate.action === name);
  if (!found) throw new Error(`the timeline has no ${name} action`);
  return found.at;
}

/** The first event matching `match` at or after `after`. */
function event(timeline, after, match) {
  const found = timeline.events.find((candidate) => candidate.at >= after && match(candidate));
  if (!found) throw new Error("the timeline is missing an event the edit cuts to");
  return found.at;
}

/** Where `key` sat at page time `at`: the last measurement at or before it, else the first after. */
function boxAt(timeline, key, at) {
  let before = null;
  for (const sample of timeline.samples) {
    const box = sample.boxes[key];
    if (!box) continue;
    if (sample.at > at) return before ?? box;
    before = box;
  }
  if (!before) throw new Error(`the recording never measured ${key}`);
  return before;
}

/** The box around every face in the Off room at `at`. */
function offFaces(timeline, at) {
  const room = boxAt(timeline, "room:off", at);
  const sample = [...timeline.samples].reverse().find((candidate) => candidate.at <= at) ?? timeline.samples[0];
  const faces = Object.entries(sample.boxes)
    .filter(([key, box]) => key.startsWith("person:") && box[1] >= room[1] && box[1] < room[1] + room[3])
    .map(([, box]) => box);
  if (!faces.length) return room;
  const left = Math.min(...faces.map((box) => box[0]));
  const top = Math.min(...faces.map((box) => box[1]));
  const right = Math.max(...faces.map((box) => box[0] + box[2]));
  const bottom = Math.max(...faces.map((box) => box[1] + box[3]));
  return [left, top, right - left, bottom - top];
}

/** The camera that puts the centre of page point (`cx`, `cy`) at the centre of the clear area, at zoom `z`. */
function aim(cx, cy, z) {
  const scale = PAGE_SCALE * z;
  return {
    x: cx - ((CLEAR.left + CLEAR.right) / 2 - WIDTH / 2) / scale,
    y: cy - ((CLEAR.top + CLEAR.bottom) / 2 - HEIGHT / 2) / scale,
    z,
  };
}

/** The camera that fits `box` ([x, y, width, height] in CSS pixels) into the clear area, zoomed at most `most`. */
function frameBox(box, most = ZOOM_MAX) {
  const width = box[2] + 2 * FRAME_PAD;
  const height = box[3] + 2 * FRAME_PAD;
  const fit = Math.min((CLEAR.right - CLEAR.left) / width, (CLEAR.bottom - CLEAR.top) / height) / PAGE_SCALE;
  return aim(box[0] + box[2] / 2, box[1] + box[3] / 2, Math.min(most, Math.max(ZOOM_MIN, fit)));
}

/** A camera that follows face `name` as it walks, its place averaged over a short span. */
function follow(timeline, name) {
  return (at) => {
    const offsets = [-FOLLOW_SPAN_MS, -FOLLOW_SPAN_MS / 2, 0, FOLLOW_SPAN_MS / 2, FOLLOW_SPAN_MS];
    const boxes = offsets.map((offset) => boxAt(timeline, `person:${name}`, at + offset));
    const cx = boxes.reduce((sum, box) => sum + box[0] + box[2] / 2, 0) / boxes.length;
    const cy = boxes.reduce((sum, box) => sum + box[1] + box[3] / 2, 0) / boxes.length;
    return aim(cx, cy, FOLLOW_ZOOM);
  };
}

/** A shot that holds `camera` still. */
const still = (camera) => () => camera;

/**
 * The shots in order: each starts at `at`, takes `move` milliseconds to
 * arrive from wherever the camera was, and aims by `target(at)`.
 */
export function shots(timeline) {
  const { start } = bodyWindow(timeline);
  const up = command(timeline, "muster up").at;
  const stop = command(timeline, "muster stop worker").at;
  const crash = command(timeline, "touch crash").at;
  const failed = event(timeline, crash, (e) => e.kind === "pin" && e.pin === "fail" && e.name === "payments");
  const cardOpen = action(timeline, "open-card");
  const cardClose = action(timeline, "close-card");
  const note = command(timeline, "muster note set").at;
  const notesOpen = action(timeline, "open-notes");
  const notesClose = action(timeline, "close-notes");
  return [
    { at: start, move: 0, target: still(frameBox(offFaces(timeline, start), OPENING_ZOOM_MAX)) },
    { at: up + 250, move: 3200, target: still(OVERVIEW) },
    { at: stop + 300, move: 1500, target: still(frameBox(boxAt(timeline, "room:off", stop + 1500))) },
    { at: failed, move: 1000, target: still(frameBox(boxAt(timeline, "room:failed", failed))) },
    { at: cardOpen + 200, move: 900, target: still(frameBox(boxAt(timeline, "card", cardOpen + 800))) },
    { at: cardClose, move: 1000, target: follow(timeline, "payments") },
    { at: note - 200, move: 1100, target: still(frameBox(boxAt(timeline, "chip", note))) },
    { at: notesOpen + 150, move: 900, target: still(frameBox(boxAt(timeline, "notes", notesOpen + 800))) },
    { at: notesClose + 100, move: 2400, target: still(OVERVIEW) },
  ];
}

/** Where the camera looks at page time `at`: `{ x, y }` the page point at the frame's centre, `z` the zoom. */
export function cameraAt(list, at) {
  let index = list.length - 1;
  while (index > 0 && list[index].at > at) index -= 1;
  const shot = list[index];
  const target = shot.target(at);
  if (index === 0 || shot.move === 0) return target;
  const from = cameraAt(list.slice(0, index), at);
  const t = MOVE(Math.min(1, Math.max(0, (at - shot.at) / shot.move)));
  return { x: from.x + (target.x - from.x) * t, y: from.y + (target.y - from.y) * t, z: from.z + (target.z - from.z) * t };
}

/** The captions, one at a time: `{ at, until, text, code }`, `code` set in monospace after `text`. */
export function captions(timeline) {
  const up = command(timeline, "muster up").at;
  const crash = command(timeline, "touch crash").at;
  const failed = event(timeline, crash, (e) => e.kind === "pin" && e.pin === "fail" && e.name === "payments");
  const notesOpen = action(timeline, "open-notes");
  const search = command(timeline, "muster start search").at;
  const hold = (at) => ({ at, until: at + CAPTION_HOLD_MS });
  return [
    { ...hold(up + 1600), text: "Every process, in its room" },
    { ...hold(failed + 300), text: "Something breaks, you see it" },
    { ...hold(notesOpen + 200), text: "Agents leave notes for each other" },
    { at: search + 900, until: timeline.terminal.endMs + CAPTION_HOLD_MS, text: "One command: ", code: "muster view" },
  ];
}

/** How far caption `caption` is shown at page time `at`, 0 to 1. */
export function captionShown(caption, at) {
  return interpolate(
    at,
    [caption.at, caption.at + CAPTION_FADE_MS, caption.until - CAPTION_FADE_MS, caption.until],
    [0, 1, 1, 0],
    { easing: Easing.inOut(Easing.quad), extrapolateLeft: "clamp", extrapolateRight: "clamp" },
  );
}

/** How far the terminal is lifted at page time `at`, 0 to 1: up while a command is typed and just after it is entered. */
export function liftAt(timeline, at) {
  let lift = 0;
  for (const { at: entered, text } of timeline.commands) {
    const typed = entered - Math.min(LIFT_LEAD_MAX_MS, text.length * TYPING_MS_PER_CHAR);
    const shown = interpolate(
      at,
      [typed - LIFT_EASE_MS, typed, entered + LIFT_HOLD_MS, entered + LIFT_HOLD_MS + LIFT_EASE_MS],
      [0, 1, 1, 0],
      { easing: Easing.inOut(Easing.quad), extrapolateLeft: "clamp", extrapolateRight: "clamp" },
    );
    lift = Math.max(lift, shown);
  }
  return lift;
}
