#!/bin/bash
# Remove all proxy environment variables and tool proxy settings.
# Clear system environment variables such as `http_proxy`, `https_proxy`, and `all_proxy`.
# Also clear the global proxy settings for Git, NPM, Yarn, and pnpm.
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

# Clear system environment variable proxies.
echo "🧹 Clearing system environment variable proxies..."
unset http_proxy
unset https_proxy
unset HTTP_PROXY
unset HTTPS_PROXY
unset all_proxy
unset ALL_PROXY
unset no_proxy
unset NO_PROXY

# Clear Git proxy settings.
echo "🔧 Clearing Git proxy..."
git config --global --unset http.proxy 2>/dev/null || echo "  ℹ️ Git http.proxy was not set or has already been cleared"
git config --global --unset https.proxy 2>/dev/null || echo "  ℹ️ Git https.proxy was not set or has already been cleared"

# Clear NPM proxy settings.
echo "📦 Clearing NPM proxy..."
npm config delete proxy 2>/dev/null || echo "  ℹ️ NPM proxy was not set or has already been cleared"
npm config delete https-proxy 2>/dev/null || echo "  ℹ️ NPM https-proxy was not set or has already been cleared"

# Clear Yarn proxy settings, if available.
if command -v yarn >/dev/null 2>&1; then
    echo "🧶 Clearing Yarn proxy..."
    yarn_version=$(yarn --version)
    case "$yarn_version" in
        1.*)
            yarn config delete proxy 2>/dev/null || echo "  ℹ️ Yarn proxy was not set or has already been cleared"
            yarn config delete https-proxy 2>/dev/null || echo "  ℹ️ Yarn https-proxy was not set or has already been cleared"
            ;;
        2.*|3.*)
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

# Clear pnpm proxy settings, if available.
if command -v pnpm >/dev/null 2>&1; then
    echo "📌 Clearing pnpm proxy..."
    pnpm config delete proxy 2>/dev/null || echo "  ℹ️ pnpm proxy was not set or has already been cleared"
    pnpm config delete https-proxy 2>/dev/null || echo "  ℹ️ pnpm https-proxy was not set or has already been cleared"
else
    echo "ℹ️ pnpm is not installed; skipping"
fi

echo ""
echo "✅ Proxy settings have been removed!"
echo ""
echo "📝 Notes:"
echo "  - Environment variable proxies have been cleared (current terminal session only)"
echo "  - Global proxy settings for Git/NPM/Yarn/pnpm have been cleared"
echo "  - To set the proxy again, run: source setproxy"
