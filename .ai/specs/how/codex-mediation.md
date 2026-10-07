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

## Task 9 synthetic verification and correction checkpoint — 2026-10-07

The following defaults-discovery amendment status supersedes this pre-amendment F1/default-discovery disposition; Task 9 remains open for release gates.

Provider-free checkpoint only; Task 9 and the milestone remain open.

Task-scoped verification review: Approved. Broad implementation review: Partial
spec compliance; quality Needs follow-up. B1–B3 (pending-ack recovery, omitted
shared settings and receipt-correlated safe failures) are addressed. B4 is
partial: explicit initial/reset selection works for full-capability admission,
but automatic server-default display requires an approved interface amendment.
F1: unresolved Important reasoning-only UI deadlock. Optional capabilities are
independent; an effort-only admission cannot establish the guard-required model
through its hidden selector. This prevents initial/reset invocation and blocks
whole-milestone/merge readiness. No second broad fix wave or waiver is implied.

| Owner | Final reviewed local revision |
|---|---|
| contracts | `85baf86e574276fcd036e53e23641af6aad602f9` |
| broker | `fd05cb48b8508e7939f9cdf9df275742a06fc4f8` |
| mediator | `6c3d96b4763871b9addc9bc7223e71ee7d38abd9` |
| worker | `b0d43ec2b5b0c8da035d4ccff754545132b978d4` |
| ui | `e51d67e9e1a986601df6b5e1acf68aaf7ae0870d` |
| platform | `637a69279f0fe5019560b1e54d28f48c1c715897` |

All six reviewed worktrees were clean when this checkpoint was prepared.
Runtime is committed only on isolated local branches; originals retain their
runtime/scaffold and unrelated edits. Contracts v1.0.0 and dependency lockfiles
remain unchanged.

Controller final verification on platform revision above: `node
scripts/smoke-codex-conversation.mjs --fake` (session85023) exit0, six original
crash/commit boundaries, 11 fake native turns, exact baseline/delta/source IDs,
targeted/cross-room exclusion, duplicate=one logical turn, worker restart and
fresh reset, shared omitted settings, rejected-completion recovery and exact
execution_failed/session_unavailable/runtime_unavailable projections. `node
--test scripts/tests/codex-conversation-smoke.test.mjs` (session23304) exit0,
13 passed, zero failed. This is synthetic native/identity/private-DNS adapter
coverage, not whole packaged Compose, real Keycloak/browser or provider proof.

Inspected owner logs and independent review record broker150 passed + one
pre-existing ignored live test; mediator65 passed, zero failed/ignored (correcting
the earlier reported68); worker46; UI121 + pin/tamper checks and production
build. Owner fixture tests3, actual assertion-failure/SIGTERM/SIGINT cleanup3,
package/startup checks10 passed. Earlier contracts/regression/legal/pin evidence
is retained with original attribution, not presented as rerun here.

New ARM64 worker image:
`sha256:2d8bfade27802f910cf68e832722c93b4a2acc2addb825711e1223617a4cd385`.
Compiled runtime revision `b418a76e0e7ca047b5fe995eb17519aced369a06`; worker
head above adds evidence documentation. Immutable image readiness checks used
network-none/read-only/cap-drop-all, both admission markers; default invocation
refused as expected. Actual native 44-setting/eight-model/resume/six unsolicited
tool refusal evidence remains attributed to its earlier source/image, not this
new image. CLI/catalog/control hashes are unchanged. x86_64 native admission and
Rust1.85 minimum-version checks remain unrun.

Default discovery: pending specification approval. The local proposed How is
`thought-khoral-codex-agent/.ai/specs/how/default-settings-discovery-proposal.md`.
It proposes a read-only authenticated defaults query in a new immutable v1.1.0
artifact and independent mixed-capability controls, covering absent conversation
and explicit New/reset. It authorizes no runtime or published contract changes.
Live provider verification: pending. Account/model availability, actual native
history, live tool/egress/key isolation, packaged deployment/private DNS and real
browser/identity evidence remain separately gated. No merge, push, publication,
service activation or provider inference occurred.

Independent review artifacts are retained outside Git at
`/private/tmp/codex-conversation-task9/final-fix-review.md`,
`final-implementation-review.md`, and `task9-fix-review.md`; owner evidence at
`/private/tmp/Task9-final-fix-evidence/`. Final root/documentation/source-reference
and identity gate results will be recorded in the controller checkpoint after
these source-derived record updates. The aggregate release checklist remains
unchecked; passing synthetic checks do not resolve F1 or default discovery.

## Defaults-discovery amendment status — 2026-10-07

The amendment was approved and its four-task local candidate passed independent
reviews and the composed synthetic verification. The UI finding F1 is accepted
as corrected for that candidate. Mediation behavior was intentionally unchanged:
the composed candidate used this repository's reviewed revision
`6c3d96b4763871b9addc9bc7223e71ee7d38abd9` and the published v1.0 profile.
This repository's default branch retains its prior runtime; the reviewed
mediator revision remains on its local branch. Publication, packaged-stack
verification and authorized live-provider checks remain pending. Full evidence
is in the [coordinated checkpoint](https://github.com/thoughtkhoral/thought-khoral/blob/main/.ai/specs/how/codex-room-conversations-implementation-plan.md).

## Safe failed-turn projection clarification (2026-10-07)

JSON-RPC negative error codes are envelope metadata, parsed separately from
strict nonnegative profile values. Duplicate/unknown envelope keys, mismatched
RPC IDs, mixed result/error envelopes, fractional/unsafe counters, and unknown
error strings cannot authorize success or a specific safe failure. A known
worker rejection is forwarded only after an authenticated receipt read matches
the requested task, conversation and generation, is terminal failed/interrupted,
and has the same bounded profile error code with no result, acknowledgement or
runtime binding. The same strict receipt rule applies to recovered failures.
Transport/malformed/unknown states remain conversation_interrupted. Raw error
text never becomes a broker update or public message; no profile artifact or
counter validation changes.

## Local main integration checkpoint — 2026-10-07

The reviewed mediator at source commit
`6c3d96b4763871b9addc9bc7223e71ee7d38abd9` is integrated into this repository's
local `main` under the user's explicit merge authorization. It remains on the
published v1.0 conversation profile; the unreleased v1.1 candidate is consumed
by contracts, broker, and UI only. Fresh serial `cargo test --locked --offline`
passed. Publication, packaged-stack verification, provider use, activation and
push remain separate gates.

## Pushed POC checkpoint — 2026-10-07

The reviewed mediator implementation is pushed to GitHub `main` in
`efb29b3f9afa3ed51ddad409a66cd48cf8bf59dd`. Provider-free tests passed in the
reviewed source. Packaged-stack, identity/browser, and separately authorized
live verification remain open; production readiness is not claimed.
