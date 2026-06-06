#!/bin/bash
# Copy a file's contents to the local clipboard via OSC52.
# Usage: copyfile <file>

copyfile_main() {
    if [[ $# -eq 0 ]]; then
        echo "Usage: copyfile <file>" >&2
        return 1
    fi

    local file="$1"

    if [[ ! -f "$file" ]]; then
        echo "copyfile: not a file: $file" >&2
        return 1
    fi

    printf '\033]52;c;%s\a' "$(base64 < "$file" | tr -d '\n')"
    echo "Copied file contents to local clipboard via OSC52: $file"
}

copyfile_main "$@"
