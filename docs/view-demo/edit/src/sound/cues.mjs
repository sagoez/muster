// The interface sounds: a bright pop when a process comes up, a playful
// "uh-oh" when one fails, a quick upward blip when it restarts, and a soft
// whoosh under each camera move. Each lands on the frame its picture shows.
import { RATE, TAU, bandpass, lowpass, panGains, ramp } from "./dsp.mjs";

/** Pins closer than this to the first of a run share one pop, so a stack coming up is one sound. */
const PIN_CLUSTER_MS = 500;
/** The pop: two rounded blips a fifth apart, each gliding up into its pitch. */
const POP = { notes: [1174.66, 1760], step: 0.055, attack: 0.002, glide: 0.018, from: 0.7, decay: 0.07, level: 0.45, overtone: 0.18, pan: 0.15 };
/** The uh-oh: two soft blips falling a minor third, the second bending down at its end. */
const UH_OH = { notes: [880, 739.99], lengths: [0.1, 0.17], step: 0.14, attack: 0.006, bend: 0.94, level: 0.26, triangle: 0.35, pan: 0.1 };
/** The restart: a quick upward sweep into a pop. */
const RESTART = { from: 380, to: 1500, sweep: 0.13, level: 0.32, noise: 0.25, q: 2, pan: 0.1 };
/** The whoosh: its level, band sweep for a push (reversed for a pull, arched for a pan), the band's Q and the low-pass after it. */
const WHOOSH = { level: 0.5, sweep: [350, 1800], pan: [600, 1100], q: 1.2, cutoff: 3000, drift: 0.3 };
/** The odd harmonics of the uh-oh's soft triangle. */
const TRIANGLE_HARMONICS = [3, 5];

/**
 * The status cues, each `{ kind, at }` in video seconds: ok, fail and again
 * (an ok after a fail of the same process). `secondsOf(pageMs)` gives the
 * second of the frame a page moment shows on; `inBody(pageMs)` says whether it shows.
 */
export function statusCues(timeline, secondsOf, inBody) {
  const list = [];
  const failed = new Set();
  let runStart = -Infinity;
  for (const event of timeline.events) {
    if (event.kind !== "pin" || !inBody(event.at)) continue;
    const key = `${event.project}/${event.name}`;
    if (event.pin === "fail") {
      failed.add(key);
      list.push({ kind: "fail", at: secondsOf(event.at) });
    } else if (event.pin === "ok" && failed.has(key)) {
      failed.delete(key);
      list.push({ kind: "again", at: secondsOf(event.at) });
    } else if (event.pin === "ok" && event.at - runStart > PIN_CLUSTER_MS) {
      runStart = event.at;
      list.push({ kind: "ok", at: secondsOf(event.at) });
    }
  }
  return list;
}

/** One blip at `freq` from `at`: a sine gliding up from below its pitch, with a touch of octave. */
function blip(mix, at, freq, level, pan) {
  let phase = 0;
  mix.render(at, POP.decay * 7, pan, (index) => {
    const t = index / RATE;
    const f = freq * (1 - (1 - POP.from) * Math.exp(-t / POP.glide));
    phase += (TAU * f) / RATE;
    const envelope = ramp(t / POP.attack) * Math.exp(-t / POP.decay);
    return level * envelope * (Math.sin(phase) + POP.overtone * Math.sin(2 * phase));
  });
}

/** The ok pop at `at`. */
export function pop(mix, at) {
  POP.notes.forEach((freq, index) => blip(mix, at + index * POP.step, freq, POP.level * (index ? 0.8 : 1), POP.pan));
}

/** The fail's "uh-oh" at `at`. */
export function uhOh(mix, at) {
  UH_OH.notes.forEach((freq, index) => {
    const length = UH_OH.lengths[index];
    const bends = index === UH_OH.notes.length - 1;
    let phase = 0;
    mix.render(at + index * UH_OH.step, length + UH_OH.attack * 4, UH_OH.pan, (sample) => {
      const t = sample / RATE;
      const f = bends ? freq * (1 - (1 - UH_OH.bend) * ramp(t / length)) : freq;
      phase += (TAU * f) / RATE;
      let tone = Math.sin(phase);
      for (const harmonic of TRIANGLE_HARMONICS) tone += (UH_OH.triangle * Math.sin(harmonic * phase)) / (harmonic * harmonic);
      const envelope = ramp(t / UH_OH.attack) * (1 - ramp((t - length) / (UH_OH.attack * 4)));
      return UH_OH.level * envelope * tone;
    });
  });
}

/** The restart's upward swoosh-blip, ending on its pitch at `at` + the sweep. */
export function upBlip(mix, at, random) {
  const band = bandpass(RESTART.q);
  let phase = 0;
  mix.render(at, RESTART.sweep, RESTART.pan, (index) => {
    const t = index / RATE;
    const x = t / RESTART.sweep;
    const f = RESTART.from * (RESTART.to / RESTART.from) ** x;
    phase += (TAU * f) / RATE;
    const envelope = RESTART.level * ramp(x * 3) * (0.4 + 0.6 * x);
    return envelope * (Math.sin(phase) + RESTART.noise * band(random(), f * 2));
  });
  blip(mix, at + RESTART.sweep, RESTART.to * 0.78, POP.level * 0.8, RESTART.pan);
}

/** A filtered-noise whoosh from `at` for `seconds`, its band rising for a push (`direction` 1), falling for a pull (-1), arching for a pan (0). */
export function whoosh(mix, at, seconds, direction, random) {
  const [low, high] = direction === 0 ? WHOOSH.pan : WHOOSH.sweep;
  const bands = [bandpass(WHOOSH.q), bandpass(WHOOSH.q)];
  const smooth = [lowpass(), lowpass()];
  mix.render(at, seconds, 0, (index) => {
    const x = index / (seconds * RATE);
    const sweep = direction === 0 ? Math.sin(Math.PI * x) : direction > 0 ? x : 1 - x;
    const centre = low * (high / low) ** sweep;
    const envelope = WHOOSH.level * Math.sin(Math.PI * x) ** 2;
    const [gl, gr] = panGains(Math.sin(Math.PI * x) * WHOOSH.drift * (direction || 1));
    return [
      envelope * gl * smooth[0](bands[0](random(), centre), WHOOSH.cutoff),
      envelope * gr * smooth[1](bands[1](random(), centre), WHOOSH.cutoff),
    ];
  });
}
