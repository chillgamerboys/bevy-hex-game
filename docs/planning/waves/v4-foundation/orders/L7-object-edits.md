# L7: Persistent object transactions

Authority: world. Builder: worker in `feat/v4-object-edits`. Coordinator: root V4 task.

Implement only [crates/hex_world_runtime/src/object_edits.rs, crates/hex_world_runtime/tests/runtime/object_edits.rs]. Consume the shared V4 package and exact query contracts.
Keep named world/transaction dependencies, bounded active work, source/revision checks,
and strict failure behavior. Game wiring belongs to the coordinator.

This follow-up is integrated into the wave; source commits and focused checks are
recorded in the manifest. Combined renderer/game validation remains the wave gate.
