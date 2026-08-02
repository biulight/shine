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

function Write-UpdateStatus {
    param(
        [Parameter(Mandatory = $true)] [string] $State,
        [string] $Detail = "",
        [string] $Command = ""
    )
    Write-Output "SHINE_SYS_UPDATE`t$State`t$Detail`t$Command"
}

# Set by `shine sys bootstrap --proxy` (the shine-owned signal, not $env:HTTP_PROXY,
# so an ambient HTTP_PROXY never turns on winget proxying without the flag).
# winget ignores http_proxy/https_proxy env vars; it only honors `--proxy <uri>`.
$script:ProxyUri = if ($env:SHINE_SYS_PROXY) { $env:SHINE_SYS_PROXY.Trim() } else { $null }
$script:ProxyOptionAttempted = $false

function Enable-WinGetProxyOption {
    if (-not $script:ProxyUri -or $script:ProxyOptionAttempted) {
        return
    }
    $script:ProxyOptionAttempted = $true
    Write-Host "Using proxy $script:ProxyUri for winget."
    try {
        # winget's `--proxy` CLI option is disabled by default and must be enabled
        # once by an administrator. Best-effort: succeeds when this shell is
        # elevated, harmless no-op otherwise (the winget call then reports the
        # failure and the caller surfaces the remediation command).
        winget settings --enable ProxyCommandLineOptions 2>&1 | Out-Null
    } catch {
        # Non-admin shells cannot enable it; ignore and let the winget call report.
    }
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
        throw "This sys bootstrap preset only supports Windows."
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
    $wingetArgs = @(
        "install",
        "--exact",
        "--id", $PackageId,
        "--accept-package-agreements",
        "--accept-source-agreements"
    )
    if ($script:ProxyUri) {
        Enable-WinGetProxyOption
        $wingetArgs += @("--proxy", $script:ProxyUri)
    }
    winget @wingetArgs
    if ($LASTEXITCODE -ne 0) {
        # winget is a native exe, so a nonzero exit never triggers PowerShell's
        # ErrorActionPreference=Stop. The Test-CommandExists guard above means we
        # only reach here on a genuine install, so a nonzero code is a real failure.
        $hint = if ($script:ProxyUri) {
            " If winget's proxy option is disabled, run once as administrator: winget settings --enable ProxyCommandLineOptions"
        } else {
            ""
        }
        Write-Status "failed" "$PackageId install failed (exit $LASTEXITCODE).$hint"
        throw "winget install $PackageId failed with exit code $LASTEXITCODE"
    }
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

function Check-WinGetUpdate {
    param([Parameter(Mandatory = $true)] [string] $PackageId)
    # `winget upgrade --id` performs the upgrade. `winget list` is the
    # documented read-only query and `--upgrade-available` narrows it to
    # packages WinGet has verified as outdated.
    $wingetArgs = @("list", "--upgrade-available", "--exact", "--id", $PackageId, "--accept-source-agreements")
    if ($script:ProxyUri) {
        # Keep update checks read-only: unlike the install path, do not enable
        # ProxyCommandLineOptions here. If it was not enabled already, winget
        # returns a normal checker failure with the remediation below.
        $wingetArgs += @("--proxy", $script:ProxyUri)
    }
    $output = & winget @wingetArgs 2>&1 | Out-String
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) {
        $hint = if ($script:ProxyUri) { " Enable it once as administrator: winget settings --enable ProxyCommandLineOptions" } else { "" }
        Write-UpdateStatus "failed" "winget update query failed for $PackageId (exit $exitCode).$hint" ""
        return
    }
    if ($output -match [regex]::Escape($PackageId)) {
        $command = "winget upgrade --exact --id $PackageId"
        if ($script:ProxyUri) { $command += " --proxy $script:ProxyUri" }
        Write-UpdateStatus "available" "winget reports an available update" $command
    } else {
        Write-UpdateStatus "current" "winget reports no applicable update" ""
    }
}

function Check-Update {
    param([Parameter(Mandatory = $true)] [string] $Item)
    $packages = @{
        "rust" = "Rustlang.Rustup"; "yazi" = "sxyazi.yazi"; "starship" = "Starship.Starship"
        "zoxide" = "ajeetdsouza.zoxide"; "atuin" = "Atuinsh.Atuin"; "fzf" = "junegunn.fzf"
        "bat" = "sharkdp.bat"; "eza" = "eza-community.eza"; "zerotier" = "ZeroTier.ZeroTierOne"
        "bun" = "Oven-sh.Bun"; "pnpm" = "pnpm.pnpm"; "mise" = "jdx.mise"
    }
    if ($packages.ContainsKey($Item)) { Check-WinGetUpdate $packages[$Item] }
    else { Write-UpdateStatus "unsupported" "This item has no update checker" "" }
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
        default { throw "Unknown Windows sys bootstrap item: $Item" }
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
if ($args.Count -gt 1 -and $args[1] -eq "check-update") {
    Check-Update $item
} else {
    Install-Item $item $(if ($args.Count -gt 1) { $args[1] } else { "apply" })
}
