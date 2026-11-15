// The keyboard: a mechanical key click for every character the tape types,
// and a heavier thock for each Enter. Each command's Enter is when the shell
// logged it; its characters come before it at the tape's typing speed, read
// from view.tape, on the page's clock.
import { RATE, TAU, bandpass, ramp } from "./dsp.mjs";

/** The tape's typing speed, its per-command overrides, and its quoted text. */
const SPEED = /^Set TypingSpeed (\d+)ms/;
const TYPE = /^Type(?:@(\d+)ms)? (["`])(.*)\2\s*$/;
/** How long after Enter the shell logs a command, on the page's clock: measured off the terminal footage, it varies by about this much again. */
const SHELL_LAG_MS = 30;
/** How far a key may stray from its nominal moment, in seconds, and a hair of lag so the sound never leads the picture. */
const JITTER_S = 0.004;
const LAG_S = 0.008;
/** How far a key's pitch and level may stray, as a ratio and in decibels. */
const PITCH_SPREAD = 0.08;
const LEVEL_SPREAD_DB = 2;
/** The keyboard's rows, so each key pans by where it sits, and the pan of its left and right edges. */
const ROWS = ["`1234567890-=", "qwertyuiop[]\\", "asdfghjkl;'", "zxcvbnm,./"];
const PAN_LEFT = -0.5;
const PAN_RIGHT = 0.05;
/** The three key kinds: the top click, the body's ring, how long it rings, the bottom-out after the press, and the level. */
const KEYS = {
  key: { click: 4200, clickDecay: 0.0022, body: 360, bodyDecay: 0.016, bottom: 0.011, level: 0.24, pan: null },
  space: { click: 3000, clickDecay: 0.003, body: 210, bodyDecay: 0.026, bottom: 0.014, level: 0.28, pan: -0.2 },
  enter: { click: 2600, clickDecay: 0.0035, body: 150, bodyDecay: 0.045, bottom: 0.017, level: 0.42, pan: 0.15 },
};
/** How loud the bottom-out is to the press, how much the body's noise adds to its tone, and the band's Q. */
const BOTTOM_LEVEL = 0.45;
const BODY_NOISE = 0.5;
const CLICK_Q = 0.9;
const BODY_Q = 3;
/** How long a key sound lasts, in its body decays. */
const KEY_LENGTH = 6;

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
 * Every key press, `{ at, kind, char }` in video seconds: a command's
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
    [...text].forEach((char, index) => {
      presses.push({ at: secondsOf(enter - (text.length - index) * step), kind: char === " " ? "space" : "key", char });
    });
    presses.push({ at: secondsOf(enter), kind: "enter", char: "\n" });
  }
  return presses;
}

/** One key press `press`, varied by `random`. */
export function keyPress(mix, press, random) {
  const key = KEYS[press.kind];
  const pitch = 1 + PITCH_SPREAD * random();
  const level = key.level * 10 ** ((LEVEL_SPREAD_DB * random()) / 20);
  const at = press.at + LAG_S + JITTER_S * random();
  const clickBand = bandpass(CLICK_Q);
  const bottomBand = bandpass(CLICK_Q);
  const bodyBand = bandpass(BODY_Q);
  const body = key.body * pitch;
  const pan = key.pan ?? panOf(press.char);
  mix.render(at, key.bodyDecay * KEY_LENGTH, pan, (index) => {
    const t = index / RATE;
    const top = clickBand(random(), key.click * pitch) * Math.exp(-t / key.clickDecay);
    const since = t - key.bottom;
    const bottom = since >= 0 ? BOTTOM_LEVEL * bottomBand(random(), key.click * pitch * 0.7) * Math.exp(-since / key.clickDecay) : 0;
    const ring = ramp(t / key.clickDecay) * Math.exp(-t / key.bodyDecay);
    const tone = ring * (Math.sin(TAU * body * t) + BODY_NOISE * bodyBand(random(), body * 2));
    return level * (top + bottom + tone);
  });
}
