# muster's prompt hook for bash. Load it from ~/.bashrc with:
#   eval "$(muster shell-init bash)"
# A pane `muster open` made only shows a process, so its shell never
# occupies the project.
if [[ $- == *i* && -z ${__muster_hook-} && -z ${@MUSTER_OPEN_ENV@+set} ]]; then
    __muster_hook=1
    __muster_bin=@MUSTER_BIN@
    __muster_notes=@MUSTER_NOTES@
    # The load's start in nanoseconds, then the pid of a process it spawns,
    # which breaks a tie of a coarse clock: a later load always compares
    # greater, so a late report from before an exec is told apart.
    __muster_load=${EPOCHREALTIME-}
    __muster_load=${__muster_load//[!0-9]/}
    if [[ $__muster_load =~ ^[0-9]{1,16}$ ]]; then
        __muster_load=${__muster_load}000
    else
        __muster_load=$(command date +%s 2>/dev/null)
        if [[ $__muster_load =~ ^[0-9]{1,10}$ ]]; then
            __muster_load=${__muster_load}000000000
        else
            __muster_load=0
        fi
    fi
    __muster_load_pid=$(command sh -c 'echo "$$"' 2>/dev/null)
    [[ $__muster_load_pid =~ ^[0-9]{1,10}$ ]] || __muster_load_pid=$$
    __muster_seq=0
    # The folder of the last prompt, and the first prompt's time, in
    # nanoseconds, at which the shell was seen in it: a project registered
    # after that was registered around the shell, not walked into.
    __muster_pwd=
    __muster_arrived=
    export @MUSTER_SHELL_ENV@=$$

    __muster_prompt() {
        local status=$? note line failglob= now=${EPOCHREALTIME-}
        local -a arrived=()
        now=${now//[!0-9]/}
        [[ $now =~ ^[0-9]{16}$ ]] && now=${now}000 || now=
        if [[ $PWD != "$__muster_pwd" ]]; then
            __muster_pwd=$PWD
            __muster_arrived=$now
        fi
        [[ -n $__muster_arrived ]] && arrived=(--arrived "$__muster_arrived")
        __muster_seq=$((__muster_seq + 1))
        if shopt -q failglob; then
            failglob=1
            shopt -u failglob
        fi
        for note in "$__muster_notes"/"$$"-*.note; do
            [[ -f $note ]] || continue
            while IFS= read -r line || [[ -n $line ]]; do
                printf '%s\n' "$line"
            done < "$note"
            command rm -f -- "$note"
        done
        if [[ -n $failglob ]]; then
            shopt -s failglob
        fi
        (command "$__muster_bin" @MUSTER_REPORT@ --pid "$$" --load "$__muster_load" \
            --load-pid "$__muster_load_pid" --seq "$__muster_seq" ${arrived[@]+"${arrived[@]}"} \
            < /dev/null > /dev/null 2>&1 &)
        return "$status"
    }

    if ((BASH_VERSINFO[0] > 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] >= 1))); then
        PROMPT_COMMAND=(__muster_prompt "${PROMPT_COMMAND[@]}")
    else
        PROMPT_COMMAND="__muster_prompt${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
    fi
fi
