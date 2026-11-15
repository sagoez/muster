// The whole video: the intro fading into the body, the outro fading in over its end.
import { AbsoluteFill, Sequence } from "remotion";
import { Body } from "./Body.jsx";
import { bodyWindow } from "./edit.js";
import { Intro, Outro } from "./TitleCards.jsx";
import { FADE_FRAMES, INTRO_FRAMES, OUTRO_FRAMES, WINDOW } from "./theme.js";

/** Where the body starts: under the intro's fade. */
const BODY_FROM = INTRO_FRAMES - FADE_FRAMES;

/** The frame the outro starts on, over the body's last frames. */
function outroFrom(timeline) {
  return BODY_FROM + bodyWindow(timeline).frames - FADE_FRAMES;
}

/** The video's length in frames for `timeline`. */
export function totalFrames(timeline) {
  return outroFrom(timeline) + OUTRO_FRAMES;
}

/** The composition. */
export function MusterView({ timeline }) {
  return (
    <AbsoluteFill style={{ background: WINDOW }}>
      <Sequence from={BODY_FROM} durationInFrames={bodyWindow(timeline).frames}>
        <Body timeline={timeline} />
      </Sequence>
      <Sequence durationInFrames={INTRO_FRAMES}>
        <Intro />
      </Sequence>
      <Sequence from={outroFrom(timeline)} durationInFrames={OUTRO_FRAMES}>
        <Outro />
      </Sequence>
    </AbsoluteFill>
  );
}
