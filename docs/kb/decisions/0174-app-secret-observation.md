# 0174 — Observe App secret files without decryption or write access

- **Status** — Accepted
- **Date** — 2026-11-25

App status needs private receipts and keyed fingerprints. The CLI supplies only those
operations to its query frontend. Approved mutations receive the full provider during
separate runtime assembly. A caller that omitted secret observation receives
`frontend_app_secret_observation_unavailable` when inspecting a declared secret or querying
existing private state.

For hypothetical install plans, a filesystem observation Host can establish that a target,
rollback path, receipt or journal is absent. An existing object needs the private observer.
This preserves empty synthetic planning and prevents an unavailable fingerprint from being
reported as an absent file. Receipt, journal, approval and CLI report formats stay compatible.
