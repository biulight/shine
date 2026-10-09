# ADR 0105: Serialize App lifecycles with persistent OS operation locks

Date: 2026-10-09
Status: Accepted

## Context

App operations load and save an entire manifest. Atomic file replacement alone cannot prevent two
operations from discarding each other's receipts. The admin directory lock also removed its lock
after a 30-second wait even when the owner was alive; that owner's later destructor could remove
the replacement owner's lock.

## Decision

- Add a scoped operation-lock port to `FileSystemHost`. Approved App install, uninstall, upgrade,
  generator refresh, artifact execution, and recovery lock `<shine_dir>/app-lifecycle.lock` before
  replanning and approval validation, retaining the guard through effects and receipt saves.
- Keep privileged transaction serialization separate. Acquire the App lock first, then the global
  privileged lock where required; privileged self-install shares only the latter.
- Implement both RealHost locks with `fs2` exclusive OS locks on persistent regular files. Poll
  contention for at most 30 seconds and fail without deleting or replacing the lock file. Dropping
  the file handle or exiting the process releases ownership. InMemoryHost uses keyed async mutexes.
- Use `$TMPDIR/shine-admin.lockfile` for the privileged lock, avoiding collision with a legacy
  `shine-admin.lock` directory. On Unix reject symlink lock files and create files with mode 0600.
- Preserve existing snapshot-bound approval: waiting is not authorization for changed state.
  Files marked Preserve are removed from execution assessment before any generator can run.

## Consequences

Concurrent unrelated App changes retain all receipts; relevant changes require a new reviewed Plan.
Long-running operations can make another command time out, but do not lose their lock. Persistent
lock files are infrastructure, not abandoned operation journals, and must not be removed to unlock
a running process. Old binaries using the directory-lock protocol do not coordinate with the new
privileged lock. Cross-process lock behavior is covered by real file-lock tests; native Windows
execution still requires platform validation.
