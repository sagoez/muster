// Fits the user's track to the picture. It plays from near its start, so its
// own intro opens the video, and at the install line it cuts to the track's
// ending so that last hit lands with the line. Where a camera move would
// fall too far off the beat, the track is cut at that move to its nearest
// downbeat (less than half a bar away), so the move lands on a bar line.
// Which moves get such a cut is chosen to keep the moves on the beat with as
// few cuts as possible.
import { Mix, RATE, ramp } from "./dsp.mjs";

/** How far into the track the video may start, in seconds; below zero the track starts after the video does. */
const START_RANGE = [-1, 1];
const START_STEP = 0.001;
/** What a cut costs, in squared seconds of off-beat error it must save to be worth making. */
const CUT_COST = 0.004;
/** The fewest bars between two cuts. */
const CUT_GAP_BARS = 1;
/** The most moves the search tries every combination of cuts for. */
const MAX_SEARCHED_MOVES = 16;
/** How long the outgoing part fades over after a cut, and how early the incoming part fades in before it. */
const CUT_FADE_OUT_S = 0.025;
const CUT_FADE_IN_S = 0.005;

/** How far `at` seconds of track lies from its nearest beat on `grid`, signed. */
function offBeat(grid, at) {
  const beats = (at - grid.downbeat) / grid.beat;
  return (beats - Math.round(beats)) * grid.beat;
}

/** The downbeat of `grid` nearest `at` seconds of track. */
function nearestDownbeat(grid, at) {
  return grid.downbeat + Math.round((at - grid.downbeat) / grid.bar) * grid.bar;
}

/** The squared off-beat error of `moves` (video seconds) with the track `offset` seconds ahead of the video. */
function cost(grid, moves, offset) {
  return moves.reduce((sum, at) => sum + offBeat(grid, at + offset) ** 2, 0);
}

/** The start offset that puts `moves` nearest the beat. */
function bestStart(grid, moves) {
  let best = { cost: Infinity, offset: 0 };
  for (let offset = START_RANGE[0]; offset <= START_RANGE[1]; offset += START_STEP) {
    const value = cost(grid, moves, offset);
    if (value < best.cost) best = { cost: value, offset };
  }
  return best;
}

/**
 * The parts the video plays, `{ from, to, offset }` in video seconds with the
 * track `offset` seconds ahead, and how each move lands: `{ at, error, cut }`.
 */
export function arrange(grid, moves, install, seconds) {
  if (moves.length > MAX_SEARCHED_MOVES) throw new Error(`too many camera moves to search: ${moves.length}`);
  let best = null;
  for (let mask = 0; mask < 1 << moves.length; mask += 1) {
    const cuts = moves.map((_, index) => index).filter((index) => mask & (1 << index));
    if (cuts.some((index, at) => at > 0 && moves[index] - moves[cuts[at - 1]] < CUT_GAP_BARS * grid.bar)) continue;
    const head = moves.slice(0, cuts[0] ?? moves.length);
    const start = bestStart(grid, head);
    let total = start.cost + CUT_COST * cuts.length;
    cuts.forEach((index, at) => {
      const inner = moves.slice(index + 1, cuts[at + 1] ?? moves.length);
      total += cost(grid, inner, grid.downbeat - moves[index]);
    });
    if (!best || total < best.total) best = { total, cuts, start: start.offset };
  }
  const parts = [{ from: 0, offset: best.start }];
  for (const index of best.cuts) {
    const at = moves[index];
    parts.push({ from: at, offset: nearestDownbeat(grid, at + parts.at(-1).offset) - at });
  }
  parts.push({ from: install, offset: grid.ending - install });
  parts.forEach((part, index) => (part.to = parts[index + 1]?.from ?? seconds));
  const landing = moves.map((at) => {
    const part = parts.findLast((candidate) => candidate.from <= at);
    return { at, error: offBeat(grid, at + part.offset), cut: best.cuts.includes(moves.indexOf(at)) };
  });
  return { parts, landing };
}

/** The track played as `parts` into a mix of `seconds`, each cut crossfaded so the incoming downbeat stays sharp. */
export function playTrack(track, parts, seconds) {
  const mix = new Mix(seconds);
  const length = track.left.length;
  parts.forEach((part, index) => {
    const first = index === 0;
    const last = index === parts.length - 1;
    const from = first ? part.from : part.from - CUT_FADE_IN_S;
    const to = last ? part.to : part.to + CUT_FADE_OUT_S;
    mix.render(from, to - from, 0, (sample) => {
      const t = from + sample / RATE;
      const source = Math.round((t + part.offset) * RATE);
      if (source < 0 || source >= length) return 0;
      const fadeIn = first ? 1 : ramp((t - from) / CUT_FADE_IN_S);
      const fadeOut = last ? 1 : 1 - ramp((t - part.to) / CUT_FADE_OUT_S);
      const gain = fadeIn * fadeOut;
      return [track.left[source] * gain, track.right[source] * gain];
    });
  });
  return mix;
}
