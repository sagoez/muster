// The muster mark from docs/logo.svg, drawn in stroke by stroke.
import { BRASS } from "./theme.js";

/** The mark at `size` pixels: `outer` and `inner` say how much of each ring is drawn, `dot` how far the centre has come in, each 0 to 1. */
export function Mark({ size, outer = 1, inner = 1, dot = 1 }) {
  return (
    <svg viewBox="0 0 20 20" width={size} height={size} fill="none" stroke={BRASS} strokeWidth={1.6} strokeLinecap="round">
      <circle cx="10" cy="10" r="8" pathLength="1" strokeDasharray="1 1" strokeDashoffset={1 - outer} transform="rotate(-90 10 10)" />
      <circle cx="10" cy="10" r="4.5" pathLength="1" strokeDasharray="1 1" strokeDashoffset={1 - inner} transform="rotate(-90 10 10)" />
      <circle cx="10" cy="10" r={1.2 * (0.6 + 0.4 * dot)} fill={BRASS} stroke="none" opacity={dot} />
    </svg>
  );
}
