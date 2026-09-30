# L1 — public world contracts

Implement the exact shared vocabulary in [the contract brief](../maps/contracts.md),
owning only `crates/hex_world_contracts`. All numbered decisions in the manifest bind
this lane. No gameplay, generator, renderer, or filesystem authority belongs here.
Escalate contradictions with the brief; do not silently create alternate types.

Run focused pure tests and Clippy using an isolated lightweight target; never build
the full workspace. Commit to your source branch and report API changes and test counts.
