# C1 world-authority source map

Banked at `origin/dev@a0f95e62d02c663902b864cc08a89e831d9ba437`. If refreshed source
disagrees with this map, escalate rather than guessing.

## Current symbols and dispositions

| Anchor | Current role | Disposition |
|---|---|---|
| `crates/hex_map/src/world_snapshot.rs:43` `CurrentWorldSnapshotV1` | Canonical cache of current map-owned truth | Keep; Campaign export clones this exact snapshot. |
| `crates/hex_map/src/world_snapshot.rs:74` `WorldReplicationRequestV1` | Runtime reconnect baseline/delta import | Keep transport behavior unchanged; Campaign bootstrap must not masquerade as a network authority sequence. |
| `crates/hex_map/src/world_snapshot.rs:307` `export_world_snapshot_v1` | Public generator-neutral export | Keep as the one export authority; add only Campaign-facing validation helpers if necessary. |
| `crates/hex_map/src/world_snapshot.rs:645` `prepare_world_snapshot_v1` | Transactional resolution into private map resources | Reuse inside map authority for Campaign bootstrap; do not expose private prepared state. |
| `crates/hex_map/src/world_snapshot.rs:730` `validate_world_snapshot_v1_against_content` | Non-mutating shipped-content validation | Keep and use during preflight. |
| `crates/hex_map/src/world_snapshot.rs:1117` `fingerprint_world_snapshot_v1` | Complete public semantic fingerprint | Keep as identity oracle; never use `GenerationReport::map_fingerprint`. |
| `crates/hex_map/src/grid.rs:227` `generate_world` | On-enter world bootstrap | Extend with a typed pending Campaign snapshot branch that validates/commits before ordinary grid publication. Seed regeneration remains the no-checkpoint branch. |
| `crates/hex_map/src/grid.rs:693` `publish_current_world_snapshot` | Refreshes current snapshot after authoritative generation/mutation | Keep; restored Campaign worlds must enter this same cache/publication contract. |
| `crates/hex_map/src/grid.rs:734` `apply_world_replication_requests` | Ordered live reconnect/delta adapter | Keep unchanged except shared private helper reuse; Campaign bootstrap has no transport sequence. |
| `crates/hex_map/src/grid.rs:902` `commit_prepared_world_snapshot` | Live transactional commit and deferred grid build | Extract only a private common commit helper if needed. Campaign and reconnect outcomes remain distinct typed paths. |
| `crates/hex_map/tests/contracts/world_snapshot.rs:97` onward | Authored/procedural/mutation/import contracts | Extend with Campaign bootstrap process-teardown cases and refusal atomicity. |

## Required end state

`hex_map` accepts one explicit pending Campaign world snapshot before `Screen::Gameplay`,
validates stable materials and every public projection, replaces private world resources,
publishes ordinary terrain, and emits a typed success/refusal observable by the shared
session adapter. Failure activates no partial world. No map-private type crosses into
`hex_game`, `hex_multiplayer`, or the save document.

The C1 fence compares the exported `WorldSnapshotV1`, its complete public fingerprint,
partial health, anchors/interiors/regions/blockers/view hint/lights/liquids/objects, and
actor-footing inputs after teardown/import. It covers authored plus all shipped procedural
generations and a mutated/damaged world.
