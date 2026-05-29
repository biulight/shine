# shine-template: true
# Set proxy environment variables for the current PowerShell session.
# Git, npm, and pnpm inherit these values from the current PowerShell session.
# Yarn proxy settings are updated in Yarn config because they are not reliably session-scoped.
# Usage: setproxy [auto|sock5|http]

$HttpProxyPort = '@@HTTP_PROXY_PORT@@'
$Socks5ProxyPort = '@@SOCKS5_PROXY_PORT@@'
$ProxyHost = '@@PROXY_HOST@@'
$NoProxy = '@@PROXY_NO_PROXY@@'
$HttpProxy = "http://${ProxyHost}:${HttpProxyPort}"
$Socks5Proxy = "socks5://${ProxyHost}:${Socks5ProxyPort}"

function Test-ShineTcpPort {
    param(
        [Parameter(Mandatory = $true)]
        [string]$HostName,
        [Parameter(Mandatory = $true)]
        [int]$Port
    )

    $client = [System.Net.Sockets.TcpClient]::new()
    try {
        $connect = $client.BeginConnect($HostName, $Port, $null, $null)
        if (-not $connect.AsyncWaitHandle.WaitOne(500)) {
            return $false
        }
        $client.EndConnect($connect)
        return $true
    } catch {
        return $false
    } finally {
        $client.Close()
    }
}

function Set-ShineProxy {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ProxyAddress,
        [Parameter(Mandatory = $true)]
        [string]$ProxyType
    )

    $ToolProxy = "http://${ProxyHost}:${HttpProxyPort}"

    $env:http_proxy = $ProxyAddress
    $env:https_proxy = $ProxyAddress
    $env:HTTP_PROXY = $ProxyAddress
    $env:HTTPS_PROXY = $ProxyAddress
    $env:all_proxy = $ProxyAddress
    $env:ALL_PROXY = $ProxyAddress
    $env:no_proxy = $NoProxy
    $env:NO_PROXY = $NoProxy

    $env:npm_config_proxy = $ToolProxy
    $env:npm_config_https_proxy = $ToolProxy
    $env:npm_config_registry = 'https://registry.npmjs.org/'
    $env:NPM_CONFIG_PROXY = $ToolProxy
    $env:NPM_CONFIG_HTTPS_PROXY = $ToolProxy
    $env:NPM_CONFIG_REGISTRY = 'https://registry.npmjs.org/'

    $YarnConfigured = $false

    if (Get-Command yarn -ErrorAction SilentlyContinue) {
        $yarnVersion = yarn --version
        Write-Host "Yarn proxy cannot be reliably scoped to this shell; updating Yarn@$yarnVersion config..."
        if ($yarnVersion -match '^(2|3|4)\.') {
            yarn config set httpProxy $ToolProxy
            yarn config set httpsProxy $ToolProxy
        } else {
            yarn config set proxy $ToolProxy
            yarn config set https-proxy $ToolProxy
        }
        $YarnConfigured = $true
    }

    Write-Host "Proxy setup complete."
    Write-Host ""
    Write-Host "Current proxy configuration:"
    Write-Host "System proxy type: $ProxyType"
    Write-Host "System proxy address: $ProxyAddress"
    Write-Host "Tool proxy address: $ToolProxy"
    Write-Host "Scope: current PowerShell session for Git/npm/pnpm-compatible environment variables"
    if ($YarnConfigured) {
        Write-Host "Yarn config was updated because Yarn proxy settings are not reliably session-scoped."
    }
}

$Mode = if ($args.Count -gt 0) { $args[0] } else { 'auto' }

switch ($Mode) {
    'auto' {
        Write-Host "Auto mode: checking SOCKS5 proxy first..."
        if (Test-ShineTcpPort -HostName $ProxyHost -Port ([int]$Socks5ProxyPort)) {
            Write-Host "SOCKS5 proxy is available; using SOCKS5 first"
            Set-ShineProxy -ProxyAddress $Socks5Proxy -ProxyType 'SOCKS5'
        } else {
            Write-Host "SOCKS5 proxy is unavailable; falling back to HTTP proxy"
            Set-ShineProxy -ProxyAddress $HttpProxy -ProxyType 'HTTP'
        }
    }
    'sock5' {
        Write-Host "Forcing SOCKS5 proxy..."
        Set-ShineProxy -ProxyAddress $Socks5Proxy -ProxyType 'SOCKS5'
    }
    'http' {
        Write-Host "Forcing HTTP proxy..."
        Set-ShineProxy -ProxyAddress $HttpProxy -ProxyType 'HTTP'
    }
    default {
        Write-Error "Invalid argument: $Mode"
        Write-Host "Usage: setproxy [auto|sock5|http]"
        return 1
    }
}

Write-Host ""
Write-Host "To remove the proxy settings, run: usetproxy"
