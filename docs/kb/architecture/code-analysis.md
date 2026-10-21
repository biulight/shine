# Preset code analysis implementation

Status: analysis core implemented; author-tool and runtime admission integration pending.
Contract: [ADR 0150](../decisions/0150-preset-code-analysis-admission.md).

## Source rules, version 1

`core/src/analysis/` accepts captured category-relative source bytes, parsed declarations and an
environment identity contract. It has no filesystem/process Host, ambient environment lookup,
network client or write API. URL parsing is local. `Session` caches source facts while comparing
each entry's declarations and environment independently. Cache hits preserve cumulative budgets.

The parser uses pinned Tree-sitter 0.25.10, JavaScript grammar 0.23.1, TypeScript grammar 0.23.2,
and language bindings 0.1.9. The maintainer authorized Rust 1.90 for the latest language binding.
Recovered/missing syntax nodes block with safe positions; raw parser text is never exported.
Locations are one-based lines and UTF-8 byte columns. Public diagnostics contain category-relative
paths, behavior classes, codes and optional opaque target hashes; no source excerpt or raw target.

| Rule | Supported syntax and comparison | Explicit limitation |
| --- | --- | --- |
| Environment | `process.env.NAME`, `Bun.env.NAME`, literal string subscripts; entry source/target/sensitivity mapping; exact fixed-variable names | Dynamic keys, unknown source identity and whole-env-object access are gaps; plain assignment/delete is not a read, compound assignment/update is |
| Network | Unshadowed `fetch` and supported immutable aliases, literal HTTP(S) URL; URL-normalized host compared with exact host or `any` | Dynamic/escaped URL literals are gaps; credentials, URL path/query/fragment never appear in reports |
| Node files | `readFile[Sync]`, `createReadStream`, `writeFile[Sync]`, `appendFile[Sync]`, `createWriteStream`, `unlink[Sync]`, `rm[Sync]`, `rmdir[Sync]`; supported `fs.promises` forms | Literal paths only; exact existing logical permission identities, no invented globs/inheritance; ambiguous native path spellings are gaps |
| Bun files | `Bun.write`; direct `Bun.file(literal).text/json/arrayBuffer/bytes/stream/exists/delete/writer` | Constructing a BunFile alone is not a read; indirect BunFile aliases and dynamic paths are gaps |
| Commands | `Bun.spawn[Sync]`, Node `spawn[Sync]`, `execFile[Sync]`, `fork`; literal executable or first literal array element | `fork` module paths and Node options are not treated as definite executable identities; every subprocess retains an independent effects gap; dynamic executable adds an identity gap; shell `exec[Sync]` is dynamic code |
| Dynamic code | Unshadowed `eval`, dynamic `require`, `import`, recognized Bun shell invocation | No execution, remote module download or recursive subprocess analysis |
| Bindings | Builtin namespace/default/named imports and named import aliases; direct immutable `const` aliases; parameter/block/function/catch/loop shadowing | Ambiguous/reassigned bindings and unsupported calls are gaps; no flow-sensitive propagation or interpretation of arbitrary values |
| Dependencies | Static imports/re-exports and literal `require`; category-relative paths, cycles, exact filenames or unique extension/index candidate | Escapes, missing/ambiguous dependencies, unsupported builtin APIs/languages and packages have stable gap codes; no host module resolution |

Uncorroborated declarations do not block. They also do not remove gaps. Known command identity
plus a subprocess gap remains incomplete even if file/network declarations are supplied.
Only difference and failure diagnostics block, and all diagnostic classes can coexist.

## Resource limits

Defaults are 1 MiB per source; 256 files and depth 32 per dependency graph; 16 MiB and 2,048 unique
source files per command; 50,000 accumulated facts/comparison diagnostics, 2,000,000 syntax nodes
and 100,000 parser progress callbacks. Byte, file and parser work are charged on cache misses;
per-entry diagnostic work is still charged on cache hits. These are conservative work limits,
not a time/memory guarantee or a claim of exhaustive analysis. Binding lookup is indexed by name;
more than 128 candidates for a name is unresolved rather than allowing unbounded scope matching.

Local file/graph limits and syntax errors produce blocking reports that other independent entries
can continue past. Cancellation and cumulative limits latch a terminal failure on the session,
including for later cache hits. Parser interruption and iterative node traversal check that state.
Core orchestration must propagate it before scheduling more source work or script processes.

## Evidence so far

- Six real production entry files in Surge and Clash Verge parse successfully. Largest entry:
  13,979 bytes; largest reached graph: two files. Repeated analysis is identical and adds no parses.
- Without entry-specific env mapping, these real samples yield zero definite differences and
  4–213 gap locations per entry. Most targets are dynamic and many calls are user-defined;
  these numbers are diagnostic inventory, not a security/coverage percentage or proof that their
  declarations match. Runtime integration must supply each actual environment contract.
- Manually labelled rule fixtures cover literal host/path/command differences, import aliases,
  shadowing, source-to-target mappings, env writes versus reads, unknown subprocess effects,
  dependency cycles/missing/escape/ambiguity, parse failures, privacy and cumulative limits.
- A real Rust 1.90 `cargo check --locked --workspace` passed on macOS after the MSRV update.
  Windows/Linux native parser builds and the full admission acceptance matrix are still pending.

Do not describe this module's presence as automatic runtime enforcement until the Core execution
gates, author reports and CLI projections are connected and independently validated.
