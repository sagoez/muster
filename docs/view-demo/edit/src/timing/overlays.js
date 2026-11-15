// What sits over the footage and when: the captions, one at a time, and how
// far the terminal is lifted while a command is typed.
import { Easing, interpolate } from "remotion";
import { action, command, paymentsFailed } from "./recording.js";

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

/** The captions, one at a time: `{ at, until, text, code }`, `code` set in monospace after `text`. */
export function captions(timeline) {
  const up = command(timeline, "muster up").at;
  const failed = paymentsFailed(timeline);
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
