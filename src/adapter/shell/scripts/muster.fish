# muster's prompt hook for fish. Load it from ~/.config/fish/config.fish with:
#   muster shell-init fish | source
if status is-interactive; and not set -q __muster_hook
    set -g __muster_hook 1
    set -g __muster_bin @MUSTER_BIN@
    set -g __muster_notes @MUSTER_NOTES@
    set -g __muster_load (random 1 999999999)(random 0 999999999)
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
