# VHS tape for the README demo. Render with:
#   mise exec vhs@0.11.0 ttyd@latest -- vhs docs/demo.tape
# Records against ~/.cargo/bin/muster with XDG dirs isolated to /tmp,
# so the real project registry and session store stay untouched.

Output docs/muster-demo.gif

Set Shell bash
Set FontSize 17
Set Width 1200
Set Height 720
Set Padding 16
Set TypingSpeed 55ms
Set Theme "GruvboxDark"

Hide
Type "export PATH=$HOME/.cargo/bin:$PATH XDG_CONFIG_HOME=/tmp/muster-vhs/xdg-config XDG_STATE_HOME=/tmp/muster-vhs/xdg-state; rm -rf /tmp/muster-vhs /tmp/demo; mkdir -p /tmp/demo/blog; cd /tmp/demo/blog; clear"
Enter
Show

Type "muster init"
Enter
Sleep 1.5s
Type "muster check"
Enter
Sleep 1.2s
Type "muster doctor"
Enter
Sleep 3s

Hide
Type "cd ~/Projects/muster; clear"
Enter
Show

Type "muster"
Enter
Sleep 2s

# shell terminal: start it, attach, run a command
Down
Sleep 500ms
Type "s"
Sleep 1s
Enter
Sleep 1s
Type "echo hello from muster"
Enter
Sleep 1.2s

# leader back to the sidebar, start the clock command
Ctrl+a
Sleep 300ms
Type "h"
Sleep 500ms
Down
Sleep 400ms
Type "s"
Sleep 2.5s

# launch a live claude session and attach to it
Up
Sleep 300ms
Up
Sleep 400ms
Type "s"
Sleep 1s
Enter
Sleep 9s

# back to the sidebar: the named session row is live
Ctrl+a
Sleep 300ms
Type "h"
Sleep 2.5s

Type "q"
Sleep 800ms
