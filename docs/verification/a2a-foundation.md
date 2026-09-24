# Local A2A foundation verification

Run the vertical-slice check from the sibling platform repository after starting
the rootless local stack:

```sh
podman-compose up --build -d
bash scripts/validate-kube.sh
bash scripts/smoke.sh
```

The smoke uses Alice's browser OIDC flow to create a fresh room and invoke both
allowed skills, `summarize-context` and `extract-action-items`. Each task must
persist the fixed three progress events followed by one source-cited terminal
result. The test also writes a Bob-only targeted message, captures the room
gateway's task-scoped packet through its local workload-authenticated harness,
and rejects the run if that hidden message appears in the packet.

The local reference agent listens only on `127.0.0.1:9090`. In Compose, the
two agents share network and PID namespaces with the trusted credential-free
egress sidecar. It installs default-deny policy before they start, refreshes
allowed room and identity service IPs, and stops both agent processes if its
PID 1 exits. Recreate the egress owner and both agents together after such a
failure; restarting one agent alone is not an isolation recovery procedure.
Kubernetes instead uses a one-shot init container and CNI NetworkPolicy.
The reference UID 10002 has loopback only; gateway UID 10001 additionally has
broker/Keycloak TCP 8080 and DNS resolver access. Both drop all capabilities.
Neither workload has a host-published port or a database
credential. The local `agent-gateway-client-dev-only` Keycloak credential is
gateway-only; the distinct `reference-agent-inbound-dev-only` bearer secret is
the only credential received by the reference agent. Deployment systems must
inject managed service credentials, a distinct inbound secret, workload
identity, and mTLS material without checking a real secret or certificate into
source control.

## Remediation verification, 2026-09-22

All-features gateway tests cover the actual official transport over HTTP,
raw SSE byte/frame/comment limits and truncated EOF, lease-aware timeout,
incremental progress before stream completion, stable IDs/timestamps across
renewed leases, and immediate terminal handling of deterministic rejection.
The official client source and Apache license are vendored with a documented
local parser patch; contract tests consume hash-verified local schemas.

The room gateway's separate `agent_packet_live_test` was explicitly run with
this reference-agent binary on loopback: actual broker packets for both skills
completed with citations. The public invocation event omits task input; the
reference agent uses the packet input authorized from task storage.

The broker owns a five-minute lease. Unsupported gateway lease/rate knobs
are rejected. Each dispatch admits three distinct progress events and one
terminal event; the worker's poll interval is the configurable control.

Podman kernel probes confirmed gateway access to the room service, rejection
of the reference UID at that same IP/port, and rejection of arbitrary IPv4
and IPv6 destinations for both UIDs. The complete platform smoke remains a
separate gate requiring the full source checkout and running Keycloak; kernel
probes and local broker/A2A tests do not substitute for that gate.
