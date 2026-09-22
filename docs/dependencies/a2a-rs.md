# Official A2A Rust SDK review

## Decision

This gateway slice uses only the official [A2A Rust SDK
repository](https://github.com/a2aproject/a2a-rs) core and client packages. It
does not include `a2a-server-lf`; hosting the deterministic reference agent is
explicitly deferred to Task 5.

| Dependency alias | Published package | Exact version | Official tag and commit |
| --- | --- | --- | --- |
| `a2a` | `a2a-lf` | `0.3.0` | [release/tag](https://github.com/a2aproject/a2a-rs/releases/tag/a2a-lf-v0.3.0): `a2a-lf-v0.3.0`, `903c7b564feae9fde30828e8037e3d0ea4dd9ca4` |
| `a2a-client` | `a2a-client-lf` | `0.2.3` | [release/tag](https://github.com/a2aproject/a2a-rs/releases/tag/a2a-client-lf-v0.2.3): `a2a-client-lf-v0.2.3`, `33b522ee17449bb93fdbe442aed1edf5498e78c7` |

The two package names differ from their Rust library names: `a2a-lf` exports
the `a2a` library and `a2a-client-lf` exports `a2a_client`. The client release
tag's workspace manifest pins `a2a-lf` at `0.3.0`; this is the compatible core
release adopted here. Cargo requirements use exact (`=`) versions and the lock
file is committed.

## Review evidence

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
  streaming responses. Task 4 does not invoke an A2A task or run an A2A
  server, but its pinned client is retained for the Task 5 adapter boundary.
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
