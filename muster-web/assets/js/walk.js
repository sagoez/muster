// The walk: a face that changes room crosses the floor as a ghost in a
// page-level layer, so no well's scroll clips it; a face that only shifts
// within its well hops in place.

/** How long a walk takes. */
const DURATION_MS = 1000;
/** The walk's easing. */
const EASING = "cubic-bezier(.45,0,.2,1)";
/** How high a face hops at the walk's middle, in pixels. */
const HOP_PX = 18;
/** How much a face grows at the walk's middle. */
const HOP_SCALE = 1.12;
/** Where in the walk the hop peaks. */
const HOP_AT = 0.5;
/** Moves shorter than this, in pixels, are not walked. */
const STILL_PX = 2;
/** The floor, while one is drawn. */
const MAP_SELECTOR = ".map";
/** The faces on the floor. */
const FACE_SELECTOR = ".map .person";
/** The attributes a ghost's copy drops, so only the real face carries them. */
const REAL_ONLY = ["data-pin", "data-float"];
/** The media query that turns the walk off. */
const REDUCED_MOTION = "(prefers-reduced-motion: reduce)";

/** Where each face stood before the next render: `{ project, faces }`, or none. */
let before = null;
/** Each face walking now, by name: `{ ghost, animation }`. */
const walking = new Map();
/** The layer ghosts walk in, made on first use. */
let layer = null;

/** The layer ghosts walk in. */
function walkLayer() {
  if (!layer) {
    layer = document.createElement("div");
    layer.className = "walk-layer";
    layer.setAttribute("aria-hidden", "true");
    document.body.append(layer);
  }
  return layer;
}

/** The face named `name` on the floor, if it is there. */
function faceNamed(name) {
  for (const face of document.querySelectorAll(FACE_SELECTOR)) {
    if (face.dataset.name === name) return face;
  }
  return null;
}

/** Records where every face of `project`'s floor stands, once per render, while a floor is drawn. */
export function measure(project) {
  if (before || !document.querySelector(MAP_SELECTOR)) return;
  const faces = new Map();
  for (const face of document.querySelectorAll(FACE_SELECTOR)) {
    const name = face.dataset.name;
    const ghost = walking.get(name)?.ghost;
    faces.set(name, { rect: (ghost ?? face).getBoundingClientRect(), room: face.dataset.room });
  }
  before = { project, faces };
}

/** The keyframes of a walk from `dx`, `dy` away back to where a face stands. */
function frames(dx, dy) {
  return [
    { transform: `translate(${dx}px, ${dy}px) scale(1)` },
    {
      transform: `translate(${dx * HOP_AT}px, ${dy * HOP_AT - HOP_PX}px) scale(${HOP_SCALE})`,
      offset: HOP_AT,
    },
    { transform: "none" },
  ];
}

/** Fills `ghost` with a copy of `face` as it is drawn now. */
function copyInto(ghost, face) {
  ghost.dataset.room = face.dataset.room ?? "";
  ghost.replaceChildren(...Array.from(face.childNodes, (node) => node.cloneNode(true)));
  for (const attribute of REAL_ONLY) {
    for (const marked of ghost.querySelectorAll(`[${attribute}]`)) marked.removeAttribute(attribute);
  }
}

/**
 * Ends the walk of `name` at once: its ghost goes and its face shows again.
 * A browser reports a cancelled animation only later, so nothing waits for
 * that report.
 */
function arrive(name) {
  const walk = walking.get(name);
  if (!walk) return;
  walking.delete(name);
  walk.animation.onfinish = null;
  walk.animation.cancel();
  walk.ghost.remove();
  const face = faceNamed(name);
  if (face) face.style.visibility = "";
}

/** Walks `face` as a ghost from `from` to where it stands now, the face hidden meanwhile. */
function walkGhost(face, from) {
  const name = face.dataset.name;
  arrive(name);
  const to = face.getBoundingClientRect();
  const ghost = document.createElement("div");
  ghost.className = "ghost";
  Object.assign(ghost.style, { left: `${to.left}px`, top: `${to.top}px`, width: `${to.width}px` });
  copyInto(ghost, face);
  walkLayer().append(ghost);
  face.style.visibility = "hidden";
  const animation = ghost.animate(frames(from.left - to.left, from.top - to.top), {
    duration: DURATION_MS,
    easing: EASING,
  });
  walking.set(name, { ghost, animation });
  animation.onfinish = () => arrive(name);
}

/** Hops `face` in place from `dx`, `dy` away, inside its own well. */
function hop(face, dx, dy) {
  face.animate(frames(dx, dy), { duration: DURATION_MS, easing: EASING });
}

/**
 * Plays the walks since the last `measure`, after a render of `project`'s
 * floor, and keeps every walking ghost a copy of its face.
 */
export function play(project) {
  for (const [name, walk] of [...walking]) {
    const face = faceNamed(name);
    if (!face) {
      arrive(name);
      continue;
    }
    copyInto(walk.ghost, face);
    face.style.visibility = "hidden";
  }
  const was = before;
  before = null;
  if (!was || was.project !== project || matchMedia(REDUCED_MOTION).matches) return;
  for (const face of document.querySelectorAll(FACE_SELECTOR)) {
    const old = was.faces.get(face.dataset.name);
    if (!old) continue;
    const now = face.getBoundingClientRect();
    const dx = old.rect.left - now.left;
    const dy = old.rect.top - now.top;
    if (Math.abs(dx) + Math.abs(dy) < STILL_PX) continue;
    if (old.room === face.dataset.room && !walking.has(face.dataset.name)) hop(face, dx, dy);
    else walkGhost(face, old.rect);
  }
}

/** Ends every walk at once, as when the floor changes. */
export function settle() {
  for (const name of [...walking.keys()]) arrive(name);
}

/** Ends every walk and drops any measure, as when the floor stops being drawn. */
export function forget() {
  settle();
  before = null;
}
