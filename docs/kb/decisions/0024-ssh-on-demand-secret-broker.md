# 0024 — SSH secrets use a local, policy-bound on-demand broker

- **Status**: accepted
- **Evidence**: `cli/src/ssh/broker.rs`, `cli/src/env/broker.rs`,
  `cli/src/env/workspace.rs`, `docs/ssh-secret-broker-prd.md`

## Context

`shine ssh --with-secret` decrypts before starting SSH and embeds plaintext in the remote session
wrapper. It is convenient for a few explicitly forwarded values, but exposes them for the whole
session and cannot bind release to a particular project snapshot or command. Keeping a second
decryptable private key on the remote host would weaken YubiKey/Secure-Enclave-backed local trust.

The remote host and session token cannot be treated as authorization boundaries: both the token
and remote process environment may be observable after a remote compromise. At the same time,
requiring a project policy for every one-off command would make the safer path too cumbersome.

## Decision

Reuse the authenticated SSH reverse control channel for an on-demand broker. Ciphertext and private
key operations remain local; the remote receives plaintext only in the environment of the requested
child process. The protocol treats every remote field as untrusted and applies strict size and
character limits plus per-session nonce replay rejection.

Two authorization modes are distinct:

- Direct requests require an explicit per-session `--allow-secret KEY[=ALIAS]` and an unskippable
  local TTY confirmation. They decrypt only the stored `KEY_SECRET` ciphertext.
- Workspace requests require an exact local policy binding the SSH target, raw workspace hash,
  ordered source paths and hashes, mode, complete declared secrets, release mapping, and argv.
  Confirmation may be suppressed only by explicit `--trust-remote-session`.

Workspace authorization uses one immutable in-memory snapshot: the files hashed for the request
are the files parsed after approval. Policies are generated from a trusted local checkout by
default. Remote metadata can only be enrolled through a separate, explicitly trusted command and
local confirmation. The policy file is local-only security state, rejected if symlinked,
wrong-owned, or broader than `0600`, and replaced atomically.

## Consequences

- Hardware-backed decryption stays on the local machine and occurs only when requested.
- Plaintext still exists on the remote in the launched child's environment and memory; this is
  narrower exposure, not protection from a compromised remote kernel or privileged process.
- Exact hashes intentionally make source changes fail closed until the local policy is reviewed and
  updated.
- The broker is POSIX-remote-only in this version because secure confirmation and the reverse Unix
  socket lifecycle are not yet implemented for Windows remote shells.
- Existing eager `--with-secret` behavior remains available and unchanged for simple trusted uses.
