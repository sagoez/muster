# VHS tape for the README demo. Build first, then render from the repo root:
#   cargo build
#   mise exec vhs@0.11.0 ttyd@latest -- vhs docs/demo.tape
# Runs target/debug/muster with every XDG directory under /tmp/muster-vhs, so
# the real database and running service stay untouched.

Output docs/muster-demo.gif

Set Shell bash
Set FontSize 16
Set Width 1200
Set Height 680
Set Padding 16
Set TypingSpeed 50ms
Set Theme "GruvboxDark"

Hide
Type `export PATH="$PWD/target/debug:$PATH" TERM=xterm-256color`
Enter
Type "unset NO_COLOR; export XDG_CONFIG_HOME=/tmp/muster-vhs/config XDG_STATE_HOME=/tmp/muster-vhs/state XDG_DATA_HOME=/tmp/muster-vhs/data"
Enter
Type "rm -rf /tmp/muster-vhs && mkdir -p /tmp/muster-vhs/shop && cd /tmp/muster-vhs/shop"
Enter
Type "muster config set desktop-notifications false > /dev/null"
Enter
Type `muster add web --autostart -- 'while true; do echo "$(date +%T) GET /api/cart 200 $((RANDOM % 40 + 5))ms"; sleep 0.7; done' > /dev/null`
Enter
Type `muster add worker --autostart -- 'trap "sleep 1.5; exit 0" TERM; n=0; while true; do n=$((n + 1)); echo "$(date +%T) job $n done"; sleep 1; done' > /dev/null && clear`
Enter
Show

Type "muster up"
Enter
Sleep 3s

Type "muster ps"
Enter
Sleep 3s

Type "muster attach"
Enter
Sleep 2s
Down
Sleep 0.6s
Up
Sleep 0.8s
Enter
Sleep 4s
Ctrl+\
Sleep 2s

Type "muster down"
Enter
Sleep 4.5s

Hide
Type "muster service stop > /dev/null; cd / && rm -rf /tmp/muster-vhs"
Enter
Sleep 1s
