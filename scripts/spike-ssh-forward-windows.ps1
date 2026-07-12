<#
.SYNOPSIS
Throwaway spike for docs/ssh-local-transfer-prd.md section 9.1, Windows
variant: verifies the "Windows is local, remote is Linux/macOS" case.

Scope decision already made (see chat): only the LOCAL agent changes for
Windows support. It listens on a loopback TCP port instead of a Unix
socket. The REMOTE side is unchanged from the already-verified Unix spike
(scripts/spike-ssh-forward.sh) — remote still listens on a Unix socket
path and still gets its session env vars via the same
`env FOO=bar bash -lc '...'` wrapper, since the remote host is Linux/macOS.

This verifies:
  1. `ssh -R <remote-unix-sock>:127.0.0.1:<local-port>` — a MIXED forward
     (Unix socket on the remote end, TCP on the local end) — actually
     carries bytes end to end. OpenSSH has supported Unix domain socket
     forwarding since 6.7, and mixing socket/TCP endpoints on either side
     is documented behavior, but this has not been tested against a real
     Windows OpenSSH client (Win32-OpenSSH) before now.
  2. Whether Windows' bundled ssh.exe is new enough / behaves correctly
     for this mixed forward type.
  3. Whether the remote socket file is cleaned up automatically on
     disconnect (expected: NO, matching the Unix-only spike's finding —
     this is a property of the remote sshd, not the Windows client, so it
     should be identical, but worth reconfirming here).
  4. Whether Windows Defender Firewall prompts/blocks a loopback-only TCP
     listener (loopback traffic should never require a firewall prompt,
     but note in your results if one appears).

This script makes NO changes to the shine binary, its config, or repo
state; it is a manual diagnostic tool. Delete it once findings are
recorded.

.PARAMETER SshHost
An SSH destination (alias from ~/.ssh/config or user@host) that you
already have password-less SSH access to. Must be Linux or macOS.

.EXAMPLE
.\scripts\spike-ssh-forward-windows.ps1 -SshHost dev

.NOTES
Requirements:
  - Windows 10/11 with the built-in OpenSSH client (ssh.exe on PATH).
  - password-less SSH to -SshHost already working.
  - `python3` on the REMOTE host (not required locally on Windows).

What to report back after running this:
  - Full console output of this script.
  - The "RESULT: PASS/FAIL" lines.
  - Output of `ssh -V` (printed by this script) so version-specific
    OpenSSH-for-Windows quirks can be identified if something fails.
  - Whether a Windows Defender Firewall prompt appeared at any point.
#>

param(
    [Parameter(Mandatory = $true)]
    [string]$SshHost
)

$ErrorActionPreference = "Stop"

Write-Host "[spike] ssh client version:"
& ssh -V

$runId = [guid]::NewGuid().ToString("N").Substring(0, 12)
$remoteSock = "/tmp/shine-spike-remote-$runId.sock"
$token = "test-token-$runId"
$senderScriptRemote = "/tmp/shine-spike-sender-$runId.py"

# --- 1. Start a loopback TCP listener, letting the OS pick a free port. ---
$listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
$listener.Start()
$localPort = $listener.LocalEndpoint.Port
Write-Host "[spike] local TCP listener on 127.0.0.1:$localPort"
Write-Host "[spike] remote socket: $remoteSock"

# Begin accepting asynchronously (APM pattern) so `ssh` can run in the
# foreground afterward exactly like a real interactive session would.
$asyncAccept = $listener.BeginAcceptTcpClient($null, $null)

# --- 2. Write the remote Unix-socket sender (same script as the Unix spike) ---
$pySender = @'
import socket
import sys

path = sys.argv[1]
msg = sys.argv[2].encode()

s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(path)
s.sendall(msg)
s.close()
'@

$pySender | & ssh $SshHost "cat > $senderScriptRemote"
Write-Host "[spike] wrote remote sender script to ${SshHost}:${senderScriptRemote}"

# --- 3. Open the forwarded session: mixed unix-socket(remote)<->TCP(local) ---
# The remote command wrapper is unchanged from the Unix spike: env vars set
# via `env`, then a POSIX shell runs the sender and reports back.
$innerScript = 'echo "[remote] SHINE_SPIKE_SOCK=$SHINE_SPIKE_SOCK"; ' +
    "python3 $senderScriptRemote " + '"$SHINE_SPIKE_SOCK" "hello-from-remote:$SHINE_SPIKE_TOKEN"; ' +
    'echo "[remote] sent payload over forwarded socket"; exit'

Write-Host "[spike] opening ssh -R session (mixed unix-socket/TCP forward)..."
& ssh -t -R "${remoteSock}:127.0.0.1:${localPort}" $SshHost `
    "env SHINE_SPIKE_SOCK=$remoteSock SHINE_SPIKE_TOKEN=$token bash -lc '$innerScript'"
$sshExitCode = $LASTEXITCODE
Write-Host "[spike] ssh session exited with code $sshExitCode"

# --- 4. Check what the local listener actually received -------------------
if (-not $asyncAccept.IsCompleted) {
    Start-Sleep -Milliseconds 300
}

if ($asyncAccept.IsCompleted) {
    $client = $listener.EndAcceptTcpClient($asyncAccept)
    $stream = $client.GetStream()
    $ms = New-Object System.IO.MemoryStream
    $buffer = New-Object byte[] 4096
    while ($true) {
        $read = $stream.Read($buffer, 0, $buffer.Length)
        if ($read -le 0) { break }
        $ms.Write($buffer, 0, $read)
    }
    $received = [System.Text.Encoding]::UTF8.GetString($ms.ToArray())
    Write-Host "[spike] local listener received: $received"
    if ($received -eq "hello-from-remote:$token") {
        Write-Host "[spike] RESULT: PASS - mixed unix-socket/TCP forward carried correct bytes end-to-end"
    } else {
        Write-Host "[spike] RESULT: FAIL - received unexpected payload"
    }
    $client.Close()
} else {
    Write-Host "[spike] RESULT: FAIL - local listener never received a connection (forwarding did not work)"
}
$listener.Stop()

# --- 5. Check whether the remote socket file was cleaned up after disconnect
Write-Host "[spike] checking remote socket cleanup after session close..."
& ssh $SshHost "test -e '$remoteSock'" 2>$null
if ($LASTEXITCODE -eq 0) {
    Write-Host "[spike] RESULT: sshd did NOT remove the remote socket file automatically."
    Write-Host "[spike]         (expected — matches the Unix-only spike's finding)"
    & ssh $SshHost "rm -f '$remoteSock' '$senderScriptRemote'" 2>$null | Out-Null
} else {
    Write-Host "[spike] RESULT: sshd removed the remote socket file automatically on disconnect."
    Write-Host "[spike]         (unexpected — differs from the Unix-only spike's finding, note this!)"
}

Write-Host ""
Write-Host "[spike] done. Please paste the full output above back for analysis."
