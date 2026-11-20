# Codex mediation verification

## Current workspace context — 2026-11-20

Conversation v1.1 is published and the released broker/UI consumers and packaged profile are verified on local branches. This mediator retains its compatible published-v1.0 pin and runtime baseline `48a72bc`; no new mediator code or authority is introduced. Native/package/synthetic preparation evidence does not close assembled-stack/private-DNS/browser/live-provider gates. The deterministic reference-agent path and deferred remote/MCP boundary remain unchanged.

Runtime source and contract pins are unchanged. Exact clean pinned-source
snapshots were retained before this documentation-only checkout advanced.

Derived from the [local specification index](../../.ai/specs/README.md) and
[root workspace authority](https://github.com/thoughtkhoral/thought-khoral/blob/main/.ai/specs/how/workspace-state.md).
The cited tests retain their original source identities; this documentation
refresh adds no activation, publication or runtime acceptance.

This document derives from the approved [local mediation design](../../.ai/specs/how/codex-mediation.md)
and [requirements](../../.ai/specs/what/codex-mediation.md). The
[coordinated plan](https://github.com/thoughtkhoral/thought-khoral/blob/main/.ai/specs/how/codex-room-conversations-implementation-plan.md)
owns cross-repository sequencing and activation gates.

The mediator admits only Codex identity `74686f75-6768-746b-686f-72616c000004`,
its reviewed card, `chat` skill and internal worker authority
`http://thought-khoral-codex-agent:9091`. Invocation credentials are distinct
from broker workload credentials and remain outside Codex app-server requests.
The broker owns room history, disclosure, permissions, leases and ordinary
message commit. The mediator stores private transport bindings and normalized
completion receipts in a separately configured state directory.

The retained deterministic Reference Agent remains pinned to loopback and its
ten-second execution contract. Codex uses separate claim, validation, authority,
update and recovery paths. Lost authority cancels bound work. Recovery queries
broker and worker receipts before any possible submission; uncertain execution
is interrupted, and completed output replays without another provider turn.
Worker native thread/turn identifiers are checked against its durable receipt
and never enter broker updates or browser responses.

The published conversation profile is vendored under
[`contracts/agent-conversation-v1`](../../contracts/agent-conversation-v1/lock.json).
Its release commit, archive digest and all 135 content hashes are pinned.

Run the local provider-free checks:

```sh
python3 scripts/check_conversation_pin.py
cargo test --locked --offline --features reference-agent-server
cargo fmt --check
cargo clippy --locked --offline --all-targets --features reference-agent-server -- -D warnings
git diff --check
```

The conversation tests use real local HTTP services for broker and A2A worker
boundaries. They inspect requests and receipt replay rather than model answers.
These checks do not exercise a live provider, deployment egress or native tool
isolation. The independent worker owns its image and legal notices; the platform
owns opt-in activation. Their later verification remains required.
