# Codex Mediation — implementation design

## Status

Approved by the project maintainer in the Codex working session on 2026-10-05,
including this milestone-one specification and the coordinated implementation
plan. Accepted contribution: [issue 1](https://github.com/thoughtkhoral/thought-khoral-agent-gateway/issues/1).
Implementation follows the [plan](https://github.com/thoughtkhoral/thought-khoral/blob/main/.ai/specs/how/codex-room-conversations-implementation-plan.md) and its dependency gates.
Release/tag publication, provider use and service activation require their
separate later authorization. No completed runtime or live verification is claimed.

## Governing sources

- [Local requirements](../what/codex-mediation.md)
- [Root solution design](https://github.com/thoughtkhoral/thought-khoral/blob/main/.ai/specs/how/codex-chat-agent.md)
- [Exact profile](https://github.com/thoughtkhoral/thought-khoral-contracts/blob/thought-khoral-agent-conversation-v1.0.0/.ai/specs/how/agent-conversation-profile.md)
- [Repository tasks and gates](https://github.com/thoughtkhoral/thought-khoral/blob/main/.ai/specs/how/codex-room-conversations-implementation-plan.md)

## Design

Add a pinned Codex registration with identity
74686f75-6768-746b-686f-72616c000004, endpoint
http://thought-khoral-codex-agent:9091, and chat skill. Neither discovery nor an
arbitrary configured URL grants admission. Declare the profile and optional
model/effort/usage controls. A separate Codex dispatcher claims only Codex tasks
through the new workload API; do not generalize deterministic validation into
accepting free text from every agent. Reuse pinned A2A transport, with distinct
worker invocation credentials. Obtain catalog and task receipts through the
reviewed control API, never by reading session files. During execution poll broker
authority at one-second intervals and cancel the bound task on lost authority.
Recovery reconciles completed worker/broker receipts before any dispatch; uncertain
running state fails closed. Retry accepted stored results, never provider turns.

## Verification

Use the exact contract fixture cases, root What acceptance criteria, and assigned
repository tasks in the plan. Scope tests to real protocol/storage/UI behavior;
fake only the provider/app-server boundary where a live dependency is unnecessary.
Publication/runtime execution requires accepted issue links and written review.
Do not claim live model, history, sandbox, or egress coverage from schema tests.

## Reviewed catalog bridge implementation detail

The opt-in mediator exposes only `GET /internal/agent-conversations/v1/models`
at fixed internal `http://thought-khoral-agent-gateway:9092`. It requires
`THOUGHT_KHORAL_CODEX_CATALOG_BRIDGE_BIND=0.0.0.0:9092` and a distinct
`THOUGHT_KHORAL_CODEX_CATALOG_BRIDGE_SECRET`, held only by broker and mediator.
This secret is separate from worker invocation, Keycloak client and Reference
Agent secrets. The bridge rejects query strings, request bodies and duplicate
authorization headers, authenticates and checks the pinned worker card, then
returns one policy-bounded exact published CatalogPage with null nextCursor.
It exposes no task or receipt controls. The broker's CatalogQuery adapter uses
this fixed route; neither this implementation nor configuration activates the
service or grants agent admission. These details implement the approved bounded
authenticated mediation surface; deployment activation remains Task 8.

## Task 6 local implementation checkpoint — 2026-10-06

The provider-free implementation is committed on local branch
`codex-conversation-mediation`; runtime source through
`dd4ab0c2c2ea6a718699c37ce68d43171c0b1ccf` preserves the deterministic
Reference Agent alongside separate pinned Codex dispatch and validation.
All 66 local tests pass, including 44 existing deterministic checks. Formatting,
all-target feature-enabled Clippy with warnings denied, exact 135-file contract
pin verification and whitespace checks pass. The existing 299-package lock graph
is unchanged; no new dependency versions were introduced.

Real synthetic HTTP fixtures exercise admission, integer-preserving A2A input,
private durable receipts, worker artifact/binding comparison, broker authority
and cancellation, catalog authentication, prior disclosed citations, and
normalized runtime settings/usage. Completed or uncertain receipts prevent
another worker submission. Lost terminal acknowledgements replay exact stored
output; revoked/expired records are quarantined, while transient per-record
failures permit unrelated room claims. Deadline greater than lease and stale
usage without a context window are rejected. Native runtime IDs remain private.

Independent scoped review resolved recovery starvation and deadline checks;
cross-component review found and corrected broker pagination and null-window
usage semantics. The coordinated plan records the final reviewed revisions and
worker image evidence. These changes remain local and unmerged, with configuration
disabled by default. UI, opt-in activation and full-stack/provider verification
remain Tasks 7–9. No live provider, tool isolation or egress coverage is claimed.
