# The terminal half of the muster view demo. record.sh runs it from the repo
# root once the sandboxed stack and the page recorder are up; it needs
# DEMO_SANDBOX, DEMO_DIR, DEMO_HOME, DEMO_COMMANDS and MUSTER_BIN_DIR from there. Every
# command runs for real against the sandbox, and the shell logs when each one
# starts. The terminal stays shown from `mark show` to `mark hide`, so the
# footage maps onto wall time with no cut in between.

Output docs/view-demo/out/terminal.mp4

Set Shell bash
Set FontFamily "iA Writer Mono S"
Set FontSize 22
Set Width 900
Set Height 540
Set Padding 24
Set Framerate 30
Set TypingSpeed 45ms
Set CursorBlink false
# The page's own tones, so the terminal sits in the edit as part of it.
Set Theme { "name": "muster dark", "background": "#1a1b1f", "foreground": "#f1f1f3", "cursor": "#d6b47a", "selection": "#2c2d33", "black": "#1a1b1f", "red": "#ff6b4a", "green": "#4fd39a", "yellow": "#f5b544", "blue": "#4d8dff", "magenta": "#c792ea", "cyan": "#5ccfe6", "white": "#a1a2a8", "brightBlack": "#6b6c73", "brightRed": "#ff8a6e", "brightGreen": "#7be3b5", "brightYellow": "#f7c76d", "brightBlue": "#7aa8ff", "brightMagenta": "#d7a8f0", "brightCyan": "#8adcef", "brightWhite": "#f1f1f3" }

Hide
Type@5ms `source "$DEMO_SANDBOX" && cd ~/code/shop`
Enter
Sleep 1s
Type@5ms "mark show && clear"
Enter
Sleep 300ms
Show

Sleep 800ms
Type "muster up"
Enter
Sleep 4.5s

Ctrl+L
Type "muster ps"
Enter
Sleep 3s

Type "muster stop worker"
Enter
Sleep 2.5s

Ctrl+L
Type "touch crash"
Enter
Sleep 3s

Type "muster logs payments"
Enter
Sleep 3.5s

Ctrl+L
Type "rm crash && muster restart payments"
Enter
Sleep 4s

Ctrl+L
Type@30ms `muster note set deploy "api is on 8742, ready for the payments fix"`
Enter
Sleep 4s

Type "muster start search"
Enter
Sleep 3.5s

Hide
Type@5ms "mark hide"
Enter
Sleep 300ms
