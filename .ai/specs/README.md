# ThoughtKhoral agent gateway specifications

Parent requirements in the ThoughtKhoral root `.ai/specs/` apply here. The
approved A2A foundation authorizes one locally controlled deterministic
reference integration. This repository is in active development, not a general
remote-agent admission or production service.

See the [root specification index](https://github.com/thoughtkhoral/thought-khoral/blob/main/.ai/specs/README.md).

## Local areas

- [What: deferred agent gateway (superseded for the local foundation)](what/deferred-agent-gateway.md)
- [What: A2A agent gateway foundation](what/a2a-agent-gateway-foundation.md)
- [What: public documentation](what/public-documentation.md)
- [How: historical specification-only boundary](how/specification-only.md)
- [How: A2A agent gateway foundation design](how/a2a-agent-gateway-foundation.md)
- [Implementation plan](https://github.com/thoughtkhoral/thought-khoral/blob/main/docs/superpowers/plans/2026-09-21-a2a-agent-gateway-foundation.md)
- [Decision 002: ThoughtKhoral project identity](decisions/002-thoughtkhoral-identity.md)

## Approved Codex room-participation extension

- [What: codex mediation](what/codex-mediation.md)
- [How: codex mediation](how/codex-mediation.md)
- [Coordinated implementation plan](https://github.com/thoughtkhoral/thought-khoral/blob/main/.ai/specs/how/codex-room-conversations-implementation-plan.md)

Approved by the maintainer on 2026-10-05 under [issue 1](https://github.com/thoughtkhoral/thought-khoral-agent-gateway/issues/1).
Implementation follows the coordinated plan and its artifact/dependency gates.
Existing runtime behavior is unchanged until the relevant tasks pass verification.

## Task 9 synthetic verification and correction checkpoint — 2026-10-07

The pre-amendment review snapshot and local mediation evidence are recorded in
the [owning checkpoint](how/codex-mediation.md). The approved defaults amendment
and its reviewed synthetic candidate resolve the UI finding F1 for that
candidate; the mediator itself remains unchanged and v1.0-compatible at reviewed
revision `6c3d96b4763871b9addc9bc7223e71ee7d38abd9`. The candidate run does not
merge this repository's runtime or authorize publication, packaged-stack
activation, or live provider use. Those gates remain pending.

## Approved defaults-discovery amendment — 2026-10-07

The amendment is governed by the [approved Codex design](https://github.com/thoughtkhoral/thought-khoral-codex-agent/blob/main/.ai/specs/how/default-settings-discovery-proposal.md)
and the [coordinated candidate checkpoint](https://github.com/thoughtkhoral/thought-khoral/blob/main/.ai/specs/how/codex-room-conversations-implementation-plan.md).
It adds behavior in the broker, contracts, UI and platform candidate branches.
This mediator remains on the published v1.0 profile and required no runtime
change for defaults discovery.
