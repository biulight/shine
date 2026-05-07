#!/bin/bash
# shine-template: true
# Configure Claude Code to use DeepSeek in the current shell session.
# Reads the DeepSeek API key from shine.env.toml.
# Use: cc

cc_is_sourced() {
    if [ -n "${ZSH_EVAL_CONTEXT:-}" ]; then
        case "$ZSH_EVAL_CONTEXT" in
            *:file:*) return 0 ;;
        esac
    fi

    if [ -n "${BASH_VERSION:-}" ] && [ "${BASH_SOURCE[0]}" != "$0" ]; then
        return 0
    fi

    return 1
}

cc_fail() {
    echo "cc: $1" >&2
    return 1
}

cc_select_provider() {
    echo "Select Claude Code provider:" >&2
    echo "  1) deepseek" >&2
    echo "  2) glm5 (not configured yet)" >&2
    printf "Provider [1]: " >&2
    read -r provider_choice

    case "${provider_choice:-1}" in
        1|deepseek|DeepSeek|DEEPSEEK)
            echo "deepseek"
            ;;
        2|glm|glm5|GLM|GLM5)
            echo "glm5"
            ;;
        *)
            cc_fail "invalid provider: ${provider_choice}"
            ;;
    esac
}

cc_configure_deepseek() {
    local deepseek_api_key="@@DEEPSEEK_API_KEY@@"
    if [ -z "${deepseek_api_key}" ]; then
        cc_fail "DEEPSEEK_API_KEY is not set. Add DEEPSEEK_API_KEY = \"...\" to shine.env.toml."
        return 1
    fi

    export ANTHROPIC_BASE_URL="https://api.deepseek.com/anthropic"
    export ANTHROPIC_AUTH_TOKEN="$deepseek_api_key"
    export ANTHROPIC_MODEL="deepseek-v4-pro[1m]"
    export ANTHROPIC_DEFAULT_OPUS_MODEL="deepseek-v4-pro[1m]"
    export ANTHROPIC_DEFAULT_SONNET_MODEL="deepseek-v4-pro[1m]"
    export ANTHROPIC_DEFAULT_HAIKU_MODEL="deepseek-v4-flash"
    export CLAUDE_CODE_SUBAGENT_MODEL="deepseek-v4-flash"
    export CLAUDE_CODE_EFFORT_LEVEL="max"

    echo "cc: Claude Code environment configured for DeepSeek."
    echo "cc: Run 'claude' when you are ready to start Claude Code."
}

if ! cc_is_sourced; then
    echo "cc: this command must be sourced to update the current shell environment." >&2
    echo "cc: run 'source cc', or install with 'shine shell install cc' and reload your shell." >&2
    exit 1
fi

cc_provider="$(cc_select_provider)" || return 1

case "$cc_provider" in
    deepseek)
        cc_configure_deepseek || return 1
        ;;
    glm5)
        cc_fail "glm5 is not configured yet"
        return 1
        ;;
esac

unset cc_provider provider_choice
unset -f cc_is_sourced cc_fail cc_select_provider cc_configure_deepseek
