//! Runtime orchestration for paid area and terrain spell work.
//!
//! The pure [`crate::SpellResolutionState`] owns correlation and completion. This
//! adapter is deliberately the only place that advances its queued unit work, settles
//! actors after terrain publication, adopts the settled ECS projection, and releases
//! the renderer-free combat authority.

use std::collections::BTreeMap;

use bevy::prelude::*;
use hex_assets::SubstanceTable;
use hex_core::{
    Busy, PausableSystems, PendingDecision, Screen, TerrainImpactOutcome, TerrainReady,
    TerrainSystems, Turn, UnitId,
};
use hex_lattice::{LatticeSpec, LatticeState};
use hex_units::{
    plan_unsupported_actor_landing, AuthoredObjectOccupancy, Body, Downed, Footing, MovingTo,
    StandsOn, StopMovingAt, TerrainOccupancy, UnitOccupancy, UnitRegistry,
};

use crate::{CombatEvent, SpellResolutionFailure, SpellResolutionStatus};

use super::{ActorQuery, TileQuery};

type SettlementActors<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static UnitId,
        Option<&'static Body>,
        &'static StandsOn,
        Option<&'static Transform>,
        Option<&'static Turn>,
        Has<Busy>,
        Has<Downed>,
        Option<&'static LatticeSpec>,
        Option<&'static LatticeState>,
    ),
>;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        stage_applied_outcomes
            .in_set(TerrainSystems::RefreshProjections)
            .in_set(PausableSystems)
            .run_if(in_state(Screen::Gameplay)),
    )
    .add_systems(
        Update,
        settle_unsupported_actors
            .in_set(TerrainSystems::ReconcileActors)
            .in_set(PausableSystems)
            .run_if(in_state(Screen::Gameplay)),
    )
    .add_systems(
        Update,
        consume_terrain_outcomes
            .in_set(TerrainSystems::ConsumeOutcomes)
            .in_set(PausableSystems)
            .run_if(in_state(Screen::Gameplay)),
    );
}

/// Observes valid Applied answers before reconciliation without completing them.
///
/// Map outcomes are written in `ApplyWorld`. A separate reader lets settlement see
/// whether the world actually applied a batch while the authoritative correlation and
/// release remain in `ConsumeOutcomes`.
fn stage_applied_outcomes(
    mut outcomes: MessageReader<TerrainImpactOutcome>,
    mut resolution: ResMut<crate::SpellResolutionState>,
) {
    for outcome in outcomes.read() {
        resolution.stage_outcome_for_settlement(outcome);
    }
}

/// Applies synchronous work until one public defender answer is owed.
///
/// Returns whether at least one queued operation was consumed. Completion remains in
/// the transaction until the outcome consumer releases the already-held authority.
pub(super) fn pump_unit_work(
    resolution: &mut crate::SpellResolutionState,
    pending: &mut PendingDecision,
    effects: &mut crate::effects::PersistentEffects,
    round: u32,
    registry: &UnitRegistry,
    actors: &ActorQuery,
    lattices: &mut super::cast::LatticeQuery,
    events: &mut Vec<CombatEvent>,
) -> bool {
    let mut progressed = false;
    while !pending.is_open() {
        let Some(work) = resolution.pop_unit_work() else {
            break;
        };
        progressed = true;
        let target = match work {
            crate::spell_resolution::UnitResolution::Disable { target, .. }
            | crate::spell_resolution::UnitResolution::Burn { target, .. } => target,
        };
        let Some(entity) = registry.entity_of(target) else {
            resolution.freeze(SpellResolutionFailure::UnitUnavailable { unit: target });
            break;
        };
        let Ok((_, _, _, _, _, _, downed)) = actors.get(entity) else {
            resolution.freeze(SpellResolutionFailure::UnitUnavailable { unit: target });
            break;
        };
        if downed {
            continue;
        }

        match work {
            crate::spell_resolution::UnitResolution::Disable {
                source,
                target,
                count,
            } => {
                super::cast::open_disable_decision(
                    pending, events, lattices, target, entity, source, count,
                );
            }
            crate::spell_resolution::UnitResolution::Burn {
                source,
                target,
                turns,
            } => {
                if lattices.get(entity).is_err() {
                    continue;
                }
                crate::effects::apply_burn(effects, round, source, target, turns);
                events.push(CombatEvent::BurnApplied {
                    source,
                    target,
                    turns,
                });
            }
        }
    }
    progressed
}

/// Plans every unsupported landing before committing any of them.
#[expect(
    clippy::too_many_arguments,
    reason = "settlement validates world publication, ECS facts, and combat authority atomically"
)]
fn settle_unsupported_actors(
    mut commands: Commands,
    mut resolution: ResMut<crate::SpellResolutionState>,
    terrain_ready: Option<Res<TerrainReady>>,
    terrain_occupancy: Option<Res<TerrainOccupancy>>,
    authored_objects: Option<Res<AuthoredObjectOccupancy>>,
    substances: Option<Res<SubstanceTable>>,
    blockers: Option<Res<hex_core::TraversalBlockers>>,
    tiles: TileQuery,
    mut authority: Option<ResMut<crate::authority_host::CombatAuthority>>,
    order: Res<crate::TurnOrder>,
    pending: Res<PendingDecision>,
    revivals: Res<crate::turns::PendingRevivals>,
    registry: Res<UnitRegistry>,
    actors: SettlementActors,
) {
    if !matches!(resolution.status(), SpellResolutionStatus::Pending { .. }) {
        return;
    }
    if !resolution.needs_terrain_settlement_attempt() {
        return;
    }
    if terrain_ready.is_none() || terrain_occupancy.is_none() || authored_objects.is_none() {
        if resolution.terrain_settlement_required() {
            resolution.freeze(SpellResolutionFailure::SettlementUnavailable {
                reason: "complete terrain or authored-object publication is unavailable after an applied impact".to_owned(),
            });
        }
        return;
    }
    let Some(substances) = substances.as_deref() else {
        if resolution.terrain_settlement_required() {
            resolution.freeze(SpellResolutionFailure::SettlementUnavailable {
                reason: "SubstanceTable is unavailable after an applied impact".to_owned(),
            });
        }
        return;
    };
    let Some(authored_objects) = authored_objects.as_deref() else {
        if resolution.terrain_settlement_required() {
            resolution.freeze(SpellResolutionFailure::SettlementUnavailable {
                reason: "AuthoredObjectOccupancy is unavailable during settlement".to_owned(),
            });
        }
        return;
    };
    let Some(authority) = authority.as_deref_mut() else {
        resolution.freeze(SpellResolutionFailure::AuthorityUnavailable {
            reason: "combat authority is absent during settlement".to_owned(),
        });
        return;
    };
    if !authority.state.external_resolution_is_held() {
        resolution.freeze(SpellResolutionFailure::AuthorityUnavailable {
            reason: "combat authority lost the spell-resolution hold".to_owned(),
        });
        return;
    }

    let mut published = actors
        .iter()
        .map(|(entity, id, body, standing, transform, ..)| {
            (entity, *id, body.copied(), standing.0, transform.copied())
        })
        .collect::<Vec<_>>();
    published.sort_by_key(|(_, id, ..)| *id);
    if let Some((_, unit, ..)) = published.iter().find(|(_, _, body, ..)| body.is_none()) {
        resolution.freeze(SpellResolutionFailure::SettlementUnavailable {
            reason: format!("unit {unit:?} has no traversal body"),
        });
        return;
    }

    let mut occupancy = UnitOccupancy::from_positions(
        published
            .iter()
            .map(|(_, unit, _, standing, _)| (*unit, standing.pos)),
    );
    let mut planned = BTreeMap::new();
    for (entity, unit, body, standing, transform) in &published {
        let (entity, unit, body, standing) = (*entity, *unit, *body, *standing);
        let Some(body) = body else {
            resolution.freeze(SpellResolutionFailure::SettlementUnavailable {
                reason: format!("unit {unit:?} has no traversal body"),
            });
            return;
        };
        let footing = Footing::from_tiles_with_object_occupancy(
            tiles.iter(),
            substances,
            body,
            blockers.as_deref(),
            authored_objects,
        );
        if footing.at(standing.pos).is_some() {
            continue;
        }
        let landing = match plan_unsupported_actor_landing(unit, standing.pos, &footing, &occupancy)
        {
            Ok(landing) => landing,
            Err(_) => {
                resolution.freeze(SpellResolutionFailure::NoLegalLanding {
                    unit,
                    origin: standing.pos,
                });
                return;
            }
        };
        let Some(mut transform) = *transform else {
            resolution.freeze(SpellResolutionFailure::SettlementUnavailable {
                reason: format!("unit {unit:?} has no Transform projection"),
            });
            return;
        };
        transform.translation = landing.world_position();
        occupancy.relocate(unit, landing.pos);
        planned.insert(unit, (entity, landing, transform));
    }

    // Validate the exact post-landing projection against a clone first. No ECS fact
    // changes unless the refreshed arena and complete authority roster accept it.
    let mut candidate = authority.state.clone();
    let mut projection = Vec::with_capacity(candidate.units.len());
    for domain_actor in candidate.units.values() {
        let Some(entity) = registry.entity_of(domain_actor.id) else {
            resolution.freeze(SpellResolutionFailure::UnitUnavailable {
                unit: domain_actor.id,
            });
            return;
        };
        let Ok((_, found, _, standing, _, turn, busy, downed, spec, lattice)) = actors.get(entity)
        else {
            resolution.freeze(SpellResolutionFailure::UnitUnavailable {
                unit: domain_actor.id,
            });
            return;
        };
        if *found != domain_actor.id {
            resolution.freeze(SpellResolutionFailure::UnitUnavailable {
                unit: domain_actor.id,
            });
            return;
        }
        let lattice = match (&domain_actor.lattice, spec, lattice) {
            (Some(expected), Some(spec), Some(lattice)) if expected.spec == *spec => {
                Some(lattice.clone())
            }
            (None, None, None) => None,
            _ => {
                resolution.freeze(SpellResolutionFailure::AuthorityUnavailable {
                    reason: format!(
                        "authority lattice shape for {:?} cannot be settled",
                        domain_actor.id
                    ),
                });
                return;
            }
        };
        let landing = planned
            .get(&domain_actor.id)
            .map(|(_, landing, _)| *landing);
        projection.push(hex_combat_core::CombatUnitProjection {
            id: domain_actor.id,
            position: landing.map_or(standing.0.pos, |landing| landing.pos),
            turn: turn.copied(),
            busy: landing.is_none() && busy,
            downed,
            lattice,
        });
    }
    if let Err(reason) = candidate.adopt_projection(
        order.order().to_vec(),
        order.current(),
        order.round,
        pending.clone(),
        revivals.snapshot(),
        projection,
    ) {
        resolution.freeze(SpellResolutionFailure::AuthorityUnavailable { reason });
        return;
    }

    for (&unit, &(entity, landing, ref transform)) in &planned {
        commands
            .entity(entity)
            .insert((StandsOn(landing), *transform))
            .remove::<(MovingTo, Busy, hex_anim::Transformation, StopMovingAt)>();
        trace!("settled unsupported actor {unit:?} at {:?}", landing.pos);
    }
    authority.state = candidate;
    resolution.mark_terrain_settlement_adopted();
}

/// Correlates all world answers, then releases a complete transaction exactly once.
#[expect(
    clippy::too_many_arguments,
    reason = "release atomically projects every mutable combat authority fact"
)]
fn consume_terrain_outcomes(
    mut commands: Commands,
    mut outcomes: MessageReader<TerrainImpactOutcome>,
    mut resolution: ResMut<crate::SpellResolutionState>,
    mut authority: Option<ResMut<crate::authority_host::CombatAuthority>>,
    mut order: ResMut<crate::TurnOrder>,
    mut pending: ResMut<PendingDecision>,
    mut revivals: ResMut<crate::turns::PendingRevivals>,
    registry: Res<UnitRegistry>,
    actors: ActorQuery,
    lattices: super::cast::LatticeQuery,
    mut events: MessageWriter<CombatEvent>,
    mut rounds: MessageWriter<hex_core::RoundElapsed>,
) {
    for outcome in outcomes.read() {
        resolution.accept_outcome(outcome.clone());
    }
    if !resolution.obligations_complete() || pending.is_open() {
        return;
    }
    let Some(authority) = authority.as_deref_mut() else {
        resolution.freeze(SpellResolutionFailure::AuthorityUnavailable {
            reason: "combat authority is absent at spell release".to_owned(),
        });
        return;
    };
    let checkpoint = authority.state.clone();
    if let Err(reason) = authority.state.finish_external_resolution() {
        resolution.freeze(SpellResolutionFailure::AuthorityUnavailable { reason });
        return;
    }
    if let Err(reason) = super::project_authority_state(
        &authority.state,
        &mut commands,
        &mut order,
        &mut pending,
        &mut revivals,
        &registry,
        &actors,
        &lattices,
    ) {
        authority.state = checkpoint;
        resolution.freeze(SpellResolutionFailure::AuthorityUnavailable { reason });
        return;
    }
    let mut published = Vec::new();
    authority.drain_events(&mut published);
    events.write_batch(published);
    let mut elapsed = Vec::new();
    authority.drain_rounds(&mut elapsed);
    rounds.write_batch(elapsed);
    resolution.finish();
}
