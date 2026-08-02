//! Disclosure-safe adapter for presentation-only inspection and world markers.

use bevy::prelude::*;
use hex_combat::EncounterResolution;
use hex_core::{
    AppSystems, GameplayPhase, InspectionCameraSubject, Screen, TargetReticleRequest,
    WorldMarkerSuppression,
};
use hex_gameplay_model::HudState;
use hex_perception::FactionMapKnowledge;
use hex_units::{Faction, StandsOn, UnitRegistry};

use super::{GameplayUiContext, HudInspection};

pub(super) fn plugin(app: &mut App) {
    app.add_message::<hex_core::CenterInspectionCamera>()
        .add_systems(
            Update,
            (sync_inspection_subject, sync_world_markers)
                .chain()
                .in_set(AppSystems::Update)
                .in_set(hex_core::GameplaySystems::WorldFeedbackRequests)
                .run_if(in_state(Screen::Gameplay)),
        );
}

fn is_disclosed(
    unit: hex_core::UnitId,
    faction: Faction,
    knowledge: Option<&FactionMapKnowledge>,
) -> bool {
    faction == Faction::Player
        || knowledge
            .is_some_and(|knowledge| knowledge.faction(Faction::Player).unit(unit).is_some())
}

fn sync_inspection_subject(
    mut commands: Commands,
    mut inspection: ResMut<HudInspection>,
    mut hud: ResMut<HudState>,
    registry: Res<UnitRegistry>,
    knowledge: Option<Res<FactionMapKnowledge>>,
    units: Query<(&hex_core::UnitId, &Faction, &StandsOn)>,
    projected: Query<(Entity, &InspectionCameraSubject)>,
) {
    let wanted = inspection.subject.and_then(|unit| {
        let entity = registry.entity_of(unit)?;
        let (identity, faction, standing) = units.get(entity).ok()?;
        (*identity == unit && is_disclosed(unit, *faction, knowledge.as_deref()))
            .then_some((entity, InspectionCameraSubject::new(unit, standing.0.pos)))
    });

    if wanted.is_none() && inspection.subject.take().is_some() {
        let _ = hud.close_active_surface();
    }
    for (entity, subject) in &projected {
        if wanted.is_none_or(|(wanted_entity, wanted_subject)| {
            wanted_entity != entity || wanted_subject != *subject
        }) {
            commands.entity(entity).remove::<InspectionCameraSubject>();
        }
    }
    if let Some((entity, subject)) = wanted {
        if !matches!(projected.get(entity), Ok((_, current)) if *current == subject) {
            commands.entity(entity).insert(subject);
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "marker projection composes phase, outcome, actor disclosure, and target disclosure without creating gameplay facts"
)]
fn sync_world_markers(
    mut commands: Commands,
    phase: Res<GameplayPhase>,
    resolution: Option<Res<EncounterResolution>>,
    context: Res<GameplayUiContext>,
    registry: Res<UnitRegistry>,
    knowledge: Option<Res<FactionMapKnowledge>>,
    factions: Query<&Faction>,
    requests: Query<(Entity, &TargetReticleRequest)>,
    mut suppression: Option<ResMut<WorldMarkerSuppression>>,
) {
    let actor_hidden = context.acting.as_ref().is_some_and(|actor| {
        actor.faction == Faction::Hostile
            && !is_disclosed(actor.unit, actor.faction, knowledge.as_deref())
    });
    if let Some(suppression) = suppression.as_deref_mut() {
        suppression.set(
            *phase != GameplayPhase::Active
                || resolution
                    .as_deref()
                    .is_some_and(EncounterResolution::is_resolved)
                || actor_hidden,
        );
    }

    let wanted = context.target.as_ref().and_then(|(_, target)| {
        let entity = registry.entity_of(target.unit)?;
        let faction = factions.get(entity).ok()?;
        is_disclosed(target.unit, *faction, knowledge.as_deref()).then_some((entity, target.unit))
    });
    for (entity, request) in &requests {
        if wanted.is_none_or(|(wanted_entity, wanted_unit)| {
            entity != wanted_entity || request.unit != wanted_unit
        }) {
            commands.entity(entity).remove::<TargetReticleRequest>();
        }
    }
    if let Some((entity, unit)) = wanted {
        if !matches!(requests.get(entity), Ok((_, request)) if request.unit == unit) {
            commands
                .entity(entity)
                .insert(TargetReticleRequest::new(unit));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_identity_never_requires_hostile_knowledge() {
        assert!(is_disclosed(hex_core::UnitId(1), Faction::Player, None));
        assert!(!is_disclosed(hex_core::UnitId(9), Faction::Hostile, None));
    }
}
