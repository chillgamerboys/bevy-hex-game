# Audited continuation seams at 3046779

- `crates/hex_core/src/arena.rs`: ArenaTerrainView selection/anchors/solid geometry. Add finite deployment regions; no generator internals cross this seam.
- `crates/hex_map/src/arena/worlds.rs`: world-owned authored generation/anchors/projection. Publish Fort west courtyard sides(-4,2,15)/(-2,-2,15); Duel(-6,0,8)/(6,0,8), each center+ring1. Keep original spawns.
- `crates/hex_arena/src/lib.rs`: session/reset/outcome/input still assumes ordinary human0. Preserve Duel decision order and frozen fixtures while adding accepted spectator setup.
- `crates/hex_arena/src/encounters.rs` and `encounters/`: party sightings/cues/human target and victory require team-generic observer path. Do not hand hidden actor poses to decisions.
- `crates/hex_arena/src/bot/`: accepted Shadow policy reused against disclosed ID/team/shape/pose; reset target-specific memory on switch. Forecasts must use perceived shapes.
- `crates/hex_arena/src/bot/duel_goldens.ron`: 21 frozen snapshots from three accepted Duel scenarios, never regenerated to bless a regression.
- `crates/hex_game/src/arena/`: start/pause/terminal and actor0 rendering/camera/input become optional-human consumers. Spectator has camera only and no ActorIntent injection.
- `tools/arena.py`: launcher/capture parsing uses exact preset slugs, source-clean capture packs and explicit seed; no visible launch without user request.

An anchor plus current safe_spawn radius6 can escape into rooftops, so spectator must
stay inside published surface sets. Fort adventure gate height1.6 cannot fit the planned
2.0-high Golem: spawn directly in courtyard and validate its exact seven-cell shape.
If reality disagrees with this map, report the discrepancy before crossing authority.
