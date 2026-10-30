# 0158 — App secret files use explicit ownership and locally authorized file release

- **Status**: implemented; native Windows and production activation require separate acceptance
- **Evidence**: `core/src/runtime/secret_files/`, `core/src/runtime/planner/app_secrets.rs`,
  `cli/src/apps/secret_files.rs`, `cli/src/ssh/file_broker.rs`

## Decision

App metadata v3 declares workspace v3 file groups through `[[secret_files]]`. Workspace and
ciphertext are ordinary managed auxiliary files; plaintext is a separate secret resource, never a
generator fallback or transform. Mappings stay inside the installed workspace. Source presets
containing a declared plaintext target are rejected before cache extraction or packing.

Core owns selection, equal-content adoption, local-edit protection, approved mutations, receipts
and recovery. A CLI provider supplies bounded decryption and native private I/O. Review observes
keyed fingerprints without decrypting. After approval, preparation decrypts only the explicit
needed subset; the complete Plan is regenerated and checked before destination writes. Unselected
and preserved resources have no execution authority. Defaults retain ownership while automatic
decryption is disabled; stale removal is explicit. Ordinary resources cannot claim a secret-owned
target. Secret relocation is blocked pending explicit removal of the previous installation.

Fingerprints use domain-separated HMAC-SHA256 and a private installation key. Before the first
mutation, a process-stable random key can support read-only review; it becomes durable before
secret receipts or journals. A missing or changed durable key blocks existing ownership operations.
No plaintext fingerprint enters public reports. Missing configuration is not permission to
reconstruct or discard secret ownership.

The private secret manifest and journal use version 2. Before secret mutation, App manifest v2
is persisted so older clients fail closed. A per-file journal binds preimage identity, content
fingerprint and permissions, desired receipt and observed applied identity. Updates first restrict the old file to installation-user private access, then rename it to a
same-directory rollback path, privately create the new file, commit its receipt,
recheck the destination and remove unchanged rollback material. Recovery completes committed
cleanup or restores the preimage and its original permissions; replacements and changed receipts block. This is per-file
recovery, not category-wide atomicity or secure erasure. Hostile final-interval filesystem races
remain outside the portable guarantee.

## SSH boundary

Protocol v4 adds file-status discovery and bounded, nonce/index/offset/length-associated chunks.
Frames remain bounded at 2 MiB and decoded chunks at 256 KiB. Each session admits one file body
transfer at a time; requests and responses retain existing file-group limits. Disconnect during
local approval/decryption cancels the request. Buffers carrying plaintext and encoded plaintext
are zeroized. No target is created until the complete response arrives.

Policy format v2 adds distinct file policies; v1 environment policies remain readable. File
policy creation/update explicitly migrates the store. A file policy binds SSH target, operation,
workspace, full mapping/source hashes, explicit allowed release set, destination and permissions;
App requests additionally bind effective category metadata and snapshot identity. A request may
release a subset of that explicit set; it cannot add future files or change a source. Inspect and
trusted enrollment never release plaintext. File confirmations retain the local TTY guard through
backend interaction. Session tokens alone grant no release authority.

App operations prefer an enabled broker session and never fall back after broker failure. Manual
file export requires `--secret-broker` explicitly and retains exclusive creation, private defaults,
and user-managed plaintext. Private keys stay local, but plaintext necessarily becomes readable
to the authorized remote account and privileged remote processes. The remote path shown during
approval is a release constraint, not a claim to enforce filesystem behavior on a compromised host.

## Compatibility and operations

Existing ordinary App metadata v1/v2 remains supported; metadata v3 must not be silently migrated
back to v2. Both SSH peers require protocol v4. Existing App manifests are read compatibly and
written as v2. Plan, approval and review schema v4 reject clients unaware of secret resources. The secret manifest/key/journal must be retained with the installation state for
recovery. Windows uses private current-user ACLs, with no Unix ownership overrides; SSH file
release still requires a POSIX remote. Service startup, container recreation and CA volume
backup remain separate operations.
