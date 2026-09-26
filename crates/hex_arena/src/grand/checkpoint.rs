//! Versioned gameplay owner record. The world owner commits terrain atomically beside it.
use super::*;
use hex_core::{arena::ArenaBurrowOutcome, TerrainImpactOutcome};

const FORMAT: u32 = 1;
const MAX_BYTES: usize = 8 * 1024 * 1024;

/// Exact world/content identity supplied by the durable run owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrandCheckpointIdentity {
    /// Fresh Grand world identity; legacy V4 saves cannot be imported.
    pub world_id: String,
    /// Package and gameplay content fingerprint, checked before adoption.
    pub content_revision: String,
}

#[derive(Serialize)]
struct Encoded<'a> {
    format: u32,
    identity: &'a GrandCheckpointIdentity,
    session: &'a ArenaSession,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decoded {
    format: u32,
    identity: GrandCheckpointIdentity,
    session: ArenaSession,
}

fn decode(bytes: &[u8], expected: &GrandCheckpointIdentity) -> Result<ArenaSession, String> {
    if bytes.len() > MAX_BYTES {
        return Err("Grand gameplay checkpoint exceeds the size limit".into());
    }
    let value: Decoded = ron::de::from_bytes(bytes)
        .map_err(|e| format!("Invalid Grand gameplay checkpoint: {e}"))?;
    if value.format != FORMAT || &value.identity != expected {
        return Err("Grand gameplay checkpoint belongs to different content or format".into());
    }
    validate(&value.session)?;
    Ok(value.session)
}

fn validate(session: &ArenaSession) -> Result<(), String> {
    session.validate_grand_records()?;
    session
        .serialize(super::finite::Finite)
        .map_err(|e| e.to_string())?;
    if session.tick > u64::MAX - 1_000_000
        || session.next_projectile > u64::MAX - 1_000_000
        || session.next_impact > u64::MAX - 1_000_000
        || !session
            .progression
            .as_ref()
            .is_some_and(crate::progression::ProgressState::valid_checkpoint)
    {
        return Err("Grand checkpoint progression or clock is invalid".into());
    }
    let grand = session
        .grand
        .as_ref()
        .ok_or("Checkpoint is not a Grand run")?;
    grand.tuning.validate()?;
    if !grand.start.is_finite()
        || !grand.respawn_position.is_finite()
        || !grand.teleport_cooldown.is_finite()
        || !(0.0..=6.0).contains(&grand.teleport_cooldown)
        || session.outcome.is_some()
        || session.exploration
        || session.progression.is_none()
        || session.actors.is_empty()
        || session.actors.len() > 128
        || session.projectiles.len() > 4096
        || session.pending_walls.len() > 4096
    {
        return Err("Grand gameplay checkpoint has invalid state bounds".into());
    }
    let mut ids = BTreeSet::new();
    for actor in session.actors.iter().chain(
        session
            .encounter
            .dormant
            .values()
            .flat_map(|p| p.actors.iter()),
    ) {
        if !ids.insert(actor.id)
            || !actor.feet.is_finite()
            || !actor.previous_feet.is_finite()
            || !actor.aim.is_finite()
            || !actor.hp.is_finite()
            || !actor.max_hp.is_finite()
            || actor.max_hp <= 0.0
            || actor.hp < 0.0
            || actor.hp > actor.max_hp
            || !actor.dimensions.is_finite()
            || actor.dimensions.min_element() <= 0.0
            || actor.dimensions.max_element() > 100.0
            || actor
                .charge
                .is_some_and(|c| !(0.0..=30.0).contains(&c.elapsed))
            || actor.cooldowns.iter().any(|v| !v.is_finite() || *v < 0.0)
        {
            return Err("Grand gameplay checkpoint contains an invalid actor".into());
        }
    }
    if ids.len() > 128
        || session
            .actors
            .iter()
            .filter(|a| a.id == 0 && a.species == Species::Human)
            .count()
            != 1
    {
        return Err("Grand checkpoint player or actor identities are invalid".into());
    }
    let mut projectile_ids = BTreeSet::new();
    for projectile in session.projectiles.iter().chain(
        session
            .encounter
            .dormant
            .values()
            .flat_map(|p| p.projectiles.iter()),
    ) {
        if !projectile_ids.insert(projectile.id)
            || projectile.id >= session.next_projectile
            || !projectile.position.is_finite()
            || !projectile.velocity.is_finite()
            || !projectile.age.is_finite()
            || projectile.age < 0.0
            || !ids.contains(&projectile.owner)
        {
            return Err("Grand checkpoint contains an invalid projectile".into());
        }
    }
    if projectile_ids.len() > 4096 {
        return Err("Grand checkpoint has too many projectiles".into());
    }
    Ok(())
}

impl ArenaSession {
    /// Consume correlated world acknowledgements without advancing gameplay or emitting edits.
    /// Replayed already-consumed acknowledgements are harmless at the pause/save boundary.
    pub fn settle_checkpoint_outcomes(
        &mut self,
        terrain: &[TerrainImpactOutcome],
        burrows: &[ArenaBurrowOutcome],
    ) -> Result<(), String> {
        for outcome in terrain {
            if let Some(expected) = self.pending_impacts.get(&outcome.batch) {
                if !outcome.is_consistent_with(expected) {
                    return Err("Cannot save an inconsistent terrain acknowledgement".into());
                }
                self.accept_outcome(outcome);
            }
        }
        for outcome in burrows {
            if self.generation == Some(outcome.generation)
                && self
                    .pending_burrows
                    .get(&outcome.actor)
                    .is_some_and(|p| p.sequence == outcome.sequence)
            {
                self.pending_burrows.remove(&outcome.actor);
            }
        }
        if !self.pending_impacts.is_empty() || !self.pending_burrows.is_empty() {
            return Err("Grand save awaits terrain acknowledgements".into());
        }
        Ok(())
    }

    /// Encode every persistent gameplay clock, actor, AI, attack and projectile after world settlement.
    pub fn encode_grand_checkpoint(
        &self,
        identity: &GrandCheckpointIdentity,
    ) -> Result<Vec<u8>, String> {
        if !self.pending_impacts.is_empty() || !self.pending_burrows.is_empty() {
            return Err("Settle terrain before saving Grand gameplay".into());
        }
        validate(self)?;
        let text = ron::ser::to_string(&Encoded {
            format: FORMAT,
            identity,
            session: self,
        })
        .map_err(|e| e.to_string())?;
        if text.len() > MAX_BYTES {
            return Err("Grand gameplay checkpoint exceeds the size limit".into());
        }
        Ok(text.into_bytes())
    }

    /// Inspect a validated checkpoint destination before streaming its saved collision neighborhood.
    pub fn grand_checkpoint_position(
        bytes: &[u8],
        expected: &GrandCheckpointIdentity,
    ) -> Result<Vec3, String> {
        let session = decode(bytes, expected)?;
        session
            .actors
            .iter()
            .find(|a| a.id == 0)
            .map(|a| {
                if a.hp > 0.0 {
                    a.feet
                } else {
                    session
                        .grand
                        .as_ref()
                        .map_or(a.feet, |g| g.respawn_position)
                }
            })
            .ok_or_else(|| "Grand checkpoint has no player".into())
    }

    /// Rebind collision after the world owner admits the saved neighborhood; no gameplay tick.
    pub fn rebind_grand_terrain(
        &mut self,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        generation: u64,
    ) -> Result<(), String> {
        if !self.is_grand_run() || world.selection.map != hex_core::arena::ArenaMap::GrandV4 {
            return Err("Grand checkpoint/world mismatch".into());
        }
        self.generation = Some(generation);
        self.collision.refresh(world, geometry);
        self.collision.sync_barriers(&self.encounter.barriers);
        if self.actors.iter().filter(|a| a.hp > 0.0).any(|a| {
            self.collision.needs_terrain(
                a.feet,
                Vec3::ZERO,
                a.dimensions.y,
                a.dimensions.x.max(a.dimensions.z) * 0.5,
            )
        }) || self
            .projectiles
            .iter()
            .any(|p| p.waiting_for_terrain(&self.collision))
            || self
                .pending_walls
                .iter()
                .any(|w| w.waiting_for_terrain(world, geometry))
        {
            return Err("Grand checkpoint destination terrain is not ready".into());
        }
        if let Some(actor) = self
            .actors
            .iter()
            .filter(|a| a.hp > 0.0 && a.species != Species::Worm)
            .find(|a| !crate::shapes::clear(&self.collision, a, a.feet, a.body_yaw))
        {
            return Err(format!(
                "Grand checkpoint contains embedded actor {}",
                actor.id
            ));
        }
        Ok(())
    }

    /// Correlated terrain operations still requiring world acknowledgements.
    #[must_use]
    pub fn grand_pending_world_operations(&self) -> usize {
        self.pending_impacts.len() + self.pending_burrows.len()
    }

    /// Check already-converted Worm volumes against authoritative world materials.
    /// This never requests terrain conversion or advances a movement/AI tick.
    pub fn validate_grand_burrow_poses(
        &mut self,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        materials: hex_core::arena::ArenaMaterials,
        policy: &hex_core::arena::ArenaBurrowMaterials,
    ) -> Result<(), String> {
        self.burrow_query.refresh(world);
        for actor in self
            .actors
            .iter()
            .filter(|a| a.hp > 0.0 && a.species == Species::Worm)
        {
            let parts = actor
                .body_prism_snapshot()
                .ok_or("Grand checkpoint Worm has no body components")?;
            let pose = crate::worm_geometry::PrismPose {
                feet: actor.feet,
                parts,
            };
            for part in parts.iter() {
                if self.collision.needs_terrain(
                    actor.feet + part.offset,
                    Vec3::ZERO,
                    part.height,
                    hex_core::config::HEX_SMALL_DIAMETER * 0.5,
                ) {
                    return Err("Grand checkpoint destination terrain is not ready".into());
                }
            }
            let context = crate::worm_geometry::BurrowContext {
                world,
                geometry,
                policy,
                dirt: materials.dirt,
                bodies: &self.actors,
                owner: actor.id,
            };
            if !matches!(
                self.burrow_query.admit(pose, pose, &context),
                crate::worm_geometry::Admission::Clear
            ) {
                return Err(format!(
                    "Grand checkpoint contains invalid buried Worm {}",
                    actor.id
                ));
            }
        }
        Ok(())
    }

    /// Stage a complete restored session; the caller adopts it only after all owners succeed.
    pub fn decode_grand_checkpoint(
        bytes: &[u8],
        expected: &GrandCheckpointIdentity,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        generation: u64,
    ) -> Result<Self, String> {
        if world.selection.map != hex_core::arena::ArenaMap::GrandV4 {
            return Err("Grand gameplay requires the Grand world".into());
        }
        let mut session = decode(bytes, expected)?;
        session.generation = Some(generation);
        session.collision.refresh(world, geometry);
        Ok(session)
    }
}
