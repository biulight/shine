#!/bin/bash
# shine-template: true
# Configure Claude Code to use a selected provider in the current shell session.
# Reads provider API keys or base64-encoded GPG secrets from the active shine env config.
# Use: ccenv

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
    echo "ccenv: $1" >&2
    return 1
}

cc_select_provider() {
    echo "Select Claude Code provider:" >&2
    echo "  1) deepseek" >&2
    echo "  2) qwen" >&2
    echo "  3) glm5 (not configured yet)" >&2
    printf "Provider [1]: " >&2
    read -r provider_choice

    case "${provider_choice:-1}" in
        1|deepseek|DeepSeek|DEEPSEEK)
            echo "deepseek"
            ;;
        2|qwen|Qwen|QWEN)
            echo "qwen"
            ;;
        3|glm|glm5|GLM|GLM5)
            echo "glm5"
            ;;
        *)
            cc_fail "invalid provider: ${provider_choice}"
            ;;
    esac
}

cc_configure_deepseek() {
    local deepseek_api_key="@@DEEPSEEK_API_KEY@@"
    local anthropic_auth_token

    if shine env get DEEPSEEK_API_KEY_GPG_SECRET >/dev/null 2>&1; then
        if ! anthropic_auth_token="$(shine env decrypt DEEPSEEK_API_KEY_GPG_SECRET)"; then
            cc_fail "failed to decrypt DEEPSEEK_API_KEY_GPG_SECRET with gpg"
            return 1
        fi
    else
        anthropic_auth_token="$deepseek_api_key"
    fi

    if [ -z "${anthropic_auth_token}" ]; then
        cc_fail "DeepSeek API key is not set. Add DEEPSEEK_API_KEY or DEEPSEEK_API_KEY_GPG_SECRET to the active shine env config."
        return 1
    fi

    export ANTHROPIC_BASE_URL="https://api.deepseek.com/anthropic"
    export ANTHROPIC_AUTH_TOKEN="$anthropic_auth_token"
    export ANTHROPIC_MODEL="deepseek-v4-pro[1m]"
    export ANTHROPIC_DEFAULT_OPUS_MODEL="deepseek-v4-pro[1m]"
    export ANTHROPIC_DEFAULT_SONNET_MODEL="deepseek-v4-pro[1m]"
    export ANTHROPIC_DEFAULT_HAIKU_MODEL="deepseek-v4-flash"
    export CLAUDE_CODE_SUBAGENT_MODEL="deepseek-v4-flash"
    export CLAUDE_CODE_EFFORT_LEVEL="max"
    unset CLAUDE_CODE_MAX_CONTEXT_TOKENS

    echo "ccenv: Claude Code environment configured for DeepSeek."
    echo "ccenv: Run 'claude' when you are ready to start Claude Code."
}

cc_configure_qwen() {
    local qwen_api_key="@@QWEN_API_KEY@@"
    local anthropic_auth_token

    if shine env get QWEN_API_KEY_GPG_SECRET >/dev/null 2>&1; then
        if ! anthropic_auth_token="$(shine env decrypt QWEN_API_KEY_GPG_SECRET)"; then
            cc_fail "failed to decrypt QWEN_API_KEY_GPG_SECRET with gpg"
            return 1
        fi
    else
        anthropic_auth_token="$qwen_api_key"
    fi

    if [ -z "${anthropic_auth_token}" ]; then
        cc_fail "Qwen API key is not set. Add QWEN_API_KEY or QWEN_API_KEY_GPG_SECRET to the active shine env config."
        return 1
    fi

    export ANTHROPIC_BASE_URL="https://token-plan.cn-beijing.maas.aliyuncs.com/apps/anthropic"
    export ANTHROPIC_AUTH_TOKEN="$anthropic_auth_token"
    export ANTHROPIC_MODEL="qwen3.8-max-preview"
    export ANTHROPIC_DEFAULT_HAIKU_MODEL="qwen3.6-flash"
    export ANTHROPIC_DEFAULT_SONNET_MODEL="qwen3.8-max-preview"
    export ANTHROPIC_DEFAULT_OPUS_MODEL="qwen3.8-max-preview"
    export CLAUDE_CODE_SUBAGENT_MODEL="qwen3.7-max"
    export CLAUDE_CODE_MAX_CONTEXT_TOKENS="983616"
    unset CLAUDE_CODE_EFFORT_LEVEL

    echo "ccenv: Claude Code environment configured for Qwen."
    echo "ccenv: Run 'claude' when you are ready to start Claude Code."
}

if ! cc_is_sourced; then
    echo "ccenv: this command must be sourced to update the current shell environment." >&2
    echo "ccenv: run 'source ccenv', or install with 'shine shell install agent' and reload your shell." >&2
    exit 1
fi

cc_provider="$(cc_select_provider)" || return 1

case "$cc_provider" in
    deepseek)
        cc_configure_deepseek || return 1
        ;;
    qwen)
        cc_configure_qwen || return 1
        ;;
    glm5)
        cc_fail "glm5 is not configured yet"
        return 1
        ;;
esac

unset cc_provider provider_choice
unset -f cc_is_sourced cc_fail cc_select_provider cc_configure_deepseek cc_configure_qwen
