// Records the page half of the demo: a screencast of `muster view` at 1280x800
// (at twice the pixels, so the edit's zooms stay sharp), every SSE message the
// page receives, where its rooms, faces and panels sit over time, and the
// clicks it makes when the tape types a command a viewer would follow up on.
// Usage: DEMO_VIEW_URL=<url> node page.mjs <out-dir> <commands-file> <stop-file>
// The view's address comes through the environment, since it carries the
// view's token and every local user can read a process's arguments.
// It writes <out-dir>/frames/*.jpg and <out-dir>/page.json, touches
// <out-dir>/ready once the first snapshot is in, and stops once <stop-file> exists.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "playwright";
import { KINDS } from "../../../muster-web/assets/js/stream.js";

/** The page's size in CSS pixels. */
const VIEWPORT = { width: 1280, height: 800 };
/** Device pixels per CSS pixel in the footage. */
const SCALE = 2;
/** The screencast's JPEG quality. */
const JPEG_QUALITY = 92;
/** How often the rooms, faces and panels are measured. */
const SAMPLE_MS = 150;
/** How often the command log and the stop file are checked. */
const POLL_MS = 100;
/** How long the page settles after the first snapshot before it reports ready. */
const SETTLE_MS = 1500;
/** How long the last frames run after the stop file appears. */
const TAIL_MS = 400;
/** The variable that carries the view's address. */
const URL_ENV = "DEMO_VIEW_URL";
/**
 * What the viewer does in the page when a typed command line contains `when`:
 * after `delay`, click `selector` or press `key`.
 */
const REACTIONS = [
  { when: "muster logs payments", delay: 700, action: "open-card", selector: '.floor .person[data-name="payments"] .face' },
  { when: "muster restart payments", delay: 500, action: "close-card", key: "Escape" },
  { when: "muster note set", delay: 900, action: "open-notes", selector: "#notes-chip" },
  { when: "muster start search", delay: 600, action: "close-notes", key: "Escape" },
];

const url = process.env[URL_ENV];
const [outDir, commandsFile, stopFile] = process.argv.slice(2);
if (!url || !outDir || !commandsFile || !stopFile) {
  console.error(`usage: ${URL_ENV}=<url> node page.mjs <out-dir> <commands-file> <stop-file>`);
  process.exit(2);
}
const framesDir = join(outDir, "frames");
mkdirSync(framesDir, { recursive: true });

const frames = [];
const events = [];
const samples = [];
const actions = [];

// Headless Chromium screencasts in CSS pixels under an emulated scale alone; forcing the scale makes the frames device pixels.
const browser = await chromium.launch({ args: [`--force-device-scale-factor=${SCALE}`] });
const context = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: SCALE, colorScheme: "dark" });
const page = await context.newPage();

await page.exposeFunction("__demoEvent", (event) => events.push(event));
await page.addInitScript((kinds) => {
  const Native = window.EventSource;
  window.EventSource = class extends Native {
    constructor(...args) {
      super(...args);
      for (const kind of kinds) {
        this.addEventListener(kind, (event) => {
          const at = Date.now();
          let message;
          try {
            message = JSON.parse(event.data);
          } catch {
            return;
          }
          window.__demoEvent({
            at,
            kind,
            project: message.project ?? null,
            name: message.name ?? message.process?.name ?? null,
            pin: message.pin ?? null,
            room: message.process?.room ?? null,
          });
        });
      }
    }
  };
}, KINDS);

const cdp = await context.newCDPSession(page);
cdp.on("Page.screencastFrame", ({ data, metadata, sessionId }) => {
  const file = `${String(frames.length).padStart(6, "0")}.jpg`;
  writeFileSync(join(framesDir, file), Buffer.from(data, "base64"));
  frames.push({ file, at: metadata.timestamp * 1000 });
  cdp.send("Page.screencastFrameAck", { sessionId }).catch(() => {});
});

/** Where each room, face and open panel of the shown floor sits, in CSS pixels. */
function measure() {
  const box = (element) => {
    if (!element) return null;
    const rect = element.getBoundingClientRect();
    return [rect.x, rect.y, rect.width, rect.height].map(Math.round);
  };
  const boxes = {
    floor: box(document.querySelector(".floor")),
    chip: box(document.querySelector("#notes-chip")),
    notes: box(document.querySelector("#notes")),
    card: box(document.querySelector(".desk")),
  };
  for (const room of document.querySelectorAll(".floor .room[data-room]")) boxes[`room:${room.dataset.room}`] = box(room);
  for (const person of document.querySelectorAll(".floor .person[data-name]")) {
    boxes[`person:${person.dataset.name}`] ??= box(person);
  }
  return Object.fromEntries(Object.entries(boxes).filter(([, value]) => value));
}

await cdp.send("Page.startScreencast", {
  format: "jpeg",
  quality: JPEG_QUALITY,
  maxWidth: VIEWPORT.width * SCALE,
  maxHeight: VIEWPORT.height * SCALE,
});
await page.goto(url);
await page.waitForFunction(() => document.querySelector(".floor .room"));
await page.waitForTimeout(SETTLE_MS);
writeFileSync(join(outDir, "ready"), "");

let lastBoxes = "";
let seenLines = 0;
let lastSample = 0;
while (!existsSync(stopFile)) {
  const now = Date.now();
  if (now - lastSample >= SAMPLE_MS) {
    lastSample = now;
    const boxes = await page.evaluate(measure);
    const json = JSON.stringify(boxes);
    if (json !== lastBoxes) {
      lastBoxes = json;
      samples.push({ at: now, boxes });
    }
  }
  const lines = existsSync(commandsFile) ? readFileSync(commandsFile, "utf8").split("\n").filter(Boolean) : [];
  for (const line of lines.slice(seenLines)) {
    const text = line.slice(line.indexOf(" ") + 1);
    for (const reaction of REACTIONS.filter((candidate) => text.includes(candidate.when))) {
      setTimeout(async () => {
        actions.push({ at: Date.now(), action: reaction.action });
        if (reaction.selector) await page.click(reaction.selector).catch((error) => console.error(error.message));
        else await page.keyboard.press(reaction.key);
      }, reaction.delay);
    }
  }
  seenLines = lines.length;
  await page.waitForTimeout(POLL_MS);
}

await page.waitForTimeout(TAIL_MS);
await cdp.send("Page.stopScreencast");
writeFileSync(join(outDir, "page.json"), JSON.stringify({ viewport: VIEWPORT, scale: SCALE, frames, events, samples, actions }));
await browser.close();
