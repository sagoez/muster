// The signal building blocks every sound is made of: pitch and gain
// conversions, seeded noise, oscillators, filters, and the stereo buffer the
// parts render into.

/** The output's sample rate. */
export const RATE = 48000;
export const TAU = 2 * Math.PI;
/** MIDI note 69, A4, and its frequency. */
const A4_NOTE = 69;
const A4_HZ = 440;
/** The highest a Chamberlin state-variable filter stays stable at, as a share of the rate. */
const SVF_LIMIT = 1 / 6;

/** The frequency of MIDI note `note`. */
export function hz(note) {
  return A4_HZ * 2 ** ((note - A4_NOTE) / 12);
}

/** Decibels as a linear gain. */
export function gainOf(db) {
  return 10 ** (db / 20);
}

/** The left and right gains of a constant-power pan, `pan` from -1 (left) to 1 (right). */
export function panGains(pan) {
  const angle = ((Math.max(-1, Math.min(1, pan)) + 1) * Math.PI) / 4;
  return [Math.cos(angle), Math.sin(angle)];
}

/** A seeded uniform noise source in [-1, 1) (mulberry32), so a render is the same every time. */
export function noise(seed) {
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
export function ramp(x) {
  if (x <= 0) return 0;
  if (x >= 1) return 1;
  return 0.5 - 0.5 * Math.cos(Math.PI * x);
}

/** The polyBLEP correction that takes the alias out of a saw's jump, for phase `t` and step `dt`. */
function polyBlep(t, dt) {
  if (t < dt) {
    const x = t / dt;
    return x + x - x * x - 1;
  }
  if (t > 1 - dt) {
    const x = (t - 1) / dt;
    return x * x + x + x + 1;
  }
  return 0;
}

/** A band-limited saw at `freq`: returns `step()` giving the next sample in [-1, 1]. */
export function saw(freq) {
  const dt = freq / RATE;
  let phase = 0;
  return () => {
    phase += dt;
    if (phase >= 1) phase -= 1;
    return 2 * phase - 1 - polyBlep(phase, dt);
  };
}

/** A band-limited square at `freq`, from two saws half a cycle apart. */
export function square(freq) {
  const dt = freq / RATE;
  let phase = 0;
  return () => {
    phase += dt;
    if (phase >= 1) phase -= 1;
    const shifted = (phase + 0.5) % 1;
    return 2 * phase - 1 - polyBlep(phase, dt) - (2 * shifted - 1 - polyBlep(shifted, dt));
  };
}

/** A two-pole low-pass: returns `step(input, cutoff)`. */
export function lowpass() {
  let first = 0;
  let second = 0;
  return (input, cutoff) => {
    const a = 1 - Math.exp((-TAU * cutoff) / RATE);
    first += a * (input - first);
    second += a * (first - second);
    return second;
  };
}

/** A two-pole high-pass: the input less a two-pole low-pass of it. */
export function highpass() {
  const low = lowpass();
  return (input, cutoff) => input - low(input, cutoff);
}

/** A state-variable band-pass of quality `q`, retuned per sample: returns `step(input, centre)`. */
export function bandpass(q) {
  let low = 0;
  let band = 0;
  return (input, centre) => {
    const f = 2 * Math.sin(Math.PI * Math.min(centre / RATE, SVF_LIMIT));
    low += f * band;
    const high = input - low - band / q;
    band += f * high;
    return band;
  };
}

/** A stereo buffer `seconds` long that parts render into. */
export class Mix {
  /** An empty mix of `seconds`. */
  constructor(seconds) {
    this.length = Math.ceil(seconds * RATE);
    this.left = new Float64Array(this.length);
    this.right = new Float64Array(this.length);
  }

  /** Adds `sample(i)` for `seconds` from second `at`, panned to `pan`; `sample` may return [left, right] instead. */
  render(at, seconds, pan, sample) {
    const from = Math.round(at * RATE);
    const to = Math.min(this.length, Math.round((at + seconds) * RATE));
    const [gl, gr] = panGains(pan);
    for (let index = Math.max(0, from); index < to; index += 1) {
      const value = sample(index - from);
      if (typeof value === "number") {
        this.left[index] += value * gl;
        this.right[index] += value * gr;
      } else {
        this.left[index] += value[0];
        this.right[index] += value[1];
      }
    }
  }

  /** Adds `other`, each sample scaled by `gain(index)`. */
  add(other, gain = () => 1) {
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

  /** Adds a ping-pong echo of itself: `delay` seconds per bounce, `feedback` per bounce, `wet` overall. */
  echo(delay, feedback, wet) {
    const step = Math.round(delay * RATE);
    const left = new Float64Array(this.length);
    const right = new Float64Array(this.length);
    for (let index = step; index < this.length; index += 1) {
      left[index] = (this.right[index - step] + right[index - step]) * feedback;
      right[index] = (this.left[index - step] + left[index - step]) * feedback;
    }
    for (let index = 0; index < this.length; index += 1) {
      this.left[index] += (wet * left[index]) / feedback;
      this.right[index] += (wet * right[index]) / feedback;
    }
  }
}
