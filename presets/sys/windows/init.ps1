# Initialize Windows with selectable Rust, terminal tools, network, and JavaScript runtime setup steps.
$ErrorActionPreference = "Stop"

$ProfileSentinelStart = "# >>> shine windows sys >>>"
$ProfileSentinelEnd = "# <<< shine windows sys <<<"

function Assert-Windows {
    if (-not $IsWindows -and $env:OS -ne "Windows_NT") {
        throw "This sys init preset only supports Windows."
    }
}

function Assert-WinGet {
    if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
        throw "winget is required for this preset. Install App Installer from Microsoft Store, then rerun this command."
    }
}

function Test-CommandExists {
    param(
        [Parameter(Mandatory = $true)]
        [string] $CommandName
    )

    [bool] (Get-Command $CommandName -ErrorAction SilentlyContinue)
}

function Install-WinGetPackage {
    param(
        [Parameter(Mandatory = $true)]
        [string] $PackageId,

        [Parameter(Mandatory = $true)]
        [string] $CommandName
    )

    if (Test-CommandExists $CommandName) {
        Write-Host "${PackageId}: already installed ($CommandName found)."
        return
    }

    Write-Host "Installing $PackageId..."
    winget install --exact --id $PackageId --accept-package-agreements --accept-source-agreements
}

function Remove-ManagedProfileBlock {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Path
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }

    $lines = Get-Content -LiteralPath $Path
    $output = New-Object System.Collections.Generic.List[string]
    $skip = $false

    foreach ($line in $lines) {
        if ($line -eq $ProfileSentinelStart) {
            $skip = $true
            continue
        }
        if ($line -eq $ProfileSentinelEnd) {
            $skip = $false
            continue
        }
        if (-not $skip) {
            $output.Add($line)
        }
    }

    Set-Content -LiteralPath $Path -Value $output -Encoding UTF8
}

function Get-ManagedProfileBlock {
    @'
# Managed by `shine sys init` for Windows. Existing user config is left untouched.

# User-local binaries
$shineUserPaths = @(
    "$HOME\.cargo\bin",
    "$HOME\.local\bin",
    "$HOME\.bun\bin",
    "$env:LOCALAPPDATA\pnpm",
    "$env:LOCALAPPDATA\Microsoft\WinGet\Packages"
) | Where-Object { $_ -and (Test-Path -LiteralPath $_) }

foreach ($shinePath in $shineUserPaths) {
    if (($env:Path -split ';') -notcontains $shinePath) {
        $env:Path = "$shinePath;$env:Path"
    }
}

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
'@
}

function Update-PowerShellProfiles {
    $profilePaths = @(
        (Join-Path $HOME "Documents\PowerShell\Microsoft.PowerShell_profile.ps1"),
        (Join-Path $HOME "Documents\WindowsPowerShell\Microsoft.PowerShell_profile.ps1")
    )

    $block = Get-ManagedProfileBlock

    foreach ($profilePath in $profilePaths) {
        $parent = Split-Path -Parent $profilePath
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
        if (-not (Test-Path -LiteralPath $profilePath)) {
            New-Item -ItemType File -Force -Path $profilePath | Out-Null
        }

        Remove-ManagedProfileBlock $profilePath
        Add-Content -LiteralPath $profilePath -Value ""
        Add-Content -LiteralPath $profilePath -Value $ProfileSentinelStart
        Add-Content -LiteralPath $profilePath -Value $block
        Add-Content -LiteralPath $profilePath -Value $ProfileSentinelEnd
        Write-Host "Updated $profilePath"
    }
}

function Install-Rust {
    Install-WinGetPackage "Rustlang.Rustup" "rustup"
}

function Install-Yazi {
    Install-WinGetPackage "sxyazi.yazi" "yazi"
}

function Install-Starship {
    Install-WinGetPackage "Starship.Starship" "starship"
}

function Install-zoxide {
    Install-WinGetPackage "ajeetdsouza.zoxide" "zoxide"
}

function Install-Atuin {
    Install-WinGetPackage "Atuinsh.Atuin" "atuin"
}

function Install-fzf {
    Install-WinGetPackage "junegunn.fzf" "fzf"
}

function Install-bat {
    Install-WinGetPackage "sharkdp.bat" "bat"
}

function Install-eza {
    Install-WinGetPackage "eza-community.eza" "eza"
}

function Install-ZeroTier {
    Install-WinGetPackage "ZeroTier.ZeroTierOne" "zerotier-cli"
}

function Install-Bun {
    Install-WinGetPackage "Oven-sh.Bun" "bun"
}

function Install-pnpm {
    Install-WinGetPackage "pnpm.pnpm" "pnpm"
}

function Install-mise {
    Install-WinGetPackage "jdx.mise" "mise"
}

function Install-Item {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Item
    )

    switch ($Item) {
        "rust" { Install-Rust }
        "yazi" { Install-Yazi }
        "starship" { Install-Starship }
        "zoxide" { Install-zoxide }
        "atuin" { Install-Atuin }
        "fzf" { Install-fzf }
        "bat" { Install-bat }
        "eza" { Install-eza }
        "zerotier" { Install-ZeroTier }
        "bun" { Install-Bun }
        "pnpm" { Install-pnpm }
        "mise" { Install-mise }
        default { throw "Unknown Windows sys init item: $Item" }
    }
}

Assert-Windows

if ($args.Count -eq 0) {
    Write-Host "No Windows sys init items selected."
    exit 0
}

Assert-WinGet

foreach ($item in $args) {
    Install-Item $item
}

Update-PowerShellProfiles
Write-Host "Windows system initialization complete."
