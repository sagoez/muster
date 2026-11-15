// The master: the music ducked under the interface sounds and the typing, the fades, and
// the whole brought to its integrated loudness (ITU-R BS.1770) with a
// lookahead limiter holding every peak under the ceiling.
import { RATE, gainOf, ramp } from "./dsp.mjs";

/** The integrated loudness the mix is brought to, in LUFS, and the sample peak it may reach, in dBFS. */
const TARGET_LUFS = -16;
const PEAK_CEILING_DB = -2.5;
/** How many measure-and-adjust passes the normalization may take, and how close it must land, in LU. */
const NORMALIZE_PASSES = 8;
const NORMALIZE_TOLERANCE = 0.05;
/** The limiter's lookahead, which is also its attack and release, in seconds. */
const LIMITER_LOOKAHEAD_S = 0.004;
/** The duck's attack and release in seconds, how long a cue holds it before releasing, and how many releases its tail runs. */
const DUCK_ATTACK_S = 0.01;
const DUCK_RELEASE_S = 0.28;
const DUCK_HOLD_S = 0.12;
const DUCK_TAIL_RELEASES = 6;
/** The K-weighting filters of BS.1770 at 48 kHz, as [b0, b1, b2, a1, a2]. */
const K_SHELF = [1.53512485958697, -2.69169618940638, 1.19839281085285, -1.69065929318241, 0.73248077421585];
const K_HIGHPASS = [1, -2, 1, -1.99004745483398, 0.99007225036621];
/** BS.1770's gating blocks, their step, its absolute gate and its relative gate below the ungated level. */
const GATE_BLOCK_S = 0.4;
const GATE_STEP_S = 0.1;
const ABSOLUTE_GATE_LUFS = -70;
const RELATIVE_GATE_LU = -10;
const LOUDNESS_OFFSET = -0.691;

/** The music's gain at sample `index` of a mix `length` long, ducked under each of `cues` (`{ at, depth }`, seconds and a share). */
export function duck(cues, length) {
  const depth = new Float64Array(length);
  for (const cue of cues) {
    const from = Math.max(0, Math.round((cue.at - DUCK_ATTACK_S) * RATE));
    const to = Math.min(length, Math.round((cue.at + DUCK_HOLD_S + DUCK_TAIL_RELEASES * DUCK_RELEASE_S) * RATE));
    for (let index = from; index < to; index += 1) {
      const since = index / RATE - cue.at;
      const shape = since < 0 ? ramp(1 + since / DUCK_ATTACK_S) : since < DUCK_HOLD_S ? 1 : Math.exp(-(since - DUCK_HOLD_S) / DUCK_RELEASE_S);
      depth[index] = Math.max(depth[index], cue.depth * shape);
    }
  }
  return (index) => 1 - depth[index];
}

/** A gain that fades in over `fadeIn` seconds and out over the last `fadeOut` of `seconds`. */
export function fades(seconds, fadeIn, fadeOut) {
  return (index) => {
    const t = index / RATE;
    return ramp(t / fadeIn) * (1 - ramp((t - (seconds - fadeOut)) / fadeOut));
  };
}

/** One biquad pass of `coefficients` over `channel`. */
function biquad(channel, [b0, b1, b2, a1, a2]) {
  const out = new Float64Array(channel.length);
  let [x1, x2, y1, y2] = [0, 0, 0, 0];
  for (let index = 0; index < channel.length; index += 1) {
    const x = channel[index];
    const y = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
    [x2, x1, y2, y1] = [x1, x, y1, y];
    out[index] = y;
  }
  return out;
}

/** The mix's integrated loudness in LUFS, gated as BS.1770 gates it. */
export function loudness(mix) {
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
  let head = 0;
  for (let index = 0; index < mix.length + reach; index += 1) {
    if (index < mix.length) {
      while (window.length > head && needed[window[window.length - 1]] >= needed[index]) window.pop();
      window.push(index);
    }
    const centre = index - reach;
    if (centre < 0) continue;
    while (window[head] < centre - reach) head += 1;
    floor[centre] = needed[window[head]];
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
export function normalize(mix) {
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
