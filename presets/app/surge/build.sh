#!/usr/bin/env bash
set -euo pipefail

# Placeholder artifact build script.
#
# `shine app build surge` runs this to patch the active Surge profile's
# per-section `#!include` lines so they also include the local files that
# `shine app install surge` copies next to the profile. The real, provider-aware
# patch logic lives in an overlay's presets/app/surge/build.sh (see the shine
# overlay), which takes priority over this built-in placeholder when linked.
#
# Without an overlay there is nothing to patch, so this placeholder just prints a
# hint and exits successfully.
echo "shine: no overlay build.sh for surge — nothing to patch."
echo "shine: provide presets/app/surge/build.sh in your overlay to enable patching."
