#!/usr/bin/env bash
# Throwaway spike for docs/ssh-local-transfer-prd.md section 9.1.
#
# Verifies, against a REAL remote host you already have password-less SSH
# access to, whether the architecture sketched in the PRD actually works:
#
#   1. `ssh -R <remote-unix-sock>:<local-unix-sock>` forwards a Unix domain
#      socket end to end (local <- remote), carrying real bytes.
#   2. Session identity/token can be handed to the remote shell purely via
#      an `env FOO=bar ... exec "$SHELL" -l` wrapper, without SendEnv/
#      AcceptEnv, without breaking TTY allocation.
#   3. Whether sshd removes the remote-side socket file automatically once
#      the SSH connection closes, or whether shine must clean it up itself
#      (e.g. via a shell EXIT trap in the wrapper command).
#
# This script makes NO changes to the shine binary or repo state; it is a
# manual diagnostic tool. Delete it once the spike findings are recorded.
#
# Usage:
#   ./scripts/spike-ssh-forward.sh <ssh-host-or-alias>
#
# Requirements:
#   - password-less SSH to <ssh-host-or-alias> (key-based auth already set up)
#   - `python3` available both locally and on the remote host
#   - remote host must support OpenSSH >= 6.7 (Unix domain socket forwarding)
#
# What to report back after running this:
#   - Full stdout/stderr of this script
#   - Whether it printed "RESULT: PASS" or "RESULT: FAIL" for each check
#   - If you also use ControlMaster/ProxyJump for this host, re-run once
#     with your normal ~/.ssh/config in place (default) to confirm it still
#     works, since that's the realistic case `shine ssh` must support.

set -euo pipefail

HOST="${1:?usage: $0 <ssh-host-or-alias>}"
RUN_ID="$$-$(date +%s)"
LOCAL_SOCK="/tmp/shine-spike-local-${RUN_ID}.sock"
REMOTE_SOCK="/tmp/shine-spike-remote-${RUN_ID}.sock"
LOCAL_RESULT_FILE="/tmp/shine-spike-result-${RUN_ID}.bin"
LISTENER_SCRIPT="/tmp/shine-spike-listener-${RUN_ID}.py"
REMOTE_SENDER_SCRIPT="/tmp/shine-spike-sender-${RUN_ID}.py"
TOKEN="test-token-${RUN_ID}"

cleanup() {
  echo "[spike] cleaning up local temp files"
  rm -f "$LOCAL_SOCK" "$LISTENER_SCRIPT" "$LOCAL_RESULT_FILE" 2>/dev/null || true
  if [[ -n "${LISTENER_PID:-}" ]]; then
    kill "$LISTENER_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo "[spike] target host: $HOST"
echo "[spike] local socket: $LOCAL_SOCK"
echo "[spike] remote socket: $REMOTE_SOCK"
echo

# --- 1. Write the local Unix-socket listener -------------------------------
cat > "$LISTENER_SCRIPT" <<'PYEOF'
import socket
import sys
import os

path = sys.argv[1]
out_path = sys.argv[2]

try:
    os.unlink(path)
except FileNotFoundError:
    pass

srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
srv.bind(path)
os.chmod(path, 0o600)
srv.listen(1)
sys.stderr.write("LISTENING\n")
sys.stderr.flush()

conn, _ = srv.accept()
data = b""
while True:
    chunk = conn.recv(4096)
    if not chunk:
        break
    data += chunk
conn.close()
srv.close()

with open(out_path, "wb") as f:
    f.write(data)
PYEOF

echo "[spike] starting local listener..."
python3 "$LISTENER_SCRIPT" "$LOCAL_SOCK" "$LOCAL_RESULT_FILE" 2>/tmp/shine-spike-listener-stderr-${RUN_ID}.log &
LISTENER_PID=$!

# Wait for the listener to actually bind before invoking ssh -R against it.
for _ in $(seq 1 50); do
  if [[ -S "$LOCAL_SOCK" ]]; then
    break
  fi
  sleep 0.1
done
if [[ ! -S "$LOCAL_SOCK" ]]; then
  echo "[spike] RESULT: FAIL - local listener never bound $LOCAL_SOCK"
  exit 1
fi
echo "[spike] local listener is up"
echo

# --- 2. Write the remote Unix-socket sender ---------------------------------
ssh "$HOST" "cat > $REMOTE_SENDER_SCRIPT" <<'PYEOF'
import socket
import sys

path = sys.argv[1]
msg = sys.argv[2].encode()

s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(path)
s.sendall(msg)
s.close()
PYEOF

echo "[spike] wrote remote sender script to $HOST:$REMOTE_SENDER_SCRIPT"
echo

# --- 3. Open the forwarded session, mimicking the real `shine ssh` wrapper -
# This is the actual thing under test: `-R remote:local` forwarding, plus
# handing the remote shell a session token purely through `env FOO=bar exec`,
# with a TTY allocated (-t) as a real interactive login would need.
echo "[spike] opening ssh -R session (this should feel like a normal interactive login)..."
ssh -t -R "${REMOTE_SOCK}:${LOCAL_SOCK}" "$HOST" "
  env SHINE_SPIKE_SOCK='${REMOTE_SOCK}' SHINE_SPIKE_TOKEN='${TOKEN}' bash -lc '
    echo \"[remote] SHINE_SPIKE_SOCK=\$SHINE_SPIKE_SOCK\";
    echo \"[remote] SHINE_SPIKE_TOKEN=\$SHINE_SPIKE_TOKEN\";
    python3 ${REMOTE_SENDER_SCRIPT} \"\$SHINE_SPIKE_SOCK\" \"hello-from-remote:\$SHINE_SPIKE_TOKEN\";
    echo \"[remote] sent payload over forwarded socket\";
    if [[ -S \"\$SHINE_SPIKE_SOCK\" ]]; then
      echo \"[remote] socket file still present immediately after send (expected)\";
    fi
    exit
  '
"
SSH_EXIT=$?
echo "[spike] ssh session exited with code $SSH_EXIT"
echo

# --- 4. Check what the local listener actually received ---------------------
sleep 0.3
if [[ -s "$LOCAL_RESULT_FILE" ]]; then
  RECEIVED="$(cat "$LOCAL_RESULT_FILE")"
  echo "[spike] local listener received: $RECEIVED"
  if [[ "$RECEIVED" == "hello-from-remote:${TOKEN}" ]]; then
    echo "[spike] RESULT: PASS - forwarded Unix socket carried correct bytes end-to-end"
  else
    echo "[spike] RESULT: FAIL - received unexpected payload"
  fi
else
  echo "[spike] RESULT: FAIL - local listener received nothing (forwarding did not work)"
fi
echo

# --- 5. Check whether the remote socket file was cleaned up after disconnect
echo "[spike] checking remote socket cleanup after session close..."
if ssh "$HOST" "test -e '${REMOTE_SOCK}'" 2>/dev/null; then
  echo "[spike] RESULT: sshd did NOT remove the remote socket file automatically."
  echo "[spike]         shine must clean it up itself (EXIT trap in the wrapper command)."
  ssh "$HOST" "rm -f '${REMOTE_SOCK}' '${REMOTE_SENDER_SCRIPT}'" 2>/dev/null || true
else
  echo "[spike] RESULT: sshd removed the remote socket file automatically on disconnect."
fi

echo
echo "[spike] done. Please paste the full output above back for analysis."
