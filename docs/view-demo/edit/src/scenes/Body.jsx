// The body: the page footage under a moving camera, the terminal footage as a
// picture in picture at bottom left, and the captions.
import { useMemo } from "react";
import { AbsoluteFill, OffthreadVideo, staticFile, useCurrentFrame } from "remotion";
import { BRASS, HAIR, HEIGHT, INK, MONO, SANS, TILE, WIDTH, framesOf } from "../brand/theme.js";
import { PAGE_SCALE, cameraAt, shots } from "../timing/camera.js";
import { captionShown, captions, liftAt } from "../timing/overlays.js";
import { bodyWindow, pageTimeAt } from "../timing/video.js";

/** The terminal footage's size, from view.tape, and how small it sits in the frame. */
const TERMINAL_WIDTH = 900;
const TERMINAL_HEIGHT = 540;
const PIP_SCALE = 0.72;
/** The pip's title bar height and its distance from the frame's bottom left corner. */
const PIP_BAR = 30;
const PIP_INSET = 48;
/** How much a lifted pip grows and brightens. */
const LIFT_GROW = 0.035;
const DIM = 0.86;
const LIFT_BRIGHTEN = 0.16;
/** The pip's title bar dots. */
const DOT_COLORS = ["#ff6b4a", "#f5b544", "#4fd39a"];
/** Where a caption starts: right of the pip. */
const CAPTION_LEFT = 740;
const CAPTION_BOTTOM = 86;

/** The page footage, placed by the camera. */
function Page({ timeline, at }) {
  const list = useMemo(() => shots(timeline), [timeline]);
  const camera = cameraAt(list, at);
  const scale = PAGE_SCALE * camera.z;
  return (
    <OffthreadVideo
      src={staticFile("page.mp4")}
      trimBefore={framesOf(bodyWindow(timeline).start)}
      muted
      style={{
        position: "absolute",
        left: WIDTH / 2 - camera.x * scale,
        top: HEIGHT / 2 - camera.y * scale,
        width: timeline.page.width * scale,
        height: timeline.page.height * scale,
        maxWidth: "none",
      }}
    />
  );
}

/** The terminal footage in a rounded window, lifted while a command is typed. */
function Pip({ timeline, at }) {
  const lift = liftAt(timeline, at);
  const { startMs, rate } = timeline.terminal;
  const width = TERMINAL_WIDTH * PIP_SCALE;
  return (
    <div
      style={{
        position: "absolute",
        left: PIP_INSET,
        bottom: PIP_INSET,
        width,
        borderRadius: 16,
        overflow: "hidden",
        background: TILE,
        border: `1px solid ${lift > 0 ? `rgba(214, 180, 122, ${0.08 + 0.3 * lift})` : HAIR}`,
        boxShadow: `0 ${24 + 14 * lift}px ${70 + 30 * lift}px rgba(0, 0, 0, ${0.5 + 0.15 * lift})`,
        transform: `scale(${1 + LIFT_GROW * lift})`,
        transformOrigin: "bottom left",
        filter: `brightness(${DIM + LIFT_BRIGHTEN * lift})`,
      }}
    >
      <div style={{ height: PIP_BAR, display: "flex", alignItems: "center", gap: 8, paddingLeft: 14 }}>
        {DOT_COLORS.map((color) => (
          <span key={color} style={{ width: 11, height: 11, borderRadius: 6, background: color, opacity: 0.75 }} />
        ))}
      </div>
      <OffthreadVideo
        src={staticFile("terminal.mp4")}
        trimBefore={framesOf((bodyWindow(timeline).start - startMs) * rate)}
        playbackRate={rate}
        muted
        style={{ display: "block", width, height: TERMINAL_HEIGHT * PIP_SCALE }}
      />
    </div>
  );
}

/** The captions, each a lower third over a shade that comes up with it. */
function Captions({ timeline, at }) {
  const list = useMemo(() => captions(timeline), [timeline]);
  const shown = list.map((caption) => captionShown(caption, at));
  const shade = Math.max(0, ...shown);
  return (
    <>
      <AbsoluteFill
        style={{ opacity: shade, background: "linear-gradient(to top, rgba(10, 11, 13, 0.82) 0%, rgba(10, 11, 13, 0) 34%)" }}
      />
      {list.map((caption, index) => (
        <div
          key={caption.text}
          style={{
            position: "absolute",
            left: CAPTION_LEFT,
            bottom: CAPTION_BOTTOM,
            opacity: shown[index],
            transform: `translateY(${(1 - shown[index]) * 12}px)`,
          }}
        >
          <div style={{ width: 40, height: 3, borderRadius: 2, background: BRASS, marginBottom: 18 }} />
          <div
            style={{
              fontFamily: SANS,
              fontSize: 50,
              fontWeight: 540,
              letterSpacing: "-0.02em",
              color: INK,
              textShadow: "0 2px 22px rgba(0, 0, 0, 0.55)",
              whiteSpace: "pre",
            }}
          >
            {caption.text}
            {caption.code && <span style={{ fontFamily: MONO, fontWeight: 400, color: BRASS, letterSpacing: 0 }}>{caption.code}</span>}
          </div>
        </div>
      ))}
    </>
  );
}

/** The body, `frame` 0 at its first frame. */
export function Body({ timeline }) {
  const frame = useCurrentFrame();
  const at = pageTimeAt(timeline, frame);
  return (
    <AbsoluteFill style={{ overflow: "hidden" }}>
      <Page timeline={timeline} at={at} />
      <Captions timeline={timeline} at={at} />
      <Pip timeline={timeline} at={at} />
    </AbsoluteFill>
  );
}
