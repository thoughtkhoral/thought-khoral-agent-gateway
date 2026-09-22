# Source provenance and bounded-stream patch

Source: official `a2aproject/a2a-rs`, published `a2a-client-lf` 0.2.3,
tag `a2a-client-lf-v0.2.3`, commit
`33b522ee17449bb93fdbe442aed1edf5498e78c7`.

Crates.io archive SHA-256:
`ab51522c48871829447ee296d65f98918a93891440d31dedd9800145cbaad159`.
The crate's `.cargo_vcs_info.json` records packaging commit
`4fdb6a9e6016978cb35e3f91cc50ffd056ce21b5`. Its entire `src/` tree was
compared byte-for-byte with the official tag archive above: identical before
the local patch. The packaging commit and release tag commit are distinct.
The normalized published Cargo.toml, original Cargo.toml, README, and source
files are retained. LICENSE.md is the official tag's Apache-2.0 license.
The upstream files in src/ are byte-identical except `jsonrpc.rs`.

Local modifications (2026-09-22):

- Bound streaming JSON-RPC fallback bodies to 65,536 bytes before parsing.
- Bound buffered SSE bytes to 65,536, total SSE bytes to 131,072, and raw SSE
  frames (including ignored comments) to eight. Reject before extending the
  buffer when the bound would be exceeded. Terminate after a limit error.
- Reject EOF within an unfinished frame.
- Add one upstream-module regression for unterminated data/comment floods;
  repository integration tests also exercise the official transport over HTTP.

This is a local security patch to the official transport, not a claim that the
unmodified release supplies these limits. The fixed gateway protocol still
admits exactly four typed events. Application-level timeout is ten seconds,
further capped by the packet lease deadline minus two seconds for failure
submission. Revisit/remove the patch when an upstream release provides
equivalent raw-stream limits. Review upstream changes before updating.
