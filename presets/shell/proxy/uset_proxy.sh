#!/bin/bash
# Remove proxy environment variables for the current shell session.
# Yarn proxy settings are also cleared because setproxy may update Yarn config.
# Usage: source usetproxy

if ! (return 0 2>/dev/null); then
    echo "usetproxy must be sourced to update the current shell environment." >&2
    echo "Run: source usetproxy" >&2
    echo "If you just installed it, first reload your shell config or open a new shell." >&2
    exit 1
fi

echo "🔄 Removing proxy configuration..."

# Show the current proxy settings, if any.
if [ -n "$http_proxy" ] || [ -n "$HTTP_PROXY" ]; then
    echo "🔍 Current detected proxy settings:"
    [ -n "$http_proxy" ] && echo "  http_proxy: $http_proxy"
    [ -n "$https_proxy" ] && echo "  https_proxy: $https_proxy"
    [ -n "$HTTP_PROXY" ] && echo "  HTTP_PROXY: $HTTP_PROXY"
    [ -n "$HTTPS_PROXY" ] && echo "  HTTPS_PROXY: $HTTPS_PROXY"
    [ -n "$all_proxy" ] && echo "  all_proxy: $all_proxy"
    [ -n "$ALL_PROXY" ] && echo "  ALL_PROXY: $ALL_PROXY"
    echo ""
else
    echo "ℹ️ No proxy environment variables were detected"
fi

# Clear session environment variable proxies.
echo "🧹 Clearing session environment variable proxies..."
unset http_proxy
unset https_proxy
unset HTTP_PROXY
unset HTTPS_PROXY
unset all_proxy
unset ALL_PROXY
unset no_proxy
unset NO_PROXY
unset npm_config_proxy
unset npm_config_https_proxy
unset npm_config_registry
unset NPM_CONFIG_PROXY
unset NPM_CONFIG_HTTPS_PROXY
unset NPM_CONFIG_REGISTRY

# Clear Yarn proxy settings, if available.
if command -v yarn >/dev/null 2>&1; then
    echo "🧶 Clearing Yarn proxy config..."
    echo "  Yarn proxy settings are persistent; removing the entries set by setproxy."
    yarn_version=$(yarn --version)
    case "$yarn_version" in
        1.*)
            yarn config delete proxy 2>/dev/null || echo "  ℹ️ Yarn proxy was not set or has already been cleared"
            yarn config delete https-proxy 2>/dev/null || echo "  ℹ️ Yarn https-proxy was not set or has already been cleared"
            ;;
        2.*|3.*|4.*)
            yarn config delete httpProxy 2>/dev/null || echo "  ℹ️ Yarn httpProxy was not set or has already been cleared"
            yarn config delete httpsProxy 2>/dev/null || echo "  ℹ️ Yarn httpsProxy was not set or has already been cleared"
            ;;
        *)
            echo "⚠️ Unknown Yarn version: ${yarn_version}; trying a generic configuration"
            yarn config delete proxy 2>/dev/null || echo "  ℹ️ Yarn proxy was not set or has already been cleared"
            yarn config delete https-proxy 2>/dev/null || echo "  ℹ️ Yarn https-proxy was not set or has already been cleared"
            ;;
    esac
else
    echo "ℹ️ Yarn is not installed; skipping"
fi

echo ""
echo "✅ Proxy settings have been removed!"
echo ""
echo "📝 Notes:"
echo "  - Environment variable proxies have been cleared (current terminal session only)"
echo "  - Git/npm/pnpm global config was not modified"
echo "  - Yarn proxy config was cleared if Yarn was available"
echo "  - To set the proxy again, run: source setproxy"
