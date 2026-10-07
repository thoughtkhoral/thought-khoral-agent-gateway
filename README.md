# ThoughtKhoral agent gateway

`thought-khoral-agent-gateway` is the mediated boundary between governed rooms
and admitted local A2A agents. It uses the Room
Context Broker and is intentionally not a general remote-agent or MCP runtime.

## Status

The local foundation is runnable only through the sibling ThoughtKhoral
platform's rootless development stack. See [the verification
procedure](docs/verification/a2a-foundation.md). It accepts exactly the pinned
Reference Agent and its two deterministic skills by default. The approved
[Codex mediation extension](.ai/specs/how/codex-mediation.md) is merged and
pushed to GitHub `main` at `efb29b3f9afa3ed51ddad409a66cd48cf8bf59dd`.
Provider-free mediator tests passed. Codex remains opt-in; packaged-stack and
separately authorized live-provider verification remain open. See the [shared
status guide](https://github.com/thoughtkhoral/thought-khoral/blob/main/docs/codex-conversation-status.md) for the cross-project state and remaining gates.

The gateway pins a reviewed retained-v1 room contract revision in
[`contracts/lock.json`](contracts/lock.json). Its authenticated internal task
interface and local A2A transport are separate from the browser room protocol;
neither grants direct room storage or active-decision authority.

With rootless Podman and sibling platform, room-gateway, memory-engine, and UI
checkouts available, run the integration gate from the platform directory:

```sh
podman-compose up --build -d
bash scripts/smoke.sh
```

For the gateway's own tests, run `cargo fmt --check` and `cargo test` here.
Verify the independently published conversation artifact with
`python3 scripts/check_conversation_pin.py`. Codex routing uses its own exact
profile validators and private transport records; the deterministic validator
retains its existing contract. The broker owns room authorization and storage.

Read the [local specification index](.ai/specs/README.md), the
[deferred capability specification](.ai/specs/what/deferred-agent-gateway.md),
[approved local A2A foundation](.ai/specs/what/a2a-agent-gateway-foundation.md),
and the [ThoughtKhoral repository map](https://github.com/thoughtkhoral/thought-khoral/blob/main/docs/repository-map.md).

## Contributing

Use an issue to propose scope, contracts, security boundaries, or evidence for
an implementation plan. Changes remain governed by the approved What, How,
and implementation plan. See the
[organization contribution guide](https://github.com/thoughtkhoral/.github/blob/main/CONTRIBUTING.md).
