# Initialize Windows with selectable Rust, terminal tools, network, and JavaScript runtime setup steps.
$ErrorActionPreference = "Stop"

function Write-Status {
    param(
        [Parameter(Mandatory = $true)]
        [string] $State,

        [string] $Detail = ""
    )

    Write-Output "SHINE_SYS_STATUS`t$State`t$Detail"
}

$ScriptPathCandidates = @(
    $env:SHINE_SYS_PRESET_ROOT,
    $PSScriptRoot,
    $(if ($MyInvocation.PSCommandPath) { Split-Path -Parent $MyInvocation.PSCommandPath }),
    $(if ($PSCommandPath) { Split-Path -Parent $PSCommandPath }),
    $(if ($MyInvocation.MyCommand.Path) { Split-Path -Parent $MyInvocation.MyCommand.Path }),
    $(if ($MyInvocation.MyCommand.Definition -and (Test-Path -LiteralPath $MyInvocation.MyCommand.Definition)) {
            Split-Path -Parent $MyInvocation.MyCommand.Definition
        }),
    (Get-Location).Path
) | Where-Object { $_ }

$SysPresetRoot = $ScriptPathCandidates |
    ForEach-Object {
        $path = [string] $_
        if ($path.StartsWith("\\?\")) {
            $path.Substring(4)
        } else {
            $path
        }
    } |
    Select-Object -First 1

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
        Write-Status "already-installed" "$CommandName found"
        return
    }

    Write-Host "Installing $PackageId..."
    winget install --exact --id $PackageId --accept-package-agreements --accept-source-agreements
    Write-Status "installed" $PackageId
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
        [string] $Item,

        [string] $Action = "apply"
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
        "__shine_finalize" { Write-Status "completed" "profile is managed by shine CLI" }
        default { throw "Unknown Windows sys init item: $Item" }
    }
}

Assert-Windows

if ($args.Count -eq 0) {
    Write-Status "completed" "no item selected"
    exit 0
}

$item = $args[0]
if ($item -ne "__shine_finalize") {
    Assert-WinGet
}

Install-Item $item $(if ($args.Count -gt 1) { $args[1] } else { "apply" })
