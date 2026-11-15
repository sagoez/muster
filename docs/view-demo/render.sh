#!/usr/bin/env bash
# Edits the footage record.sh left in out/public into docs/muster-view.mp4 and
# docs/muster-view.gif: Remotion renders the edit, ffmpeg fits the MP4 under
# its size budget and makes the GIF, and gifsicle shrinks the GIF. Node comes
# from mise; Remotion and gifsicle from video/node_modules, where Remotion also
# keeps its headless browser.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
public=$here/out/public
video=$here/video
edit=$here/out/edit.mp4
mp4=$repo/docs/muster-view.mp4
gif=$repo/docs/muster-view.gif
# The MP4's and the GIF's size budgets, in bytes.
mp4_budget=$((10 * 1000 * 1000))
gif_budget=$((8 * 1000 * 1000))
# The CRF the MP4 starts at, the step it rises by while over budget, and the most it may reach.
crf_start=16
crf_step=2
crf_max=34
# The GIF's palette size, and how lossy gifsicle may make its frames to fit its budget.
gif_colors=192
gif_lossy=20

[ -f "$public/timeline.json" ] || { echo "no recording in $public; run record.sh first" >&2; exit 1; }
node=$(mise exec node@22 -- which node)
export PATH=$(dirname "$node"):$PATH
[ -d "$video/node_modules" ] || (cd "$video" && npm ci --no-fund --no-audit)
cp "$repo/muster-web/assets/fonts/inter-latin-wght-normal.woff2" "$public/inter.woff2"

(cd "$video" && ./node_modules/.bin/remotion render src/index.jsx MusterView "$edit" \
  --public-dir "$public" --codec h264 --crf 12 --image-format jpeg --jpeg-quality 95)

crf=$crf_start
while :; do
  ffmpeg -loglevel error -y -i "$edit" -an -c:v libx264 -preset slow -crf "$crf" -pix_fmt yuv420p -movflags +faststart "$mp4"
  [ "$(stat -c %s "$mp4")" -le "$mp4_budget" ] && break
  [ "$crf" -ge "$crf_max" ] && { echo "$mp4 is over budget even at CRF $crf" >&2; exit 1; }
  crf=$((crf + crf_step))
done

# The GIF comes from the full-quality edit, so the MP4's compression noise costs it no diffs. No dither:
# a dither pattern changes every frame the camera moves and doubles the size.
ffmpeg -loglevel error -y -i "$edit" -vf \
  "fps=15,scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=$gif_colors:stats_mode=diff[p];[b][p]paletteuse=dither=none:diff_mode=rectangle" \
  "$here/out/palette.gif"
"$video/node_modules/gifsicle/vendor/gifsicle" --no-warnings -O3 --lossy="$gif_lossy" "$here/out/palette.gif" -o "$gif"
[ "$(stat -c %s "$gif")" -le "$gif_budget" ] || echo "warning: $gif is over its budget" >&2
echo "rendered $mp4 (CRF $crf) and $gif"
