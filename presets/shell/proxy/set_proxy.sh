#!/bin/bash
# shine-template: true
# Set proxy environment variables (`http_proxy`, `https_proxy`, `all_proxy`).
# Also configure tool proxies for Git, NPM, Yarn, and pnpm.
# The HTTP proxy port is controlled by `HTTP_PROXY_PORT` in `~/.shine/env.toml`.
# Usage: source setproxy [auto|sock5|http]
# In `auto` mode, SOCKS5 is preferred and HTTP is used as a fallback.

if ! (return 0 2>/dev/null); then
    echo "setproxy must be sourced to update the current shell environment." >&2
    echo "Run: source setproxy [auto|sock5|http]" >&2
    echo "If you just installed it, first reload your shell config or open a new shell." >&2
    exit 1
fi

HTTP_PROXY_PORT=@@HTTP_PROXY_PORT@@
SOCKS5_PROXY_PORT=@@SOCKS5_PROXY_PORT@@
PROXY_HOST=@@PROXY_HOST@@
HTTP_PROXY="http://${PROXY_HOST}:${HTTP_PROXY_PORT}"
SOCKS5_PROXY="socks5://${PROXY_HOST}:${SOCKS5_PROXY_PORT}"

echo "🚀 Setting up proxy configuration..."

# Check whether the SOCKS5 proxy is available.
check_socks5_proxy() {
    if command -v nc >/dev/null 2>&1 && nc -z ${PROXY_HOST} ${SOCKS5_PROXY_PORT} 2>/dev/null; then
        echo "🔍 Detected an open SOCKS5 proxy port with netcat"
        return 0
    fi
    if command -v lsof >/dev/null 2>&1 && lsof -i :${SOCKS5_PROXY_PORT} >/dev/null 2>&1; then
        echo "🔍 Detected the SOCKS5 proxy port being listened to with lsof"
        return 0
    fi
    if command -v netstat >/dev/null 2>&1 && netstat -an 2>/dev/null | grep -q ":${SOCKS5_PROXY_PORT}.*LISTEN"; then
        echo "🔍 Detected the SOCKS5 proxy port in LISTEN state with netstat"
        return 0
    fi
    if command -v telnet >/dev/null 2>&1 && timeout 2 telnet ${PROXY_HOST} ${SOCKS5_PROXY_PORT} </dev/null >/dev/null 2>&1; then
        echo "🔍 Detected a reachable SOCKS5 proxy with telnet"
        return 0
    fi
    return 1
}

# Set proxy function.
set_proxy() {
    local proxy_address=$1
    local proxy_type=$2

    export http_proxy="${proxy_address}"
    export https_proxy="${proxy_address}"
    export HTTP_PROXY="${proxy_address}"
    export HTTPS_PROXY="${proxy_address}"
    export all_proxy="${proxy_address}"
    export ALL_PROXY="${proxy_address}"

    export no_proxy="localhost,127.0.0.1,::1,.local"
    export NO_PROXY="localhost,127.0.0.1,::1,.local"

    # Git, NPM, and similar tools typically do not support SOCKS5, so use HTTP proxy uniformly.
    local tool_proxy="http://${PROXY_HOST}:${HTTP_PROXY_PORT}"

    echo "🔧 Configuring Git proxy..."
    git config --global http.proxy "${tool_proxy}"
    git config --global https.proxy "${tool_proxy}"

    if command -v npm >/dev/null 2>&1; then
        echo "📦 Configuring NPM proxy..."
        npm config set proxy "${tool_proxy}"
        npm config set https-proxy "${tool_proxy}"
        npm config set registry https://registry.npmjs.org/
    else
        echo "ℹ️ NPM is not installed; skipping"
    fi

    if command -v yarn >/dev/null 2>&1; then
        yarn_version=$(yarn --version)
        case "$yarn_version" in
            1.*)
                echo "🧶 Configuring Yarn@${yarn_version} proxy..."
                yarn config set proxy "${tool_proxy}"
                yarn config set https-proxy "${tool_proxy}"
                ;;
            2.*|3.*)
                echo "🧶 Configuring Yarn@${yarn_version} proxy..."
                yarn config set httpProxy "${tool_proxy}"
                yarn config set httpsProxy "${tool_proxy}"
                ;;
            *)
                echo "⚠️ Unknown Yarn version: ${yarn_version}; trying a generic configuration"
                yarn config set proxy "${tool_proxy}"
                yarn config set https-proxy "${tool_proxy}"
                ;;
        esac
    fi

    if command -v pnpm >/dev/null 2>&1; then
        echo "📌 Configuring pnpm proxy..."
        pnpm config set proxy "${tool_proxy}"
        pnpm config set https-proxy "${tool_proxy}"
    fi

    echo "✅ Proxy setup complete!"
    echo ""
    echo "Current proxy configuration:"
    echo "System proxy type: ${proxy_type}"
    echo "System proxy address: ${proxy_address}"
    echo "Tool proxy address: ${tool_proxy}"
}

# Select proxy mode from the provided argument.
MODE=${1:-auto} # Defaults to auto mode.

case "$MODE" in
    auto)
        echo "🔄 Auto mode: checking SOCKS5 proxy first..."
        if check_socks5_proxy; then
            echo "✅ SOCKS5 proxy is available; using SOCKS5 first"
            set_proxy "${SOCKS5_PROXY}" "SOCKS5"
        else
            echo "⚠️ SOCKS5 proxy is unavailable; falling back to HTTP proxy"
            set_proxy "${HTTP_PROXY}" "HTTP"
        fi
        ;;
    sock5)
        echo "🔒 Forcing SOCKS5 proxy..."
        set_proxy "${SOCKS5_PROXY}" "SOCKS5"
        ;;
    http)
        echo "🌐 Forcing HTTP proxy..."
        set_proxy "${HTTP_PROXY}" "HTTP"
        ;;
    *)
        echo "❌ Invalid argument: $MODE"
        echo "Usage: source setproxy [auto|sock5|http]"
        return 1
        ;;
esac

echo ""
echo "To remove the proxy settings, run: source usetproxy"
