# Remove proxy environment variables for the current PowerShell session.
# Also clear global proxy settings for Git, NPM, Yarn, and pnpm.
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

Write-Host "Clearing system environment variable proxies..."
$ProxyVars = @(
    'http_proxy',
    'https_proxy',
    'HTTP_PROXY',
    'HTTPS_PROXY',
    'all_proxy',
    'ALL_PROXY',
    'no_proxy',
    'NO_PROXY'
)

foreach ($Name in $ProxyVars) {
    Remove-Item "Env:\$Name" -ErrorAction SilentlyContinue
}

if (Get-Command git -ErrorAction SilentlyContinue) {
    Write-Host "Clearing Git proxy..."
    git config --global --unset http.proxy 2>$null
    if ($LASTEXITCODE -ne 0) { Write-Host "  Git http.proxy was not set or has already been cleared" }
    git config --global --unset https.proxy 2>$null
    if ($LASTEXITCODE -ne 0) { Write-Host "  Git https.proxy was not set or has already been cleared" }
}

if (Get-Command npm -ErrorAction SilentlyContinue) {
    Write-Host "Clearing NPM proxy..."
    npm config delete proxy 2>$null
    npm config delete https-proxy 2>$null
} else {
    Write-Host "NPM is not installed; skipping"
}

if (Get-Command yarn -ErrorAction SilentlyContinue) {
    Write-Host "Clearing Yarn proxy..."
    $yarnVersion = yarn --version
    if ($yarnVersion -match '^(2|3)\.') {
        yarn config delete httpProxy 2>$null
        yarn config delete httpsProxy 2>$null
    } else {
        yarn config delete proxy 2>$null
        yarn config delete https-proxy 2>$null
    }
} else {
    Write-Host "Yarn is not installed; skipping"
}

if (Get-Command pnpm -ErrorAction SilentlyContinue) {
    Write-Host "Clearing pnpm proxy..."
    pnpm config delete proxy 2>$null
    pnpm config delete https-proxy 2>$null
} else {
    Write-Host "pnpm is not installed; skipping"
}

Write-Host ""
Write-Host "Proxy settings have been removed."
Write-Host ""
Write-Host "Notes:"
Write-Host "  - Environment variable proxies have been cleared (current terminal session only)"
Write-Host "  - Global proxy settings for Git/NPM/Yarn/pnpm have been cleared"
Write-Host "  - To set the proxy again, run: setproxy"
