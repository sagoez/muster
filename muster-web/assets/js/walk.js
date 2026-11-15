// The walk: a face that changes room is a new element in its new well, so
// it crosses the floor as a ghost in a page-level layer, where no well's
// scroll clips it; a face that only shifts within its well hops in place.

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
/** A face's pin. */
const PIN_SELECTOR = ".pin";
/** What a ghost leaves out: the floats rising off its face. */
const LEFT_OUT_SELECTOR = ".float";
/**
 * The classes a ghost's copy wears in place of the face's own, so a
 * selector for a real face never finds a ghost; the stylesheet draws both
 * alike.
 */
const GHOST_CLASSES = { face: "ghost-face", speaking: "ghost-speaking", who: "ghost-who", pin: "ghost-pin" };
/** The attributes a ghost's copy wears in place of the face's own. */
const GHOST_ATTRIBUTES = { "data-mark": "data-ghost-mark", "data-unconfirmed": "data-ghost-unconfirmed" };
/** The attributes a ghost's copy drops. */
const REAL_ONLY = ["data-pin", "data-float"];
/** The media query that turns the walk off. */
const REDUCED_MOTION = "(prefers-reduced-motion: reduce)";

/** Where each face stood before the next render: `{ project, faces }`, or none. */
let before = null;
/** Each face walking now, by name: `{ ghost, animation, pin }`, `pin` the real pin its ghost copies. */
const walking = new Map();
/** The layer ghosts walk in, made on first use. */
let layer = null;
/** What runs each time the floor has been drawn. */
const drawnListeners = new Set();

/** Runs `listener` each time the floor has been drawn; returns what stops it. */
export function onDrawn(listener) {
  drawnListeners.add(listener);
  return () => drawnListeners.delete(listener);
}

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

/** The face named `name` on the floor, if it is there; never a walking ghost. */
export function faceNamed(name) {
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

/** A copy of `node` as a ghost wears it: its classes and attributes renamed, its floats left out. */
function ghostCopy(node) {
  const copy = node.cloneNode(true);
  if (!(copy instanceof Element)) return copy;
  for (const left of copy.querySelectorAll(LEFT_OUT_SELECTOR)) left.remove();
  for (const element of [copy, ...copy.querySelectorAll("*")]) {
    for (const [real, ghost] of Object.entries(GHOST_CLASSES)) {
      element.classList.replace(real, ghost);
    }
    for (const [real, ghost] of Object.entries(GHOST_ATTRIBUTES)) {
      if (!element.hasAttribute(real)) continue;
      element.setAttribute(ghost, element.getAttribute(real));
      element.removeAttribute(real);
    }
    for (const attribute of REAL_ONLY) element.removeAttribute(attribute);
  }
  return copy;
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

/** Walks `face` as a ghost, copied once, from `from` to where it stands now, the face hidden meanwhile. */
function walkGhost(face, from) {
  const name = face.dataset.name;
  arrive(name);
  const to = face.getBoundingClientRect();
  const ghost = document.createElement("div");
  ghost.className = "ghost";
  ghost.dataset.room = face.dataset.room ?? "";
  Object.assign(ghost.style, { left: `${to.left}px`, top: `${to.top}px`, width: `${to.width}px` });
  ghost.append(...Array.from(face.childNodes, ghostCopy));
  walkLayer().append(ghost);
  face.style.visibility = "hidden";
  const animation = ghost.animate(frames(from.left - to.left, from.top - to.top), {
    duration: DURATION_MS,
    easing: EASING,
  });
  walking.set(name, { ghost, animation, pin: face.querySelector(PIN_SELECTOR) });
  animation.onfinish = () => arrive(name);
}

/**
 * Keeps the walking `face` hidden, and gives its ghost the pin that dropped
 * on it since the walk began, so the pin walks with it; nothing else of the
 * ghost is copied again, so no animation on it restarts.
 */
function keepWalking(walk, face) {
  face.style.visibility = "hidden";
  const pin = face.querySelector(PIN_SELECTOR);
  if (pin === walk.pin) return;
  walk.pin = pin;
  for (const old of walk.ghost.querySelectorAll(`.${GHOST_CLASSES.pin}`)) old.remove();
  if (pin) walk.ghost.prepend(ghostCopy(pin));
}

/** Hops `face` in place from `dx`, `dy` away, inside its own well. */
function hop(face, dx, dy) {
  face.animate(frames(dx, dy), { duration: DURATION_MS, easing: EASING });
}

/** Plays the walks since the last `measure`, after a render of `project`'s floor, then tells who listens. */
export function play(project) {
  for (const [name, walk] of [...walking]) {
    const face = faceNamed(name);
    if (face) keepWalking(walk, face);
    else arrive(name);
  }
  const was = before;
  before = null;
  if (was && was.project === project && !matchMedia(REDUCED_MOTION).matches) {
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
  for (const listener of drawnListeners) listener();
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
