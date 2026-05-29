$ErrorActionPreference = 'Stop'

if ([Net.ServicePointManager]::SecurityProtocol -band [Net.SecurityProtocolType]::Tls12) {
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} else {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
}

$ShineRepo = if ($env:SHINE_REPO) { $env:SHINE_REPO } else { 'biulight/shine' }
$ShineInstallDir = if ($env:SHINE_INSTALL_DIR) {
    $env:SHINE_INSTALL_DIR
} else {
    if (-not $env:LOCALAPPDATA) {
        Write-Error 'LOCALAPPDATA is not set; set SHINE_INSTALL_DIR to choose an install directory.'
        exit 1
    }
    Join-Path $env:LOCALAPPDATA 'Programs\shine'
}
$ShineVersion = if ($env:SHINE_VERSION) { $env:SHINE_VERSION } else { 'latest' }

function Write-Log {
    param([string]$Message)
    Write-Host $Message
}

function Fail {
    param([string]$Message)
    Write-Error "error: $Message"
    exit 1
}

function Get-Target {
    $arch = if ($env:PROCESSOR_ARCHITEW6432) {
        $env:PROCESSOR_ARCHITEW6432
    } else {
        $env:PROCESSOR_ARCHITECTURE
    }

    switch -Regex ($arch) {
        '^(AMD64|x86_64)$' { return 'windows-x86_64' }
        '^(ARM64|aarch64)$' { return 'windows-aarch64' }
        default { Fail "unsupported architecture: $arch" }
    }
}

function Resolve-Version {
    $uri = "https://api.github.com/repos/$ShineRepo/releases/latest"
    try {
        $release = Invoke-RestMethod -Uri $uri -Headers @{ 'User-Agent' = 'shine-install' }
    } catch {
        Fail "could not resolve latest version from GitHub API: $($_.Exception.Message)"
    }

    $tag = [string]$release.tag_name
    if (-not $tag.StartsWith('v')) {
        Fail "latest release tag is not a stable version tag: $tag"
    }
    return $tag.Substring(1)
}

function Get-DownloadUrl {
    param(
        [string]$Version,
        [string]$Target
    )

    $asset = "shine-v$Version-$Target.tar.gz"
    return "https://github.com/$ShineRepo/releases/download/v$Version/$asset"
}

function Test-PathContainsDir {
    param([string]$Dir)

    $target = [System.IO.Path]::GetFullPath($Dir).TrimEnd('\')
    $pathValue = [Environment]::GetEnvironmentVariable('Path', 'Process')
    if (-not $pathValue) {
        return $false
    }

    foreach ($entry in $pathValue -split ';') {
        if (-not $entry) {
            continue
        }
        try {
            $candidate = [System.IO.Path]::GetFullPath($entry).TrimEnd('\')
        } catch {
            continue
        }
        if ([string]::Equals($candidate, $target, [System.StringComparison]::OrdinalIgnoreCase)) {
            return $true
        }
    }
    return $false
}

function Add-PathHint {
    param([string]$Dir)

    $escaped = $Dir.Replace("'", "''")
    Write-Log "Warning: $Dir is not in PATH"
    Write-Log 'Add it to your user PATH, then open a new shell:'
    Write-Log "  [Environment]::SetEnvironmentVariable('Path', [Environment]::GetEnvironmentVariable('Path', 'User') + ';$escaped', 'User')"
}

function Main {
    if (-not (Get-Command tar -ErrorAction SilentlyContinue)) {
        Fail 'required command not found: tar'
    }

    $target = Get-Target
    if ($ShineVersion -eq 'latest') {
        Write-Log 'Resolving latest version...'
        $assetVersion = Resolve-Version
        Write-Log "Latest version: v$assetVersion"
    } else {
        $assetVersion = $ShineVersion
    }

    $url = Get-DownloadUrl -Version $assetVersion -Target $target
    $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) "shine-install-$([System.Guid]::NewGuid())"
    $archive = Join-Path $tmpDir 'shine.tar.gz'

    New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null
    try {
        Write-Log "Downloading shine for $target from $url"
        Invoke-WebRequest -Uri $url -OutFile $archive -Headers @{ 'User-Agent' = 'shine-install' }

        tar -xzf $archive -C $tmpDir
        if ($LASTEXITCODE -ne 0) {
            Fail 'failed to extract release archive'
        }

        $binary = Join-Path $tmpDir 'shine.exe'
        if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
            Fail 'release archive did not contain a shine.exe binary'
        }

        New-Item -ItemType Directory -Force -Path $ShineInstallDir | Out-Null
        $installPath = Join-Path $ShineInstallDir 'shine.exe'
        Move-Item -Force -LiteralPath $binary -Destination $installPath

        Write-Log "Installed shine to $installPath"
        if (-not (Test-PathContainsDir -Dir $ShineInstallDir)) {
            Add-PathHint -Dir $ShineInstallDir
        }
    } finally {
        if (Test-Path -LiteralPath $tmpDir) {
            Remove-Item -Recurse -Force -LiteralPath $tmpDir
        }
    }
}

Main
