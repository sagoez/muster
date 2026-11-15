# Sourced, never run: points muster at the sandbox record.sh made, its state in
# $DEMO_DIR and its home in $DEMO_HOME. record.sh sources it in a subshell
# around each muster call, and the tape's shell sources it before it shows, so
# the service, its processes, the view and the typed commands all share one
# isolated state. HOME moves too, so the page and the prompt show the projects
# as ~/code/<name>.

export HOME=$DEMO_HOME
export XDG_STATE_HOME=$DEMO_DIR/s XDG_CONFIG_HOME=$DEMO_DIR/c XDG_DATA_HOME=$DEMO_DIR/d XDG_RUNTIME_DIR=$DEMO_DIR/r
export TERM=xterm-256color
unset NO_COLOR MUSTER_PROJECT $(compgen -e | grep '^HERDR_')
# The stand-ins run the system python3, not a mise shim that would look for its
# config under the moved HOME.
PATH=$MUSTER_BIN_DIR:$(tr : '\n' <<< "$PATH" | grep -v /mise/ | paste -sd:)
export PATH

# The tape's shell only: a plain prompt, and each command line's start time
# appended to $DEMO_COMMANDS, which makes up the edit's timeline. PS0 is a
# prompt string, so it holds no backslash a prompt escape would eat.
if [[ $- == *i* ]]; then
  unset PROMPT_COMMAND
  HISTFILE=/dev/null
  HISTCONTROL=
  set -o history
  PS1='\[\e[38;5;179m\]\w\[\e[0m\] $ '
  unset HISTTIMEFORMAT
  PS0='$(echo "$(date +%s%3N) $(history 1 | sed -E "s/^ *[0-9]+ *//")" >> "$DEMO_COMMANDS")'
  # mark <name>: logs when the tape shows or hides the terminal, so the footage can be placed on the timeline.
  mark() { date +%s%3N > "$DEMO_DIR/mark-$1"; }
fi
