# Remove proxy environment variables for the current PowerShell session.
# Yarn proxy settings are also cleared because setproxy may update Yarn config.
# Usage: usetproxy

Write-Host "Removing proxy configuration..."

if ($env:http_proxy -or $env:HTTP_PROXY) {
    Write-Host "Current detected proxy settings:"
    if ($env:http_proxy) { Write-Host "  http_proxy: $env:http_proxy" }
    if ($env:https_proxy) { Write-Host "  https_proxy: $env:https_proxy" }
    if ($env:HTTP_PROXY) { Write-Host "  HTTP_PROXY: $env:HTTP_PROXY" }
    if ($env:HTTPS_PROXY) { Write-Host "  HTTPS_PROXY: $env:HTTPS_PROXY" }
    if ($env:all_proxy) { Write-Host "  all_proxy: $env:all_proxy" }
    if ($env:ALL_PROXY) { Write-Host "  ALL_PROXY: $env:ALL_PROXY" }
    Write-Host ""
} else {
    Write-Host "No proxy environment variables were detected"
}

Write-Host "Clearing session environment variable proxies..."
$ProxyVars = @(
    'http_proxy',
    'https_proxy',
    'HTTP_PROXY',
    'HTTPS_PROXY',
    'all_proxy',
    'ALL_PROXY',
    'no_proxy',
    'NO_PROXY',
    'npm_config_proxy',
    'npm_config_https_proxy',
    'npm_config_registry',
    'NPM_CONFIG_PROXY',
    'NPM_CONFIG_HTTPS_PROXY',
    'NPM_CONFIG_REGISTRY'
)

foreach ($Name in $ProxyVars) {
    Remove-Item "Env:\$Name" -ErrorAction SilentlyContinue
}

if (Get-Command yarn -ErrorAction SilentlyContinue) {
    Write-Host "Clearing Yarn proxy config..."
    Write-Host "  Yarn proxy settings are persistent; removing the entries set by setproxy."
    $yarnVersion = yarn --version
    if ($yarnVersion -match '^(2|3|4)\.') {
        yarn config delete httpProxy 2>$null
        yarn config delete httpsProxy 2>$null
    } else {
        yarn config delete proxy 2>$null
        yarn config delete https-proxy 2>$null
    }
} else {
    Write-Host "Yarn is not installed; skipping"
}

Write-Host ""
Write-Host "Proxy settings have been removed."
Write-Host ""
Write-Host "Notes:"
Write-Host "  - Environment variable proxies have been cleared (current terminal session only)"
Write-Host "  - Git/npm/pnpm global config was not modified"
Write-Host "  - Yarn proxy config was cleared if Yarn was available"
Write-Host "  - To set the proxy again, run: setproxy"
