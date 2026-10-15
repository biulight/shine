# 0110 — Explicit workspace secret file groups use scoped plaintext ownership

- **Status**: accepted; native Linux/macOS/Windows regression CI passed; hardware validation pending
- **Evidence**: `cli/src/env/workspace/file_groups.rs`, `tests/workspace_secret_files.rs`,
  [PRD](../../workspace-secret-files-prd.md)

## Decision

Workspace version 3 adds named `file_groups.<name>.files` mappings from ciphertext `source`
to plaintext `target`. Version 2 remains supported; old readers reject version 3. File-only
workspaces may omit env sources. Repeatable `--file-group` explicitly selects groups; `--mode`
expands both paths. Ordinary env commands never implicitly release file groups.

Sealing reads targets and writes sources using existing GPG/age/hybrid policies and storage
encoding. Binary APIs preserve empty/non-UTF-8 data without changing env string APIs. Optional
`--remove-plaintext` verifies all written ciphertext before starting deletion. Multi-file writes
and deletion are not transactions; partial progress is reported. Builds never write secrets back.

Run captures and decrypts all selected sources before privately creating absent targets. New
Unix files/directories use 0600/0700. Windows uses protected current-user DACLs and fails on
filesystems without persistent ACLs. Cleanup holds handles and compares stable object identities,
retains replaced files, removes only owned empty directories, and precedes exit-code propagation.
Unix process groups and Windows suspended-start Job Object assignment contain ordinary descendants;
detached background tasks are outside the supported workflow.

Paths remain inside the workspace; links, hard-link aliases, case/Unicode aliases, overlaps, tracked or
non-ignored Git plaintext targets fail closed. Git inspection disables fsmonitor and ambient Git
redirection. Persistent per-user, path-hashed OS locks coordinate seal/run across config directories.
Workspace/mapping overlap checks and lock hashes share canonical Unicode decomposition plus
lowercasing, conservatively rejecting equivalent names even on case-sensitive filesystems.
Git checks start at each target's closest existing parent, honor nested repository policy, and
reject ambiguous discovery failures. Execution previews use stderr to preserve child stdout;
dry-run previews remain on stdout.
Final snapshot checks protect cooperating processes; hostile final-interval filesystem races are
outside the portable guarantee. Each plaintext is bounded to 16 MiB, with 64 MiB per invocation.

## Consequences

Applications consume ordinary files without runtime Shine coupling. The build entry point needs
Shine, and authorized children/local users can read plaintext. Abrupt termination may leave files;
subsequent runs refuse existing targets without automatically claiming or deleting them. Cleanup
is not secure erasure. No new plaintext cache, broker release protocol, crypto scheme, SOPS dependency,
or recipient revocation guarantee is introduced.

## Validation

Native Linux, macOS and Windows regression suites, quality/security audit and Rust 1.88 checks
passed for `5da8168` on 2026-10-13 ([CI](https://github.com/biulight/shine/actions/runs/38350246715)).
Hardware authorization and real Android signing remain unverified; automated platform regression
does not establish those acceptance results.
