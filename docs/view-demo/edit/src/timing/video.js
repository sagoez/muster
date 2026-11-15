// Where the recording sits in the video: the body's window of page time, the
// frames the intro, body and outro start on, and the frame any page moment
// shows on. The composition and the soundtrack both place things by these, so
// a sound always lands with its picture.
import { FADE_FRAMES, FPS, INTRO_FRAMES, MS_PER_SECOND, OUTRO_FRAMES, framesOf } from "../brand/theme.js";

/** How much of the terminal's first second is shown before the first command lands. */
const LEAD_IN_MS = 150;

/** The soundtrack's file in the public folder, which the sound render writes and the composition plays. */
export const SOUNDTRACK = "soundtrack.wav";

/** The video frame the body starts on: under the intro's fade. */
export const BODY_FROM = INTRO_FRAMES - FADE_FRAMES;

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

/** The video frame the outro starts on, over the body's last frames. */
export function outroFrom(timeline) {
  return BODY_FROM + bodyWindow(timeline).frames - FADE_FRAMES;
}

/** The video's length in frames for `timeline`. */
export function totalFrames(timeline) {
  return outroFrom(timeline) + OUTRO_FRAMES;
}

/** The video second page time `at` plays at, unrounded: for what the page footage does not show, like a key press. */
export function videoTimeOf(timeline, at) {
  return BODY_FROM / FPS + (at - bodyWindow(timeline).start) / MS_PER_SECOND;
}

/** The video frame on which page time `at` first shows: the body plays the page footage from its frame at the body's start. */
export function videoFrameOf(timeline, at) {
  return BODY_FROM + Math.ceil((at / MS_PER_SECOND) * FPS) - framesOf(bodyWindow(timeline).start);
}
