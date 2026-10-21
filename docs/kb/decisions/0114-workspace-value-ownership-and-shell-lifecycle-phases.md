# ADR 0114: Workspace value ownership and Shell lifecycle phases

- **Date**: 2026-10-16
- **Status**: Accepted

## Context

Workspace execution, export and cache paths relied on manual plaintext cleanup after successful
awaits. Errors and cancelled futures could bypass that cleanup. Local compilation and broker
release separately encoded source precedence. Shell orchestration mixed target selection, resource
preparation, journal execution, receipt persistence and reporting in large functions. Manifest and
Action receipt consumers also independently validated the same launcher delivery combinations.

## Decision

- Own compiled and explicit environment maps in `SensitiveValues`, which wipes replaced values
  and remaining owned values on drop. Its consuming iterator retains the cleanup guard for
  unconsumed entries. Serialized/decrypted plaintext buffers use `Zeroizing`. Child execution
  releases the owned maps before propagating a process exit status. Existing public broker map
  APIs transfer ownership explicitly at their successful return boundary.
  Replace decrypted-cache parser errors with a generic diagnostic so their source snippets cannot
  outlive the zeroizing buffer or enter warning logs.
- Compute the winning plain/secret declaration and source index through a shared pure resolver.
  Omitted secrets still shadow earlier plain values. Decrypt and validate every sealed source,
  including shadowed sources; complete broker declarations and policy hashes remain independent
  of effective winners and release approval.
- Split Shell deployment and removal into selection, preparation, application and report phases
  with domain-specific state types. Retain the outer lifecycle lock, fresh Plan approval and
  existing filesystem/journal/receipt ordering. Preparation retains legacy effects and is not an
  observation-only planner. Recovery remains the existing typed Action executor.
- Validate receipt mode, runtime, dependency mode and launcher-format combinations once in
  `action/shell_receipt.rs`, using borrowed internal enums. Manifest reconstruction, Action IR
  validation and recovery conversion share those rules. Preserve persisted DTO fields, schema
  versions and supported legacy/native-cmd-v2/live-bun-v2 encodings. Unknown dependency modes
  are rejected consistently before recovery as well as launcher reconstruction.

## Consequences

Cleanup follows ownership on normal completion, early errors and cancellation. This covers owned
buffers; it does not promise erasure of allocator, serializer, OS or child-process copies. Source
precedence, complete declaration validation, wire formats and approval boundaries remain intact.
Regression coverage includes failed export publication, cancelled compilation consumers, shadowed
pending secrets, receipt configuration compatibility, and partial Shell uninstall with receipt
failure, explicit recovery and retry while retaining a sibling command's shared state.
