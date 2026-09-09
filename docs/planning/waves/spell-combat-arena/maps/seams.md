# Banked seams

Base495a73d: TerrainImpact/Outcome and TerrainEdit live in hex_core; hex_map/src/terrain_damage.rs owns TerrainDamageState; hex_map/src/voxel.rs owns VoxelMap; ordinary grid.rs couples terrain publication/render rebuild. New isolated world producer reuses voxel/damage implementation and shared messages, with no production path changes.

Accepted movement source: /Users/alberto/Documents/Codex/2026-09-06/for/work/hex-polish-lab/crates/hex_game/src/fly/{controller,collision}.rs at lab9ffc6bf. They are inspection-only; extract math, not FlyPawn or Preparing-phase tactic suppression. Input must be captured mouse rather than existing right-drag viewpoint. Body horizontal velocity is overwritten each tick, so external impulse is independent.

If these facts disagree with source, report the discrepancy before crossing an ownership boundary.
