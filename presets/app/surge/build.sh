#!/usr/bin/env bash
set -euo pipefail

# Placeholder artifact build script for the `surge` app preset.
#
# `shine app build surge` runs this to patch the active Surge profile's
# per-section `#!include` lines so they also load the local files that
# `shine app install surge` copies next to the profile. The real, provider-aware
# patch logic is intentionally NOT shipped in shine core (see ADR 0009) — it
# belongs in an overlay's presets/app/surge/build.sh, which wins over this
# placeholder when linked.
#
# Without an overlay there is nothing to patch, so this placeholder prints a hint
# and exits successfully. Everything below `exit 0` is an inert, commented
# reference implementation (the canonical overlay lives in a private repo, so it
# ships here as the public "how to write your overlay build.sh" example).
echo "shine: no overlay build.sh for surge — nothing to patch."
echo "shine: provide presets/app/surge/build.sh in your overlay to enable patching."
exit 0

# ═══════════════════════════════════════════════════════════════════════════════
# EXAMPLE overlay implementation — reference only; everything below is inert.
#
# Copy this into your overlay at  <overlay>/app/surge/build.sh , delete the
# placeholder above, and adapt it. shine injects the active `[env]` table into
# the build, so point it at your profile once:
#     shine env set SURGE_PROFILE /abs/path/to/Profile.conf
# It is idempotent (never double-appends) and only writes when something changes.
#
#   : "${SURGE_PROFILE:?run: shine env set SURGE_PROFILE /abs/path/to/Profile.conf}"
#
#   # shine injects `[env]` values verbatim, so expand a leading `~` ourselves.
#   case "$SURGE_PROFILE" in
#     "~")   SURGE_PROFILE="$HOME" ;;
#     "~/"*) SURGE_PROFILE="$HOME/${SURGE_PROFILE#\~/}" ;;
#   esac
#   [[ -f "$SURGE_PROFILE" ]] || { echo "error: not a file: $SURGE_PROFILE" >&2; exit 1; }
#   dir=$(cd "$(dirname "$SURGE_PROFILE")" && pwd)
#
#   # Only patch a section when its local file exists beside the profile
#   # (installed by `shine app install surge`).
#   proxies=""; rules=""
#   [[ -f "$dir/local-proxies.conf" ]] && proxies="local-proxies.conf"
#   [[ -f "$dir/local-rules.conf"   ]] && rules="local-rules.conf"
#   [[ -n "$proxies$rules" ]] || { echo "error: run: shine app install surge" >&2; exit 1; }
#
#   tmp=$(mktemp "$dir/.surge-patch.XXXXXX"); trap 'rm -f "$tmp"' EXIT
#
#   # Patch the first `#!include` in each of [Proxy]/[Rule] to also load the local
#   # file, unless already present. [Proxy] appends (proxy defs are order-free);
#   # [Rule] prepends so local rules win (Surge matches top-to-bottom, first match
#   # wins). Tracks the current section header; every other line is untouched.
#   awk -v proxies="$proxies" -v rules="$rules" '
#     /^\[/            { section=$0; sub(/[ \t]+$/,"",section); print; next }
#     /^#!include[ \t]/{
#       want=""; prepend=0
#       if (section=="[Proxy]") want=proxies
#       else if (section=="[Rule]") { want=rules; prepend=1 }
#       if (want!="" && !(want in done)) {
#         match($0,/^#!include[ \t]+/); pre=substr($0,1,RLENGTH); pay=substr($0,RLENGTH+1)
#         # Drop any existing occurrence of the local file, then re-insert it at the
#         # front ([Rule]) or back ([Proxy]) — also moves a misplaced prior entry.
#         n=split(pay,p,/[ \t]*,[ \t]*/); rest=""
#         for(i=1;i<=n;i++){t=p[i]; gsub(/^[ \t]+|[ \t]+$/,"",t); if(t==""||t==want)continue
#           rest=(rest=="")?t:rest", "t}
#         pay=prepend?((rest=="")?want:want", "rest):((rest=="")?want:rest", "want)
#         $0=pre pay; done[want]=1
#       }
#     }
#     { print }
#   ' "$SURGE_PROFILE" > "$tmp"
#
#   if cmp -s "$SURGE_PROFILE" "$tmp"; then
#     echo "surge: profile already patched — no change ($SURGE_PROFILE)"; rm -f "$tmp"; trap - EXIT
#   else
#     chmod 644 "$tmp"; mv "$tmp" "$SURGE_PROFILE"; trap - EXIT; echo "surge: patched $SURGE_PROFILE"
#   fi
#
#   cli="/Applications/Surge.app/Contents/Applications/surge-cli"
#   [[ -x "$cli" ]] && { "$cli" reload || echo "surge: surge-cli reload failed (is Surge running?)" >&2; }
# ═══════════════════════════════════════════════════════════════════════════════
