// Synthesizes the demo's soundtrack from the same timeline and edit the
// camera uses: a warm pad under the whole video, and a soft cue on each event
// on the frame its picture shows. Every sound is made here, from sines and
// seeded noise; no samples, no audio library. It writes a stereo 48 kHz WAV
// normalized to the loudness and peak below.
//
// Usage: node soundtrack.mjs <public-dir>
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { SOUNDTRACK, bodyWindow, cameraAt, outroFrom, shots, totalFrames, videoFrameOf } from "./src/edit.js";
import { FPS, INSTALL_IN, MS_PER_SECOND } from "./src/theme.js";

/** The output's sample rate and channel count. */
const RATE = 48000;
const CHANNELS = 2;
/** The integrated loudness the mix is brought to, in LUFS, and the sample peak it may reach, in dBFS. */
const TARGET_LUFS = -16;
const PEAK_CEILING_DB = -2;
/** How many measure-and-adjust passes the normalization may take, and how close it must land, in LU. */
const NORMALIZE_PASSES = 6;
const NORMALIZE_TOLERANCE = 0.05;
/** The limiter's lookahead, which is also its attack and release, in seconds. */
const LIMITER_LOOKAHEAD_S = 0.004;
/** The fade in at the start and out at the end, in seconds. */
const FADE_IN_S = 1.6;
const FADE_OUT_S = 1.4;

/** The pad's chords as MIDI notes, played in turn and ending on the dominant before the outro resolves. */
const PROGRESSION = [
  [38, 45, 53, 60, 64], // D minor 9
  [46, 50, 57, 60, 65], // B flat major 9
  [43, 50, 53, 57, 58], // G minor 9
  [45, 52, 57, 62, 64], // A sus 4
];
/** The chord the outro resolves to: D major 9. */
const RESOLUTION = [38, 45, 54, 57, 64, 69];
/** How long each pad chord lasts and how long consecutive chords overlap, in seconds. */
const CHORD_S = 6.4;
const CHORD_XFADE_S = 2.6;
/** The pad's level, its detune between the two voices of a note in cents, and how much its triangle adds to the sine. */
const PAD_GAIN = 0.07;
const PAD_DETUNE_CENTS = 6;
const PAD_TRIANGLE = 0.35;
/** The odd harmonics the pad's band-limited triangle sums. */
const TRIANGLE_HARMONICS = [1, 3, 5, 7];
/** The pad's slow swell: its period range in seconds and its depth. */
const SWELL_PERIOD_S = [6.5, 11];
const SWELL_DEPTH = 0.25;
/** The pad's low-pass: its centre cutoff, how far it sweeps and the sweep's period. */
const PAD_CUTOFF_HZ = 900;
const PAD_CUTOFF_SWING_HZ = 300;
const PAD_CUTOFF_PERIOD_S = 13;
/** How far the pad ducks under a cue, and the duck's attack and release in seconds. */
const DUCK_DEPTH = 0.3;
const DUCK_ATTACK_S = 0.06;
const DUCK_RELEASE_S = 1.1;

/** Pins closer than this to the first of a run share one chime, so a stack coming up is one sound. */
const PIN_CLUSTER_MS = 500;
/** The ok chime: two rising notes, the second this many seconds after the first. */
const CHIME_NOTES = [81, 86];
const CHIME_STEP_S = 0.095;
const CHIME_GAIN = 0.16;
const CHIME_PAN = 0.2;
/** The fail tone: a low note and the semitone above it, muted. */
const FAIL_NOTES = [50, 51, 38];
const FAIL_GAINS = [0.26, 0.15, 0.12];
const FAIL_DECAY_S = 0.7;
const FAIL_ATTACK_S = 0.03;
/** The restart's "again" figure: a soft broken chord rising back up. */
const AGAIN_NOTES = [74, 81, 86];
const AGAIN_STEP_S = 0.12;
const AGAIN_GAIN = 0.12;
const AGAIN_PAN = 0.1;
/** A bell note's partials as [ratio, level, decay scale], its attack and its base decay in seconds. */
const BELL_PARTIALS = [
  [1, 1, 1],
  [2, 0.22, 0.55],
  [3.01, 0.06, 0.3],
];
const BELL_ATTACK_S = 0.005;
const BELL_DECAY_S = 0.9;
/** The camera whoosh: its longest length, level, band sweep for a push (reversed for a pull) and the band's Q. */
const WHOOSH_MAX_S = 1.8;
const WHOOSH_GAIN = 0.11;
const WHOOSH_SWEEP_HZ = [320, 1500];
const WHOOSH_PAN_HZ = [500, 900];
const WHOOSH_Q = 1.3;
/** The low-pass after the whoosh's band, which keeps hiss out of it. */
const WHOOSH_CUTOFF_HZ = 2400;
/** How much the zoom must change for a move to count as a push or a pull rather than a pan. */
const ZOOM_EPSILON = 0.05;
/** The typing tick: a short noise burst through a high band with a faint tone in it, on the terminal's side. */
const TICK_S = 0.03;
const TICK_GAIN = 0.07;
const TICK_BAND_HZ = 3600;
const TICK_Q = 2.5;
const TICK_TONE_HZ = 2200;
const TICK_TONE_GAIN = 0.25;
const TICK_DECAY_S = 0.008;
const TICK_PAN = -0.35;
/** The outro chord: its level, attack and decay, and the bell notes over it. */
const OUTRO_GAIN = 0.075;
const OUTRO_ATTACK_S = 0.25;
const OUTRO_DECAY_S = 2.6;
const OUTRO_BELLS = [81, 86];
const OUTRO_BELL_GAIN = 0.1;
/** The seed of the noise, so a render is the same every time. */
const NOISE_SEED = 0x6d757374;

/** MIDI note 69, A4, and its frequency. */
const A4_NOTE = 69;
const A4_HZ = 440;
/** The K-weighting filters of ITU-R BS.1770 at 48 kHz, as [b0, b1, b2, a1, a2]. */
const K_SHELF = [1.53512485958697, -2.69169618940638, 1.19839281085285, -1.69065929318241, 0.73248077421585];
const K_HIGHPASS = [1, -2, 1, -1.99004745483398, 0.99007225036621];
/** BS.1770's gating blocks, their step, its absolute gate and its relative gate below the ungated level. */
const GATE_BLOCK_S = 0.4;
const GATE_STEP_S = 0.1;
const ABSOLUTE_GATE_LUFS = -70;
const RELATIVE_GATE_LU = -10;
const LOUDNESS_OFFSET = -0.691;
/** The 16-bit PCM full scale the WAV is written at. */
const PCM_FULL_SCALE = 32767;
const BITS_PER_SAMPLE = 16;

const TAU = 2 * Math.PI;

/** The frequency of MIDI note `note`. */
function hz(note) {
  return A4_HZ * 2 ** ((note - A4_NOTE) / 12);
}

/** Decibels as a linear gain. */
function gainOf(db) {
  return 10 ** (db / 20);
}

/** The left and right gains of a constant-power pan, `pan` from -1 (left) to 1 (right). */
function panGains(pan) {
  const angle = ((pan + 1) * Math.PI) / 4;
  return [Math.cos(angle), Math.sin(angle)];
}

/** A seeded uniform noise source in [-1, 1) (mulberry32). */
function noise(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return (((t ^ (t >>> 14)) >>> 0) / 2 ** 32) * 2 - 1;
  };
}

/** A raised-cosine ramp: 0 at or before 0, 1 at or after 1. */
function ramp(x) {
  if (x <= 0) return 0;
  if (x >= 1) return 1;
  return 0.5 - 0.5 * Math.cos(Math.PI * x);
}

/** A state-variable band-pass, retuned per sample: returns `step(input, centreHz)`. */
function bandpass(q) {
  let low = 0;
  let band = 0;
  return (input, centre) => {
    const f = 2 * Math.sin((Math.PI * Math.min(centre, RATE / 6)) / RATE);
    low += f * band;
    const high = input - low - band / q;
    band += f * high;
    return band;
  };
}

/** A two-pole low-pass at `cutoff`: returns `step(input)`. */
function lowpass(cutoff) {
  const a = 1 - Math.exp((-TAU * cutoff) / RATE);
  let first = 0;
  let second = 0;
  return (input) => {
    first += a * (input - first);
    second += a * (first - second);
    return second;
  };
}

/** The stereo mix under construction, `seconds` long. */
class Mix {
  /** An empty mix of `seconds`. */
  constructor(seconds) {
    this.length = Math.ceil(seconds * RATE);
    this.left = new Float64Array(this.length);
    this.right = new Float64Array(this.length);
  }

  /** Adds `sample(i)` for `count` samples from second `at`, panned to `pan`; `sample` may return [left, right]. */
  render(at, seconds, pan, sample) {
    const from = Math.max(0, Math.round(at * RATE));
    const to = Math.min(this.length, Math.round((at + seconds) * RATE));
    const [gl, gr] = panGains(pan);
    for (let index = from; index < to; index += 1) {
      const value = sample(index - from);
      if (Array.isArray(value)) {
        this.left[index] += value[0];
        this.right[index] += value[1];
      } else {
        this.left[index] += value * gl;
        this.right[index] += value * gr;
      }
    }
  }

  /** Adds `other`, each sample scaled by `gain(index)`. */
  addScaled(other, gain) {
    for (let index = 0; index < this.length; index += 1) {
      const g = gain(index);
      this.left[index] += other.left[index] * g;
      this.right[index] += other.right[index] * g;
    }
  }

  /** Scales every sample by `gain(index)`. */
  scale(gain) {
    for (let index = 0; index < this.length; index += 1) {
      const g = gain(index);
      this.left[index] *= g;
      this.right[index] *= g;
    }
  }
}

/** One bell note of `note` at second `at`, `gain` loud and panned to `pan`, decaying over about `decay` seconds. */
function bell(mix, at, note, gain, pan, decay = BELL_DECAY_S) {
  const base = hz(note);
  const seconds = decay * 6;
  mix.render(at, seconds, pan, (index) => {
    const t = index / RATE;
    const attack = ramp(t / BELL_ATTACK_S);
    let value = 0;
    for (const [ratio, level, decayScale] of BELL_PARTIALS) {
      value += level * Math.sin(TAU * base * ratio * t) * Math.exp(-t / (decay * decayScale));
    }
    return gain * attack * value;
  });
}

/** The pad: each chord's detuned sine and triangle voices, swelling slowly, crossfading into the next. */
function pad(seconds, schedule) {
  const mix = new Mix(seconds);
  for (const { from, to, notes } of schedule) {
    const start = Math.max(0, from - CHORD_XFADE_S / 2);
    const end = Math.min(seconds, to + CHORD_XFADE_S / 2);
    notes.forEach((note, voice) => {
      const level = 1 / (1 + voice * 0.18);
      const swell = SWELL_PERIOD_S[0] + ((SWELL_PERIOD_S[1] - SWELL_PERIOD_S[0]) * voice) / notes.length;
      const phase = voice * 1.7;
      for (const side of [-1, 1]) {
        const f = hz(note) * 2 ** ((side * PAD_DETUNE_CENTS) / 1200);
        mix.render(start, end - start, side * (0.25 + 0.1 * voice), (index) => {
          const t = start + index / RATE;
          const envelope = ramp((t - (from - CHORD_XFADE_S / 2)) / CHORD_XFADE_S) * (1 - ramp((t - (to - CHORD_XFADE_S / 2)) / CHORD_XFADE_S));
          const breathe = 1 - SWELL_DEPTH + SWELL_DEPTH * Math.sin((TAU * t) / swell + phase);
          let triangle = 0;
          for (const harmonic of TRIANGLE_HARMONICS) {
            const sign = ((harmonic - 1) / 2) % 2 === 0 ? 1 : -1;
            triangle += (sign * Math.sin(TAU * f * harmonic * t)) / (harmonic * harmonic);
          }
          const tone = Math.sin(TAU * f * t) + PAD_TRIANGLE * triangle;
          return PAD_GAIN * level * envelope * breathe * tone;
        });
      }
    });
  }
  for (const channel of [mix.left, mix.right]) {
    let first = 0;
    let second = 0;
    for (let index = 0; index < mix.length; index += 1) {
      const cutoff = PAD_CUTOFF_HZ + PAD_CUTOFF_SWING_HZ * Math.sin((TAU * index) / RATE / PAD_CUTOFF_PERIOD_S);
      const a = 1 - Math.exp((-TAU * cutoff) / RATE);
      first += a * (channel[index] - first);
      second += a * (first - second);
      channel[index] = second;
    }
  }
  return mix;
}

/** The pad's chords as `{ from, to, notes }` seconds, counted back from `resolveAt` so the dominant falls just before it. */
function chordSchedule(resolveAt, seconds) {
  const count = Math.ceil(resolveAt / CHORD_S);
  const list = [];
  for (let index = 0; index < count; index += 1) {
    const chord = (((index - count) % PROGRESSION.length) + PROGRESSION.length) % PROGRESSION.length;
    list.push({ from: resolveAt - (count - index) * CHORD_S, to: resolveAt - (count - index - 1) * CHORD_S, notes: PROGRESSION[chord] });
  }
  list.push({ from: resolveAt, to: seconds + CHORD_XFADE_S, notes: RESOLUTION });
  return list;
}

/** A filtered-noise whoosh from `at` for `seconds`, its band rising for a push, falling for a pull, arching for a pan. */
function whoosh(mix, at, seconds, direction, random) {
  const [low, high] = direction === 0 ? WHOOSH_PAN_HZ : WHOOSH_SWEEP_HZ;
  const filters = [bandpass(WHOOSH_Q), bandpass(WHOOSH_Q)];
  const smooth = [lowpass(WHOOSH_CUTOFF_HZ), lowpass(WHOOSH_CUTOFF_HZ)];
  mix.render(at, seconds, 0, (index) => {
    const x = index / (seconds * RATE);
    const sweep = direction === 0 ? Math.sin(Math.PI * x) : direction > 0 ? x : 1 - x;
    const centre = low * (high / low) ** sweep;
    const envelope = Math.sin(Math.PI * x) ** 2;
    const drift = Math.sin(Math.PI * x) * 0.3 * (direction || 1);
    const [gl, gr] = panGains(drift);
    const [left, right] = [0, 1].map((side) => smooth[side](filters[side](random(), centre)));
    return [WHOOSH_GAIN * envelope * gl * left, WHOOSH_GAIN * envelope * gr * right];
  });
}

/** A soft key tick at `at`. */
function tick(mix, at, random) {
  const filter = bandpass(TICK_Q);
  mix.render(at, TICK_S, TICK_PAN, (index) => {
    const t = index / RATE;
    const decay = Math.exp(-t / TICK_DECAY_S);
    return TICK_GAIN * decay * (filter(random(), TICK_BAND_HZ) + TICK_TONE_GAIN * Math.sin(TAU * TICK_TONE_HZ * t));
  });
}

/** The low, muted, slightly dissonant tone of a failure at `at`. */
function failTone(mix, at) {
  const seconds = FAIL_DECAY_S * 6;
  FAIL_NOTES.forEach((note, index) => {
    const f = hz(note);
    mix.render(at, seconds, 0, (sample) => {
      const t = sample / RATE;
      const envelope = ramp(t / FAIL_ATTACK_S) * Math.exp(-t / FAIL_DECAY_S);
      const tone = Math.sin(TAU * f * t) + 0.3 * Math.sin(TAU * 2 * f * t) * Math.exp(-t / (FAIL_DECAY_S / 3));
      return FAIL_GAINS[index] * envelope * tone;
    });
  });
}

/** The warm resolving chord at `at`, with two bell notes over it. */
function outroChord(mix, at, seconds) {
  RESOLUTION.forEach((note, index) => {
    const f = hz(note);
    mix.render(at, seconds - at, (index / (RESOLUTION.length - 1)) * 0.8 - 0.4, (sample) => {
      const t = sample / RATE;
      const envelope = ramp(t / OUTRO_ATTACK_S) * Math.exp(-t / OUTRO_DECAY_S);
      return OUTRO_GAIN * envelope * (Math.sin(TAU * f * t) + 0.2 * Math.sin(TAU * 2 * f * t));
    });
  });
  OUTRO_BELLS.forEach((note, index) => bell(mix, at + index * CHIME_STEP_S * 1.5, note, OUTRO_BELL_GAIN, CHIME_PAN, BELL_DECAY_S * 2));
}

/** The video second frame `frame` starts at. */
function secondsOfFrame(frame) {
  return frame / FPS;
}

/**
 * The cues, each `{ kind, at }` in video seconds: ok, fail and restart pins
 * (an ok after a fail of the same process is its restart), camera moves with
 * their direction and length, entered commands, and the outro's install line.
 */
function cues(timeline) {
  const { start } = bodyWindow(timeline);
  const end = timeline.terminal.endMs;
  const inBody = (at) => at >= start && at <= end;
  const at = (pageMs) => secondsOfFrame(videoFrameOf(timeline, pageMs));
  const list = [];
  const failed = new Set();
  let runStart = -Infinity;
  for (const event of timeline.events) {
    if (event.kind !== "pin" || !inBody(event.at)) continue;
    const key = `${event.project}/${event.name}`;
    if (event.pin === "fail") {
      failed.add(key);
      list.push({ kind: "fail", at: at(event.at) });
    } else if (event.pin === "ok" && failed.has(key)) {
      failed.delete(key);
      list.push({ kind: "again", at: at(event.at) });
    } else if (event.pin === "ok" && event.at - runStart > PIN_CLUSTER_MS) {
      runStart = event.at;
      list.push({ kind: "ok", at: at(event.at) });
    }
  }
  const camera = shots(timeline);
  camera.forEach((shot, index) => {
    if (index === 0 || shot.move === 0 || !inBody(shot.at)) return;
    const before = cameraAt(camera, shot.at - 1).z;
    const after = cameraAt(camera, shot.at + shot.move).z;
    const direction = Math.abs(after - before) < ZOOM_EPSILON ? 0 : Math.sign(after - before);
    list.push({ kind: "move", at: at(shot.at), seconds: Math.min(WHOOSH_MAX_S, shot.move / MS_PER_SECOND), direction });
  });
  for (const command of timeline.commands) {
    if (inBody(command.at)) list.push({ kind: "tick", at: at(command.at) });
  }
  list.push({ kind: "outro", at: secondsOfFrame(outroFrom(timeline) + INSTALL_IN[0]) });
  return list.sort((a, b) => a.at - b.at);
}

/** How far the pad is ducked at second `t` under `list`'s cues, 0 to `DUCK_DEPTH`. */
function duckAt(list, t) {
  let duck = 0;
  for (const cue of list) {
    if (cue.kind === "move" || cue.kind === "tick") continue;
    const since = t - cue.at;
    if (since < -DUCK_ATTACK_S || since > DUCK_RELEASE_S * 4) continue;
    const shape = since < 0 ? ramp(1 + since / DUCK_ATTACK_S) : Math.exp(-since / DUCK_RELEASE_S);
    duck = Math.max(duck, shape);
  }
  return DUCK_DEPTH * duck;
}

/** One biquad pass of `coefficients` over `channel`. */
function biquad(channel, [b0, b1, b2, a1, a2]) {
  const out = new Float64Array(channel.length);
  let x1 = 0;
  let x2 = 0;
  let y1 = 0;
  let y2 = 0;
  for (let index = 0; index < channel.length; index += 1) {
    const x = channel[index];
    const y = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
    x2 = x1;
    x1 = x;
    y2 = y1;
    y1 = y;
    out[index] = y;
  }
  return out;
}

/** The mix's integrated loudness in LUFS, gated as BS.1770 gates it. */
function loudness(mix) {
  const weighted = [mix.left, mix.right].map((channel) => biquad(biquad(channel, K_SHELF), K_HIGHPASS));
  const block = Math.round(GATE_BLOCK_S * RATE);
  const step = Math.round(GATE_STEP_S * RATE);
  const powers = [];
  for (let from = 0; from + block <= mix.length; from += step) {
    let power = 0;
    for (const channel of weighted) {
      let sum = 0;
      for (let index = from; index < from + block; index += 1) sum += channel[index] * channel[index];
      power += sum / block;
    }
    powers.push(power);
  }
  const lufs = (power) => LOUDNESS_OFFSET + 10 * Math.log10(power);
  const mean = (list) => list.reduce((sum, power) => sum + power, 0) / list.length;
  const audible = powers.filter((power) => lufs(power) > ABSOLUTE_GATE_LUFS);
  const relative = lufs(mean(audible)) + RELATIVE_GATE_LU;
  return lufs(mean(audible.filter((power) => lufs(power) > relative)));
}

/** Pulls every peak under `ceiling` with a lookahead limiter whose gain never jumps. */
function limit(mix, ceiling) {
  const reach = Math.round(LIMITER_LOOKAHEAD_S * RATE);
  const needed = new Float64Array(mix.length);
  for (let index = 0; index < mix.length; index += 1) {
    const peak = Math.max(Math.abs(mix.left[index]), Math.abs(mix.right[index]));
    needed[index] = peak > ceiling ? ceiling / peak : 1;
  }
  // The smallest gain needed within reach of each sample, then averaged over the same reach: every sample in a peak's window holds no more gain than the peak needs, so their mean holds no more either.
  const floor = new Float64Array(mix.length);
  const window = [];
  for (let index = 0; index < mix.length + reach; index += 1) {
    if (index < mix.length) {
      while (window.length && needed[window[window.length - 1]] >= needed[index]) window.pop();
      window.push(index);
    }
    const centre = index - reach;
    if (centre < 0) continue;
    while (window[0] < centre - reach) window.shift();
    floor[centre] = needed[window[0]];
  }
  const gain = new Float64Array(mix.length);
  let sum = 0;
  for (let index = 0; index < mix.length + reach; index += 1) {
    if (index < mix.length) sum += floor[index];
    if (index - 2 * reach - 1 >= 0) sum -= floor[index - 2 * reach - 1];
    const centre = index - reach;
    if (centre >= 0) gain[centre] = Math.min(1, sum / (2 * reach + 1));
  }
  mix.scale((index) => Math.min(gain[index], needed[index]));
}

/** The mix's sample peak, linear. */
function peakOf(mix) {
  let peak = 0;
  for (let index = 0; index < mix.length; index += 1) peak = Math.max(peak, Math.abs(mix.left[index]), Math.abs(mix.right[index]));
  return peak;
}

/** Brings the mix to `TARGET_LUFS` with its peaks held under the ceiling, and returns its loudness and peak. */
function normalize(mix) {
  const ceiling = gainOf(PEAK_CEILING_DB);
  let measured = loudness(mix);
  for (let pass = 0; pass < NORMALIZE_PASSES && Math.abs(measured - TARGET_LUFS) > NORMALIZE_TOLERANCE; pass += 1) {
    const gain = gainOf(TARGET_LUFS - measured);
    mix.scale(() => gain);
    limit(mix, ceiling);
    measured = loudness(mix);
  }
  return { lufs: measured, peakDb: 20 * Math.log10(peakOf(mix)) };
}

/** The mix as a 16-bit PCM WAV, with triangular dither. */
function wav(mix, random) {
  const bytes = BITS_PER_SAMPLE / 8;
  const dataSize = mix.length * CHANNELS * bytes;
  const buffer = Buffer.alloc(44 + dataSize);
  buffer.write("RIFF", 0);
  buffer.writeUInt32LE(36 + dataSize, 4);
  buffer.write("WAVEfmt ", 8);
  buffer.writeUInt32LE(16, 16);
  buffer.writeUInt16LE(1, 20);
  buffer.writeUInt16LE(CHANNELS, 22);
  buffer.writeUInt32LE(RATE, 24);
  buffer.writeUInt32LE(RATE * CHANNELS * bytes, 28);
  buffer.writeUInt16LE(CHANNELS * bytes, 32);
  buffer.writeUInt16LE(BITS_PER_SAMPLE, 34);
  buffer.write("data", 36);
  buffer.writeUInt32LE(dataSize, 40);
  let offset = 44;
  for (let index = 0; index < mix.length; index += 1) {
    for (const channel of [mix.left, mix.right]) {
      const dither = (random() + random()) / 2;
      const value = Math.round(channel[index] * PCM_FULL_SCALE + dither);
      buffer.writeInt16LE(Math.max(-PCM_FULL_SCALE, Math.min(PCM_FULL_SCALE, value)), offset);
      offset += bytes;
    }
  }
  return buffer;
}

const publicDir = process.argv[2];
if (!publicDir) throw new Error("usage: node soundtrack.mjs <public-dir>");
const timeline = JSON.parse(readFileSync(join(publicDir, "timeline.json"), "utf8"));
const seconds = secondsOfFrame(totalFrames(timeline));
const list = cues(timeline);
const outroAt = list.find((cue) => cue.kind === "outro").at;
const random = noise(NOISE_SEED);

const mix = new Mix(seconds);
mix.addScaled(pad(seconds, chordSchedule(outroAt, seconds)), (index) => 1 - duckAt(list, index / RATE));
for (const cue of list) {
  if (cue.kind === "ok") CHIME_NOTES.forEach((note, step) => bell(mix, cue.at + step * CHIME_STEP_S, note, CHIME_GAIN, CHIME_PAN));
  if (cue.kind === "again") AGAIN_NOTES.forEach((note, step) => bell(mix, cue.at + step * AGAIN_STEP_S, note, AGAIN_GAIN, AGAIN_PAN));
  if (cue.kind === "fail") failTone(mix, cue.at);
  if (cue.kind === "move") whoosh(mix, cue.at, cue.seconds, cue.direction, random);
  if (cue.kind === "tick") tick(mix, cue.at, random);
  if (cue.kind === "outro") outroChord(mix, cue.at, seconds);
}
const fadeOutFrom = seconds - FADE_OUT_S;
mix.scale((index) => {
  const t = index / RATE;
  return ramp(t / FADE_IN_S) * (1 - ramp((t - fadeOutFrom) / FADE_OUT_S));
});

const { lufs, peakDb } = normalize(mix);
writeFileSync(join(publicDir, SOUNDTRACK), wav(mix, random));
const counts = list.reduce((all, cue) => ({ ...all, [cue.kind]: (all[cue.kind] ?? 0) + 1 }), {});
console.log(
  `soundtrack: ${seconds.toFixed(2)} s, ${lufs.toFixed(1)} LUFS, peak ${peakDb.toFixed(1)} dBFS, cues ${JSON.stringify(counts)}`,
);
for (const cue of list) console.log(`  ${cue.at.toFixed(3)} s ${cue.kind}`);
