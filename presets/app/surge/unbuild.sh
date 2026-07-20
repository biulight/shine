#!/usr/bin/env bash
set -euo pipefail

# Placeholder artifact teardown script for the `surge` app preset.
#
# `shine app unbuild surge` (and, best-effort, `shine app uninstall surge`) runs
# this to reverse what `build.sh` did — remove the local `#!include` entries it
# added to the active Surge profile's [Proxy]/[Rule] sections. The real,
# provider-aware logic is intentionally NOT shipped in shine core (see ADR 0012)
# — it belongs in an overlay's presets/app/surge/unbuild.sh, which wins over this
# placeholder when linked.
#
# Without an overlay there is nothing to reverse, so this placeholder prints a
# hint and exits successfully. Everything below `exit 0` is an inert, commented
# reference implementation (the canonical overlay lives in a private repo, so it
# ships here as the public "how to write your overlay unbuild.sh" example).
echo "shine: no overlay unbuild.sh for surge — nothing to reverse."
echo "shine: provide presets/app/surge/unbuild.sh in your overlay to enable teardown."
exit 0

# ═══════════════════════════════════════════════════════════════════════════════
# EXAMPLE overlay implementation — reference only; everything below is inert.
#
# Copy this into your overlay at  <overlay>/app/surge/unbuild.sh , delete the
# placeholder above, and adapt it. shine injects the active `[env]` table, so it
# reads the same profile path build.sh used:
#     shine env set SURGE_PROFILE /abs/path/to/Profile.conf
# Idempotent: removes the local file from each section's `#!include` list, and
# only writes when something actually changes.
#
#   : "${SURGE_PROFILE:?run: shine env set SURGE_PROFILE /abs/path/to/Profile.conf}"
#   case "$SURGE_PROFILE" in
#     "~")   SURGE_PROFILE="$HOME" ;;
#     "~/"*) SURGE_PROFILE="$HOME/${SURGE_PROFILE#\~/}" ;;
#   esac
#   [[ -f "$SURGE_PROFILE" ]] || { echo "error: not a file: $SURGE_PROFILE" >&2; exit 1; }
#
#   tmp=$(mktemp "$(dirname "$SURGE_PROFILE")/.surge-unpatch.XXXXXX"); trap 'rm -f "$tmp"' EXIT
#
#   # Drop `local-proxies.conf` from [Proxy], `local-proxy-groups.conf` from
#   # [Proxy Group], and `local-rules.conf` from [Rule], tracking the current
#   # section header; leave every other line untouched. If a section's `#!include`
#   # list becomes empty, keep the (now bare) directive.
#   awk '
#     /^\[/            { section=$0; sub(/[ \t]+$/,"",section); print; next }
#     /^#!include[ \t]/{
#       want=""
#       if (section=="[Proxy]") want="local-proxies.conf"
#       else if (section=="[Proxy Group]") want="local-proxy-groups.conf"
#       else if (section=="[Rule]") want="local-rules.conf"
#       if (want!="") {
#         match($0,/^#!include[ \t]+/); pre=substr($0,1,RLENGTH); pay=substr($0,RLENGTH+1)
#         n=split(pay,p,/[ \t]*,[ \t]*/); rest=""
#         for(i=1;i<=n;i++){t=p[i]; gsub(/^[ \t]+|[ \t]+$/,"",t); if(t==""||t==want)continue
#           rest=(rest=="")?t:rest", "t}
#         $0=(rest=="")?pre:pre rest
#       }
#     }
#     { print }
#   ' "$SURGE_PROFILE" > "$tmp"
#
#   if cmp -s "$SURGE_PROFILE" "$tmp"; then
#     echo "surge: profile already un-patched — no change ($SURGE_PROFILE)"; rm -f "$tmp"; trap - EXIT
#   else
#     chmod 644 "$tmp"; mv "$tmp" "$SURGE_PROFILE"; trap - EXIT; echo "surge: un-patched $SURGE_PROFILE"
#   fi
#
#   cli="/Applications/Surge.app/Contents/Applications/surge-cli"
#   [[ -x "$cli" ]] && { "$cli" reload || echo "surge: surge-cli reload failed (is Surge running?)" >&2; }
# ═══════════════════════════════════════════════════════════════════════════════
