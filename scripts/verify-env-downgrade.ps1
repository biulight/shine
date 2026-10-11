#requires -Version 7.0
<#
.SYNOPSIS
Exercise env upgrade, retired scripts and binary downgrade with synthetic data.
.DESCRIPTION
Supply the new executable and an independently verified stable executable. All
configuration, backups and output stay in a fresh temporary directory. No real
presets, config or secrets are read. The directory is retained as evidence.
.EXAMPLE
pwsh -NoProfile -File scripts/verify-env-downgrade.ps1 -CurrentBinary C:/new/shine.exe -PreviousBinary C:/stable/shine.exe
#>
param(
    [Parameter(Mandatory)][string]$CurrentBinary,
    [Parameter(Mandatory)][string]$PreviousBinary
)
$ErrorActionPreference = 'Stop'
$CurrentBinary = (Resolve-Path $CurrentBinary).Path
$PreviousBinary = (Resolve-Path $PreviousBinary).Path
$root = Join-Path ([IO.Path]::GetTempPath()) ("shine-env-downgrade-" + [guid]::NewGuid())
$state = Join-Path $root 'state'
$project = Join-Path $root 'isolated project'
$outside = Join-Path $root 'outside'
$fixtureHome = Join-Path $root 'home'
$overlay = Join-Path $root 'overlay'
$backup = Join-Path $root 'backup'
foreach ($path in @($state, $project, $outside, $overlay, $backup, $fixtureHome,
        "$fixtureHome/AppData/Roaming", "$fixtureHome/AppData/Local")) {
    New-Item -ItemType Directory -Path $path | Out-Null
}
$records = [Collections.Generic.List[object]]::new()
function Write-Fixture([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, $Text, [Text.UTF8Encoding]::new($false))
}
function Invoke-Probe([string]$Label, [string]$Executable, [string[]]$Arguments,
        [string]$WorkingDirectory = $project, [int]$ExpectedExit = 0) {
    $info = [Diagnostics.ProcessStartInfo]::new($Executable)
    $info.UseShellExecute = $false
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.WorkingDirectory = $WorkingDirectory
    foreach ($argument in $Arguments) { $info.ArgumentList.Add($argument) }
    $info.Environment['SHINE_CONFIG_DIR'] = $state
    $info.Environment.Remove('SHINE_PRESETS') | Out-Null
    $info.Environment.Remove('SUDO_USER') | Out-Null
    $info.Environment['HOME'] = $fixtureHome
    $info.Environment['USERPROFILE'] = $fixtureHome
    $info.Environment['APPDATA'] = "$fixtureHome/AppData/Roaming"
    $info.Environment['LOCALAPPDATA'] = "$fixtureHome/AppData/Local"
    $process = [Diagnostics.Process]::Start($info)
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(30000)) {
        $process.Kill($true)
        throw "$Label timed out"
    }
    $record = [pscustomobject]@{
        label = $Label; exit = $process.ExitCode
        stdout = $stdout.GetAwaiter().GetResult(); stderr = $stderr.GetAwaiter().GetResult()
    }
    $records.Add($record)
    $records | ConvertTo-Json -Depth 5 | Set-Content "$root/commands.json" -Encoding utf8
    if ($process.ExitCode -ne $ExpectedExit) { throw "$Label failed: $($record | ConvertTo-Json -Compress)" }
    $process.Dispose()
    return $record
}
function Invoke-ShineProbe([string]$Label, [string]$Binary, [string[]]$Arguments, [int]$ExitCode = 0) {
    Invoke-Probe $Label $Binary (@('--config-dir', $state) + $Arguments) -ExpectedExit $ExitCode
}
function Assert-Value([object]$Record, [string]$Expected) {
    if ($Record.stdout.Trim() -ne $Expected) { throw "Unexpected value for $($Record.label): $($Record.stdout)" }
}
$files = @("$state/config.toml", "$state/shine.env.toml",
    "$project/shine.config.toml", "$project/shine.env.toml", "$overlay/shine.env.toml")
function File-Hashes { @($files | ForEach-Object { (Get-FileHash $_ -Algorithm SHA256).Hash }) -join ',' }
function Assert-Hashes([string]$Expected) {
    if ((File-Hashes) -ne $Expected) { throw 'Unexpected configuration mutation' }
}
Write-Fixture $files[0] "schema_version = 2`nlast_cleared_schema_version = 2`npresets_overlay_dir = '../overlay'`n[env]`nHTTP_PROXY_PORT = { value = 'global-config', description = 'global-description' }`n"
Write-Fixture $files[1] "# global sidecar`nHTTP_PROXY_PORT = 'global-env'`n"
Write-Fixture $files[2] "# project comment`n[env]`nHTTP_PROXY_PORT = 'project-config'`nROLLBACK_ONLY = 'restore-from-backup'`n"
Write-Fixture $files[3] "# project sidecar`nOTHER = 'keep'`n"
Write-Fixture $files[4] "# baseline comment`nHTTP_PROXY_PORT = 'overlay-baseline'`n"

$newVersion = (Invoke-ShineProbe 'new-version' $CurrentBinary @('--version')).stdout.Trim()
$oldVersion = (Invoke-ShineProbe 'old-version' $PreviousBinary @('--version')).stdout.Trim()
# Prime historical default expansion before taking the pre-upgrade backup.
Assert-Value (Invoke-ShineProbe 'old-order-before-upgrade' $PreviousBinary @('env', 'get', 'HTTP_PROXY_PORT')) 'overlay-baseline'
for ($i = 0; $i -lt $files.Length; $i++) { Copy-Item $files[$i] "$backup/$i.toml" }
$original = File-Hashes
Assert-Value (Invoke-ShineProbe 'new-order-after-upgrade' $CurrentBinary @('env', 'get', 'HTTP_PROXY_PORT')) 'project-config'
$explain = Invoke-ShineProbe 'new-explain' $CurrentBinary @('env', 'explain', 'HTTP_PROXY_PORT')
if ($explain.stdout -notmatch 'Value source: project config') { throw 'Incorrect project source' }
Assert-Hashes $original
$retired = Invoke-ShineProbe 'new-retired-force' $CurrentBinary @('env', 'set', 'HTTP_PROXY_PORT', 'ignored', '--force') 1
if ($retired.stderr -notmatch '--overlay') { throw 'Retired script replacement chose the wrong target' }
Assert-Hashes $original
Assert-Value (Invoke-Probe 'new-outside-project' $CurrentBinary @('--config-dir', $state, 'env', 'get', 'HTTP_PROXY_PORT') -WorkingDirectory $outside) 'global-env'
Assert-Value (Invoke-Probe 'old-outside-project' $PreviousBinary @('--config-dir', $state, 'env', 'get', 'HTTP_PROXY_PORT') -WorkingDirectory $outside) 'overlay-baseline'
Assert-Hashes $original
Invoke-ShineProbe 'new-explicit-global' $CurrentBinary @('env', 'set', 'HTTP_PROXY_PORT', 'new-global', '--global') | Out-Null
Assert-Value (Invoke-ShineProbe 'new-global-still-shadowed' $CurrentBinary @('env', 'get', 'HTTP_PROXY_PORT')) 'project-config'
Invoke-ShineProbe 'new-delete-project-port' $CurrentBinary @('env', 'delete', 'HTTP_PROXY_PORT', '--project') | Out-Null
Assert-Value (Invoke-ShineProbe 'new-inherits-global-env' $CurrentBinary @('env', 'get', 'HTTP_PROXY_PORT')) 'global-env'
Invoke-ShineProbe 'new-delete-project-entry' $CurrentBinary @('env', 'delete', 'ROLLBACK_ONLY', '--project') | Out-Null
$afterEdits = File-Hashes
Assert-Value (Invoke-ShineProbe 'old-order-after-downgrade' $PreviousBinary @('env', 'get', 'HTTP_PROXY_PORT')) 'overlay-baseline'
Invoke-ShineProbe 'old-cannot-restore-deleted-entry' $PreviousBinary @('env', 'get', 'ROLLBACK_ONLY') 1 | Out-Null
if ([IO.File]::ReadAllText($files[2]) -match 'ROLLBACK_ONLY|HTTP_PROXY_PORT') { throw 'Deleted project entries reappeared' }
Assert-Hashes $afterEdits
# Downgrading also resumes the old global builtin-default expansion behavior.
Invoke-ShineProbe 'new-delete-builtin-entry' $CurrentBinary @('env', 'delete', 'SOCKS5_PROXY_PORT', '--global') | Out-Null
if ([IO.File]::ReadAllText($files[0]) -match 'SOCKS5_PROXY_PORT') { throw 'New delete retained the explicit builtin entry' }
$withoutBuiltin = File-Hashes
Assert-Value (Invoke-ShineProbe 'new-builtin-fallback-without-write' $CurrentBinary @('env', 'get', 'SOCKS5_PROXY_PORT')) '6153'
Assert-Hashes $withoutBuiltin
Assert-Value (Invoke-ShineProbe 'old-resumes-default-expansion' $PreviousBinary @('env', 'get', 'SOCKS5_PROXY_PORT')) '6153'
if ([IO.File]::ReadAllText($files[0]) -notmatch 'SOCKS5_PROXY_PORT') { throw 'Old default expansion was not observed' }
# Confirm the retired command's old write destination matches the new diagnostic.
Invoke-ShineProbe 'old-force-writes-overlay' $PreviousBinary @('env', 'set', 'HTTP_PROXY_PORT', 'old-force', '--force') | Out-Null
if ([IO.File]::ReadAllText($files[4]) -notmatch 'old-force') { throw 'Old force wrote the wrong target' }

for ($i = 0; $i -lt $files.Length; $i++) { Copy-Item "$backup/$i.toml" $files[$i] -Force }
Assert-Hashes $original
Assert-Value (Invoke-ShineProbe 'old-order-after-backup-restore' $PreviousBinary @('env', 'get', 'HTTP_PROXY_PORT')) 'overlay-baseline'
Assert-Value (Invoke-ShineProbe 'old-restored-entry' $PreviousBinary @('env', 'get', 'ROLLBACK_ONLY')) 'restore-from-backup'
Assert-Value (Invoke-ShineProbe 'new-order-after-backup-restore' $CurrentBinary @('env', 'get', 'HTTP_PROXY_PORT')) 'project-config'
Assert-Hashes $original

# Launch the new executable through both installed PowerShell generations.
foreach ($shell in @('powershell.exe', 'pwsh.exe')) {
    $command = "& '$($CurrentBinary.Replace("'", "''"))' --config-dir '$($state.Replace("'", "''"))' env get HTTP_PROXY_PORT; exit `$LASTEXITCODE"
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    Assert-Value (Invoke-Probe "launch-$shell" $shell @('-NoProfile', '-EncodedCommand', $encoded)) 'project-config'
}
Assert-Hashes $original
[pscustomobject]@{
    status = 'PASS'; root = $root; current = $newVersion; previous = $oldVersion
    commandCount = $records.Count; restoredConfigHashes = $original
} | ConvertTo-Json | Tee-Object -FilePath "$root/result.json"
