#!/usr/bin/env bash
# Records the muster view demo for real: builds a sandboxed `shop` stack and a
# neighbouring `blog`, serves `muster view --no-open` on it, records the page
# with page.mjs while view.tape types commands into a terminal, then writes
# the footage and timeline.json into out/public for render.sh.
#
# It runs the muster in $MUSTER_BIN_DIR (target/debug by default) with every
# state and config folder under $DEMO_DIR, HOME and the projects under
# $DEMO_HOME (fresh mktemp dirs by default) and no HERDR_* variables, so the
# real database, service and herdr session stay untouched. $DEMO_HOME sits in
# the temp folder so a path muster prints names nobody. On any exit it stops
# both projects, the view, the page recorder and the service, and removes the
# folders it made itself; a $DEMO_DIR or $DEMO_HOME you set is left in place.
# The view's address, which carries its token, reaches the page recorder
# through the environment, never its arguments. It needs mise (Node at
# $NODE_VERSION, which `just demo-view` pins, vhs@0.11.0, ttyd), ffmpeg, the
# system python3 and ports 8741 to 8743 free.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
out=$here/out
raw=$out/raw
public=$out/public
node_version=${NODE_VERSION:?set NODE_VERSION, or run just demo-view, which pins it}
export MUSTER_BIN_DIR=${MUSTER_BIN_DIR:-${CARGO_TARGET_DIR:-$repo/target}/debug}
# The folders this script made, which it alone removes.
made=()
if [ -z "${DEMO_DIR:-}" ]; then
  DEMO_DIR=$(mktemp -d)
  made+=("$DEMO_DIR")
fi
if [ -z "${DEMO_HOME:-}" ]; then
  DEMO_HOME=$(mktemp -d "${TMPDIR:-/tmp}/demo.XXXX")
  made+=("$DEMO_HOME")
fi
export DEMO_DIR DEMO_HOME
export DEMO_SANDBOX=$here/sandbox.sh
export DEMO_COMMANDS=$raw/commands.log
# How long view.tape waits after `mark show` before it shows the terminal.
show_after_mark_ms=300
# How long to wait for the view's address and the recorder's first snapshot.
startup_timeout_s=30

[ -x "$MUSTER_BIN_DIR/muster" ] || { echo "no muster in $MUSTER_BIN_DIR; run cargo build first" >&2; exit 1; }
node=$(mise exec "node@$node_version" -- which node)
tools=$(mise exec vhs@0.11.0 ttyd@latest -- sh -c 'dirname "$(command -v vhs)"; dirname "$(command -v ttyd)"' | paste -sd:)

view_pid='' page_pid=''

# sandboxed <command...>: runs a command with the sandbox's environment, in a subshell.
sandboxed() { ( source "$DEMO_SANDBOX" && "$@" ); }

# cleanup: stops everything the recording started, then removes the sandbox.
cleanup() {
  set +e
  [ -n "$page_pid" ] && kill "$page_pid" 2> /dev/null
  [ -n "$view_pid" ] && kill "$view_pid" 2> /dev/null
  if [ -d "$DEMO_DIR/s" ]; then
    sandboxed muster down -p shop > /dev/null 2>&1
    sandboxed muster down -p blog > /dev/null 2>&1
    sandboxed muster service stop > /dev/null 2>&1
    # Anything still running with the sandbox's state folder in its environment is a leftover holder or child.
    for environ in /proc/[0-9]*/environ; do
      grep -qzx "XDG_STATE_HOME=$DEMO_DIR/s" "$environ" 2> /dev/null && kill "$(basename "$(dirname "$environ")")" 2> /dev/null
    done
  fi
  wait 2> /dev/null
  [ "${#made[@]}" -gt 0 ] && rm -rf "${made[@]}"
  made=()
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# wait_for <file> <pattern>: waits until <file> holds a line matching <pattern>.
wait_for() {
  for _ in $(seq $((startup_timeout_s * 10))); do
    grep -q -- "$2" "$1" 2> /dev/null && return 0
    sleep 0.1
  done
  echo "timed out waiting for $2 in $1" >&2
  return 1
}

[ -d "$here/node_modules" ] || (cd "$here" && PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 "$node" "$(dirname "$node")/npm" ci --ignore-scripts --no-fund --no-audit)

rm -rf "$raw" "$public" "$out/terminal.mp4"
mkdir -p "$raw" "$public"
mkdir -p -m 700 "$DEMO_DIR/r"
shop=$DEMO_HOME/code/shop
blog=$DEMO_HOME/code/blog
mkdir -p "$shop" "$blog"
cp "$here"/stand-ins/*.sh "$shop/"
cp "$here/stand-ins/loop.sh" "$blog/"
sandboxed muster config set desktop-notifications false > /dev/null

# shop <args...>: runs `muster add` in the shop folder.
shop() { (cd "$shop" && sandboxed muster add "$@" > /dev/null); }
shop db --icon postgresql --autostart --port 8741 --ready-port 8741 --ready-timeout 10s -- ./serve.sh 8741
shop api --icon rust --autostart --port 8742 --ready-port 8742 --ready-timeout 15s -- ./serve.sh 8742 3
shop web --icon vite --autostart --port 8743 --ready-port 8743 --ready-timeout 10s -- ./serve.sh 8743
shop queue --icon redis --autostart -- ./loop.sh 3 0 jobs waiting
shop worker --icon rust --autostart -- ./loop.sh 2 job done
shop payments --icon rust --autostart --restart never -- ./payments.sh
shop search --icon docker --no-autostart -- ./loop.sh 4 index warm

(cd "$blog" && sandboxed muster add site --icon astro --autostart -- ./loop.sh 5 rebuilt 12 pages > /dev/null)
(cd "$blog" && sandboxed muster add preview --icon nodedotjs --autostart -- ./loop.sh 6 preview served > /dev/null)
sandboxed muster up -p blog > /dev/null

(cd "$shop" && sandboxed "$node" "$here/agent.mjs" claude migrations "orders table migrated, db is on 8741")
(cd "$shop" && sandboxed "$node" "$here/agent.mjs" codex payments-fix "retry the gateway once before payments exits")

(cd "$shop" && source "$DEMO_SANDBOX" && exec muster view --no-open) > "$raw/view.out" 2>&1 &
view_pid=$!
wait_for "$raw/view.out" "http://127.0.0.1:"
url=$(grep -o 'http://127\.0\.0\.1:[0-9]*/v/[^ ]*' "$raw/view.out" | head -1)

DEMO_VIEW_URL=$url "$node" "$here/page.mjs" "$raw" "$DEMO_COMMANDS" "$raw/stop" &
page_pid=$!
for _ in $(seq $((startup_timeout_s * 10))); do
  [ -e "$raw/ready" ] && break
  kill -0 "$page_pid" 2> /dev/null || { echo "the page recorder exited early" >&2; exit 1; }
  sleep 0.1
done
[ -e "$raw/ready" ] || { echo "the page recorder never saw the floor" >&2; exit 1; }

(cd "$repo" && PATH=$tools:$PATH vhs docs/view-demo/view.tape)

touch "$raw/stop"
wait "$page_pid"
page_pid=''
cp "$DEMO_DIR"/mark-* "$raw/"
cleanup
view_pid=''

"$node" "$here/timeline.mjs" "$raw" "$out/terminal.mp4" "$public" "$show_after_mark_ms"
ffmpeg -loglevel error -y -f concat -safe 0 -i "$raw/frames.txt" -fps_mode cfr -r 30 \
  -c:v libx264 -preset slow -crf 14 -pix_fmt yuv420p "$public/page.mp4"
cp "$out/terminal.mp4" "$public/terminal.mp4"
echo "recorded into $public"
