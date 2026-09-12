# Deferred agent gateway

## Planned responsibility

`thought-khoral-agent-gateway` is reserved for a future, independently approved boundary between governed rooms and external agent runtimes. This repository records the project boundary and identity only; it does not authorize an implementation.

## Compatibility boundary

Any future implementation must consume `n2n.room.v1` without changing its wire values and must preserve existing database identifiers and persisted values unless a separate migration decision is accepted.

## Explicit exclusions

Runtime code, packages, binaries, agent integrations, tool execution, credentials, deployment artifacts, database schemas, and migrations are excluded from this specification-only repository.
