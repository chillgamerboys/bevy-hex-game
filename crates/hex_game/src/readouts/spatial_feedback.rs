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
            sync_inspection_subject
                .in_set(AppSystems::Update)
                .after(hex_core::GameplaySystems::Casting)
                .before(hex_core::GameplaySystems::UiContext)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            sync_world_markers
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

    if wanted.is_none() {
        if let Some(stale) = inspection.subject.take() {
            let _ = hud.close_character(stale);
        }
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
    use crate::readouts::{context::TargetProvenance, UiUnitIdentity};
    use hex_perception::{
        apply_observations, FactionObservation, FactionObservations, ObservedUnit, SurfaceSnapshot,
        SurfaceSnapshots,
    };

    #[derive(Resource, Default)]
    struct InspectionAtUiContext {
        subject: Option<hex_core::UnitId>,
        main_view: hex_gameplay_model::MainViewDestination,
    }

    fn observe_inspection_at_ui_context(
        inspection: Res<HudInspection>,
        hud: Res<HudState>,
        mut observed: ResMut<InspectionAtUiContext>,
    ) {
        observed.subject = inspection.subject;
        observed.main_view = hud.stored_main_view();
    }

    fn observed_hostile_knowledge(
        unit: hex_core::UnitId,
        position: hex_core::TilePos,
    ) -> FactionMapKnowledge {
        let surfaces = SurfaceSnapshots::try_from_iter([SurfaceSnapshot {
            pos: position,
            span: hex_core::HexSpan::from_ground(1.0),
            substance: hex_core::SubstanceId(1),
            headroom: hex_core::Headroom(2),
            is_solid: true,
            blocked: false,
            domain: hex_core::LightDomain::Exterior,
        }])
        .expect("the marker fixture has one unique surface");
        let mut observation = FactionObservation::new();
        observation.insert_surface(position);
        observation
            .try_insert_unit(ObservedUnit {
                id: unit,
                faction: Faction::Hostile,
                pos: position,
                provides_sight: true,
            })
            .expect("the marker fixture has one unique hostile");
        let mut knowledge = FactionMapKnowledge::new();
        apply_observations(
            &mut knowledge,
            &surfaces,
            &FactionObservations::with_faction(Faction::Player, observation),
        );
        knowledge
    }

    #[test]
    fn player_identity_never_requires_hostile_knowledge() {
        assert!(is_disclosed(hex_core::UnitId(1), Faction::Player, None));
        assert!(!is_disclosed(hex_core::UnitId(9), Faction::Hostile, None));
    }

    #[test]
    fn disclosure_loss_closes_inspection_before_ui_context_projection() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<HudState>()
            .init_resource::<HudInspection>()
            .init_resource::<UnitRegistry>()
            .init_resource::<InspectionAtUiContext>()
            .configure_sets(
                Update,
                (
                    hex_core::GameplaySystems::Casting,
                    hex_core::GameplaySystems::UiContext,
                )
                    .chain()
                    .in_set(AppSystems::Update),
            )
            .add_systems(
                Update,
                sync_inspection_subject
                    .in_set(AppSystems::Update)
                    .after(hex_core::GameplaySystems::Casting)
                    .before(hex_core::GameplaySystems::UiContext),
            )
            .add_systems(
                Update,
                observe_inspection_at_ui_context
                    .in_set(AppSystems::Update)
                    .in_set(hex_core::GameplaySystems::UiContext),
            );
        let unit = hex_core::UnitId(9);
        let entity = app
            .world_mut()
            .spawn((
                unit,
                Faction::Hostile,
                StandsOn(hex_units::Standing {
                    pos: hex_core::TilePos::ORIGIN,
                    span: hex_core::HexSpan::from_ground(1.0),
                }),
            ))
            .id();
        app.world_mut()
            .resource_mut::<UnitRegistry>()
            .register(unit, entity);
        app.world_mut().resource_mut::<HudInspection>().subject = Some(unit);
        app.world_mut().resource_mut::<HudState>().open_character(
            unit,
            hex_gameplay_model::HudContext::standard(
                hex_gameplay_model::HudContextEligibility::all(),
            ),
        );

        app.update();

        let observed = app.world().resource::<InspectionAtUiContext>();
        assert_eq!(observed.subject, None);
        assert_eq!(
            observed.main_view,
            hex_gameplay_model::MainViewDestination::Closed
        );
        assert_eq!(
            app.world().resource::<HudState>().stored_main_view(),
            hex_gameplay_model::MainViewDestination::Closed
        );
        assert!(app.world().get::<InspectionCameraSubject>(entity).is_none());
    }

    #[test]
    fn world_markers_conceal_and_reveal_with_current_hostile_observation() {
        let mut app = App::new();
        app.insert_resource(GameplayPhase::Active)
            .insert_resource(WorldMarkerSuppression::default())
            .init_resource::<UnitRegistry>()
            .init_resource::<GameplayUiContext>()
            .add_systems(Update, sync_world_markers);

        let unit = hex_core::UnitId(9);
        let position = hex_core::TilePos::ORIGIN;
        let entity = app.world_mut().spawn((unit, Faction::Hostile)).id();
        app.world_mut()
            .resource_mut::<UnitRegistry>()
            .register(unit, entity);
        let identity = UiUnitIdentity {
            unit,
            name: "wolf #9".to_owned(),
            faction: Faction::Hostile,
            party_slot: None,
            disclosed: true,
        };
        {
            let mut context = app.world_mut().resource_mut::<GameplayUiContext>();
            context.acting = Some(identity.clone());
            context.target = Some((TargetProvenance::Pinned, identity));
        }
        app.insert_resource(observed_hostile_knowledge(unit, position));

        app.update();
        assert_eq!(
            app.world().get::<TargetReticleRequest>(entity),
            Some(&TargetReticleRequest::new(unit))
        );
        assert!(!app
            .world()
            .resource::<WorldMarkerSuppression>()
            .is_suppressed());

        app.world_mut().remove_resource::<FactionMapKnowledge>();
        app.update();
        assert!(app.world().get::<TargetReticleRequest>(entity).is_none());
        assert!(app
            .world()
            .resource::<WorldMarkerSuppression>()
            .is_suppressed());

        app.insert_resource(observed_hostile_knowledge(unit, position));
        app.update();
        assert_eq!(
            app.world().get::<TargetReticleRequest>(entity),
            Some(&TargetReticleRequest::new(unit))
        );
        assert!(!app
            .world()
            .resource::<WorldMarkerSuppression>()
            .is_suppressed());
    }
}
