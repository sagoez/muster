# muster's prompt hook for fish. Load it from ~/.config/fish/config.fish with:
#   muster shell-init fish | source
if status is-interactive; and not set -q __muster_hook
    set -g __muster_hook 1
    set -g __muster_bin @MUSTER_BIN@
    set -g __muster_notes @MUSTER_NOTES@
    # The exported count survives an exec, so a reload never reuses a load id.
    set -l loads 0
    string match -qr '^[0-9]{1,9}$' -- "$@MUSTER_LOADS_ENV@"; and set loads $@MUSTER_LOADS_ENV@
    set -gx @MUSTER_LOADS_ENV@ (math $loads + 1)
    set -l tick (command date +%N 2>/dev/null)
    string match -qr '^[0-9]{9}$' -- "$tick"; or set tick (random 100000000 999999999)
    set -g __muster_load $@MUSTER_LOADS_ENV@$tick
    set -g __muster_seq 0
    set -gx @MUSTER_SHELL_ENV@ $fish_pid

    function __muster_prompt --on-event fish_prompt
        set -l status_kept $status
        set __muster_seq (math $__muster_seq + 1)
        for note in $__muster_notes/$fish_pid-*.note
            test -f $note; or continue
            while read -l line
                printf '%s\n' "$line"
            end <$note
            command rm -f -- $note
        end
        if test -x $__muster_bin
            command $__muster_bin @MUSTER_REPORT@ --pid $fish_pid --load $__muster_load \
                --seq $__muster_seq </dev/null >/dev/null 2>&1 &
            disown 2>/dev/null
        end
        return $status_kept
    end
end
