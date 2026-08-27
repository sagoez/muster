# VHS tape for the README demo. Render with:
#   mise exec vhs@0.11.0 ttyd@latest -- vhs docs/demo.tape
# Runs ~/.cargo/bin/muster against an isolated copy of the repo's muster.yml in
# /tmp, with XDG dirs isolated too, so the real project registry, session store,
# and workspace config all stay untouched (the delete demo mutates only the copy).

Output docs/muster-demo.gif

Set Shell bash
Set FontSize 17
Set Width 1200
Set Height 720
Set Padding 16
Set TypingSpeed 55ms
Set Theme "GruvboxDark"

Hide
Type "export PATH=$HOME/Projects/muster/target/release:$HOME/.cargo/bin:$PATH XDG_CONFIG_HOME=/tmp/muster-vhs/xdg-config XDG_STATE_HOME=/tmp/muster-vhs/xdg-state"
Enter
Type "rm -rf /tmp/muster-vhs /tmp/muster-blog /tmp/muster-demo"
Enter
Type "mkdir -p /tmp/muster-blog /tmp/muster-demo"
Enter
Type "cp $HOME/Projects/muster/muster.yml /tmp/muster-demo/muster.yml"
Enter
Type "cd /tmp/muster-blog; clear"
Enter
Show

# CLI: scaffold, validate, and check a new project.
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
Type "cd /tmp/muster-demo; clear"
Enter
Show

# Launch the workspace TUI: agents, terminals, and commands side by side.
Type "muster"
Enter
Sleep 2s

# Attach the login-shell terminal and run a couple of commands.
Down
Sleep 500ms
Type "s"
Sleep 800ms
Enter
Sleep 900ms
Type "echo hello from muster"
Enter
Sleep 900ms
Type "ls"
Enter
Sleep 1.2s

# Leader detaches back to the sidebar; start the clock command.
Ctrl+a
Sleep 300ms
Type "h"
Sleep 500ms
Down
Sleep 400ms
Type "s"
Sleep 2.5s

# Launch a live Claude agent session and attach to it.
Up
Sleep 300ms
Up
Sleep 400ms
Type "s"
Sleep 900ms
Enter
Sleep 8s

# Detach, then open the keybinding overlay.
Ctrl+a
Sleep 300ms
Type "h"
Sleep 800ms
Type "?"
Sleep 2.5s
Escape
Sleep 500ms

# Delete the running clock command from muster.yml: pick it, confirm, gone.
Down
Sleep 300ms
Down
Sleep 400ms
Type "d"
Sleep 1.8s
Enter
Sleep 1.8s

Type "q"
Sleep 800ms
