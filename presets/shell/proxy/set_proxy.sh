#!/bin/bash

# 代理设置脚本
# HTTP代理端口: 6152
# SOCKS5代理端口: 6153

HTTP_PROXY_PORT=6152
SOCKS5_PROXY_PORT=6153
HTTP_PROXY="http://127.0.0.1:${HTTP_PROXY_PORT}"
SOCKS5_PROXY="socks5://127.0.0.1:${SOCKS5_PROXY_PORT}"

echo "🚀 设置代理配置..."

# 检测SOCKS5代理是否可用
check_socks5_proxy() {
    if command -v nc >/dev/null 2>&1 && nc -z 127.0.0.1 ${SOCKS5_PROXY_PORT} 2>/dev/null; then
        echo "🔍 使用netcat检测到SOCKS5代理端口开放"
        return 0
    fi
    if command -v lsof >/dev/null 2>&1 && lsof -i :${SOCKS5_PROXY_PORT} >/dev/null 2>&1; then
        echo "🔍 使用lsof检测到SOCKS5代理端口被监听"
        return 0
    fi
    if command -v netstat >/dev/null 2>&1 && netstat -an 2>/dev/null | grep -q ":${SOCKS5_PROXY_PORT}.*LISTEN"; then
        echo "🔍 使用netstat检测到SOCKS5代理端口监听中"
        return 0
    fi
    if command -v telnet >/dev/null 2>&1 && timeout 2 telnet 127.0.0.1 ${SOCKS5_PROXY_PORT} </dev/null >/dev/null 2>&1; then
        echo "🔍 使用telnet检测到SOCKS5代理可连接"
        return 0
    fi
    return 1
}

# 设置代理函数
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

    # Git、NPM等工具通常不支持SOCKS5，统一使用HTTP代理
    local tool_proxy="http://127.0.0.1:${HTTP_PROXY_PORT}"

    echo "🔧 配置Git代理..."
    git config --global http.proxy "${tool_proxy}"
    git config --global https.proxy "${tool_proxy}"

    echo "📦 配置NPM代理..."
    npm config set proxy "${tool_proxy}"
    npm config set https-proxy "${tool_proxy}"
    npm config set registry https://registry.npmjs.org/

    if command -v yarn >/dev/null 2>&1; then
        yarn_version=$(yarn --version)
        case "$yarn_version" in
            1.*)
                echo "🧶 配置Yarn@${yarn_version}代理..."
                yarn config set proxy "${tool_proxy}"
                yarn config set https-proxy "${tool_proxy}"
                ;;
            2.*|3.*)
                echo "🧶 配置Yarn@${yarn_version}代理..."
                yarn config set httpProxy "${tool_proxy}"
                yarn config set httpsProxy "${tool_proxy}"
                ;;
            *)
                echo "⚠️ 未知的Yarn版本: ${yarn_version}，尝试使用通用配置"
                yarn config set proxy "${tool_proxy}"
                yarn config set https-proxy "${tool_proxy}"
                ;;
        esac
    fi

    if command -v pnpm >/dev/null 2>&1; then
        echo "📌 配置pnpm代理..."
        pnpm config set proxy "${tool_proxy}"
        pnpm config set https-proxy "${tool_proxy}"
    fi

    echo "✅ 代理设置完成！"
    echo ""
    echo "当前代理配置："
    echo "系统代理类型: ${proxy_type}"
    echo "系统代理地址: ${proxy_address}"
    echo "工具代理地址: ${tool_proxy}"
}

# 根据传入参数选择代理模式
MODE=${1:-auto} # 默认为auto模式

case "$MODE" in
    auto)
        echo "🔄 自动模式：优先检测SOCKS5代理..."
        if check_socks5_proxy; then
            echo "✅ SOCKS5代理可用，优先使用SOCKS5代理"
            set_proxy "${SOCKS5_PROXY}" "SOCKS5"
        else
            echo "⚠️  SOCKS5代理不可用，回退到HTTP代理"
            set_proxy "${HTTP_PROXY}" "HTTP"
        fi
        ;;
    sock5)
        echo "🔒 强制使用SOCKS5代理..."
        set_proxy "${SOCKS5_PROXY}" "SOCKS5"
        ;;
    http)
        echo "🌐 强制使用HTTP代理..."
        set_proxy "${HTTP_PROXY}" "HTTP"
        ;;
    *)
        echo "❌ 无效参数: $MODE"
        echo "用法: source set_proxy.sh [auto|sock5|http]"
        return 1
        ;;
esac

echo ""
echo "要取消代理设置，请运行: source unset_proxy.sh"
