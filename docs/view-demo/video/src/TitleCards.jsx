// The intro and the outro: the brass mark on the page's dark, a line under it.
import { AbsoluteFill, Easing, interpolate, useCurrentFrame } from "remotion";
import { Mark } from "./Mark.jsx";
import { BRASS, DEEP, FADE_FRAMES, HAIR, INK, INTRO_FRAMES, MONO, SANS, TILE, WINDOW } from "./theme.js";

/** How the title cards ease: out of a gentle start into a long settle, never past the mark. */
const SETTLE = Easing.bezier(0.16, 1, 0.3, 1);
/** The intro's line. */
const TAGLINE = "Your dev stack, live.";
/** The outro's two commands. */
const INSTALL = "cargo install muster-workspace";
const VIEW = "muster view";

/** `frame` mapped from `[from, to]` onto 0 to 1, eased and held at both ends. */
function progress(frame, from, to, easing = SETTLE) {
  return interpolate(frame, [from, to], [0, 1], { easing, extrapolateLeft: "clamp", extrapolateRight: "clamp" });
}

/** The cards' backdrop: the page's window color, falling off into a deeper edge, faded to `opacity` over the footage. */
function Backdrop({ opacity, children }) {
  return (
    <AbsoluteFill
      style={{
        opacity,
        background: `radial-gradient(ellipse 70% 70% at 50% 46%, ${WINDOW} 0%, ${DEEP} 100%)`,
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      {children}
    </AbsoluteFill>
  );
}

/** The intro: the mark draws in, then the tagline rises under it, and both fade into the footage. */
export function Intro() {
  const frame = useCurrentFrame();
  const out = 1 - progress(frame, INTRO_FRAMES - FADE_FRAMES - 2, INTRO_FRAMES, Easing.inOut(Easing.cubic));
  const line = progress(frame, 30, 52);
  return (
    <Backdrop opacity={out}>
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 44 }}>
        <Mark size={132} outer={progress(frame, 2, 34)} inner={progress(frame, 10, 38)} dot={progress(frame, 26, 40)} />
        <div
          style={{
            fontFamily: SANS,
            fontSize: 60,
            fontWeight: 520,
            letterSpacing: "-0.022em",
            color: INK,
            opacity: line,
            transform: `translateY(${(1 - line) * 16}px)`,
          }}
        >
          {TAGLINE}
        </div>
      </div>
    </Backdrop>
  );
}

/** One command line of the outro, prompt in brass. */
function Command({ text, shown }) {
  return (
    <div style={{ opacity: shown, transform: `translateY(${(1 - shown) * 10}px)`, whiteSpace: "pre" }}>
      <span style={{ color: BRASS }}>$ </span>
      {text}
    </div>
  );
}

/** The outro: the mark, then how to get muster and the one command that opens the view. */
export function Outro() {
  const frame = useCurrentFrame();
  const card = progress(frame, 14, 34);
  return (
    <Backdrop opacity={progress(frame, 0, FADE_FRAMES, Easing.inOut(Easing.cubic))}>
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 56 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 26 }}>
          <Mark size={92} outer={progress(frame, 0, 26)} inner={progress(frame, 6, 30)} dot={progress(frame, 20, 32)} />
          <span style={{ fontFamily: SANS, fontSize: 72, fontWeight: 560, letterSpacing: "-0.03em", color: BRASS }}>muster</span>
        </div>
        <div
          style={{
            fontFamily: MONO,
            fontSize: 36,
            lineHeight: 1.65,
            color: INK,
            background: TILE,
            border: `1px solid ${HAIR}`,
            borderRadius: 18,
            padding: "26px 44px",
            boxShadow: "0 30px 80px rgba(0, 0, 0, 0.45)",
            opacity: card,
            transform: `translateY(${(1 - card) * 14}px)`,
          }}
        >
          <Command text={INSTALL} shown={progress(frame, 20, 38)} />
          <Command text={VIEW} shown={progress(frame, 30, 48)} />
        </div>
      </div>
    </Backdrop>
  );
}
