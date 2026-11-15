// Renders the demo's soundtrack from the same timeline and edit the camera
// uses: the user's music track fitted to the cuts, a typewriter strike from
// the user's recording for every typed character, and an interface sound on
// each event's frame, mixed and written as a stereo 48 kHz WAV.
//
// Usage: node src/sound/render.mjs <public-dir> <music.wav> <typewriter.wav>
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { FPS, INSTALL_IN, MS_PER_SECOND } from "../brand/theme.js";
import { cameraAt, shots } from "../timing/camera.js";
import { TIMELINE } from "../timing/recording.js";
import { SOUNDTRACK, bodyWindow, outroFrom, totalFrames, videoFrameOf, videoTimeOf } from "../timing/video.js";
import { beatGrid } from "./beats.mjs";
import { pop, statusCues, uhOh, upBlip, whoosh } from "./cues.mjs";
import { Mix, gainOf, noise } from "./dsp.mjs";
import { keystrokes, strikes, typeOut } from "./keyboard.mjs";
import { duck, fades, loudness, normalize } from "./mix.mjs";
import { arrange, playTrack } from "./music.mjs";
import { readWav, wav } from "./wav.mjs";

/** The tape that typed the commands, for its typing speeds. */
const TAPE = new URL("../../../record/view.tape", import.meta.url);
/** The seeds of each part's randomness, apart so a change to one part leaves the others as they were. */
const SEEDS = { cues: 0x63756573, keys: 0x6b657973, dither: 0x64697468 };
/** The loudness the music is set to before the sounds go over it, in LUFS. */
const MUSIC_LUFS = -17;
/** How far the music ducks under a status sound and under typing, as a share of its level. */
const STATUS_DUCK = 0.35;
const TYPING_DUCK = 0.15;
/** The fade in at the start and out over the last frames, in seconds. */
const FADE_IN_S = 0.4;
const FADE_OUT_S = 0.6;
/** How much the zoom must change for a move to count as a push or a pull rather than a pan. */
const ZOOM_EPSILON = 0.05;
/** The fewest and most beats a whoosh lasts. */
const WHOOSH_BEATS = [1, 2];

const [publicDir, musicPath, typewriterPath] = process.argv.slice(2);
if (!typewriterPath) throw new Error("usage: node src/sound/render.mjs <public-dir> <music.wav> <typewriter.wav>");
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
const installAt = (outroFrom(timeline) + INSTALL_IN[0]) / FPS;

const track = readWav(musicPath);
const grid = beatGrid(track);
const { parts, landing } = arrange(grid, moves.map((move) => move.at), installAt, seconds);
const music = playTrack(track, parts, seconds);
const musicGain = gainOf(MUSIC_LUFS - loudness(music));
music.scale(() => musicGain);

const cues = new Mix(seconds);
const cueRandom = noise(SEEDS.cues);
const status = statusCues(timeline, frameSeconds, inBody);
for (const cue of status) {
  if (cue.kind === "ok") pop(cues, cue.at);
  if (cue.kind === "fail") uhOh(cues, cue.at);
  if (cue.kind === "again") upBlip(cues, cue.at, cueRandom);
}
for (const move of moves) {
  const beats = Math.min(WHOOSH_BEATS[1], Math.max(WHOOSH_BEATS[0], Math.round(move.seconds / grid.beat)));
  whoosh(cues, move.at, beats * grid.beat, move.direction, cueRandom);
}

const keys = new Mix(seconds);
const presses = keystrokes(timeline, readFileSync(TAPE, "utf8"), (at) => videoTimeOf(timeline, at), inBody);
typeOut(keys, presses, strikes(readWav(typewriterPath)), noise(SEEDS.keys));

const master = new Mix(seconds);
const ducks = [...status.map((cue) => ({ at: cue.at, depth: STATUS_DUCK })), ...presses.map((press) => ({ at: press.at, depth: TYPING_DUCK }))];
master.add(music, duck(ducks, master.length));
master.add(cues);
master.add(keys);
master.scale(fades(seconds, FADE_IN_S, FADE_OUT_S));
const { lufs, peakDb } = normalize(master);
writeFileSync(join(publicDir, SOUNDTRACK), wav(master, noise(SEEDS.dither)));

const ms = (value) => `${(value * MS_PER_SECOND).toFixed(0)} ms`;
console.log(`soundtrack: ${seconds.toFixed(2)} s, ${lufs.toFixed(1)} LUFS, peak ${peakDb.toFixed(1)} dBFS, ${presses.length} key presses`);
console.log(`  track: ${(60 / grid.beat).toFixed(2)} BPM, first downbeat ${grid.downbeat.toFixed(3)} s, ending ${grid.ending.toFixed(3)} s`);
for (const part of parts) console.log(`  video ${part.from.toFixed(3)}-${part.to.toFixed(3)} s plays track from ${(part.from + part.offset).toFixed(3)} s`);
for (const move of landing) console.log(`  move ${move.at.toFixed(3)} s: ${move.cut ? "cut to a downbeat" : `${ms(move.error)} off the beat`}`);
for (const cue of status) console.log(`  ${cue.at.toFixed(3)} s ${cue.kind}`);
