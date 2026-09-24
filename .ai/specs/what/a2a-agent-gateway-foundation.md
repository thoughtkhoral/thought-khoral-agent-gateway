# A2A agent gateway foundation

## Status

Approved for the local deterministic reference integration. Remote agent
admission, MCP transport, and production deployment remain deferred.

## Purpose

Establish `thought-khoral-agent-gateway` as the zero-trust mediated boundary
between a governed ThoughtKhoral room and one locally controlled A2A reference
agent. Prove that humans and agents can collaborate from the same authorized,
provenance-bearing room context without exposing the room store or decision
authority to the agent runtime.

## Scope

- Consume the official `a2aproject/a2a-rs` SDK through a pinned, reviewed
  release as the A2A client implementation.
- Register exactly one local deterministic reference agent through a pinned
  Agent Card and endpoint.
- Support exactly two declared skills: `summarize-context` and
  `extract-action-items`.
- Build an expiring Room Context Packet containing the full ordered room
  history authorized for the invoking human and agent, active decisions,
  provenance, and a context revision.
- Invoke the selected A2A skill, receive task status/artifact updates, and
  submit only validated normalized updates to the room gateway.
- Preserve a human-readable, source-cited result and meaningful in-progress
  task status in the governed room.

## Acceptance criteria

1. A human can invoke either declared reference-agent skill from a governed
   room; no other skill or agent is invocable.
2. The agent receives a complete, ordered packet of every event it is
   authorized to see, the active-decision projection, source IDs, a high-water
   sequence, task binding, and expiry.
3. The deterministic `summarize-context` result cites source event IDs, and
   `extract-action-items` returns only explicit parsed action items while
   citing its source message.
4. A2A status updates are represented as durable room task-progress events;
   rate-limited updates never obscure terminal task outcomes.
5. No agent or agent-gateway process has direct PostgreSQL, room-event-store,
   active-context-transition, filesystem, shell, or arbitrary outbound-network
   authority.
6. Forged, duplicate, stale, expired, oversized, unsupported, rate-excessive,
   or task/revision-mismatched A2A updates are rejected without a room event.
7. A user can see an optional agent-provided external-handoff instruction and
   HTTPS destination, but ThoughtKhoral never auto-opens, embeds, or collects
   data from that external experience.

## Interfaces

The agent gateway consumes a task-scoped room-gateway service interface for
authorized context snapshots and normalized task-update submission. It uses
A2A v1 Agent Card discovery and task invocation only for the registered local
reference agent. Cross-project interfaces remain contract-owned and require
additive schemas, protocol documentation, and compatibility fixtures before
runtime work starts.

## Explicit exclusions

- Remote or user-supplied agent admission
- MCP transport or tool integration
- Model-backed execution, agent-side tools, credentials, or autonomous side
  effects
- Context compaction, deltas, semantic retrieval, and agent-private context
  leases
- Agent-to-agent task invocation
- Browser-embedded or ThoughtKhoral-owned human-in-the-loop forms
- Direct agent access to room storage, decisions, or memory-engine storage
