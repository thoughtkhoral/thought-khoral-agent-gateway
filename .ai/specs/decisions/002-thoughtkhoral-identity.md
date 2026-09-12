# 002 — ThoughtKhoral project identity

## Status

Accepted

## Decision

This repository implements root [Decision 003 — ThoughtKhoral product identity](../../../../.ai/specs/decisions/003-thoughtkhoral-product-identity.md). The planned direct-child identifier is renamed exactly from `n2n-agent-gateway` to `thought-khoral-agent-gateway`; no `n2n-agent-gateway` repository was instantiated before this accepted name was applied.

The existing `n2n.room.v1` protocol values remain wire-compatible and unchanged. Database identifiers, database contents, persisted records, persisted fields, and persisted values are excluded from this rename.

## Consequences

- This repository contains local specifications only and introduces no runtime code.
- A future implementation, protocol rename, or data rename requires separate approved specifications and migration decisions.
