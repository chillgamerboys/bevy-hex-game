# C2 gameplay-authority source map

Banked at `origin/dev@a0f95e62d02c663902b864cc08a89e831d9ba437`. If refreshed source
disagrees with this map, escalate rather than guessing.

## Current symbols and dispositions

| Anchor | Current role | Disposition |
|---|---|---|
| `crates/hex_game/src/save.rs:352` `CampaignSave` | V1 combined persistence record | L3 retains it as a strict read/migration shape. L2 must not turn it into the new owner adapter. |
| `crates/hex_game/src/save.rs:371` `CampaignUnitSave` | V1 unit record | L2 may extract validation/application behavior, but L3 retains this exact legacy schema for decoding. |
| `crates/hex_game/src/save.rs:864` validation helpers | Build/scenario/roster/formation compatibility | Separate reusable gameplay checks from document-version checks without weakening either. |
| `crates/hex_game/src/save.rs:1286` `SaveWorld` and `save_exploration` unit query | Current unit/lattice/downed export | Move authoritative unit projection into `campaign_authority.rs`; no map query is allowed there. |
| `crates/hex_game/src/save.rs:1418` `restore_pending_campaign` | Current V1 unit restore after generated terrain | Extract the all-before-mutation roster, lattice, and footing preflight into the gameplay adapter. L3 retains orchestration and legacy translation. |
| `crates/hex_combat/src/effects.rs:86` `PersistentEffects` | Ordered authority-private effect ledger | Add exact authority snapshot/replace APIs that preserve effect ids, next-id monotonicity, and an empty due queue. Replica replacement remains distinct. |
| `crates/hex_core/src/formation.rs:132` `PartyFormation` | Serializable session formation | Reuse directly; validate preset and player membership before replacement. |
| `crates/hex_multiplayer/src/replica.rs:91` `UnitReplica` | Disclosure-safe client projection | Do not reuse as persistence: hostile/private lattice and effect disclosure differ. `HostCampaignCheckpointV2` is authority-only and never registered as a wire message. |

## Required end state

The new gameplay-owned adapter exports the complete authoritative Campaign actor state
without importing `hex_map` or another world-private type. It validates every unit id,
faction, archetype, lattice shape/content, position candidate, downed flag, effect
reference, effect id/next counter, and formation membership before applying any mutation.
Footing is supplied through existing public tile/substance/blocker contracts in the
composition root; the adapter does not inspect `VoxelMap`.

Saving remains exploration-only, but the checkpoint format is complete enough to retain
the persistent-effect ledger. Selection is deliberately dropped. Restoring chooses the
ordinary local default selection after activation and never treats a prior UI choice as
authority.
