# Banked baseline exploration

Baseline 873a2c37. ArenaMap declaration: crates/hex_core/src/arena.rs:35; capabilities: arena/exploration.rs:23. ArenaExpeditionSites: arena/expedition.rs:20. ActorId currently u8: crates/hex_arena/src/creatures.rs:7. V4 streamed map: crates/hex_map/src/arena/streamed/. Pure compiler uses hex_schematic and immutable packages. Arena finite edits intentionally differ from constrained WorldRuntime edits; export/replay them without changing destruction semantics. Root owns shared adapters and application orchestration.

Approved design: September14 hex-grand-v3-remake-plan.md and hex-grand-v3-layout-notes.md, with September25 decisions quoted in manifest. Relative sketch is not a literal cell oracle. Preserve relative composition, measure original mainland separately from full disk, use full-cardinality proxy first. Latest user decisions authorize terrain iteration; no exact-cell tracing approval is outstanding.

Build cache retained at /Users/alberto/Documents/Codex/2026-09-04/there-were-a-few-issues-i/work/cargo-target-explore (~40GiB). Free disk ~28GiB. No active build at start. Reuse existing profile, do not clean durable evidence or launch parallel heavy Bevy builds.
