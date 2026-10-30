# VHS tape for the README demo. Build first, then render from the repo root:
#   cargo build
#   mise exec vhs@0.11.0 ttyd@latest -- vhs docs/demo.tape
# Runs the muster in $MUSTER_BIN_DIR (target/debug by default) with every XDG
# directory under $DEMO_DIR (a fresh mktemp dir by default) and no HERDR_*
# variables, so the real database, service and herdr session stay untouched.
# It needs python3 off mise's shims and port 8731 free, and stops the service
# and every process it started before it removes $DEMO_DIR.

Output docs/muster-demo.gif

Set Shell bash
Set FontSize 15
Set Width 1200
Set Height 640
Set Padding 16
Set Framerate 20
Set TypingSpeed 35ms
Set Theme "GruvboxDark"

Hide
Type `export PATH="${MUSTER_BIN_DIR:-$PWD/target/debug}:$PATH" TERM=xterm-256color D="${DEMO_DIR:-$(mktemp -d)}"`
Enter
Type `unset NO_COLOR MUSTER_PROJECT $(compgen -e | grep '^HERDR_'); PATH=$(tr : '\n' <<< "$PATH" | grep -v /mise/shims | paste -sd:)`
Enter
Type `export XDG_STATE_HOME=$D/state XDG_CONFIG_HOME=$D/config XDG_DATA_HOME=$D/data XDG_RUNTIME_DIR=$D/run && mkdir -p -m 700 $D/run $D/shop && cd $D`
Enter
Type `printf '#!/bin/sh\nn=0\nwhile true; do n=$((n + 1)); echo "$(date +%%T) job $n done"; sleep 1; done\n' > shop/worker.sh && chmod +x shop/worker.sh`
Enter
Type `muster config set desktop-notifications false > /dev/null && eval "$(muster shell-init bash)" && cd shop && clear`
Enter
Sleep 1s
Show

Type "muster add web --autostart --port 8731 --ready-port 8731 --ready-timeout 10s -- python3 -m http.server 8731"
Enter
Sleep 2s
Type "muster add worker --autostart -- ./worker.sh"
Enter
Sleep 2s

Type "muster up"
Enter
Sleep 3.5s

Hide
Type "curl -s http://localhost:8731/ > /dev/null; clear"
Enter
Show

Type "muster logs worker -n 5"
Enter
Sleep 3s

Type "muster attach web"
Enter
Sleep 4s
Ctrl+\
Sleep 1.5s

Hide
Type "clear"
Enter
Show

Type "muster ps"
Enter
Sleep 4.5s

Hide
Type "clear"
Enter
Show

Type "muster down"
Enter
Sleep 2.5s

Type "cd .."
Enter
Sleep 1s
Type "cd shop"
Enter
Sleep 3s
Enter
Sleep 2.5s

Type "cd .."
Enter
Sleep 1.5s
Type "muster ps -p shop"
Enter
Sleep 5s

Hide
Type `muster down -p shop > /dev/null; muster service stop > /dev/null; cd / && rm -rf "$D"`
Enter
Sleep 2s
