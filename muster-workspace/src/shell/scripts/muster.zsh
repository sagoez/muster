# muster's prompt hook for zsh. Load it from ~/.zshrc with:
#   eval "$(muster shell-init zsh)"
# A pane `muster open` made only shows a process, so its shell never
# occupies the project.
if [[ -o interactive ]] && (( ! ${+__muster_hook} && ! ${+@MUSTER_OPEN_ENV@} )); then
    typeset -g __muster_hook=1
    typeset -g __muster_bin=@MUSTER_BIN@
    typeset -g __muster_notes=@MUSTER_NOTES@
    typeset -g __muster_load __muster_load_pid
    typeset -gi __muster_seq=0
    # The folder of the last prompt, and the first prompt's time, in
    # nanoseconds, at which the shell was seen in it: a project registered
    # after that was registered around the shell, not walked into.
    typeset -g __muster_pwd= __muster_arrived=
    export @MUSTER_SHELL_ENV@=$$

    # The load's start in nanoseconds, then the pid of a process it spawns,
    # which breaks a tie of a coarse clock: a later load always compares
    # greater, so a late report from before an exec is told apart.
    __muster_load_id() {
        emulate -L zsh
        local now fraction
        zmodload -F zsh/datetime p:EPOCHREALTIME 2>/dev/null && now=$EPOCHREALTIME
        if [[ $now == <->.<-> && ${#${now%.*}} -le 10 ]]; then
            fraction=${now#*.}
            __muster_load=${now%.*}${(r:9::0:)fraction}
        else
            now=$(command date +%s 2>/dev/null)
            if [[ $now == <-> && ${#now} -le 10 ]]; then
                __muster_load=${now}000000000
            else
                __muster_load=0
            fi
        fi
        __muster_load_pid=$(command sh -c 'echo "$$"' 2>/dev/null)
        [[ $__muster_load_pid == <-> && ${#__muster_load_pid} -le 10 ]] || __muster_load_pid=$$
    }
    __muster_load_id
    unfunction __muster_load_id

    __muster_precmd() {
        local -i status_kept=$?
        emulate -L zsh
        local note line now=${EPOCHREALTIME-}
        local -a arrived
        if [[ $PWD != "$__muster_pwd" ]]; then
            __muster_pwd=$PWD
            __muster_arrived=
            if [[ $now == <->.<-> && ${#${now%.*}} -le 10 ]]; then
                __muster_arrived=${now%.*}${(r:9::0:)${now#*.}}
            fi
        fi
        [[ -n $__muster_arrived ]] && arrived=(--arrived $__muster_arrived)
        (( __muster_seq += 1 ))
        for note in "$__muster_notes"/"$$"-*.note(N.on); do
            while IFS= read -r line || [[ -n $line ]]; do
                printf '%s\n' "$line"
            done < "$note"
            command rm -f -- "$note"
        done
        (command "$__muster_bin" @MUSTER_REPORT@ --pid "$$" --load "$__muster_load" \
            --load-pid "$__muster_load_pid" --seq "$__muster_seq" "${arrived[@]}" \
            < /dev/null > /dev/null 2>&1 &!)
        return status_kept
    }

    autoload -Uz add-zsh-hook
    add-zsh-hook precmd __muster_precmd
fi
