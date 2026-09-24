# Deferred agent gateway

## Status

Superseded for the local reference integration by the approved
[A2A foundation](a2a-agent-gateway-foundation.md). Remote third-party
admission, MCP, and production deployment remain deferred.

## Planned responsibility

This historical placeholder reserved `thought-khoral-agent-gateway` as the
boundary between governed rooms and external runtimes. It did not authorize
runtime work; the later A2A foundation and root implementation plan do so for
one deterministic local reference agent only.

## Compatibility boundary

The approved local foundation communicates with the room gateway through a
separate authenticated, task-scoped internal interface. It does not change
retained `n2n.room.v1` wire values, database identifiers, or persisted values
without a separate migration decision.

## Explicit exclusions

This placeholder does not authorize general runtime admission, tool execution,
production credentials or deployment, or database schemas and migrations.
