// The band: each instrument renders one hit or note into a mix at a second,
// all from oscillators and seeded noise.
import { RATE, TAU, bandpass, highpass, hz, lowpass, ramp, saw, square } from "./dsp.mjs";

/** The kick: a sine falling from its click pitch to its body, short and light. */
const KICK = { seconds: 0.32, from: 150, to: 50, sweep: 0.035, decay: 0.15, attack: 0.001, click: 0.004, clickLevel: 0.25 };
/** The clap: three quick noise bursts and a short tail in a mid band. */
const CLAP = { seconds: 0.25, band: 1400, q: 1.1, bursts: [0, 0.009, 0.019], burst: 0.004, tail: 0.075, tailLevel: 0.6 };
/** The snare of a fill: a short tone under a noise band. */
const SNARE = { seconds: 0.18, tone: 196, toneDecay: 0.05, toneLevel: 0.5, band: 2200, q: 0.8, decay: 0.07 };
/** The hats: noise above their cutoff, closed short and open long. */
const HAT = { cutoff: 7500, closed: 0.028, open: 0.16 };
/** The crash: bright noise ringing out on a cut. */
const CRASH = { seconds: 1.4, cutoff: 4200, decay: 0.55, attack: 0.002 };
/** The bass: a saw and a sine an octave down, through a low-pass that snaps shut. */
const BASS = { attack: 0.003, release: 0.03, sub: 0.6, cutoffFrom: 1600, cutoffTo: 280, cutoffDecay: 0.07 };
/** The arpeggio's pluck: a saw through a bright low-pass that closes fast. */
const PLUCK = { seconds: 0.45, attack: 0.002, decay: 0.13, cutoffTo: 500, cutoffDecay: 0.05 };
/** The chord stab: detuned saws, short and filtered. */
const STAB = { decay: 0.11, detuneCents: 8, cutoff: 2600, cutoffTo: 900, cutoffDecay: 0.08 };
/** The lead: a soft square with a little vibrato. */
const LEAD = { attack: 0.006, release: 0.06, sustain: 0.55, decay: 0.12, cutoff: 3200, vibratoHz: 5.5, vibratoCents: 9, vibratoDelay: 0.12 };
/** The riser: noise whose band climbs into the cut. */
const RISER = { from: 500, to: 7000, q: 1.6 };
/** The sustained chord the cadence lands on. */
const PAD = { attack: 0.01, decay: 1.1, cutoffFrom: 4200, cutoffTo: 900, cutoffDecay: 0.5, detuneCents: 7 };

/** Cents as a frequency ratio. */
function cents(value) {
  return 2 ** (value / 1200);
}

/** A kick at `at`. */
export function kick(mix, at, gain, random) {
  let phase = 0;
  mix.render(at, KICK.seconds, 0, (index) => {
    const t = index / RATE;
    phase += (TAU * (KICK.to + (KICK.from - KICK.to) * Math.exp(-t / KICK.sweep))) / RATE;
    const click = KICK.clickLevel * random() * Math.exp(-t / KICK.click);
    return gain * (Math.sin(phase) * Math.exp(-t / KICK.decay) * ramp(t / KICK.attack) + click);
  });
}

/** A clap at `at`, panned to `pan`. */
export function clap(mix, at, gain, pan, random) {
  const band = bandpass(CLAP.q);
  const lastBurst = CLAP.bursts.at(-1);
  mix.render(at, CLAP.seconds, pan, (index) => {
    const t = index / RATE;
    let envelope = 0;
    for (const start of CLAP.bursts) if (t >= start) envelope += Math.exp(-(t - start) / CLAP.burst);
    if (t >= lastBurst) envelope += CLAP.tailLevel * Math.exp(-(t - lastBurst) / CLAP.tail);
    return gain * envelope * band(random(), CLAP.band);
  });
}

/** A fill's snare hit at `at`. */
export function snare(mix, at, gain, pan, random) {
  const band = bandpass(SNARE.q);
  mix.render(at, SNARE.seconds, pan, (index) => {
    const t = index / RATE;
    const tone = SNARE.toneLevel * Math.sin(TAU * SNARE.tone * t) * Math.exp(-t / SNARE.toneDecay);
    return gain * (tone + band(random(), SNARE.band) * Math.exp(-t / SNARE.decay));
  });
}

/** A hat at `at`, closed or `open`. */
export function hat(mix, at, gain, pan, open, random) {
  const high = [highpass(), highpass()];
  const decay = open ? HAT.open : HAT.closed;
  mix.render(at, decay * 6, pan, (index) => {
    const t = index / RATE;
    return gain * Math.exp(-t / decay) * high[1](high[0](random(), HAT.cutoff), HAT.cutoff);
  });
}

/** A crash at `at`, wide. */
export function crash(mix, at, gain, random) {
  const high = [highpass(), highpass()];
  mix.render(at, CRASH.seconds, 0, (index) => {
    const t = index / RATE;
    const envelope = gain * ramp(t / CRASH.attack) * Math.exp(-t / CRASH.decay);
    return [envelope * high[0](random(), CRASH.cutoff), envelope * high[1](random(), CRASH.cutoff)];
  });
}

/** A bass note `note` at `at`, held `seconds`. */
export function bass(mix, at, note, seconds, gain) {
  const tone = saw(hz(note));
  const filter = lowpass();
  const f = hz(note) / 2;
  mix.render(at, seconds + BASS.release, 0, (index) => {
    const t = index / RATE;
    const envelope = ramp(t / BASS.attack) * (1 - ramp((t - seconds) / BASS.release));
    const cutoff = BASS.cutoffTo + (BASS.cutoffFrom - BASS.cutoffTo) * Math.exp(-t / BASS.cutoffDecay);
    return gain * envelope * (filter(tone(), cutoff) + BASS.sub * Math.sin(TAU * f * t));
  });
}

/** An arpeggio pluck of `note` at `at`, its filter opened to `brightness` hertz. */
export function pluck(mix, at, note, gain, pan, brightness) {
  const tone = saw(hz(note));
  const filter = lowpass();
  mix.render(at, PLUCK.seconds, pan, (index) => {
    const t = index / RATE;
    const cutoff = PLUCK.cutoffTo + (brightness - PLUCK.cutoffTo) * Math.exp(-t / PLUCK.cutoffDecay);
    return gain * ramp(t / PLUCK.attack) * Math.exp(-t / PLUCK.decay) * filter(tone(), cutoff);
  });
}

/** A chord stab of `notes` at `at`, spread across the field. */
export function stab(mix, at, notes, gain) {
  const voices = notes.flatMap((note) => [-1, 1].map((side) => ({ side, tone: saw(hz(note) * cents(side * STAB.detuneCents)) })));
  const filters = [lowpass(), lowpass()];
  mix.render(at, STAB.decay * 6, 0, (index) => {
    const t = index / RATE;
    const envelope = gain * Math.exp(-t / STAB.decay);
    const cutoff = STAB.cutoffTo + (STAB.cutoff - STAB.cutoffTo) * Math.exp(-t / STAB.cutoffDecay);
    let left = 0;
    let right = 0;
    for (const voice of voices) {
      const value = voice.tone();
      if (voice.side < 0) left += value;
      else right += value;
    }
    return [envelope * filters[0](left, cutoff), envelope * filters[1](right, cutoff)];
  });
}

/** A lead note `note` at `at`, held `seconds`. */
export function lead(mix, at, note, seconds, gain, pan) {
  const f = hz(note);
  const tone = square(f);
  const wobble = square(f * cents(LEAD.vibratoCents));
  const filter = lowpass();
  mix.render(at, seconds + LEAD.release, pan, (index) => {
    const t = index / RATE;
    const held = LEAD.sustain + (1 - LEAD.sustain) * Math.exp(-t / LEAD.decay);
    const envelope = ramp(t / LEAD.attack) * held * (1 - ramp((t - seconds) / LEAD.release));
    const vibrato = ramp((t - LEAD.vibratoDelay) / LEAD.vibratoDelay) * (0.5 + 0.5 * Math.sin(TAU * LEAD.vibratoHz * t));
    return gain * envelope * filter((1 - vibrato) * tone() + vibrato * wobble(), LEAD.cutoff);
  });
}

/** A riser from `at` for `seconds`, swelling into whatever comes next. */
export function riser(mix, at, seconds, gain, random) {
  const bands = [bandpass(RISER.q), bandpass(RISER.q)];
  mix.render(at, seconds, 0, (index) => {
    const x = index / (seconds * RATE);
    const centre = RISER.from * (RISER.to / RISER.from) ** x;
    const envelope = gain * x * x;
    return [envelope * bands[0](random(), centre), envelope * bands[1](random(), centre)];
  });
}

/** A sustained chord of `notes` at `at`, ringing out over `seconds`. */
export function chord(mix, at, notes, seconds, gain) {
  const voices = notes.flatMap((note) => [-1, 1].map((side) => ({ side, tone: saw(hz(note) * cents(side * PAD.detuneCents)) })));
  const filters = [lowpass(), lowpass()];
  mix.render(at, seconds, 0, (index) => {
    const t = index / RATE;
    const envelope = gain * ramp(t / PAD.attack) * Math.exp(-t / PAD.decay);
    const cutoff = PAD.cutoffTo + (PAD.cutoffFrom - PAD.cutoffTo) * Math.exp(-t / PAD.cutoffDecay);
    let left = 0;
    let right = 0;
    for (const voice of voices) {
      const value = voice.tone();
      if (voice.side < 0) left += value;
      else right += value;
    }
    return [envelope * filters[0](left, cutoff), envelope * filters[1](right, cutoff)];
  });
}
