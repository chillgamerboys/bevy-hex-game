//! ECS projection and lifecycle for the headless perception rules.

use bevy::prelude::*;
use hex_assets::{PerceptionSettings, SubstanceTable};
use hex_core::{
    ExteriorIllumination, GameplayLight, GameplaySetupFailure, Headroom, HexSpan, HexTile,
    IlluminationLevel, InteriorRegions, LightDomain, LocalMapKnowledge, PerceptionSystems, Screen,
    SubstanceId, TerrainReady, TilePos, TraversalBlockers, UnitId,
};
use hex_units::{Body, Faction, MovementSystems, StandsOn};

use crate::{
    apply_observations, resolve_observations, FactionMapKnowledge, FactionObservations,
    LightSourceSnapshot, ObservedUnit, ResolvedIllumination, SurfaceSnapshot, SurfaceSnapshots,
};

type TileProjectionQuery<'w, 's> = Query<
    'w,
    's,
    (
        Option<&'static TilePos>,
        Option<&'static HexSpan>,
        Option<&'static SubstanceId>,
        Option<&'static Headroom>,
    ),
    With<HexTile>,
>;

type LightProjectionQuery<'w, 's> =
    Query<'w, 's, (Option<&'static TilePos>, &'static GameplayLight)>;

type UnitProjectionQuery<'w, 's> = Query<
    'w,
    's,
    (
        Option<&'static UnitId>,
        Option<&'static Faction>,
        Option<&'static StandsOn>,
    ),
    With<Body>,
>;

/// Local-light inputs captured with the surface frame they illuminated.
///
/// Observation of deleted remembered positions must use the same public light
/// snapshot as current surfaces. Keeping it between the ordered stages prevents an
/// unrelated system from moving a light halfway through one perception update.
#[derive(Resource, Debug, Default)]
struct PerceptionFrame {
    lights: Vec<LightSourceSnapshot>,
}

/// Adds authoritative illumination, faction sight, and session knowledge systems.
pub fn plugin(app: &mut App) {
    app.register_type::<GameplayLight>()
        .register_type::<LightDomain>()
        .register_type::<IlluminationLevel>()
        .add_systems(
            OnEnter(Screen::Gameplay),
            resolve_illumination
                .in_set(PerceptionSystems::ResolveIllumination)
                .run_if(resource_exists::<TerrainReady>)
                .run_if(not(resource_exists::<GameplaySetupFailure>)),
        )
        .add_systems(
            OnEnter(Screen::Gameplay),
            resolve_observation
                .in_set(PerceptionSystems::ResolveObservation)
                .run_if(resource_exists::<TerrainReady>)
                .run_if(not(resource_exists::<GameplaySetupFailure>)),
        )
        .add_systems(
            OnEnter(Screen::Gameplay),
            publish_knowledge
                .in_set(PerceptionSystems::PublishKnowledge)
                .run_if(resource_exists::<TerrainReady>)
                .run_if(not(resource_exists::<GameplaySetupFailure>)),
        )
        .add_systems(
            Update,
            resolve_illumination
                .in_set(PerceptionSystems::ResolveIllumination)
                .run_if(in_state(Screen::Gameplay))
                .run_if(resource_exists::<TerrainReady>)
                .run_if(not(resource_exists::<GameplaySetupFailure>)),
        )
        .add_systems(
            Update,
            resolve_observation
                .in_set(PerceptionSystems::ResolveObservation)
                .after(MovementSystems::Reconcile)
                .run_if(in_state(Screen::Gameplay))
                .run_if(resource_exists::<TerrainReady>)
                .run_if(not(resource_exists::<GameplaySetupFailure>)),
        )
        .add_systems(
            Update,
            publish_knowledge
                .in_set(PerceptionSystems::PublishKnowledge)
                .run_if(in_state(Screen::Gameplay))
                .run_if(resource_exists::<TerrainReady>)
                .run_if(not(resource_exists::<GameplaySetupFailure>)),
        )
        .add_systems(OnExit(Screen::Gameplay), clear_session);
}

fn resolve_illumination(
    mut commands: Commands,
    tiles: TileProjectionQuery,
    light_entities: LightProjectionQuery,
    table: Option<Res<SubstanceTable>>,
    exterior: Option<Res<ExteriorIllumination>>,
    interiors: Option<Res<InteriorRegions>>,
    blockers: Option<Res<TraversalBlockers>>,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    let Some(table) = table else {
        fail(
            &mut commands,
            &mut next_screen,
            "Perception cannot resolve without the substance table.",
        );
        return;
    };
    let Some(exterior) = exterior else {
        fail(
            &mut commands,
            &mut next_screen,
            "Perception cannot resolve without exterior illumination.",
        );
        return;
    };

    let surfaces =
        match snapshot_surfaces(&tiles, &table, interiors.as_deref(), blockers.as_deref()) {
            Ok(surfaces) if !surfaces.is_empty() => surfaces,
            Ok(_) => {
                fail(
                    &mut commands,
                    &mut next_screen,
                    "Perception received no exposed terrain surfaces.",
                );
                return;
            }
            Err(reason) => {
                fail(&mut commands, &mut next_screen, reason);
                return;
            }
        };
    let lights = match snapshot_lights(&light_entities, interiors.as_deref()) {
        Ok(lights) => lights,
        Err(reason) => {
            fail(&mut commands, &mut next_screen, reason);
            return;
        }
    };
    let illumination = match ResolvedIllumination::from_surfaces(&surfaces, *exterior, &lights) {
        Ok(illumination) => illumination,
        Err(error) => {
            fail(
                &mut commands,
                &mut next_screen,
                format!("Perception could not resolve illumination: {error}."),
            );
            return;
        }
    };

    commands.insert_resource(surfaces);
    commands.insert_resource(PerceptionFrame { lights });
    commands.insert_resource(illumination);
}

fn resolve_observation(
    mut commands: Commands,
    units: UnitProjectionQuery,
    surfaces: Option<Res<SurfaceSnapshots>>,
    illumination: Option<Res<ResolvedIllumination>>,
    frame: Option<Res<PerceptionFrame>>,
    exterior: Option<Res<ExteriorIllumination>>,
    settings: Option<Res<PerceptionSettings>>,
    prior_knowledge: Option<Res<FactionMapKnowledge>>,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    let (Some(surfaces), Some(illumination), Some(frame), Some(exterior), Some(settings)) =
        (surfaces, illumination, frame, exterior, settings)
    else {
        fail(
            &mut commands,
            &mut next_screen,
            "Perception observation started without a complete illumination frame.",
        );
        return;
    };
    if surfaces.is_empty() {
        fail(
            &mut commands,
            &mut next_screen,
            "Perception observation received an empty surface frame.",
        );
        return;
    }

    let units = match snapshot_units(&units) {
        Ok(units) => units,
        Err(reason) => {
            fail(&mut commands, &mut next_screen, reason);
            return;
        }
    };
    let empty_knowledge = FactionMapKnowledge::new();
    let prior = prior_knowledge.as_deref().unwrap_or(&empty_knowledge);
    let observations = match resolve_observations(
        units,
        &illumination,
        prior,
        *exterior,
        &frame.lights,
        settings.active_profile(),
    ) {
        Ok(observations) => observations,
        Err(error) => {
            fail(
                &mut commands,
                &mut next_screen,
                format!("Perception could not resolve faction sight: {error}."),
            );
            return;
        }
    };

    commands.insert_resource(observations);
}

fn publish_knowledge(
    mut commands: Commands,
    surfaces: Option<Res<SurfaceSnapshots>>,
    observations: Option<Res<FactionObservations>>,
    knowledge: Option<ResMut<FactionMapKnowledge>>,
    local: Option<ResMut<LocalMapKnowledge>>,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    let (Some(surfaces), Some(observations)) = (surfaces, observations) else {
        fail(
            &mut commands,
            &mut next_screen,
            "Perception knowledge publication started without current observations.",
        );
        return;
    };

    let projection = match knowledge {
        Some(mut knowledge) => {
            apply_observations(&mut knowledge, &surfaces, &observations);
            knowledge.player_local_map_knowledge()
        }
        None => {
            let mut knowledge = FactionMapKnowledge::new();
            apply_observations(&mut knowledge, &surfaces, &observations);
            let projection = knowledge.player_local_map_knowledge();
            commands.insert_resource(knowledge);
            projection
        }
    };
    match local {
        Some(mut local) => *local = projection,
        None => {
            commands.insert_resource(projection);
        }
    }
}

fn snapshot_surfaces(
    tiles: &TileProjectionQuery,
    table: &SubstanceTable,
    interiors: Option<&InteriorRegions>,
    blockers: Option<&TraversalBlockers>,
) -> Result<SurfaceSnapshots, String> {
    let mut snapshots = Vec::new();
    for (pos, span, substance, headroom) in tiles {
        let (Some(pos), Some(span), Some(substance), Some(headroom)) =
            (pos, span, substance, headroom)
        else {
            return Err(
                "A HexTile is missing TilePos, HexSpan, SubstanceId, or Headroom.".to_owned(),
            );
        };
        if headroom.0 <= 0 {
            continue;
        }
        snapshots.push(SurfaceSnapshot {
            pos: *pos,
            span: *span,
            substance: *substance,
            headroom: *headroom,
            is_solid: table.is_solid(*substance),
            blocked: blockers.is_some_and(|blockers| blockers.contains(*pos)),
            domain: domain_at(*pos, interiors),
        });
    }
    SurfaceSnapshots::try_from_iter(snapshots)
        .map_err(|error| format!("Perception received invalid terrain projections: {error}."))
}

fn snapshot_lights(
    light_entities: &LightProjectionQuery,
    interiors: Option<&InteriorRegions>,
) -> Result<Vec<LightSourceSnapshot>, String> {
    let mut lights = Vec::new();
    for (pos, light) in light_entities {
        let Some(pos) = pos else {
            return Err("A GameplayLight is missing its exact TilePos.".to_owned());
        };
        lights.push(LightSourceSnapshot {
            pos: *pos,
            domain: domain_at(*pos, interiors),
            light: *light,
        });
    }
    lights.sort_by_key(|source| {
        (
            source.pos,
            source.domain,
            source.light.level,
            source.light.radius,
        )
    });
    Ok(lights)
}

fn snapshot_units(units: &UnitProjectionQuery) -> Result<Vec<ObservedUnit>, String> {
    let mut snapshots = Vec::new();
    for (id, faction, standing) in units {
        let (Some(id), Some(faction), Some(standing)) = (id, faction, standing) else {
            return Err("A Body is missing UnitId, Faction, or StandsOn.".to_owned());
        };
        snapshots.push(ObservedUnit {
            id: *id,
            faction: *faction,
            pos: standing.0.pos,
        });
    }
    snapshots.sort_by_key(|unit| unit.id);
    Ok(snapshots)
}

fn domain_at(pos: TilePos, interiors: Option<&InteriorRegions>) -> LightDomain {
    interiors
        .and_then(|interiors| interiors.get(pos))
        .map_or(LightDomain::Exterior, LightDomain::Interior)
}

fn fail(commands: &mut Commands, next_screen: &mut NextState<Screen>, reason: impl Into<String>) {
    let reason = reason.into();
    error!("gameplay perception failed: {reason}");
    commands.insert_resource(GameplaySetupFailure::new(reason));
    next_screen.set(Screen::Title);
}

fn clear_session(mut commands: Commands) {
    commands.remove_resource::<PerceptionFrame>();
    commands.remove_resource::<SurfaceSnapshots>();
    commands.remove_resource::<ResolvedIllumination>();
    commands.remove_resource::<FactionObservations>();
    commands.remove_resource::<FactionMapKnowledge>();
    commands.remove_resource::<LocalMapKnowledge>();
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use bevy::platform::collections::HashMap;
    use bevy::state::app::StatesPlugin;
    use hex_assets::{SightPreset, Substance, SubstanceFile};
    use hex_core::{HexCoord, InteriorRegionId, KnowledgeState, SightProfile, TraversalProfile};
    use hex_units::Standing;

    use super::*;

    #[derive(Clone, Copy)]
    struct TestSubstances {
        stone: SubstanceId,
        water: SubstanceId,
    }

    fn test_table() -> (SubstanceTable, TestSubstances) {
        let mut substances = HashMap::default();
        for (name, solid) in [("air", false), ("stone", true), ("water", false)] {
            substances.insert(
                name.to_owned(),
                Substance {
                    color: (0.5, 0.5, 0.5),
                    solid,
                    diggable: true,
                },
            );
        }
        let table = SubstanceTable::from_file(&SubstanceFile { substances });
        let ids = TestSubstances {
            stone: table.id("stone").expect("stone fixture"),
            water: table.id("water").expect("water fixture"),
        };
        (table, ids)
    }

    fn runtime_app(exterior: IlluminationLevel) -> (App, TestSubstances) {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.configure_sets(
            Update,
            (
                PerceptionSystems::PublishAmbient,
                PerceptionSystems::ResolveIllumination,
                PerceptionSystems::ResolveObservation,
                PerceptionSystems::PublishKnowledge,
                PerceptionSystems::ApplyPresentation,
            )
                .chain(),
        );
        app.configure_sets(
            OnEnter(Screen::Gameplay),
            (
                PerceptionSystems::PublishAmbient,
                PerceptionSystems::ResolveIllumination,
                PerceptionSystems::ResolveObservation,
                PerceptionSystems::PublishKnowledge,
                PerceptionSystems::ApplyPresentation,
            )
                .chain(),
        );
        let (table, substances) = test_table();
        app.insert_resource(table);
        app.insert_resource(PerceptionSettings::default());
        app.insert_resource(ExteriorIllumination::new(exterior));
        app.insert_resource(InteriorRegions::new());
        app.insert_resource(TraversalBlockers::new());
        app.insert_resource(TerrainReady);
        app.add_plugins(plugin);
        (app, substances)
    }

    fn pos(q: i32, r: i32, level: i32) -> TilePos {
        TilePos::new(HexCoord::from_axial(q, r), level)
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "test terrain levels stay far inside f32's exact integer range"
    )]
    fn span(level: i32) -> HexSpan {
        let top = (level + 1) as f32;
        HexSpan::new(top - 1.0, top)
    }

    fn spawn_tile(
        app: &mut App,
        position: TilePos,
        substance: SubstanceId,
        headroom: i32,
    ) -> Entity {
        app.world_mut()
            .spawn((
                HexTile,
                position,
                span(position.level),
                substance,
                Headroom(headroom),
            ))
            .id()
    }

    fn spawn_unit(app: &mut App, id: u64, faction: Faction, position: TilePos) -> Entity {
        app.world_mut()
            .spawn((
                UnitId(id),
                faction,
                Body::new(TraversalProfile::WALKER),
                StandsOn(Standing {
                    pos: position,
                    span: span(position.level),
                }),
            ))
            .id()
    }

    fn enter(app: &mut App, screen: Screen) {
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(screen);
        app.update();
    }

    #[test]
    fn ecs_boundary_preserves_exact_exposed_surface_facts() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Dim);
        let exterior = pos(0, 0, 5);
        let water = pos(1, 0, 5);
        let buried = pos(2, 0, 5);
        let interior = pos(0, 1, 6);
        let region = InteriorRegionId(7);

        spawn_tile(&mut app, exterior, substances.stone, 2);
        spawn_tile(&mut app, water, substances.water, 3);
        spawn_tile(&mut app, buried, substances.stone, 0);
        spawn_tile(&mut app, interior, substances.stone, 4);
        app.world_mut()
            .resource_mut::<TraversalBlockers>()
            .insert(exterior);
        app.world_mut()
            .resource_mut::<InteriorRegions>()
            .insert_surface(interior, region);
        app.world_mut()
            .spawn((interior, GameplayLight::new(IlluminationLevel::Bright, 0)));

        enter(&mut app, Screen::Gameplay);

        let surfaces = app.world().resource::<SurfaceSnapshots>();
        assert_eq!(surfaces.len(), 3);
        assert!(surfaces.get(buried).is_none());
        let exterior_snapshot = surfaces.get(exterior).expect("exterior surface");
        assert!(exterior_snapshot.is_solid);
        assert!(exterior_snapshot.blocked);
        assert_eq!(exterior_snapshot.domain, LightDomain::Exterior);
        let water_snapshot = surfaces.get(water).expect("water surface");
        assert!(!water_snapshot.is_solid);
        assert_eq!(water_snapshot.headroom, Headroom(3));
        assert_eq!(
            surfaces.get(interior).expect("interior surface").domain,
            LightDomain::Interior(region)
        );

        let illumination = app.world().resource::<ResolvedIllumination>();
        assert_eq!(
            illumination.get(exterior).expect("exterior light").level,
            IlluminationLevel::Dim
        );
        assert_eq!(
            illumination.get(interior).expect("interior light").level,
            IlluminationLevel::Bright
        );
    }

    #[test]
    fn malformed_tile_fails_persistently_instead_of_disappearing() {
        let (mut app, _substances) = runtime_app(IlluminationLevel::Bright);
        app.world_mut().spawn((HexTile, pos(0, 0, 5)));

        enter(&mut app, Screen::Gameplay);

        let failure = app.world().resource::<GameplaySetupFailure>();
        assert!(failure.reason.contains("missing TilePos, HexSpan"));
        assert!(!app.world().contains_resource::<ResolvedIllumination>());
        app.update();
        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Title
        );
        assert!(app.world().contains_resource::<GameplaySetupFailure>());
    }

    #[test]
    fn profile_changes_hide_units_without_leaking_hidden_edits() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Dim);
        let player_pos = pos(0, 0, 5);
        let hostile_pos = pos(8, 0, 5);
        spawn_tile(&mut app, player_pos, substances.stone, 2);
        let hostile_tile = spawn_tile(&mut app, hostile_pos, substances.stone, 2);
        spawn_unit(&mut app, 0, Faction::Player, player_pos);
        spawn_unit(&mut app, 1, Faction::Hostile, hostile_pos);

        enter(&mut app, Screen::Gameplay);

        let knowledge = app.world().resource::<FactionMapKnowledge>();
        assert_eq!(
            knowledge.faction(Faction::Player).state(hostile_pos),
            KnowledgeState::Observed
        );
        assert!(knowledge.faction(Faction::Player).unit(UnitId(1)).is_some());

        app.world_mut().resource_mut::<PerceptionSettings>().active = SightPreset::Tight;
        app.update();
        app.world_mut()
            .entity_mut(hostile_tile)
            .insert(substances.water);
        app.world_mut()
            .resource_mut::<TraversalBlockers>()
            .insert(hostile_pos);
        app.update();

        let remembered = app
            .world()
            .resource::<FactionMapKnowledge>()
            .faction(Faction::Player)
            .surface(hostile_pos)
            .expect("remembered hostile surface");
        assert_eq!(remembered.state(), KnowledgeState::Remembered);
        assert_eq!(remembered.snapshot().substance, substances.stone);
        assert!(!remembered.snapshot().blocked);
        assert!(app
            .world()
            .resource::<FactionMapKnowledge>()
            .faction(Faction::Player)
            .unit(UnitId(1))
            .is_none());

        app.world_mut().resource_mut::<PerceptionSettings>().active = SightPreset::Expansive;
        app.update();
        let observed = app
            .world()
            .resource::<FactionMapKnowledge>()
            .faction(Faction::Player)
            .surface(hostile_pos)
            .expect("re-observed hostile surface");
        assert_eq!(observed.state(), KnowledgeState::Observed);
        assert_eq!(observed.snapshot().substance, substances.water);
        assert!(observed.snapshot().blocked);
        assert_eq!(
            app.world()
                .resource::<LocalMapKnowledge>()
                .state(hostile_pos),
            KnowledgeState::Observed
        );
    }

    #[derive(Resource)]
    struct MoveOnce {
        destination: TilePos,
        move_now: bool,
    }

    fn reconcile_test_move(
        mut request: ResMut<MoveOnce>,
        mut units: Query<&mut StandsOn, With<Body>>,
    ) {
        if !request.move_now {
            return;
        }
        for mut standing in &mut units {
            standing.0 = Standing {
                pos: request.destination,
                span: span(request.destination.level),
            };
        }
        request.move_now = false;
    }

    #[test]
    fn observation_sees_reconciled_movement_in_the_same_frame() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Dark);
        let start = pos(0, 0, 5);
        let destination = pos(1, 0, 5);
        let newly_near = pos(2, 0, 5);
        for position in [start, destination, newly_near] {
            spawn_tile(&mut app, position, substances.stone, 2);
        }
        spawn_unit(&mut app, 0, Faction::Player, start);
        app.insert_resource(MoveOnce {
            destination,
            move_now: false,
        });
        app.add_systems(
            Update,
            reconcile_test_move
                .in_set(MovementSystems::Reconcile)
                .run_if(in_state(Screen::Gameplay)),
        );

        enter(&mut app, Screen::Gameplay);
        assert_eq!(
            app.world()
                .resource::<FactionMapKnowledge>()
                .faction(Faction::Player)
                .state(newly_near),
            KnowledgeState::Unknown
        );

        app.world_mut().resource_mut::<MoveOnce>().move_now = true;
        app.update();

        assert_eq!(
            app.world()
                .resource::<FactionMapKnowledge>()
                .faction(Faction::Player)
                .state(newly_near),
            KnowledgeState::Observed
        );
    }

    #[test]
    fn moving_light_rederives_its_domain_in_the_same_frame() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        let observer = pos(0, 0, 6);
        let cave_target = pos(4, 0, 6);
        let exterior = pos(0, 1, 15);
        let region = InteriorRegionId(2);
        for position in [observer, cave_target, exterior] {
            spawn_tile(&mut app, position, substances.stone, 3);
        }
        for position in [observer, cave_target] {
            app.world_mut()
                .resource_mut::<InteriorRegions>()
                .insert_surface(position, region);
        }
        spawn_unit(&mut app, 0, Faction::Player, observer);
        let lamp = app
            .world_mut()
            .spawn((observer, GameplayLight::new(IlluminationLevel::Bright, 4)))
            .id();

        enter(&mut app, Screen::Gameplay);
        assert_eq!(
            app.world()
                .resource::<ResolvedIllumination>()
                .get(cave_target)
                .expect("cave target")
                .level,
            IlluminationLevel::Bright
        );

        app.world_mut().entity_mut(lamp).insert(exterior);
        app.update();

        assert_eq!(
            app.world()
                .resource::<ResolvedIllumination>()
                .get(cave_target)
                .expect("cave target")
                .level,
            IlluminationLevel::Dark
        );
        assert_eq!(
            app.world()
                .resource::<FactionMapKnowledge>()
                .faction(Faction::Player)
                .state(cave_target),
            KnowledgeState::Remembered
        );
    }

    #[derive(Resource)]
    struct RebuildOnce {
        old: Entity,
        replacement: TilePos,
        substance: SubstanceId,
        rebuild_now: bool,
    }

    fn rebuild_before_perception(mut commands: Commands, mut rebuild: ResMut<RebuildOnce>) {
        if !rebuild.rebuild_now {
            return;
        }
        commands.entity(rebuild.old).despawn();
        commands.spawn((
            HexTile,
            rebuild.replacement,
            span(rebuild.replacement.level),
            rebuild.substance,
            Headroom(2),
        ));
        rebuild.rebuild_now = false;
    }

    #[test]
    fn deferred_terrain_rebuild_is_flushed_before_snapshotting() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        let player = pos(0, 0, 5);
        let old = pos(2, 0, 5);
        let replacement = pos(3, 0, 5);
        spawn_tile(&mut app, player, substances.stone, 2);
        let old_entity = spawn_tile(&mut app, old, substances.stone, 2);
        spawn_unit(&mut app, 0, Faction::Player, player);
        app.insert_resource(RebuildOnce {
            old: old_entity,
            replacement,
            substance: substances.stone,
            rebuild_now: false,
        });
        app.add_systems(
            Update,
            rebuild_before_perception.before(PerceptionSystems::ResolveIllumination),
        );

        enter(&mut app, Screen::Gameplay);
        app.world_mut().resource_mut::<RebuildOnce>().rebuild_now = true;
        app.update();

        let surfaces = app.world().resource::<SurfaceSnapshots>();
        assert!(surfaces.get(old).is_none());
        assert!(surfaces.get(replacement).is_some());
        assert_eq!(
            app.world()
                .resource::<FactionMapKnowledge>()
                .faction(Faction::Player)
                .state(old),
            KnowledgeState::Unknown
        );
    }

    #[test]
    fn gameplay_exit_clears_memory_before_reentry() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Dim);
        let player = pos(0, 0, 5);
        let distant = pos(8, 0, 5);
        spawn_tile(&mut app, player, substances.stone, 2);
        spawn_tile(&mut app, distant, substances.stone, 2);
        spawn_unit(&mut app, 0, Faction::Player, player);

        enter(&mut app, Screen::Gameplay);
        assert_eq!(
            app.world()
                .resource::<FactionMapKnowledge>()
                .faction(Faction::Player)
                .state(distant),
            KnowledgeState::Observed
        );

        enter(&mut app, Screen::Title);
        assert!(!app.world().contains_resource::<SurfaceSnapshots>());
        assert!(!app.world().contains_resource::<ResolvedIllumination>());
        assert!(!app.world().contains_resource::<FactionObservations>());
        assert!(!app.world().contains_resource::<FactionMapKnowledge>());
        assert!(!app.world().contains_resource::<LocalMapKnowledge>());

        app.world_mut().resource_mut::<PerceptionSettings>().active = SightPreset::Tight;
        enter(&mut app, Screen::Gameplay);
        assert_eq!(
            app.world()
                .resource::<FactionMapKnowledge>()
                .faction(Faction::Player)
                .state(distant),
            KnowledgeState::Unknown
        );
    }

    #[test]
    #[ignore = "manual headless radius-40 perception recomputation benchmark"]
    fn radius_40_recomputation_benchmark() {
        let snapshots = SurfaceSnapshots::try_from_iter(
            HexCoord::ORIGIN.within_radius(40).into_iter().map(|coord| {
                let position = TilePos::new(coord, 15);
                SurfaceSnapshot {
                    pos: position,
                    span: span(position.level),
                    substance: SubstanceId(1),
                    headroom: Headroom(8),
                    is_solid: true,
                    blocked: false,
                    domain: LightDomain::Exterior,
                }
            }),
        )
        .expect("radius-40 surfaces");
        let ambient = ExteriorIllumination::new(IlluminationLevel::Bright);
        let units = [
            ObservedUnit {
                id: UnitId(0),
                faction: Faction::Player,
                pos: TilePos::new(HexCoord::ORIGIN, 15),
            },
            ObservedUnit {
                id: UnitId(1),
                faction: Faction::Hostile,
                pos: TilePos::new(HexCoord::from_axial(40, 0), 15),
            },
        ];
        let mut samples = Vec::new();
        for _ in 0..12 {
            let started = Instant::now();
            let illumination = ResolvedIllumination::from_surfaces(&snapshots, ambient, &[])
                .expect("benchmark illumination");
            let mut knowledge = FactionMapKnowledge::new();
            let observations = resolve_observations(
                units,
                &illumination,
                &knowledge,
                ambient,
                &[],
                SightProfile::DEFAULT,
            )
            .expect("benchmark observations");
            apply_observations(&mut knowledge, &snapshots, &observations);
            std::hint::black_box((illumination, observations, knowledge));
            samples.push(started.elapsed());
        }
        samples.sort_unstable();
        let median = samples
            .get(samples.len() / 2)
            .copied()
            .expect("twelve samples");
        let p95 = samples.last().copied().expect("twelve samples");
        eprintln!("radius-40 perception recomputation: median={median:?} p95={p95:?}");
        let budget = if cfg!(debug_assertions) {
            Duration::from_millis(250)
        } else {
            Duration::from_millis(50)
        };
        assert!(median < budget && p95 < budget);
    }
}
