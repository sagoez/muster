# muster's prompt hook for zsh. Load it from ~/.zshrc with:
#   eval "$(muster shell-init zsh)"
if [[ -o interactive ]] && (( ! ${+__muster_hook} )); then
    typeset -g __muster_hook=1
    typeset -g __muster_bin=@MUSTER_BIN@
    typeset -g __muster_notes=@MUSTER_NOTES@
    typeset -gi __muster_seq=0
    export @MUSTER_SHELL_ENV@=$$

    __muster_precmd() {
        local -i status_kept=$?
        emulate -L zsh
        local note line
        (( __muster_seq += 1 ))
        for note in "$__muster_notes"/"$$"-*.note(N.on); do
            while IFS= read -r line || [[ -n $line ]]; do
                printf '%s\n' "$line"
            done < "$note"
            command rm -f -- "$note"
        done
        (command "$__muster_bin" @MUSTER_REPORT@ --pid "$$" --seq "$__muster_seq" \
            < /dev/null > /dev/null 2>&1 &!)
        return status_kept
    }

    autoload -Uz add-zsh-hook
    add-zsh-hook precmd __muster_precmd
fi
