// The music: an upbeat major-key groove on the beat grid, one section per
// cut. It starts sparse while the terminal opens, builds as the floor fills,
// drops its kick for a bar when something breaks, peaks while the agents
// work, and builds into a final cadence on the outro's install line.
import { Mix } from "./dsp.mjs";
import { bass, chord, clap, crash, hat, kick, lead, pluck, riser, snare, stab } from "./instruments.mjs";

/**
 * What each section plays, one per anchor of the grid in order: the music's
 * start, each camera move, and the cadence.
 */
export const PLAN = ["intro", "groove", "lift", "break", "lift", "peak", "peak", "peak", "build", "cadence"];
/** The layers each kind of section plays. */
const LAYERS = {
  intro: new Set(["arp"]),
  groove: new Set(["arp", "kick", "clap", "hats", "bass"]),
  lift: new Set(["arp", "kick", "clap", "hats", "bass", "stabs"]),
  break: new Set(["arp", "kick", "clap", "hats", "bass", "stabs"]),
  peak: new Set(["arp", "kick", "clap", "hats", "bass", "stabs", "lead", "sixteenths", "open"]),
  build: new Set(["arp", "kick", "bass", "hats"]),
};

/** The key, D major, as the scale's pitch classes from its tonic, and the tonic's bass note. */
const SCALE = [0, 2, 4, 5, 7, 9, 11];
const TONIC = 38;
/** The chords as scale steps from the tonic, one a bar: I, V, IV, V. */
const PROGRESSION = [0, 4, 3, 4];
/** The dominant the build sits on and the tonic the cadence lands on. */
const DOMINANT = 4;
const HOME = 0;
/** How far over the bass the arpeggio, stabs and lead sit, in semitones. */
const ARP_OCTAVES = 24;
const STAB_OCTAVES = 12;
const LEAD_OCTAVES = 36;
/** The arpeggio's sixteenths, as indices into the chord's tones and the octave over them. */
const ARP_PATTERN = [0, 1, 2, 3, 2, 1, 2, 3];
/** The lead's hook in each bar: [beat, scale steps over the chord's root, length in beats]. */
const HOOK = [
  [0, 4, 0.4],
  [0.5, 2, 0.4],
  [1, 4, 0.6],
  [1.75, 5, 0.6],
  [2.5, 4, 0.4],
  [3, 2, 0.4],
  [3.5, 1, 0.4],
];
/** Beats in a bar. */
const BAR = 4;

/** Levels, before the mix is brought to its loudness. */
const LEVEL = {
  kick: 0.42,
  clap: 0.34,
  snare: 0.28,
  hat: 0.13,
  openHat: 0.08,
  crash: 0.13,
  bass: 0.12,
  arp: 0.13,
  stab: 0.07,
  lead: 0.1,
  riser: 0.07,
  chord: 0.06,
};
/** The arpeggio's filter through the intro, opening toward the groove, and once the groove is on. */
const ARP_BRIGHT = { intro: [1400, 3400], open: 4200 };
/** The arpeggio's echo: three sixteenths at the target tempo, its feedback and level. */
const ECHO = { seconds: 0.38, feedback: 0.35, wet: 0.28 };
/** The pan the arpeggio swings between and where the claps and hats sit. */
const ARP_PAN = 0.35;
const CLAP_PAN = -0.08;
const HAT_PAN = 0.25;
/** How the cadence plays: the chord's length, the run up over it and the button after it, in beats. */
const CADENCE = { ring: 4, bass: 0.9, run: 8, runStep: 0.25, button: 3, buttonLength: 0.35 };
/** The fill's snare hits in the beat before a cut, and how much each grows. */
const FILL = [0, 0.25, 0.5, 0.75];
const FILL_GROW = 0.25;
/** The beats a riser lasts into each cut. */
const RISER_BEATS = 2;

/** MIDI note `steps` scale steps above the scale degree `degree` of the key, from `base`. */
function scaleNote(base, degree, steps = 0) {
  const index = degree + steps;
  const octave = Math.floor(index / SCALE.length);
  return base + SCALE[((index % SCALE.length) + SCALE.length) % SCALE.length] + 12 * octave;
}

/** The chord on `degree`: its root in the bass, its triad, and the arpeggio's four tones. */
function chordOn(degree) {
  const root = scaleNote(TONIC, degree);
  const triad = [0, 2, 4].map((step) => scaleNote(TONIC, degree, step));
  return { degree, root, triad, arp: [...triad.map((note) => note + ARP_OCTAVES), root + ARP_OCTAVES + 12] };
}

/** The sections, each `{ kind, from, to }` in whole beats, from the grid's anchor beats. */
function sections(grid) {
  if (grid.beats.length !== PLAN.length) {
    throw new Error(`the score plans ${PLAN.length} sections but the edit has ${grid.beats.length} anchors`);
  }
  return PLAN.map((kind, index) => ({
    kind,
    from: Math.ceil(grid.beats[index]),
    to: index + 1 < PLAN.length ? Math.ceil(grid.beats[index + 1]) : null,
    accent: !Number.isInteger(grid.beats[index]) ? grid.beats[index] : null,
  }));
}

/** Plays one beat `beat` of a section of `kind`, `position` beats in, on chord `harmony`. */
function playBeat(parts, { kind, beat, position, last, harmony, intro }) {
  const { mix, arps, at, random } = parts;
  const layers = LAYERS[kind];
  const inBar = position % BAR;
  const kickless = kind === "break" && position < BAR;
  if (layers.has("kick") && !kickless && !(kind === "build" && last)) kick(mix, at(beat), LEVEL.kick, random);
  if (layers.has("clap") && (inBar === 1 || inBar === 3) && !last) clap(mix, at(beat), LEVEL.clap, CLAP_PAN, random);
  if (intro && (inBar === 1 || inBar === 3)) clap(mix, at(beat), LEVEL.clap * 0.5, CLAP_PAN, random);
  if (layers.has("hats") || (intro && position >= BAR)) {
    const steps = layers.has("sixteenths") ? [0, 0.25, 0.5, 0.75] : [0, 0.5];
    for (const step of steps) {
      const open = layers.has("open") && step === 0.5;
      const level = (open ? LEVEL.openHat : LEVEL.hat) * (step === 0.5 ? 1 : 0.55);
      hat(mix, at(beat + step), level, HAT_PAN, open, random);
    }
  }
  if (layers.has("bass")) {
    const halfTime = kickless;
    const lengthOf = (beats) => (at(beat + beats) - at(beat)) * 0.85;
    if (halfTime) bass(mix, at(beat), harmony.root, lengthOf(1), LEVEL.bass);
    else {
      bass(mix, at(beat), harmony.root, lengthOf(0.45), LEVEL.bass);
      const up = inBar === 3 ? harmony.root + 7 : harmony.root + 12;
      bass(mix, at(beat + 0.5), up, lengthOf(0.4), LEVEL.bass * 0.8);
    }
  }
  if (layers.has("stabs") && !kickless && (inBar === 0 || inBar === 2)) stab(mix, at(beat + 0.5), harmony.triad.map((n) => n + STAB_OCTAVES + 12), LEVEL.stab);
  if (layers.has("arp")) {
    const brightness = intro ? intro(beat) : ARP_BRIGHT.open;
    for (let step = 0; step < 4; step += 1) {
      const sixteenth = Math.round(beat * 4) + step;
      const note = harmony.arp[ARP_PATTERN[sixteenth % ARP_PATTERN.length]];
      pluck(arps, at(beat + step / 4), note, LEVEL.arp * (step === 0 ? 1 : 0.75), sixteenth % 2 ? ARP_PAN : -ARP_PAN, brightness);
    }
  }
  if (layers.has("lead") && inBar === 0) {
    for (const [offset, steps, length] of HOOK) {
      const start = beat + offset;
      lead(mix, at(start), scaleNote(TONIC + LEAD_OCTAVES - 12, harmony.degree, steps), at(start + length) - at(start), LEVEL.lead, 0.1);
    }
  }
}

/** The beat before a cut: a snare fill and a riser into it, the drums held back. */
function fill(parts, beat) {
  const { mix, at, random } = parts;
  FILL.forEach((step, index) => snare(mix, at(beat + step), LEVEL.snare * (1 + FILL_GROW * index), 0, random));
}

/** The cadence at `beat`: the home chord rung out under a run up, then a short button. */
function cadence(parts, beat) {
  const { mix, arps, at, random } = parts;
  const home = chordOn(HOME);
  kick(mix, at(beat), LEVEL.kick, random);
  crash(mix, at(beat), LEVEL.crash, random);
  clap(mix, at(beat), LEVEL.clap, CLAP_PAN, random);
  bass(mix, at(beat), home.root, at(beat + CADENCE.bass) - at(beat), LEVEL.bass);
  chord(mix, at(beat), [...home.triad.map((n) => n + STAB_OCTAVES), home.root + ARP_OCTAVES], at(beat + CADENCE.ring) - at(beat), LEVEL.chord);
  for (let step = 0; step < CADENCE.run; step += 1) {
    const note = home.arp[step % home.arp.length] + 12 * Math.floor(step / home.arp.length);
    pluck(arps, at(beat + step * CADENCE.runStep), note, LEVEL.arp * (1 - step / (CADENCE.run * 1.5)), step % 2 ? ARP_PAN : -ARP_PAN, ARP_BRIGHT.open);
  }
  const button = beat + CADENCE.button;
  kick(mix, at(button), LEVEL.kick, random);
  clap(mix, at(button), LEVEL.clap, CLAP_PAN, random);
  stab(mix, at(button), home.triad.map((n) => n + STAB_OCTAVES + 12), LEVEL.stab * 1.6);
  bass(mix, at(button), home.root, at(button + CADENCE.buttonLength) - at(button), LEVEL.bass);
}

/** The music, `seconds` long, on `grid`. */
export function score(grid, seconds, random) {
  const mix = new Mix(seconds);
  const arps = new Mix(seconds);
  const parts = { mix, arps, at: grid.timeAt, random };
  let bar = 0;
  for (const section of sections(grid)) {
    if (section.kind === "cadence") {
      cadence(parts, section.from);
      continue;
    }
    if (section.kind !== "intro") crash(mix, grid.timeAt(section.from), LEVEL.crash, random);
    if (section.accent !== null) {
      crash(mix, grid.timeAt(section.accent), LEVEL.crash, random);
      clap(mix, grid.timeAt(section.accent), LEVEL.clap, CLAP_PAN, random);
    }
    const length = section.to - section.from;
    // The intro's bars count back from its end, so its short bar is the pickup and its last bar leads into the groove.
    const offset = section.kind === "intro" ? (BAR - (length % BAR)) % BAR : 0;
    const intro = section.kind === "intro" ? (beat) => ARP_BRIGHT.intro[0] + ((ARP_BRIGHT.intro[1] - ARP_BRIGHT.intro[0]) * beat) / length : null;
    for (let beat = section.from; beat < section.to; beat += 1) {
      const position = beat - section.from + offset;
      if (position % BAR === 0) bar += 1;
      const degree = section.kind === "build" ? DOMINANT : PROGRESSION[bar % PROGRESSION.length];
      const last = beat === section.to - 1;
      playBeat(parts, { kind: section.kind, beat, position, last, harmony: chordOn(degree), intro });
      if (last && section.kind !== "intro") fill(parts, beat);
    }
    riser(mix, grid.timeAt(section.to - RISER_BEATS), grid.timeAt(section.to) - grid.timeAt(section.to - RISER_BEATS), LEVEL.riser, random);
  }
  arps.echo(ECHO.seconds, ECHO.feedback, ECHO.wet);
  mix.add(arps);
  return mix;
}
