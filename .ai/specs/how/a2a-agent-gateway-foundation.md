# A2A agent gateway foundation — design

## Status

Approved for the local deterministic reference integration under the root
implementation plan. Production workload identity and remote admission remain
separate work.

## Governing specifications

- Root [Decision 007 — A2A agent gateway foundation](https://github.com/thoughtkhoral/thought-khoral/blob/main/.ai/specs/decisions/007-a2a-agent-gateway-foundation.md)
- Local [What: A2A agent gateway foundation](../what/a2a-agent-gateway-foundation.md)
- Root [Decision 005 — Room-scoped POC memory](https://github.com/thoughtkhoral/thought-khoral/blob/main/.ai/specs/decisions/005-room-scoped-poc-memory.md)
- Root [Decision 006 — Initial room agent task dispatch](https://github.com/thoughtkhoral/thought-khoral/blob/main/.ai/specs/decisions/006-agent-task-dispatch.md)

## Architecture

The implementation has four isolated responsibilities:

| Component | Responsibility | Authority it does not have |
| --- | --- | --- |
| Room gateway | Authorize room operations, assemble authorized context, persist and deliver governed events. | External-agent transport or execution. |
| Agent gateway | Pin/register the local agent, mediate A2A, validate incoming A2A updates, and translate them to room-gateway submissions. | Direct database access, room authorization decisions, or active-context mutation. |
| Reference agent | Execute two deterministic A2A skills from one supplied context packet. | Room credentials, room APIs, database/filesystem/shell access, unrestricted egress. |
| Workspace UI | Render normalized task progress, terminal results, citations, and an optional external-handoff link. | A2A invocation, event synthesis, or auto-navigation. |

The agent gateway uses the official `a2aproject/a2a-rs` core types and client.
It should use A2A's Agent Card, Task, status-update, artifact, and protocol
negotiation semantics rather than duplicate a parallel external-agent wire
format. A2A library types are transport adapters, not domain types: local
domain records remain independent of the dependency.

## Invocation and context flow

1. A human requests one of the registered agent's allowed skills through an
   additive room contract.
2. The room gateway authenticates and authorizes the human, persists the task
   request, and allocates a ThoughtKhoral task ID.
3. The agent gateway requests a context snapshot with the task ID, room ID,
   requester identity, agent identity, selected skill, and expiry.
4. The room gateway applies normal room-delivery visibility rules before
   assembling `RoomContextPacket`; targeted events invisible to the requester
   or agent are absent.
5. The packet contains ordered normalized events, active decisions, event and
   decision source identifiers, high-water sequence (`contextRevision`),
   generation time, expiry, and integrity binding. It is serialized as
   structured JSON in an A2A data part.
6. The agent gateway invokes the matching skill on the pinned local A2A agent
   and records a mapping between ThoughtKhoral and A2A task/context IDs.
7. The local worker validates streamed A2A status/artifact updates and submits
   a normalized update to the room gateway. Push callbacks are a later
   capability, not part of the local reference path. The room gateway
   persists it before delivery.
8. The UI renders only room-gateway events. It displays source citations by
   their persisted identifiers and presents external-handoff URLs only after a
   human click.

## Packet and task invariants

`RoomContextPacket` is an immutable input snapshot, not a grant of standing
access. Its binding covers `taskId`, `roomId`, requester, registered agent,
selected skill, context revision, issue time, and expiry. An agent result must
cite zero or more event/decision IDs present in that packet; a result cannot
claim citation to unseen or out-of-room sources.

The initial full-authorized-history packet is intentional. A later compact or
delta packet must preserve the same packet identity, authorization filtering,
revision semantics, and reconstructable provenance. It may not silently turn
an agent cache into the source of truth.

## A2A update mapping

The A2A task lifecycle is canonical across the agent boundary. The room
contract gets additive normalized events for task acceptance, meaningful
progress, external-input waiting, terminal success/failure, and result
artifacts. Progress is durable only for meaningful state changes and is
coalesced/rate-limited. Terminal events are never coalesced.

An external handoff may expose a plain-language instruction, HTTPS URL,
destination host, task ID, and expiry. It cannot carry credentials, form
responses, or opaque sensitive payloads. The agent owns the external UX and
signals a later task update after that UX completes.

## Zero-trust controls

- Use short-lived, audience-bound service credentials for every room-gateway ↔
  agent-gateway call; the local reference uses a distinct inbound bearer secret
  for A2A. Production requires mutual TLS and workload identity.
- Pin the local Agent Card URL, card identity, allowed skills, endpoint,
  transport/protocol version, and permitted handoff origins. Treat discovery
  as untrusted metadata, not admission.
- Verify signatures when cards are signed. Reject identity, endpoint,
  capability, or protocol-version drift until an explicit administrator action
  accepts it.
- Validate A2A schema and content size before parsing; bind every update to the
  expected A2A task, ThoughtKhoral task, agent, and context revision.
- Apply per-agent/task rate limits, update ordering/idempotency rules, timeouts,
  and terminal-state immutability.
- Do not dereference artifact URLs, callback URLs, or handoff URLs in the
  gateway. Validate them as data; only the user may open a rendered handoff.
- Keep secrets out of packets, event payloads, logs, progress messages, and
  artifacts. Record only safe identifiers and failure codes in audit logs.

## Deterministic reference-agent behavior

`summarize-context` produces a deterministic summary from the supplied packet
and an ordered list of source event IDs it used. `extract-action-items` uses
the existing explicit-line grammar (`- text`, optional `owner:` and `due:`
fields) against the invocation message and emits a structured result that
cites that message event. Neither skill calls a model, tools, shell commands,
the filesystem, a database, or external network resources.

## Verification

- Unit tests prove packet filtering, ordering, binding, expiry, and citation
  validation.
- A2A adapter tests use the official SDK against the deterministic local
  reference agent and cover Agent Card pinning, task creation, streaming
  status, artifacts, and terminal errors.
- Contract fixtures cover additive task invocation, progress, handoff, and
  result event shapes without breaking retained room-protocol consumers.
- Integration tests reject forged, duplicate, stale, expired, oversized,
  out-of-order, wrong-agent, wrong-task, wrong-revision, and rate-excessive
  updates before persistence.
- End-to-end tests invoke both skills from a human room client, observe durable
  progress and source-cited results, and prove hidden targeted messages are
  absent from the reference agent packet.
