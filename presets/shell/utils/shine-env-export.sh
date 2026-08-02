#!/bin/bash
# Export a shine env variable into the current shell session.
# Usage: shine-env-export KEY [--as ALIAS]
eval "$(shine env secret export "$@")"
