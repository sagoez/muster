// Renders the demo's soundtrack from the same timeline and edit the camera
// uses: the music on a beat grid fitted to the cuts, a key click for every
// typed character, and an interface sound on each event's frame, mixed and
// written as a stereo 48 kHz WAV. Every sound is synthesized; no samples, no
// audio library.
//
// Usage: node src/sound/render.mjs <public-dir>
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { FPS, INSTALL_IN, MS_PER_SECOND } from "../brand/theme.js";
import { cameraAt, shots } from "../timing/camera.js";
import { TIMELINE } from "../timing/recording.js";
import { SOUNDTRACK, bodyWindow, outroFrom, totalFrames, videoFrameOf, videoTimeOf } from "../timing/video.js";
import { pop, statusCues, uhOh, upBlip, whoosh } from "./cues.mjs";
import { Mix, noise } from "./dsp.mjs";
import { beatGrid } from "./grid.mjs";
import { keyPress, keystrokes } from "./keyboard.mjs";
import { duck, fades, normalize } from "./mix.mjs";
import { score } from "./score.mjs";
import { wav } from "./wav.mjs";

/** The tape that typed the commands, for its typing speeds. */
const TAPE = new URL("../../../record/view.tape", import.meta.url);
/** The seeds of each part's noise, apart so a change to one part leaves the others as they were. */
const SEEDS = { music: 0x6d757374, cues: 0x63756573, keys: 0x6b657973, dither: 0x64697468 };
/** The fade in at the start and out over the last frames, in seconds. */
const FADE_IN_S = 0.25;
const FADE_OUT_S = 0.3;
/** How much the zoom must change for a move to count as a push or a pull rather than a pan. */
const ZOOM_EPSILON = 0.05;
/** The fewest and most beats a whoosh lasts. */
const WHOOSH_BEATS = [1, 2];

const publicDir = process.argv[2];
if (!publicDir) throw new Error("usage: node src/sound/render.mjs <public-dir>");
const timeline = JSON.parse(readFileSync(join(publicDir, TIMELINE), "utf8"));
const seconds = totalFrames(timeline) / FPS;
const { start } = bodyWindow(timeline);
const inBody = (at) => at >= start && at <= timeline.terminal.endMs;
const frameSeconds = (at) => videoFrameOf(timeline, at) / FPS;

const camera = shots(timeline);
const moves = camera.slice(1).filter((shot) => shot.move > 0 && inBody(shot.at)).map((shot) => {
  const before = cameraAt(camera, shot.at - 1).z;
  const after = cameraAt(camera, shot.at + shot.move).z;
  return {
    at: frameSeconds(shot.at),
    seconds: shot.move / MS_PER_SECOND,
    direction: Math.abs(after - before) < ZOOM_EPSILON ? 0 : Math.sign(after - before),
  };
});
const cadenceAt = (outroFrom(timeline) + INSTALL_IN[0]) / FPS;
const grid = beatGrid([0, ...moves.map((move) => move.at), cadenceAt]);

const music = score(grid, seconds, noise(SEEDS.music));

const cues = new Mix(seconds);
const cueRandom = noise(SEEDS.cues);
const status = statusCues(timeline, frameSeconds, inBody);
for (const cue of status) {
  if (cue.kind === "ok") pop(cues, cue.at);
  if (cue.kind === "fail") uhOh(cues, cue.at);
  if (cue.kind === "again") upBlip(cues, cue.at, cueRandom);
}
moves.forEach((move, index) => {
  const beat = grid.beats[index + 1];
  const beatSeconds = grid.timeAt(beat + 1) - grid.timeAt(beat);
  const beats = Math.min(WHOOSH_BEATS[1], Math.max(WHOOSH_BEATS[0], Math.round(move.seconds / beatSeconds)));
  whoosh(cues, move.at, grid.timeAt(beat + beats) - move.at, move.direction, cueRandom);
});

const keys = new Mix(seconds);
const keyRandom = noise(SEEDS.keys);
const presses = keystrokes(timeline, readFileSync(TAPE, "utf8"), (at) => videoTimeOf(timeline, at), inBody);
for (const press of presses) keyPress(keys, press, keyRandom);

const master = new Mix(seconds);
master.add(music, duck(status.map((cue) => cue.at)));
master.add(cues);
master.add(keys);
master.scale(fades(seconds, FADE_IN_S, FADE_OUT_S));
const { lufs, peakDb } = normalize(master);
writeFileSync(join(publicDir, SOUNDTRACK), wav(master, noise(SEEDS.dither)));

console.log(`soundtrack: ${seconds.toFixed(2)} s, ${lufs.toFixed(1)} LUFS, peak ${peakDb.toFixed(1)} dBFS, ${presses.length} key presses`);
console.log(`  tempo per span: ${grid.tempos.map((tempo) => tempo.toFixed(1)).join(", ")} BPM`);
grid.beats.forEach((beat, index) => {
  const at = index === 0 ? 0 : index <= moves.length ? moves[index - 1].at : cadenceAt;
  console.log(`  ${at.toFixed(3)} s on beat ${beat}${Number.isInteger(beat) ? "" : " (the and)"}`);
});
for (const cue of status) console.log(`  ${cue.at.toFixed(3)} s ${cue.kind}`);
