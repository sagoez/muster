// The beat grid the music plays on, fitted to the picture: every cut lands on
// a beat (or, where no tempo in range can reach it, on the eighth between
// two), and the tempo glides between cuts rather than jumping.

/** The tempo range the music may take, in beats per minute, and the one it leans toward. */
const TEMPO_MIN = 112;
const TEMPO_MAX = 124.5;
const TEMPO_TARGET = 118;
/** What the fit pays per BPM squared of change between neighbouring spans, and of distance from the target. */
const CHANGE_WEIGHT = 1;
const TARGET_WEIGHT = 0.2;
/** What the fit pays for a cut that lands between two beats. */
const OFFBEAT_PENALTY = 400;
/** Seconds in a minute, and eighths in a beat. */
const SECONDS_PER_MINUTE = 60;
const EIGHTHS = 2;

/** The eighths counts a span of `seconds` may take, each with its tempo; the nearest if none is in range. */
function choices(seconds) {
  const tempo = (eighths) => (eighths * SECONDS_PER_MINUTE) / (EIGHTHS * seconds);
  const list = [];
  const lowest = Math.ceil((TEMPO_MIN * EIGHTHS * seconds) / SECONDS_PER_MINUTE);
  const highest = Math.floor((TEMPO_MAX * EIGHTHS * seconds) / SECONDS_PER_MINUTE);
  for (let eighths = Math.max(1, lowest); eighths <= highest; eighths += 1) list.push({ eighths, tempo: tempo(eighths) });
  if (!list.length) {
    const eighths = Math.max(1, Math.round((TEMPO_TARGET * EIGHTHS * seconds) / SECONDS_PER_MINUTE));
    list.push({ eighths, tempo: tempo(eighths) });
  }
  return list;
}

/** The eighths in each span between `times`, fitted by dynamic programming; the last time always lands on a beat. */
function fit(times) {
  const spans = times.slice(1).map((time, index) => choices(time - times[index]));
  let states = new Map([["start:0", { cost: 0, tempo: null, path: [] }]]);
  spans.forEach((options, index) => {
    const next = new Map();
    const last = index === spans.length - 1;
    for (const [key, state] of states) {
      const parity = Number(key.split(":")[1]);
      for (const option of options) {
        const landing = (parity + option.eighths) % EIGHTHS;
        if (last && landing) continue;
        const change = state.tempo === null ? 0 : (option.tempo - state.tempo) ** 2;
        const cost =
          state.cost + CHANGE_WEIGHT * change + TARGET_WEIGHT * (option.tempo - TEMPO_TARGET) ** 2 + (landing ? OFFBEAT_PENALTY : 0);
        const nextKey = `${option.eighths}:${landing}`;
        if (!next.has(nextKey) || next.get(nextKey).cost > cost) {
          next.set(nextKey, { cost, tempo: option.tempo, path: [...state.path, option.eighths] });
        }
      }
    }
    states = next;
  });
  const best = [...states.values()].sort((a, b) => a.cost - b.cost)[0];
  if (!best) throw new Error("no beat grid fits the cuts");
  return best.path;
}

/** A monotone cubic (Fritsch-Carlson) through (`xs`, `ys`), extended linearly past both ends. */
function monotone(xs, ys) {
  const n = xs.length;
  const slopes = xs.slice(1).map((x, index) => (ys[index + 1] - ys[index]) / (x - xs[index]));
  const tangents = xs.map((_, index) => {
    if (index === 0) return slopes[0];
    if (index === n - 1) return slopes[n - 2];
    const [a, b] = [slopes[index - 1], slopes[index]];
    return a * b <= 0 ? 0 : (2 * a * b) / (a + b);
  });
  return (x) => {
    if (x <= xs[0]) return ys[0] + (x - xs[0]) * tangents[0];
    if (x >= xs[n - 1]) return ys[n - 1] + (x - xs[n - 1]) * tangents[n - 1];
    let index = 0;
    while (xs[index + 1] < x) index += 1;
    const h = xs[index + 1] - xs[index];
    const t = (x - xs[index]) / h;
    const t2 = t * t;
    const t3 = t2 * t;
    return (
      (2 * t3 - 3 * t2 + 1) * ys[index] +
      (t3 - 2 * t2 + t) * h * tangents[index] +
      (-2 * t3 + 3 * t2) * ys[index + 1] +
      (t3 - t2) * h * tangents[index + 1]
    );
  };
}

/**
 * The grid through `times` (video seconds, the first the music's start, the
 * last the final cadence): `timeAt(beat)` the second a beat falls on, and
 * each time's beat, a whole number or a half.
 */
export function beatGrid(times) {
  const eighths = fit(times);
  const counts = [0];
  for (const count of eighths) counts.push(counts.at(-1) + count);
  const timeOfEighth = monotone(counts, times);
  return {
    timeAt: (beat) => timeOfEighth(beat * EIGHTHS),
    beats: counts.map((count) => count / EIGHTHS),
    tempos: eighths.map((count, index) => (count * SECONDS_PER_MINUTE) / (EIGHTHS * (times[index + 1] - times[index]))),
  };
}
