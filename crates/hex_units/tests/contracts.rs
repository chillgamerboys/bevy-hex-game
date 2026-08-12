//! One linked integration target for every focused unit/ECS contract.

#[path = "contracts/movement.rs"]
mod movement;
#[path = "contracts/multiplayer_authority.rs"]
mod multiplayer_authority;
#[path = "contracts/serde_roundtrip.rs"]
mod serde_roundtrip;
#[path = "contracts/terrain_reconciliation.rs"]
mod terrain_reconciliation;
#[path = "contracts/volumes.rs"]
mod volumes;
