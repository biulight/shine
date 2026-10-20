#!/bin/bash
# Export a shine env variable into the current shell session.
# Usage: shine-env-export KEY [--as ALIAS]
eval "$(
    if _shine_export_code="$(shine env secret export "$@")"; then
        printf '%s\n' "$_shine_export_code"
    else
        printf 'return %d\n' "$?"
    fi
)"
