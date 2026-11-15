// How the camera cuts the recording: the shots in order, each aimed at a box
// the recording measured, and where the camera looks at any page moment.
import { Easing } from "remotion";
import { HEIGHT, WIDTH } from "../brand/theme.js";
import { action, boxAt, command, offFaces, paymentsFailed } from "./recording.js";
import { bodyWindow } from "./video.js";

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
  const failed = paymentsFailed(timeline);
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
