# shine-template: true
# Configure Claude Code to use DeepSeek in the current PowerShell session.
# Reads the DeepSeek API key or a base64-encoded GPG secret from the active shine env config.
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
    Write-Host "  2) glm5 (not configured yet)"
    $providerChoice = Read-Host "Provider [1]"

    switch ($providerChoice) {
        { [string]::IsNullOrWhiteSpace($_) } { return 'deepseek' }
        '1' { return 'deepseek' }
        'deepseek' { return 'deepseek' }
        'DeepSeek' { return 'deepseek' }
        'DEEPSEEK' { return 'deepseek' }
        '2' { return 'glm5' }
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

    Write-Host 'ccenv: Claude Code environment configured for DeepSeek.'
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
    'glm5' {
        Fail-CcEnv 'glm5 is not configured yet' | Out-Null
        return 1
    }
}

Remove-Item function:Fail-CcEnv -ErrorAction SilentlyContinue
Remove-Item function:Select-CcProvider -ErrorAction SilentlyContinue
Remove-Item function:Set-CcDeepSeekEnv -ErrorAction SilentlyContinue
Remove-Variable ccProvider -ErrorAction SilentlyContinue
