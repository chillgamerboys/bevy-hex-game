# L3 — resident world authority

Own `crates/hex_world_runtime`. All numbered manifest decisions bind this lane.
Consume [public contracts](../maps/contracts.md); report contradictions. Implement
exact availability-aware queries; interest union and hysteresis; bounded load jobs and
revision-checked admission; transaction pins; atomic edits, revisions and deltas;
fresh independently addressable persistence and durable modifications. Gameplay
turn/pause state and Bevy rendering are outside this crate.

Prove negative-coordinate and stacked queries, stale/cancelled jobs, separated
interests, locality, stream-out/in edits, atomic failure, crash-safe save/load and
idempotent/mismatched deltas. Use pure tests and an isolated light build target, then
commit to the source branch with API and verification notes.
