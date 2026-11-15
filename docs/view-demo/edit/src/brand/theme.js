// The edit's look: the page's own tones, its brass mark and its fonts.

/** The composition's size and rate. */
export const WIDTH = 1920;
export const HEIGHT = 1080;
export const FPS = 30;

/** The page's window color, which every scene sits on. */
export const WINDOW = "#141518";
/** The darkest edge of the title cards' vignette. */
export const DEEP = "#0d0e10";
export const TILE = "#1a1b1f";
export const HAIR = "rgba(255, 255, 255, 0.08)";
export const INK = "#f1f1f3";
export const INK_2 = "#a1a2a8";
/** The muster mark's color, from docs/logo.svg. */
export const BRASS = "#d6b47a";

export const SANS = "Inter, ui-sans-serif, system-ui, sans-serif";
export const MONO = '"iA Writer Mono S", ui-monospace, Menlo, monospace';

/** Frames the intro, the outro and each crossfade between scenes last. */
export const INTRO_FRAMES = 75;
export const OUTRO_FRAMES = 90;
export const FADE_FRAMES = 12;
/** The outro frames over which its install line comes in, which the soundtrack's last chord lands on. */
export const INSTALL_IN = [20, 38];

/** Milliseconds in a second. */
export const MS_PER_SECOND = 1000;

/** The frame `ms` milliseconds in. */
export function framesOf(ms) {
  return Math.round((ms / MS_PER_SECOND) * FPS);
}
