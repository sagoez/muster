// The keyboard: one strike of the user's typewriter recording for every
// character the tape types, and its heaviest strike for each Enter. Each
// command's Enter is when the shell logged it; its characters come before it
// at the tape's typing speed, read from view.tape, on the page's clock.
import { RATE, gainOf } from "./dsp.mjs";

/** The tape's typing speed, its per-command overrides, and its quoted text. */
const SPEED = /^Set TypingSpeed (\d+)ms/;
const TYPE = /^Type(?:@(\d+)ms)? (["`])(.*)\2\s*$/;
/** How long after Enter the shell logs a command, on the page's clock: measured off the terminal footage, it varies by about this much again. */
const SHELL_LAG_MS = 30;
/** A hair of lag so a strike never leads the character it types, in seconds. */
const LAG_S = 0.008;
/** The strike finder: its envelope step, how far back it compares, the jump that makes a strike, and how far under the loudest strike one may be. */
const ENVELOPE_S = 0.002;
const LOOKBACK_STEPS = 5;
const STRIKE_JUMP_DB = 12;
const STRIKE_FLOOR_DB = -20;
/** The closest two strikes may be, the most a slice keeps, how early it starts before its strike and how long it fades out. */
const STRIKE_GAP_S = 0.04;
const SLICE_MAX_S = 0.14;
const SLICE_LEAD_S = 0.003;
const SLICE_FADE_S = 0.025;
/** How far a strike's pitch and level may stray, as a ratio and in decibels. */
const PITCH_SPREAD = 0.05;
const LEVEL_SPREAD_DB = 1.5;
/** The level of a character and of Enter, and how far Enter's pitch drops so it lands heavier. */
const KEY_LEVEL = 0.5;
const ENTER_LEVEL = 0.7;
const ENTER_PITCH = 0.92;
/** The keyboard's rows, so each key pans by where it sits, the pan of its edges, and Enter's. */
const ROWS = ["`1234567890-=", "qwertyuiop[]\\", "asdfghjkl;'", "zxcvbnm,./"];
const PAN_LEFT = -0.5;
const PAN_RIGHT = 0.05;
const ENTER_PAN = 0.15;
/** The fewest strikes the recording must hold, so no strike repeats back to back. */
const MIN_STRIKES = 2;

/** Each typed line's speed in milliseconds per character, by its text, from the tape. */
function typingSpeeds(tape) {
  let fallback = null;
  const speeds = new Map();
  for (const line of tape.split("\n")) {
    const speed = line.match(SPEED);
    if (speed) fallback = Number(speed[1]);
    const typed = line.match(TYPE);
    if (typed) speeds.set(typed[3], typed[1] ? Number(typed[1]) : fallback);
  }
  if (fallback === null) throw new Error("view.tape sets no TypingSpeed");
  return { fallback, speeds };
}

/** Where `char` sits on the keyboard, as a pan. */
function panOf(char) {
  const lower = char.toLowerCase();
  for (const row of ROWS) {
    const column = row.indexOf(lower);
    if (column >= 0) return PAN_LEFT + ((PAN_RIGHT - PAN_LEFT) * column) / (row.length - 1);
  }
  return (PAN_LEFT + PAN_RIGHT) / 2;
}

/**
 * Every key press, `{ at, enter, char }` in video seconds: a command's
 * characters at its typing speed before its Enter, which is when it was
 * logged. `secondsOf(pageMs)` places page time in the video; `inBody(pageMs)` says whether it shows.
 */
export function keystrokes(timeline, tape, secondsOf, inBody) {
  const { fallback, speeds } = typingSpeeds(tape);
  const presses = [];
  for (const { at, text } of timeline.commands) {
    if (!inBody(at)) continue;
    // The terminal footage plays at `rate`, so a tape millisecond lasts 1 / rate on the page's clock.
    const step = (speeds.get(text) ?? fallback) / timeline.terminal.rate;
    const enter = at - SHELL_LAG_MS;
    [...text].forEach((char, index) => presses.push({ at: secondsOf(enter - (text.length - index) * step), enter: false, char }));
    presses.push({ at: secondsOf(enter), enter: true, char: "\n" });
  }
  return presses;
}

/**
 * The strikes in `recording` (`{ left, right }`), each a slice peaking at 1:
 * `{ left, right, peak }`, `peak` its level in the recording. A strike is
 * where the 2 ms peak envelope jumps by `STRIKE_JUMP_DB` within 10 ms and
 * reaches within `STRIKE_FLOOR_DB` of the loudest.
 */
export function strikes(recording) {
  const step = Math.round(ENVELOPE_S * RATE);
  const envelope = [];
  for (let from = 0; from < recording.left.length; from += step) {
    let peak = 0;
    for (let index = from; index < Math.min(recording.left.length, from + step); index += 1) {
      peak = Math.max(peak, Math.abs(recording.left[index]), Math.abs(recording.right[index]));
    }
    envelope.push(20 * Math.log10(peak + Number.EPSILON));
  }
  const loudest = Math.max(...envelope);
  const gap = Math.round(STRIKE_GAP_S / ENVELOPE_S);
  const onsets = [];
  for (let at = LOOKBACK_STEPS; at < envelope.length; at += 1) {
    const before = Math.max(...envelope.slice(at - LOOKBACK_STEPS, at));
    const fresh = !onsets.length || at - onsets.at(-1) > gap;
    if (fresh && envelope[at] - before > STRIKE_JUMP_DB && envelope[at] > loudest + STRIKE_FLOOR_DB) onsets.push(at);
  }
  const lead = Math.round(SLICE_LEAD_S * RATE);
  const list = onsets.map((at, index) => {
    const from = Math.max(0, at * step - lead);
    const next = onsets[index + 1] !== undefined ? onsets[index + 1] * step - lead : recording.left.length;
    const to = Math.min(next, from + Math.round(SLICE_MAX_S * RATE));
    const fade = Math.round(SLICE_FADE_S * RATE);
    const left = recording.left.slice(from, to);
    const right = recording.right.slice(from, to);
    let peak = 0;
    for (let index = 0; index < left.length; index += 1) peak = Math.max(peak, Math.abs(left[index]), Math.abs(right[index]));
    for (let index = 0; index < left.length; index += 1) {
      const tail = Math.min(1, (left.length - index) / fade);
      left[index] *= tail / peak;
      right[index] *= tail / peak;
    }
    return { left, right, peak };
  });
  if (list.length < MIN_STRIKES) throw new Error(`the typewriter recording has ${list.length} clear strikes; it needs at least ${MIN_STRIKES}`);
  return list;
}

/** Plays `slice` at second `at`, `pitch` times faster, `gain` loud and panned to `pan`. */
function play(mix, slice, at, pitch, gain, pan) {
  const length = slice.left.length;
  mix.render(at, length / pitch / RATE, pan, (index) => {
    const position = index * pitch;
    const base = Math.floor(position);
    const fraction = position - base;
    const left = slice.left[base] + ((slice.left[base + 1] ?? 0) - slice.left[base]) * fraction;
    const right = slice.right[base] + ((slice.right[base + 1] ?? 0) - slice.right[base]) * fraction;
    return gain * (left + right) * 0.5;
  });
}

/** Plays every press of `presses` from `pool`: characters from all but the heaviest strike, never one twice in a row, Enter on the heaviest. */
export function typeOut(mix, presses, pool, random) {
  const heaviest = pool.reduce((best, slice) => (slice.peak > best.peak ? slice : best));
  const keys = pool.filter((slice) => slice !== heaviest);
  let previous = -1;
  for (const press of presses) {
    const spread = gainOf(LEVEL_SPREAD_DB * random());
    const pitch = 1 + PITCH_SPREAD * random();
    if (press.enter) {
      play(mix, heaviest, press.at + LAG_S, ENTER_PITCH * pitch, ENTER_LEVEL * spread, ENTER_PAN);
      continue;
    }
    let choice = Math.floor(((random() + 1) / 2) * keys.length) % keys.length;
    if (choice === previous && keys.length > 1) choice = (choice + 1) % keys.length;
    previous = choice;
    play(mix, keys[choice], press.at + LAG_S, pitch, KEY_LEVEL * spread, panOf(press.char));
  }
}
