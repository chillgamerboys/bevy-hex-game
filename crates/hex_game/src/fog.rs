//! Player-facing tactical shroud derived from authoritative faction observation.
//!
//! The terrain itself remains the current, pickable map. This adapter adds a dark
//! presentation cap to every surface the player faction does not currently observe
//! and contributes only the composable [`PresentationOcclusionReason::Fog`] reason to
//! hidden hostile roots. Neither path feeds renderer state back into gameplay.

use std::collections::{BTreeMap, BTreeSet};

use bevy::light::NotShadowCaster;
use bevy::picking::Pickable;
use bevy::prelude::*;
use hex_assets::GameAssets;
use hex_core::{
    CutawayOccluder, Headroom, HexSpan, HexTile, KnowledgeState, PerceptionSystems,
    PresentationOcclusion, PresentationOcclusionReason, Screen, TilePos, UnitId,
};
use hex_perception::FactionMapKnowledge;
use hex_units::{Enemy, Faction};

const FOG_CAP_THICKNESS: f32 = 0.02;
const FOG_CAP_INSET: f32 = 0.84;
const FOG_CAP_LIFT: f32 = 0.08;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct FogOverlay(TilePos);

#[derive(Resource, Debug, Default)]
struct FogPresentationState {
    material: Option<Handle<StandardMaterial>>,
    initialized: bool,
    #[cfg(test)]
    reconciliations: u64,
}

type TileQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static TilePos,
        &'static HexSpan,
        &'static Headroom,
        Option<&'static CutawayOccluder>,
    ),
    With<HexTile>,
>;

type OverlayQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static FogOverlay,
        &'static mut Transform,
        Option<&'static CutawayOccluder>,
    ),
>;

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<FogPresentationState>()
        .add_systems(
            OnEnter(Screen::Gameplay),
            reconcile_fog.in_set(PerceptionSystems::ApplyPresentation),
        )
        .add_systems(
            Update,
            (clear_removed_hostile_fog, reconcile_fog)
                .in_set(PerceptionSystems::ApplyPresentation)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(OnExit(Screen::Gameplay), clear_fog_presentation);
}

#[expect(
    clippy::too_many_arguments,
    reason = "one reconciliation owns the complete terrain-and-hostile fog projection"
)]
fn reconcile_fog(
    mut commands: Commands,
    knowledge: Option<Res<FactionMapKnowledge>>,
    game_assets: Option<Res<GameAssets>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    tiles: TileQuery,
    changed_tiles: Query<
        (),
        (
            With<HexTile>,
            Or<(
                Added<HexTile>,
                Changed<TilePos>,
                Changed<HexSpan>,
                Changed<Headroom>,
                Changed<CutawayOccluder>,
            )>,
        ),
    >,
    tile_entities: Query<(), With<HexTile>>,
    mut removed_tiles: RemovedComponents<HexTile>,
    mut removed_positions: RemovedComponents<TilePos>,
    mut removed_spans: RemovedComponents<HexSpan>,
    mut removed_headroom: RemovedComponents<Headroom>,
    mut removed_cutaways: RemovedComponents<CutawayOccluder>,
    mut overlays: OverlayQuery,
    added_hostiles: Query<(), Added<Enemy>>,
    mut hostiles: Query<(&UnitId, &mut PresentationOcclusion), With<Enemy>>,
    mut state: ResMut<FogPresentationState>,
) {
    let tiles_removed = removed_tiles.read().count() != 0;
    let mut positions_removed = false;
    for entity in removed_positions.read() {
        positions_removed |= tile_entities.contains(entity);
    }
    let mut spans_removed = false;
    for entity in removed_spans.read() {
        spans_removed |= tile_entities.contains(entity);
    }
    let mut headroom_removed = false;
    for entity in removed_headroom.read() {
        headroom_removed |= tile_entities.contains(entity);
    }
    let mut cutaways_removed = false;
    for entity in removed_cutaways.read() {
        cutaways_removed |= tile_entities.contains(entity);
    }
    let tile_projection_removed =
        positions_removed || spans_removed || headroom_removed || cutaways_removed;
    let knowledge_changed = knowledge
        .as_ref()
        .is_none_or(|knowledge| knowledge.is_changed());
    let inputs_changed = !state.initialized
        || knowledge_changed
        || !changed_tiles.is_empty()
        || tiles_removed
        || tile_projection_removed
        || !added_hostiles.is_empty();
    if !inputs_changed {
        return;
    }
    #[cfg(test)]
    {
        state.reconciliations = state.reconciliations.saturating_add(1);
    }

    reconcile_hostiles(knowledge.as_deref(), &mut hostiles);

    let (Some(game_assets), Some(materials)) = (game_assets, materials.as_mut()) else {
        // Unit concealment remains fail-closed even if renderer assets are not ready.
        return;
    };
    let material = state
        .material
        .get_or_insert_with(|| materials.add(fog_material()))
        .clone();

    let surfaces = collect_current_surfaces(&tiles);
    let desired = desired_shaded_surfaces(knowledge.as_deref(), surfaces.keys().copied());
    let mut existing = BTreeMap::new();
    for (entity, overlay, mut transform, cutaway) in &mut overlays {
        if !desired.contains(&overlay.0) || !surfaces.contains_key(&overlay.0) {
            commands.entity(entity).despawn();
            continue;
        }
        if existing.insert(overlay.0, entity).is_some() {
            commands.entity(entity).despawn();
            continue;
        }

        let Some(surface) = surfaces.get(&overlay.0) else {
            continue;
        };
        *transform = fog_transform(overlay.0, surface.span);
        reconcile_cutaway(&mut commands, entity, cutaway.copied(), surface.cutaway);
    }

    for position in desired {
        if existing.contains_key(&position) {
            continue;
        }
        let Some(surface) = surfaces.get(&position) else {
            continue;
        };
        let mut overlay = commands.spawn((
            Mesh3d(game_assets.hex_tile.clone()),
            MeshMaterial3d(material.clone()),
            fog_transform(position, surface.span),
            Visibility::default(),
            Pickable::IGNORE,
            NotShadowCaster,
            PresentationOcclusion::default(),
            FogOverlay(position),
            Name::new("FogOverlay"),
        ));
        if let Some(cutaway) = surface.cutaway {
            overlay.insert(cutaway);
        }
    }
    state.initialized = true;
}

fn clear_removed_hostile_fog(
    mut removed_hostiles: RemovedComponents<Enemy>,
    mut former_hostiles: Query<&mut PresentationOcclusion, Without<Enemy>>,
) {
    for entity in removed_hostiles.read() {
        if let Ok(mut occlusion) = former_hostiles.get_mut(entity) {
            occlusion.remove(PresentationOcclusionReason::Fog);
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CurrentSurface {
    span: HexSpan,
    cutaway: Option<CutawayOccluder>,
}

fn collect_current_surfaces(tiles: &TileQuery) -> BTreeMap<TilePos, CurrentSurface> {
    let mut surfaces = BTreeMap::new();
    for (_, &position, &span, headroom, cutaway) in tiles {
        if headroom.0 <= 0 {
            continue;
        }
        let surface = CurrentSurface {
            span,
            cutaway: cutaway.copied(),
        };
        if surfaces.insert(position, surface).is_some() {
            error!(
                ?position,
                "duplicate rendered surface while reconciling fog"
            );
        }
    }
    surfaces
}

fn desired_shaded_surfaces(
    knowledge: Option<&FactionMapKnowledge>,
    surfaces: impl IntoIterator<Item = TilePos>,
) -> BTreeSet<TilePos> {
    surfaces
        .into_iter()
        .filter(|position| {
            knowledge.is_none_or(|knowledge| {
                knowledge.faction(Faction::Player).state(*position) != KnowledgeState::Observed
            })
        })
        .collect()
}

fn reconcile_hostiles(
    knowledge: Option<&FactionMapKnowledge>,
    hostiles: &mut Query<(&UnitId, &mut PresentationOcclusion), With<Enemy>>,
) {
    for (&unit, mut occlusion) in hostiles {
        let observed = knowledge
            .and_then(|knowledge| knowledge.faction(Faction::Player).unit(unit))
            .is_some();
        if observed {
            occlusion.remove(PresentationOcclusionReason::Fog);
        } else {
            occlusion.insert(PresentationOcclusionReason::Fog);
        }
    }
}

fn reconcile_cutaway(
    commands: &mut Commands,
    entity: Entity,
    current: Option<CutawayOccluder>,
    desired: Option<CutawayOccluder>,
) {
    if current == desired {
        return;
    }
    let mut entity = commands.entity(entity);
    if let Some(cutaway) = desired {
        entity.insert(cutaway);
    } else {
        entity.remove::<CutawayOccluder>();
    }
}

fn fog_transform(position: TilePos, span: HexSpan) -> Transform {
    Transform {
        translation: position
            .coord
            .to_world(span.top + FOG_CAP_LIFT + FOG_CAP_THICKNESS * 0.5),
        scale: Vec3::new(FOG_CAP_INSET, FOG_CAP_THICKNESS, FOG_CAP_INSET),
        ..default()
    }
}

fn fog_material() -> StandardMaterial {
    StandardMaterial {
        base_color: Color::srgba(0.03, 0.04, 0.10, 0.72),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        depth_bias: 8.0,
        ..default()
    }
}

fn clear_fog_presentation(
    mut commands: Commands,
    overlays: Query<Entity, With<FogOverlay>>,
    mut occlusions: Query<&mut PresentationOcclusion>,
    mut state: ResMut<FogPresentationState>,
) {
    for entity in &overlays {
        commands.entity(entity).despawn();
    }
    for mut occlusion in &mut occlusions {
        occlusion.remove(PresentationOcclusionReason::Fog);
    }
    state.initialized = false;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use hex_core::{HexCoord, SubstanceId};
    use hex_perception::{
        apply_observations, FactionObservation, FactionObservations, ObservedUnit, SurfaceSnapshot,
        SurfaceSnapshots,
    };
    use hex_test_support::TestAppBuilder;
    use hex_units::Player;

    fn pos(q: i32) -> TilePos {
        TilePos::new(HexCoord::from_axial(q, 0), 4)
    }

    fn surface(position: TilePos) -> SurfaceSnapshot {
        SurfaceSnapshot {
            pos: position,
            span: HexSpan::new(1.0, 2.0),
            substance: SubstanceId(1),
            headroom: Headroom(2),
            is_solid: true,
            blocked: false,
            domain: hex_core::LightDomain::Exterior,
        }
    }

    fn fog_app(knowledge: Option<FactionMapKnowledge>) -> App {
        let mut app = App::new();
        app.init_resource::<FogPresentationState>()
            .insert_resource(Assets::<StandardMaterial>::default())
            .insert_resource(GameAssets {
                hex_tile: Handle::default(),
                player_pieces: [Handle::default(), Handle::default()],
            })
            .add_systems(Update, (clear_removed_hostile_fog, reconcile_fog));
        if let Some(knowledge) = knowledge {
            app.insert_resource(knowledge);
        }
        app
    }

    fn fog_state_app(knowledge: Option<FactionMapKnowledge>) -> App {
        let mut builder = TestAppBuilder::new().with_fixed_step(Duration::ZERO);
        let app = builder.app_mut();
        app.init_resource::<FogPresentationState>()
            .insert_resource(Assets::<StandardMaterial>::default())
            .insert_resource(GameAssets {
                hex_tile: Handle::default(),
                player_pieces: [Handle::default(), Handle::default()],
            });
        plugin(app);
        if let Some(knowledge) = knowledge {
            app.insert_resource(knowledge);
        }
        builder.build()
    }

    fn spawn_surface(app: &mut App, position: TilePos, cutaway: Option<CutawayOccluder>) -> Entity {
        let mut entity = app.world_mut().spawn((
            HexTile,
            position,
            HexSpan::new(1.0, 2.0),
            Headroom(2),
            Pickable::default(),
        ));
        if let Some(cutaway) = cutaway {
            entity.insert(cutaway);
        }
        entity.id()
    }

    fn player_knowledge(
        positions: impl IntoIterator<Item = TilePos>,
        hostile: Option<ObservedUnit>,
    ) -> FactionMapKnowledge {
        let surfaces = SurfaceSnapshots::try_from_iter(positions.into_iter().map(surface))
            .expect("fixture surfaces are distinct");
        let mut observation = FactionObservation::new();
        for (position, _) in surfaces.iter() {
            observation.insert_surface(position);
        }
        if let Some(hostile) = hostile {
            observation
                .try_insert_unit(hostile)
                .expect("fixture hostile identity is unique");
        }
        let mut knowledge = FactionMapKnowledge::new();
        apply_observations(
            &mut knowledge,
            &surfaces,
            &FactionObservations::with_faction(Faction::Player, observation),
        );
        knowledge
    }

    fn overlay_count(app: &mut App) -> usize {
        let world = app.world_mut();
        let mut query = world.query::<&FogOverlay>();
        query.iter(world).count()
    }

    #[test]
    fn unknown_and_remembered_are_shaded_but_observed_is_clear() {
        let unknown = pos(0);
        let remembered = pos(1);
        let observed = pos(2);
        let surfaces = SurfaceSnapshots::try_from_iter([surface(remembered), surface(observed)])
            .expect("distinct surfaces");
        let mut knowledge = FactionMapKnowledge::new();
        let mut first = FactionObservation::new();
        first.insert_surface(remembered);
        first.insert_surface(observed);
        apply_observations(
            &mut knowledge,
            &surfaces,
            &FactionObservations::with_faction(Faction::Player, first),
        );
        let mut second = FactionObservation::new();
        second.insert_surface(observed);
        apply_observations(
            &mut knowledge,
            &surfaces,
            &FactionObservations::with_faction(Faction::Player, second),
        );

        assert_eq!(
            desired_shaded_surfaces(Some(&knowledge), [unknown, remembered, observed]),
            BTreeSet::from([unknown, remembered])
        );
        assert_eq!(
            desired_shaded_surfaces(None, [unknown, observed]),
            BTreeSet::from([unknown, observed]),
            "missing knowledge must shade every current surface"
        );
    }

    #[test]
    fn hostile_observation_is_current_only() {
        let position = pos(0);
        let current = SurfaceSnapshots::try_from_iter([surface(position)]).expect("surface");
        let hostile = ObservedUnit {
            id: UnitId(7),
            faction: Faction::Hostile,
            pos: position,
            provides_sight: true,
        };
        let mut observation = FactionObservation::new();
        observation.insert_surface(position);
        observation
            .try_insert_unit(hostile)
            .expect("unique hostile");
        let mut knowledge = FactionMapKnowledge::new();
        apply_observations(
            &mut knowledge,
            &current,
            &FactionObservations::with_faction(Faction::Player, observation),
        );
        assert!(knowledge
            .faction(Faction::Player)
            .unit(hostile.id)
            .is_some());

        apply_observations(&mut knowledge, &current, &FactionObservations::default());
        assert!(knowledge
            .faction(Faction::Player)
            .unit(hostile.id)
            .is_none());
    }

    #[test]
    fn fog_material_and_transform_are_presentation_only() {
        let material = fog_material();
        assert!(material.unlit);
        assert_eq!(material.alpha_mode, AlphaMode::Blend);
        let transform = fog_transform(pos(3), HexSpan::new(1.0, 2.0));
        assert!((transform.scale.x - FOG_CAP_INSET).abs() < f32::EPSILON);
        assert!((transform.scale.y - FOG_CAP_THICKNESS).abs() < f32::EPSILON);
        assert!(transform.translation.y > 2.0);
    }

    #[test]
    fn overlay_reconciliation_preserves_live_tile_picking_and_survives_grid_replacement() {
        let first = pos(0);
        let second = pos(1);
        let replacement = pos(2);
        let cutaway = CutawayOccluder(hex_core::InteriorRegionId(4));
        let mut app = fog_app(Some(FactionMapKnowledge::new()));
        let first_tile = spawn_surface(&mut app, first, Some(cutaway));
        let second_tile = spawn_surface(&mut app, second, None);

        app.update();

        let overlays = {
            let world = app.world_mut();
            let mut query = world.query::<(
                Entity,
                &FogOverlay,
                &Pickable,
                Has<NotShadowCaster>,
                Option<&CutawayOccluder>,
                &MeshMaterial3d<StandardMaterial>,
            )>();
            query
                .iter(world)
                .map(
                    |(entity, overlay, pickable, no_shadow, cutaway, material)| {
                        (
                            entity,
                            overlay.0,
                            *pickable,
                            no_shadow,
                            cutaway.copied(),
                            material.0.clone(),
                        )
                    },
                )
                .collect::<Vec<_>>()
        };
        assert_eq!(overlays.len(), 2);
        assert!(overlays.iter().all(|(_, _, pickable, no_shadow, _, _)| {
            *pickable == Pickable::IGNORE && *no_shadow
        }));
        assert!(overlays.iter().any(|(_, position, _, _, marker, _)| {
            *position == first && *marker == Some(cutaway)
        }));
        assert!(
            overlays
                .windows(2)
                .all(|pair| matches!(pair, [left, right] if left.5 == right.5)),
            "fog uses one material"
        );
        assert_eq!(
            app.world().resource::<Assets<StandardMaterial>>().len(),
            1,
            "reconciliation must allocate one shared material"
        );
        assert_eq!(
            app.world().get::<Pickable>(first_tile),
            Some(&Pickable::default()),
            "the live terrain remains the picking target"
        );

        app.world_mut()
            .entity_mut(first_tile)
            .remove::<CutawayOccluder>();
        app.world_mut().despawn(second_tile);
        spawn_surface(&mut app, replacement, None);
        app.update();

        let mut positions = {
            let world = app.world_mut();
            let mut query = world.query::<(&FogOverlay, Option<&CutawayOccluder>)>();
            query
                .iter(world)
                .map(|(overlay, cutaway)| (overlay.0, cutaway.copied()))
                .collect::<Vec<_>>()
        };
        positions.sort_by_key(|(position, _)| *position);
        assert_eq!(
            positions,
            vec![(first, None), (replacement, None)],
            "a rebuilt grid must not retain stale or duplicate caps"
        );

        let reconciliations = app
            .world()
            .resource::<FogPresentationState>()
            .reconciliations;
        app.update();
        assert_eq!(overlay_count(&mut app), 2);
        assert_eq!(
            app.world()
                .resource::<FogPresentationState>()
                .reconciliations,
            reconciliations,
            "all replacement removal cursors must be drained in the rebuild frame"
        );
    }

    #[test]
    fn observed_transitions_remove_and_restore_exact_surface_caps() {
        let position = pos(0);
        let mut app = fog_app(Some(player_knowledge([position], None)));
        spawn_surface(&mut app, position, None);

        app.update();
        assert_eq!(overlay_count(&mut app), 0);

        let current = SurfaceSnapshots::try_from_iter([surface(position)]).expect("surface");
        apply_observations(
            &mut app.world_mut().resource_mut::<FactionMapKnowledge>(),
            &current,
            &FactionObservations::default(),
        );
        app.update();
        assert_eq!(
            overlay_count(&mut app),
            1,
            "Remembered terrain receives the same cap as Unknown terrain"
        );

        let mut observed = FactionObservation::new();
        observed.insert_surface(position);
        apply_observations(
            &mut app.world_mut().resource_mut::<FactionMapKnowledge>(),
            &current,
            &FactionObservations::with_faction(Faction::Player, observed),
        );
        app.update();
        assert_eq!(overlay_count(&mut app), 0);
    }

    #[test]
    fn hostile_fog_composes_with_other_occlusion_and_teardown_removes_only_fog() {
        let position = pos(0);
        let hostile = ObservedUnit {
            id: UnitId(7),
            faction: Faction::Hostile,
            pos: position,
            provides_sight: true,
        };
        let mut app = fog_app(Some(player_knowledge([position], Some(hostile))));
        spawn_surface(&mut app, position, None);
        let hostile_entity = app
            .world_mut()
            .spawn((
                Enemy,
                hostile.id,
                PresentationOcclusion::from_reason(PresentationOcclusionReason::InteriorCutaway),
            ))
            .id();
        let allied_entity = app
            .world_mut()
            .spawn((Player, PresentationOcclusion::default()))
            .id();

        app.update();
        let observed = *app
            .world()
            .get::<PresentationOcclusion>(hostile_entity)
            .expect("hostile occlusion");
        assert!(!observed.contains(PresentationOcclusionReason::Fog));
        assert!(observed.contains(PresentationOcclusionReason::InteriorCutaway));

        let current = SurfaceSnapshots::try_from_iter([surface(position)]).expect("surface");
        apply_observations(
            &mut app.world_mut().resource_mut::<FactionMapKnowledge>(),
            &current,
            &FactionObservations::default(),
        );
        app.update();
        let hidden = *app
            .world()
            .get::<PresentationOcclusion>(hostile_entity)
            .expect("hostile occlusion");
        assert!(hidden.contains(PresentationOcclusionReason::Fog));
        assert!(hidden.contains(PresentationOcclusionReason::InteriorCutaway));
        assert!(!app
            .world()
            .get::<PresentationOcclusion>(allied_entity)
            .expect("allied occlusion")
            .contains(PresentationOcclusionReason::Fog));

        let mut observation = FactionObservation::new();
        observation.insert_surface(position);
        observation
            .try_insert_unit(hostile)
            .expect("fixture hostile identity is unique");
        apply_observations(
            &mut app.world_mut().resource_mut::<FactionMapKnowledge>(),
            &current,
            &FactionObservations::with_faction(Faction::Player, observation),
        );
        app.update();
        let revealed = *app
            .world()
            .get::<PresentationOcclusion>(hostile_entity)
            .expect("hostile occlusion");
        assert!(!revealed.contains(PresentationOcclusionReason::Fog));
        assert!(revealed.contains(PresentationOcclusionReason::InteriorCutaway));

        apply_observations(
            &mut app.world_mut().resource_mut::<FactionMapKnowledge>(),
            &current,
            &FactionObservations::default(),
        );
        app.update();
        assert!(app
            .world()
            .get::<PresentationOcclusion>(hostile_entity)
            .expect("hostile occlusion")
            .contains(PresentationOcclusionReason::Fog));

        app.world_mut()
            .run_system_once(clear_fog_presentation)
            .expect("fog teardown should run");
        assert_eq!(overlay_count(&mut app), 0);
        let cleared = *app
            .world()
            .get::<PresentationOcclusion>(hostile_entity)
            .expect("hostile occlusion");
        assert!(!cleared.contains(PresentationOcclusionReason::Fog));
        assert!(cleared.contains(PresentationOcclusionReason::InteriorCutaway));
    }

    #[test]
    fn withdrawn_knowledge_immediately_shades_terrain_and_conceals_hostiles() {
        let position = pos(0);
        let hostile = ObservedUnit {
            id: UnitId(7),
            faction: Faction::Hostile,
            pos: position,
            provides_sight: true,
        };
        let mut app = fog_app(Some(player_knowledge([position], Some(hostile))));
        spawn_surface(&mut app, position, None);
        let hostile_entity = app
            .world_mut()
            .spawn((Enemy, hostile.id, PresentationOcclusion::default()))
            .id();

        app.update();
        assert_eq!(overlay_count(&mut app), 0);
        assert!(!app
            .world()
            .get::<PresentationOcclusion>(hostile_entity)
            .expect("hostile occlusion")
            .contains(PresentationOcclusionReason::Fog));

        app.world_mut().remove_resource::<FactionMapKnowledge>();
        app.update();

        assert_eq!(overlay_count(&mut app), 1);
        assert!(app
            .world()
            .get::<PresentationOcclusion>(hostile_entity)
            .expect("hostile occlusion")
            .contains(PresentationOcclusionReason::Fog));
    }

    #[test]
    fn gameplay_exit_and_reentry_rebuild_the_owned_fog_projection() {
        let position = pos(0);
        let mut app = fog_state_app(Some(FactionMapKnowledge::new()));
        spawn_surface(&mut app, position, None);
        let hostile = app
            .world_mut()
            .spawn((Enemy, UnitId(7), PresentationOcclusion::default()))
            .id();

        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        app.update();
        assert_eq!(overlay_count(&mut app), 1);
        assert!(app
            .world()
            .get::<PresentationOcclusion>(hostile)
            .expect("hostile occlusion")
            .contains(PresentationOcclusionReason::Fog));

        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Title);
        app.update();
        assert_eq!(overlay_count(&mut app), 0);
        assert!(!app
            .world()
            .get::<PresentationOcclusion>(hostile)
            .expect("hostile occlusion")
            .contains(PresentationOcclusionReason::Fog));

        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        app.update();
        assert_eq!(overlay_count(&mut app), 1);
        assert!(app
            .world()
            .get::<PresentationOcclusion>(hostile)
            .expect("hostile occlusion")
            .contains(PresentationOcclusionReason::Fog));
    }

    #[test]
    fn removing_the_hostile_marker_removes_only_fog_occlusion() {
        let mut app = fog_app(None);
        let hostile = app
            .world_mut()
            .spawn((
                Enemy,
                UnitId(7),
                PresentationOcclusion::from_reason(PresentationOcclusionReason::InteriorCutaway),
            ))
            .id();

        app.update();
        assert!(app
            .world()
            .get::<PresentationOcclusion>(hostile)
            .expect("hostile occlusion")
            .contains(PresentationOcclusionReason::Fog));

        app.world_mut().entity_mut(hostile).remove::<Enemy>();
        app.update();

        let occlusion = *app
            .world()
            .get::<PresentationOcclusion>(hostile)
            .expect("former hostile occlusion");
        assert!(!occlusion.contains(PresentationOcclusionReason::Fog));
        assert!(occlusion.contains(PresentationOcclusionReason::InteriorCutaway));
    }
}
