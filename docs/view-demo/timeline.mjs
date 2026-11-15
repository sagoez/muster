// Turns one recording into the edit's timeline: every time on the page's clock
// from its first frame, the terminal footage placed on that clock, and the
// concat list that makes the page's frames a constant-rate video.
// Usage: node timeline.mjs <raw-dir> <terminal.mp4> <public-dir> <show-after-mark-ms>
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

/** How long the page's last frame holds. */
const LAST_FRAME_MS = 100;
/** The commands the edit reacts to; the tape's own `mark` and `source` lines are not among them. */
const SHOWN_COMMAND = /^(muster|touch|rm) /;
/** Milliseconds in a second. */
const MS_PER_SECOND = 1000;

const [rawDir, terminalVideo, publicDir, showAfterMark] = process.argv.slice(2);
const page = JSON.parse(readFileSync(join(rawDir, "page.json"), "utf8"));
const mark = (name) => Number(readFileSync(join(rawDir, `mark-${name}`), "utf8").trim());

const origin = page.frames[0].at;
const since = (at) => Math.round(at - origin);

const list = page.frames.flatMap((frame, index) => {
  const next = page.frames[index + 1]?.at ?? frame.at + LAST_FRAME_MS;
  return [`file 'frames/${frame.file}'`, `duration ${((next - frame.at) / MS_PER_SECOND).toFixed(4)}`];
});
list.push(`file 'frames/${page.frames.at(-1).file}'`);
writeFileSync(join(rawDir, "frames.txt"), `${list.join("\n")}\n`);

const terminalSeconds = Number(
  execFileSync("ffprobe", ["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", terminalVideo], {
    encoding: "utf8",
  }).trim(),
);
const shown = mark("show") + Number(showAfterMark);
const hidden = mark("hide");

const commands = readFileSync(join(rawDir, "commands.log"), "utf8")
  .split("\n")
  .filter(Boolean)
  .map((line) => {
    const space = line.indexOf(" ");
    return { at: since(Number(line.slice(0, space))), text: line.slice(space + 1) };
  })
  .filter((command) => SHOWN_COMMAND.test(command.text));

const timeline = {
  page: {
    width: page.viewport.width,
    height: page.viewport.height,
    scale: page.scale,
    durationMs: since(page.frames.at(-1).at + LAST_FRAME_MS),
  },
  terminal: {
    startMs: since(shown),
    endMs: since(hidden),
    // Footage seconds per wall second: vhs can capture fewer frames than it encodes.
    rate: (terminalSeconds * MS_PER_SECOND) / (hidden - shown),
  },
  commands,
  events: page.events.map((event) => ({ ...event, at: since(event.at) })),
  actions: page.actions.map((action) => ({ ...action, at: since(action.at) })),
  samples: page.samples.map((sample) => ({ ...sample, at: since(sample.at) })),
};
writeFileSync(join(publicDir, "timeline.json"), JSON.stringify(timeline));
console.log(
  `timeline: ${commands.length} commands, ${timeline.events.length} events, terminal rate ${timeline.terminal.rate.toFixed(3)}`,
);
