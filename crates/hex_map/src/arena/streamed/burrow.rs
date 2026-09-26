//! Atomic Worm conversion over admitted compact terrain; no dense-map fallback.
use super::*;
use hex_core::TerrainVoxelHealth;
use hex_core::arena::{
    ArenaBurrowChange, ArenaBurrowMaterials, ArenaBurrowOutcome, ArenaBurrowRejection,
    ArenaBurrowRequest, ArenaBurrowResult, ArenaMaterials,
};
type Admission = Result<Vec<ArenaBurrowChange>, (Option<TilePos>, ArenaBurrowRejection)>;
fn admit(world: &World, request: &ArenaBurrowRequest) -> Admission {
    use ArenaBurrowRejection as Reject;
    if let Some(reason) = request.structural_rejection() {
        return Err((None, reason));
    }
    let state = world.resource::<StreamedArena>();
    let view = world.resource::<ArenaTerrainView>();
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let substances = world.resource::<hex_assets::SubstanceTable>();
    let dirt = world.resource::<ArenaMaterials>().dirt;
    let policy = world.resource::<ArenaBurrowMaterials>();
    let damage = world.resource::<DamagedVoxels>();
    let Some(dirt_maximum) = substances
        .toughness(dirt)
        .filter(|_| policy.eligible.contains(&dirt))
    else {
        return Err((None, Reject::InvalidDestination));
    };
    if !state
        .runtime
        .manifest()
        .materials
        .iter()
        .any(|m| m.id == "dirt" && m.solid)
    {
        return Err((None, Reject::InvalidDestination));
    }
    let mut changes = Vec::new();
    for &position in &request.volume {
        let p = VoxelPosition {
            column: world_hex(position.coord),
            level: position.level,
        };
        if p.column
            .checked_distance(WorldHex::new(0, 0))
            .map_err(|_| (Some(position), Reject::OutsideWorld))?
            > u64::from(geometry.radius)
            || !(geometry.min_level..=geometry.max_level).contains(&p.level)
        {
            return Err((Some(position), Reject::OutsideWorld));
        }
        if !ready_voxel(view, geometry, position) {
            return Err((Some(position), Reject::TerrainUnavailable));
        }
        if state.edits.object_at(p).is_some() {
            return Err((Some(position), Reject::StaticObject));
        }
        if let Some(name) = state.edits.terrain_at(p) {
            if state
                .runtime
                .manifest()
                .materials
                .iter()
                .any(|m| m.id == name && !m.solid)
            {
                return Err((Some(position), Reject::Liquid));
            }
        }
        let before = view.solid_at(position).unwrap_or(SubstanceId::AIR);
        if before.is_air() || before == dirt {
            continue;
        }
        if !policy.eligible.contains(&before) {
            return Err((Some(position), Reject::IneligibleMaterial));
        }
        let Some(maximum) = substances.toughness(before) else {
            return Err((Some(position), Reject::IneligibleMaterial));
        };
        let remaining = damage
            .get(position)
            .map_or(maximum, |h| h.remaining.min(maximum));
        let health_before = TerrainVoxelHealth::new(remaining, maximum)
            .ok_or((Some(position), Reject::InvalidDestination))?;
        let health_after = TerrainVoxelHealth::new(remaining.min(dirt_maximum), dirt_maximum)
            .ok_or((Some(position), Reject::InvalidDestination))?;
        changes.push(ArenaBurrowChange {
            position,
            before,
            health_before,
            health_after,
        });
    }
    Ok(changes)
}
pub(super) fn apply(world: &mut World) {
    let mut requests = std::mem::take(&mut world.resource_mut::<ArenaInbox>().burrows);
    requests.extend(world.resource_mut::<Messages<ArenaBurrowRequest>>().drain());
    let mut outcomes = Vec::with_capacity(requests.len());
    for request in requests {
        let rejected = {
            let mut state = world.resource_mut::<ArenaWorldState>();
            if request.generation != state.generation {
                Some(ArenaBurrowRejection::StaleGeneration)
            } else if state
                .burrow_sequences
                .get(&request.actor)
                .is_some_and(|seq| *seq >= request.sequence)
            {
                Some(ArenaBurrowRejection::ReusedSequence)
            } else {
                state
                    .burrow_sequences
                    .insert(request.actor, request.sequence);
                None
            }
        };
        let result = if let Some(reason) = rejected {
            ArenaBurrowResult::Rejected {
                position: None,
                reason,
            }
        } else {
            match admit(world, &request) {
                Err((position, reason)) => ArenaBurrowResult::Rejected { position, reason },
                Ok(changed) => {
                    let edits = changed
                        .iter()
                        .map(|c| VoxelEdit {
                            position: VoxelPosition {
                                column: world_hex(c.position.coord),
                                level: c.position.level,
                            },
                            material: Some("dirt".into()),
                        })
                        .collect();
                    let result = commit(&mut world.resource_mut::<StreamedArena>(), edits);
                    if let Err(error) = result {
                        world.resource_mut::<StreamedArena>().failure = Some(error);
                        ArenaBurrowResult::Rejected {
                            position: None,
                            reason: ArenaBurrowRejection::TerrainUnavailable,
                        }
                    } else {
                        world.resource_scope(
                            |world, mut damage: Mut<crate::terrain_damage::TerrainDamageState>| {
                                damage.apply_burrow_health(
                                    &changed,
                                    &mut world.resource_mut::<DamagedVoxels>(),
                                );
                            },
                        );
                        let geometry = *world.resource::<ArenaVoxelGeometry>();
                        world.resource_scope(|world, mut state: Mut<StreamedArena>| {
                            publish(
                                &mut state,
                                &mut world.resource_mut::<ArenaTerrainView>(),
                                &geometry,
                            );
                        });
                        ArenaBurrowResult::Accepted { changed }
                    }
                }
            }
        };
        outcomes.push(ArenaBurrowOutcome {
            generation: request.generation,
            actor: request.actor,
            sequence: request.sequence,
            result,
        });
    }
    world
        .resource_mut::<Messages<ArenaBurrowOutcome>>()
        .write_batch(outcomes);
}
