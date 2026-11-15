// What the recording says, read off timeline.json: when each command was
// entered, when the page recorder acted, when the page showed an event, and
// where each box sat. Every time is in page milliseconds, counted from the
// page footage's first frame.

/** The recording's timeline in the public folder, which record/timeline.mjs writes. */
export const TIMELINE = "timeline.json";

/** The command line that starts with `prefix`; the recording is incomplete without it. */
export function command(timeline, prefix) {
  const found = timeline.commands.find((candidate) => candidate.text.startsWith(prefix));
  if (!found) throw new Error(`the timeline has no command starting with ${prefix}`);
  return found;
}

/** When the page recorder did `name`. */
export function action(timeline, name) {
  const found = timeline.actions.find((candidate) => candidate.action === name);
  if (!found) throw new Error(`the timeline has no ${name} action`);
  return found.at;
}

/** The first event matching `match` at or after `after`. */
export function event(timeline, after, match) {
  const found = timeline.events.find((candidate) => candidate.at >= after && match(candidate));
  if (!found) throw new Error("the timeline is missing an event the edit cuts to");
  return found.at;
}

/** When payments showed failed after the tape broke it. */
export function paymentsFailed(timeline) {
  const crash = command(timeline, "touch crash").at;
  return event(timeline, crash, (e) => e.kind === "pin" && e.pin === "fail" && e.name === "payments");
}

/** Where `key` sat at page time `at`: the last measurement at or before it, else the first after. */
export function boxAt(timeline, key, at) {
  let before = null;
  for (const sample of timeline.samples) {
    const box = sample.boxes[key];
    if (!box) continue;
    if (sample.at > at) return before ?? box;
    before = box;
  }
  if (!before) throw new Error(`the recording never measured ${key}`);
  return before;
}

/** The box around every face in the Off room at `at`. */
export function offFaces(timeline, at) {
  const room = boxAt(timeline, "room:off", at);
  const sample = [...timeline.samples].reverse().find((candidate) => candidate.at <= at) ?? timeline.samples[0];
  const faces = Object.entries(sample.boxes)
    .filter(([key, box]) => key.startsWith("person:") && box[1] >= room[1] && box[1] < room[1] + room[3])
    .map(([, box]) => box);
  if (!faces.length) return room;
  const left = Math.min(...faces.map((box) => box[0]));
  const top = Math.min(...faces.map((box) => box[1]));
  const right = Math.max(...faces.map((box) => box[0] + box[2]));
  const bottom = Math.max(...faces.map((box) => box[1] + box[3]));
  return [left, top, right - left, bottom - top];
}
