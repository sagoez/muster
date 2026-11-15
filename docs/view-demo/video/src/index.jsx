// Registers the muster view demo with Remotion. Its length and edit come from
// timeline.json in the public folder record.sh fills.
import { Composition, continueRender, delayRender, registerRoot, staticFile } from "remotion";
import { totalFrames } from "./edit.js";
import { MusterView } from "./MusterView.jsx";
import { FPS, HEIGHT, WIDTH } from "./theme.js";

/** Inter, the page's own vendored font, which render.sh copies into the public folder. */
const fontLoad = delayRender("Inter");
new FontFace("Inter", `url(${staticFile("inter.woff2")}) format("woff2")`, { weight: "100 900" })
  .load()
  .then((face) => {
    document.fonts.add(face);
    continueRender(fontLoad);
  });

/** Reads the timeline and sizes the video by it. */
async function fromTimeline() {
  const timeline = await fetch(staticFile("timeline.json")).then((response) => response.json());
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
