# shine-template: true
# Set proxy environment variables for the current PowerShell session.
# Also configure tool proxies for Git, NPM, Yarn, and pnpm.
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

    if (Get-Command git -ErrorAction SilentlyContinue) {
        Write-Host "Configuring Git proxy..."
        git config --global http.proxy $ToolProxy
        git config --global https.proxy $ToolProxy
    }

    if (Get-Command npm -ErrorAction SilentlyContinue) {
        Write-Host "Configuring NPM proxy..."
        npm config set proxy $ToolProxy
        npm config set https-proxy $ToolProxy
        npm config set registry https://registry.npmjs.org/
    } else {
        Write-Host "NPM is not installed; skipping"
    }

    if (Get-Command yarn -ErrorAction SilentlyContinue) {
        $yarnVersion = yarn --version
        Write-Host "Configuring Yarn@$yarnVersion proxy..."
        if ($yarnVersion -match '^(2|3)\.') {
            yarn config set httpProxy $ToolProxy
            yarn config set httpsProxy $ToolProxy
        } else {
            yarn config set proxy $ToolProxy
            yarn config set https-proxy $ToolProxy
        }
    }

    if (Get-Command pnpm -ErrorAction SilentlyContinue) {
        Write-Host "Configuring pnpm proxy..."
        pnpm config set proxy $ToolProxy
        pnpm config set https-proxy $ToolProxy
    }

    Write-Host "Proxy setup complete."
    Write-Host ""
    Write-Host "Current proxy configuration:"
    Write-Host "System proxy type: $ProxyType"
    Write-Host "System proxy address: $ProxyAddress"
    Write-Host "Tool proxy address: $ToolProxy"
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
