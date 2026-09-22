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
agent gateway shares its network namespace; in Kubernetes, it shares the
agent-gateway pod. Neither workload has a host-published port or a database
credential. The local `agent-gateway-dev-only` value is disposable fixture
data; deployment systems must inject managed service credentials, the inbound
secret, workload identity, and mTLS material without checking a real secret or
certificate into source control.
