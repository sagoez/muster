// Registers the muster view demo with Remotion. Its length and edit come from
// timeline.json in the public folder record/record.sh fills.
import { Composition, registerRoot, staticFile } from "remotion";
import { loadInter } from "./brand/font.js";
import { FPS, HEIGHT, WIDTH } from "./brand/theme.js";
import { MusterView } from "./scenes/MusterView.jsx";
import { TIMELINE } from "./timing/recording.js";
import { totalFrames } from "./timing/video.js";

loadInter();

/** Reads the timeline and sizes the video by it. */
async function fromTimeline() {
  const timeline = await fetch(staticFile(TIMELINE)).then((response) => response.json());
  return { durationInFrames: totalFrames(timeline), props: { timeline } };
}

/** The one composition, sized once the timeline is read. */
function Root() {
  return (
    <Composition
      id="MusterView"
      component={MusterView}
      width={WIDTH}
      height={HEIGHT}
      fps={FPS}
      durationInFrames={1}
      defaultProps={{ timeline: null }}
      calculateMetadata={fromTimeline}
    />
  );
}

registerRoot(Root);
