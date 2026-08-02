# Managed by `shine sys bootstrap` for Windows. Existing user config is left untouched.

# Starship prompt
if (Get-Command starship -ErrorAction SilentlyContinue) {
    Invoke-Expression (&starship init powershell | Out-String)
}

# zoxide
if (Get-Command zoxide -ErrorAction SilentlyContinue) {
    Invoke-Expression (&zoxide init powershell | Out-String)
}

# Atuin
if (Get-Command atuin -ErrorAction SilentlyContinue) {
    Invoke-Expression (&atuin init powershell | Out-String)
}

# mise
if (Get-Command mise -ErrorAction SilentlyContinue) {
    Invoke-Expression (&mise activate pwsh | Out-String)
}

# eza
if (Get-Command eza -ErrorAction SilentlyContinue) {
    Set-Alias -Name ls -Value eza -Option AllScope -Force
}

# bat
if (Get-Command bat -ErrorAction SilentlyContinue) {
    Set-Alias -Name cat -Value bat -Option AllScope -Force
}

# Yazi
if (Get-Command yazi -ErrorAction SilentlyContinue) {
    function y {
        $tmp = (New-TemporaryFile).FullName
        yazi.exe @args --cwd-file="$tmp"

        $cwd = Get-Content -Path $tmp -Encoding UTF8
        if ($cwd -and $cwd -ne $PWD.Path -and (Test-Path -LiteralPath $cwd -PathType Container)) {
            Set-Location -LiteralPath (Resolve-Path -LiteralPath $cwd).Path
        }

        Remove-Item -Path $tmp
    }
}
