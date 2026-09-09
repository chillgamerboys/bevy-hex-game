# Audited seams at 127d1ce

- hex_core/src/arena.rs: shared tick, geometry, occupancy. Extend selection/offset/bounds/anchors/column runs/static/liquid facts; preserve Duel defaults.
- hex_core/src/terrain_impact.rs: exact volume, batch, power and outcome. Add Elemental/Physical kind; mechanical existing calls preserve behavior.
- hex_map/src/arena.rs: setup, authoritative mutations and publication; render.rs renders columns. Private procedural generators remain here. Fort and Ring7 already accepted.
- hex_arena/src/lib.rs: fixed two-actor reset/intents/outcome. spells.rs uses fixed bodies and two hit types. controller.rs is ground-only; collision.rs fully rebuilds. Replace within one gameplay lane; preserve bot Duel policy.
- hex_game/src/arena.rs and arena/: input, start/pause, camera, HUD, captures. Adapt published state without owning AI or world mutation.

If reality disagrees, report the discrepancy to root before crossing authority.
