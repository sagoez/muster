// The whole video: the intro fading into the body, the outro fading in over
// its end, and under it all the soundtrack src/sound renders from the same edit.
import { AbsoluteFill, Html5Audio, Sequence, staticFile } from "remotion";
import { INTRO_FRAMES, OUTRO_FRAMES, WINDOW } from "../brand/theme.js";
import { BODY_FROM, SOUNDTRACK, bodyWindow, outroFrom } from "../timing/video.js";
import { Body } from "./Body.jsx";
import { Intro, Outro } from "./TitleCards.jsx";

/** The composition. */
export function MusterView({ timeline }) {
  return (
    <AbsoluteFill style={{ background: WINDOW }}>
      <Html5Audio src={staticFile(SOUNDTRACK)} />
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
