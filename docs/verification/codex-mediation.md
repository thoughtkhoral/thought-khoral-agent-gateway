# Codex mediation verification

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
