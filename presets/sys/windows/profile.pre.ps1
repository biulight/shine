# Managed by `shine sys bootstrap` for Windows. Existing user config is left untouched.

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
