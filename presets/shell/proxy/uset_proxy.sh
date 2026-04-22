#!/bin/bash

# 取消代理设置脚本

echo "🔄 取消代理配置..."

# 显示当前代理设置（如果有的话）
if [ -n "$http_proxy" ] || [ -n "$HTTP_PROXY" ]; then
    echo "🔍 当前检测到的代理设置："
    [ -n "$http_proxy" ] && echo "  http_proxy: $http_proxy"
    [ -n "$https_proxy" ] && echo "  https_proxy: $https_proxy"
    [ -n "$HTTP_PROXY" ] && echo "  HTTP_PROXY: $HTTP_PROXY"
    [ -n "$HTTPS_PROXY" ] && echo "  HTTPS_PROXY: $HTTPS_PROXY"
    [ -n "$all_proxy" ] && echo "  all_proxy: $all_proxy"
    [ -n "$ALL_PROXY" ] && echo "  ALL_PROXY: $ALL_PROXY"
    echo ""
else
    echo "ℹ️  当前没有检测到代理环境变量"
fi

# 取消系统环境变量代理
echo "🧹 清除系统环境变量代理..."
unset http_proxy
unset https_proxy
unset HTTP_PROXY
unset HTTPS_PROXY
unset all_proxy
unset ALL_PROXY
unset no_proxy
unset NO_PROXY



# 取消Git代理
echo "🔧 取消Git代理..."
git config --global --unset http.proxy 2>/dev/null || echo "  ℹ️ Git http.proxy 未设置或已清除"
git config --global --unset https.proxy 2>/dev/null || echo "  ℹ️ Git https.proxy 未设置或已清除"

# 取消NPM代理
echo "📦 取消NPM代理..."
npm config delete proxy 2>/dev/null || echo "  ℹ️ NPM proxy 未设置或已清除"
npm config delete https-proxy 2>/dev/null || echo "  ℹ️ NPM https-proxy 未设置或已清除"

# 取消Yarn代理（如果有的话）
if command -v yarn >/dev/null 2>&1; then
    echo "🧶 取消Yarn代理..."
    yarn_version=$(yarn --version)
    case "$yarn_version" in
        1.*)
            yarn config delete proxy 2>/dev/null || echo "  ℹ️ Yarn proxy 未设置或已清除"
            yarn config delete https-proxy 2>/dev/null || echo "  ℹ️ Yarn https-proxy 未设置或已清除"
            ;;
        2.*|3.*)
            yarn config delete httpProxy 2>/dev/null || echo "  ℹ️ Yarn httpProxy 未设置或已清除"
            yarn config delete httpsProxy 2>/dev/null || echo "  ℹ️ Yarn httpsProxy 未设置或已清除"
            ;;
        *)
            echo "⚠️ 未知的Yarn版本: ${yarn_version}，尝试使用通用配置"
            yarn config delete proxy 2>/dev/null || echo "  ℹ️ Yarn proxy 未设置或已清除"
            yarn config delete https-proxy 2>/dev/null || echo "  ℹ️ Yarn https-proxy 未设置或已清除"
            ;;
    esac
else
    echo "ℹ️ Yarn 未安装，跳过"
fi

# 取消pnpm代理（如果有的话）
if command -v pnpm >/dev/null 2>&1; then
    echo "📌 取消pnpm代理..."
    pnpm config delete proxy 2>/dev/null || echo "  ℹ️ pnpm proxy 未设置或已清除"
    pnpm config delete https-proxy 2>/dev/null || echo "  ℹ️ pnpm https-proxy 未设置或已清除"
else
    echo "ℹ️ pnpm 未安装，跳过"
fi

echo ""
echo "✅ 代理设置已取消！"
echo ""
echo "📝 提示："
echo "  - 环境变量代理已清除（仅影响当前终端会话）"
echo "  - Git/NPM/Yarn/pnpm 的全局代理配置已清除"
echo "  - 若要重新设置代理，请运行： source ~/bin/set_proxy.sh"
