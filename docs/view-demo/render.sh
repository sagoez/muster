#!/usr/bin/env bash
# Edits the footage record.sh left in out/public into out/muster-view.mp4:
# video/soundtrack.mjs synthesizes the soundtrack into out/public from the
# same timeline and edit, Remotion renders the edit with it as AAC, and ffmpeg
# fits the picture under GitHub's video upload limit, copying the audio as is.
# Nothing is committed: the README shows the MP4 uploaded to GitHub.
# Node comes from mise at $NODE_VERSION, which `just demo-view` pins;
# Remotion comes from video/node_modules, installed with no install scripts,
# where it also keeps its headless browser.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
public=$here/out/public
video=$here/video
edit=$here/out/edit.mp4
mp4=$here/out/muster-view.mp4
node_version=${NODE_VERSION:?set NODE_VERSION, or run just demo-view, which pins it}
# The MP4's size budget in bytes: GitHub's video upload limit on free accounts.
mp4_budget=$((10 * 1000 * 1000))
# The CRF the MP4 starts at, the step it rises by while over budget, and the most it may reach.
crf_start=16
crf_step=2
crf_max=34
# The soundtrack's AAC bitrate, which the budget loop leaves alone.
audio_bitrate=192k

[ -f "$public/timeline.json" ] || { echo "no recording in $public; run record.sh first" >&2; exit 1; }
node=$(mise exec "node@$node_version" -- which node)
export PATH=$(dirname "$node"):$PATH
[ -d "$video/node_modules" ] || (cd "$video" && npm ci --ignore-scripts --no-fund --no-audit)
cp "$repo/muster-web/assets/fonts/inter-latin-wght-normal.woff2" "$public/inter.woff2"
(cd "$video" && node soundtrack.mjs "$public")

(cd "$video" && ./node_modules/.bin/remotion render src/index.jsx MusterView "$edit" \
  --public-dir "$public" --codec h264 --crf 12 --image-format jpeg --jpeg-quality 95 \
  --audio-codec aac --audio-bitrate "$audio_bitrate")

crf=$crf_start
while :; do
  ffmpeg -loglevel error -y -i "$edit" -c:a copy -c:v libx264 -preset slow -crf "$crf" -pix_fmt yuv420p -movflags +faststart "$mp4"
  [ "$(stat -c %s "$mp4")" -le "$mp4_budget" ] && break
  [ "$crf" -ge "$crf_max" ] && { echo "$mp4 is over budget even at CRF $crf" >&2; exit 1; }
  crf=$((crf + crf_step))
done

echo "rendered $mp4 (CRF $crf); upload it to GitHub and put its link in the README"
