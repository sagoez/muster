// Finds a track's beat grid from its audio: an onset envelope (how sharply
// the level rises every 10 ms), the tempo and phase whose beats sit on the
// most onset, the downbeat as the beat of the bar with the most low-end
// onset (where the kick lands), and the ending as the downbeat of the last bar
// that still plays in full.
import { RATE, TAU } from "./dsp.mjs";

/** The envelope's step, in seconds. */
const HOP_S = 0.01;
/** The tempo range searched, in beats per minute, and its step. */
const BPM_MIN = 90;
const BPM_MAX = 180;
const BPM_STEP = 0.01;
/** Below this the low band holds the kick. */
const LOW_HZ = 150;
/** Beats in a bar. */
const BAR = 4;
/** How near a downbeat the low-end onset counts toward it, in hops. */
const DOWNBEAT_REACH = 3;
/** A bar still plays in full when its energy is within this many decibels of the median bar's. */
const FULL_BAR_DB = 6;
const SECONDS_PER_MINUTE = 60;

/** The positive change of log energy per hop of `mono`, after a one-pole low-pass at `cutoff` if given. */
function onsets(mono, cutoff) {
  const hop = Math.round(HOP_S * RATE);
  const frames = Math.floor(mono.length / hop);
  const a = cutoff ? 1 - Math.exp((-TAU * cutoff) / RATE) : 1;
  const flux = new Float64Array(frames);
  let state = 0;
  let previous = null;
  for (let frame = 0; frame < frames; frame += 1) {
    let energy = 0;
    for (let index = frame * hop; index < (frame + 1) * hop; index += 1) {
      state += a * (mono[index] - state);
      energy += state * state;
    }
    const level = Math.log(energy + Number.EPSILON);
    if (previous !== null) flux[frame] = Math.max(0, level - previous);
    previous = level;
  }
  return flux;
}

/** The mean of `flux` at every beat `period` hops apart from hop `phase`. */
function comb(flux, period, phase) {
  let sum = 0;
  let count = 0;
  for (let position = phase; position < flux.length; position += period) {
    sum += flux[Math.round(position)] ?? 0;
    count += 1;
  }
  return sum / count;
}

/**
 * The beat grid of `track` (`{ left, right }`): `beat` seconds per beat,
 * `downbeat` the first downbeat in seconds, `bar` seconds per bar, and
 * `ending` the downbeat of the last bar that still plays in full.
 */
export function beatGrid(track) {
  const mono = Float64Array.from(track.left, (value, index) => (value + track.right[index]) / 2);
  const flux = onsets(mono);
  let best = { score: -Infinity, period: 0, phase: 0 };
  for (let bpm = BPM_MIN; bpm <= BPM_MAX; bpm += BPM_STEP) {
    const period = SECONDS_PER_MINUTE / bpm / HOP_S;
    for (let phase = 0; phase < period; phase += 1) {
      const score = comb(flux, period, phase);
      if (score > best.score) best = { score, period, phase };
    }
  }
  const low = onsets(mono, LOW_HZ);
  const near = (hop) => {
    let sum = 0;
    for (let offset = -DOWNBEAT_REACH; offset <= DOWNBEAT_REACH; offset += 1) sum += low[Math.round(hop) + offset] ?? 0;
    return sum;
  };
  const weights = new Float64Array(BAR);
  for (let beat = 0; best.phase + beat * best.period < low.length; beat += 1) weights[beat % BAR] += near(best.phase + beat * best.period);
  const first = weights.indexOf(Math.max(...weights));
  const barHops = best.period * BAR;
  const downbeats = [];
  for (let hop = best.phase + first * best.period; hop < low.length; hop += barHops) downbeats.push(hop);
  const hop = Math.round(HOP_S * RATE);
  const energies = downbeats.map((at) => {
    const from = Math.round(at) * hop;
    const to = Math.min(mono.length, Math.round(at + barHops) * hop);
    let sum = 0;
    for (let index = from; index < to; index += 1) sum += mono[index] * mono[index];
    return 10 * Math.log10(sum / Math.max(1, to - from) + Number.EPSILON);
  });
  const median = [...energies].sort((a, b) => a - b)[Math.floor(energies.length / 2)];
  const last = energies.findLastIndex((energy) => energy >= median - FULL_BAR_DB);
  return {
    beat: best.period * HOP_S,
    bar: barHops * HOP_S,
    downbeat: downbeats[0] * HOP_S,
    ending: downbeats[last] * HOP_S,
  };
}
