# shine-template: true
# Configure Claude Code to use a selected provider in the current PowerShell session.
# Reads provider API keys or base64-encoded GPG secrets from the active shine env config.
# Use: ccenv

function Fail-CcEnv {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Message
    )

    Write-Error "ccenv: $Message"
    return $false
}

function Select-CcProvider {
    Write-Host "Select Claude Code provider:"
    Write-Host "  1) deepseek"
    Write-Host "  2) qwen"
    Write-Host "  3) glm5 (not configured yet)"
    $providerChoice = Read-Host "Provider [1]"

    switch ($providerChoice) {
        { [string]::IsNullOrWhiteSpace($_) } { return 'deepseek' }
        '1' { return 'deepseek' }
        'deepseek' { return 'deepseek' }
        'DeepSeek' { return 'deepseek' }
        'DEEPSEEK' { return 'deepseek' }
        '2' { return 'qwen' }
        'qwen' { return 'qwen' }
        'Qwen' { return 'qwen' }
        'QWEN' { return 'qwen' }
        '3' { return 'glm5' }
        'glm' { return 'glm5' }
        'glm5' { return 'glm5' }
        'GLM' { return 'glm5' }
        'GLM5' { return 'glm5' }
        default {
            Fail-CcEnv "invalid provider: $providerChoice" | Out-Null
            return $null
        }
    }
}

function Set-CcDeepSeekEnv {
    $deepseekApiKey = '@@DEEPSEEK_API_KEY@@'
    $anthropicAuthToken = $null

    shine env get DEEPSEEK_API_KEY_GPG_SECRET *> $null
    if ($LASTEXITCODE -eq 0) {
        $anthropicAuthToken = shine env decrypt DEEPSEEK_API_KEY_GPG_SECRET
        if ($LASTEXITCODE -ne 0) {
            return (Fail-CcEnv 'failed to decrypt DEEPSEEK_API_KEY_GPG_SECRET with gpg')
        }
    } else {
        $anthropicAuthToken = $deepseekApiKey
    }

    if ([string]::IsNullOrEmpty($anthropicAuthToken)) {
        return (Fail-CcEnv 'DeepSeek API key is not set. Add DEEPSEEK_API_KEY or DEEPSEEK_API_KEY_GPG_SECRET to the active shine env config.')
    }

    $env:ANTHROPIC_BASE_URL = 'https://api.deepseek.com/anthropic'
    $env:ANTHROPIC_AUTH_TOKEN = $anthropicAuthToken
    $env:ANTHROPIC_MODEL = 'deepseek-v4-pro[1m]'
    $env:ANTHROPIC_DEFAULT_OPUS_MODEL = 'deepseek-v4-pro[1m]'
    $env:ANTHROPIC_DEFAULT_SONNET_MODEL = 'deepseek-v4-pro[1m]'
    $env:ANTHROPIC_DEFAULT_HAIKU_MODEL = 'deepseek-v4-flash'
    $env:CLAUDE_CODE_SUBAGENT_MODEL = 'deepseek-v4-flash'
    $env:CLAUDE_CODE_EFFORT_LEVEL = 'max'
    Remove-Item Env:CLAUDE_CODE_MAX_CONTEXT_TOKENS -ErrorAction SilentlyContinue

    Write-Host 'ccenv: Claude Code environment configured for DeepSeek.'
    Write-Host "ccenv: Run 'claude' when you are ready to start Claude Code."
    return $true
}

function Set-CcQwenEnv {
    $qwenApiKey = '@@QWEN_API_KEY@@'
    $anthropicAuthToken = $null

    shine env get QWEN_API_KEY_GPG_SECRET *> $null
    if ($LASTEXITCODE -eq 0) {
        $anthropicAuthToken = shine env decrypt QWEN_API_KEY_GPG_SECRET
        if ($LASTEXITCODE -ne 0) {
            return (Fail-CcEnv 'failed to decrypt QWEN_API_KEY_GPG_SECRET with gpg')
        }
    } else {
        $anthropicAuthToken = $qwenApiKey
    }

    if ([string]::IsNullOrEmpty($anthropicAuthToken)) {
        return (Fail-CcEnv 'Qwen API key is not set. Add QWEN_API_KEY or QWEN_API_KEY_GPG_SECRET to the active shine env config.')
    }

    $env:ANTHROPIC_BASE_URL = 'https://token-plan.cn-beijing.maas.aliyuncs.com/apps/anthropic'
    $env:ANTHROPIC_AUTH_TOKEN = $anthropicAuthToken
    $env:ANTHROPIC_MODEL = 'qwen3.8-max-preview'
    $env:ANTHROPIC_DEFAULT_HAIKU_MODEL = 'qwen3.6-flash'
    $env:ANTHROPIC_DEFAULT_SONNET_MODEL = 'qwen3.8-max-preview'
    $env:ANTHROPIC_DEFAULT_OPUS_MODEL = 'qwen3.8-max-preview'
    $env:CLAUDE_CODE_SUBAGENT_MODEL = 'qwen3.7-max'
    $env:CLAUDE_CODE_MAX_CONTEXT_TOKENS = '983616'
    Remove-Item Env:CLAUDE_CODE_EFFORT_LEVEL -ErrorAction SilentlyContinue

    Write-Host 'ccenv: Claude Code environment configured for Qwen.'
    Write-Host "ccenv: Run 'claude' when you are ready to start Claude Code."
    return $true
}

$ccProvider = Select-CcProvider
if (-not $ccProvider) {
    return 1
}

switch ($ccProvider) {
    'deepseek' {
        if (-not (Set-CcDeepSeekEnv)) {
            return 1
        }
    }
    'qwen' {
        if (-not (Set-CcQwenEnv)) {
            return 1
        }
    }
    'glm5' {
        Fail-CcEnv 'glm5 is not configured yet' | Out-Null
        return 1
    }
}

Remove-Item function:Fail-CcEnv -ErrorAction SilentlyContinue
Remove-Item function:Select-CcProvider -ErrorAction SilentlyContinue
Remove-Item function:Set-CcDeepSeekEnv -ErrorAction SilentlyContinue
Remove-Item function:Set-CcQwenEnv -ErrorAction SilentlyContinue
Remove-Variable ccProvider -ErrorAction SilentlyContinue
