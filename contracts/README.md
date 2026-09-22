# Pinned room contract schemas

These three unmodified schemas are vendored from contracts commit
`1a44b6cfc19a4f5e668afb1ddb5241b2a0f1f72d`, the same snapshot consumed by
the room gateway. `lock.json` retains the source commit, archive checksum,
and SHA-256 of every schema. This snapshot has no release tag; do not claim
it is a tagged release. The contract remains `n2n.room.v1`.

The dispatcher's contract tests verify the local hashes and resolve schema
references from local resources. They require neither a sibling checkout nor
network retrieval. Updating the pin requires explicit review of the contract
change and replacing all schemas and the lock together.
