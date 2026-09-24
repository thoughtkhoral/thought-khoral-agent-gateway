# ThoughtKhoral agent gateway

`thought-khoral-agent-gateway` is the mediated boundary between governed rooms
and the locally controlled deterministic A2A reference agent. It uses the Room
Context Broker and is intentionally not a general remote-agent or MCP runtime.

## Status

The local foundation is runnable only through the sibling ThoughtKhoral
platform's rootless development stack. See [the verification
procedure](docs/verification/a2a-foundation.md). It accepts exactly the pinned
Reference Agent and its two deterministic skills; remote admission, MCP,
models, tools, direct storage access, and production deployment are excluded.

With rootless Podman and sibling platform, room-gateway, memory-engine, and UI
checkouts available, run the integration gate from the platform directory:

```sh
podman-compose up --build -d
bash scripts/smoke.sh
```

For the gateway's own tests, run `cargo fmt --check` and `cargo test` here.

Read the [local specification index](.ai/specs/README.md), the
[deferred capability specification](.ai/specs/what/deferred-agent-gateway.md),
[approved local A2A foundation](.ai/specs/what/a2a-agent-gateway-foundation.md),
and the [ThoughtKhoral repository map](https://github.com/thoughtkhoral/thought-khoral/blob/main/docs/repository-map.md).

## Contributing

Use an issue to propose scope, contracts, security boundaries, or evidence for
an implementation plan. Changes remain governed by the approved What, How,
and implementation plan. See the
[organization contribution guide](https://github.com/thoughtkhoral/.github/blob/main/CONTRIBUTING.md).
