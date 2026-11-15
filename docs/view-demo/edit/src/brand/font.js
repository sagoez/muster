// Inter, the page's own vendored font, which edit/render.sh copies into the public folder.
import { continueRender, delayRender, staticFile } from "remotion";

/** The font's file in the public folder. */
const INTER = "inter.woff2";

/** Loads Inter into the document, holding the render until it is ready. */
export function loadInter() {
  const handle = delayRender("Inter");
  new FontFace("Inter", `url(${staticFile(INTER)}) format("woff2")`, { weight: "100 900" })
    .load()
    .then((face) => {
      document.fonts.add(face);
      continueRender(handle);
    });
}
