# Official A2A Rust SDK review

## Decision

The agent-gateway binary uses only the official [A2A Rust SDK
repository](https://github.com/a2aproject/a2a-rs) core and client packages.
Task 5 adds the official server package only behind the
`reference-agent-server` feature, required by the separate
`thought-khoral-reference-agent` binary. The dispatch binary does not enable
that feature.

| Dependency alias | Published package | Exact version | Official tag and commit |
| --- | --- | --- | --- |
| `a2a` | `a2a-lf` | `0.3.0` | [release/tag](https://github.com/a2aproject/a2a-rs/releases/tag/a2a-lf-v0.3.0): `a2a-lf-v0.3.0`, `903c7b564feae9fde30828e8037e3d0ea4dd9ca4` |
| `a2a-client` | `a2a-client-lf` | `0.2.3` | [release/tag](https://github.com/a2aproject/a2a-rs/releases/tag/a2a-client-lf-v0.2.3): `a2a-client-lf-v0.2.3`, `33b522ee17449bb93fdbe442aed1edf5498e78c7` |
| `a2a-server` (reference-agent feature only) | `a2a-server-lf` | `0.4.3` | [release/tag](https://github.com/a2aproject/a2a-rs/releases/tag/a2a-server-lf-v0.4.3): `a2a-server-lf-v0.4.3`, `cd5a4a8fdcd3e69a505481f49e179af5315dec98` |

The package names differ from their Rust library names: `a2a-lf` exports the
`a2a` library, `a2a-client-lf` exports `a2a_client`, and `a2a-server-lf`
exports `a2a_server`. Cargo requirements use exact (`=`) versions and the lock
file is committed. The lockfile checksum for the optional server package is
`f9ea7cf23a50c687610120890983dda279e008e87f8003ff02b8e5dfbc9c17ef`.

For source-archive provenance, the official client tag archive downloaded from
`https://github.com/a2aproject/a2a-rs/archive/refs/tags/a2a-client-lf-v0.2.3.tar.gz`
on 2026-09-22 had SHA-256
`f37578041b7ce1bb275a9e2a00a474c811db1d3077712257c1f4f2394a3f23e4`.
This archive is the source snapshot for client tag commit
`33b522ee17449bb93fdbe442aed1edf5498e78c7`; the core tag/commit remains
separately recorded in the table above.

## Review evidence

The client is now vendored via `[patch.crates-io]` so limits can be enforced
before the official SSE parser allocates unbounded frames or ignores comment
floods. See [patch provenance](../../vendor/a2a-client-lf/THOUGHT-KHORAL-PATCH.md)
for the exact published source checksum, retained Apache license, and local
changes. Core and optional server remain the exact published dependencies.

- **Official source and release:** `git ls-remote --tags
  https://github.com/a2aproject/a2a-rs.git a2a-client-lf-v0.2.3
  a2a-lf-v0.3.0`, run 2026-09-22, returned the commits above. The source
  project's [README](https://github.com/a2aproject/a2a-rs/blob/a2a-client-lf-v0.2.3/README.md)
  identifies it as the A2A Rust SDK.
- **License:** the tagged workspace declares `license = "Apache-2.0"`, and
  its [`LICENSE.md`](https://github.com/a2aproject/a2a-rs/blob/a2a-client-lf-v0.2.3/LICENSE.md)
  contains Apache License 2.0.
- **MSRV:** the tagged workspace declares `rust-version = "1.85"`. The build
  environment uses Rust 1.93.1, which satisfies that floor.
- **Bindings:** the official README documents JSON-RPC 2.0 over HTTP and
  HTTP+JSON/REST client bindings; it also documents Server-Sent Events for
  streaming responses. Task 5 uses the official JSON-RPC server router only
  for the local deterministic Reference Agent and validates its stream through
  the pinned official JSON-RPC client transport. It does not expose REST,
  gRPC, push, or any non-local endpoint.
- **Advisory check:** `cargo audit --deny warnings`, 2026-09-22. The project
  security policy identifies `cargo-audit`/RustSec as its advisory source. The
  command result is recorded below against the committed lockfile.

## Advisory-check result

The development image did not initially include `cargo-audit` (`cargo audit
--version` reported `no such command: audit`), so version 0.22.2 was installed
to a temporary directory with its own `--locked` dependency graph. The
following completed successfully on 2026-09-22:

```text
cargo-audit 0.22.2
Loaded 1261 security advisories
Scanning Cargo.lock for vulnerabilities (252 crate dependencies)
```

`cargo audit --deny warnings` exited 0: no RustSec vulnerability or warning
was reported for this lockfile. Package provenance, release tag/commit,
licensing, MSRV, and official binding support were reviewed before the manifest
was created; the advisory scan was then run against the resulting exact graph.
