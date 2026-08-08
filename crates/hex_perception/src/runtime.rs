//! ECS projection and lifecycle for the headless perception rules.

use bevy_app::{App, Update};
use bevy_ecs::prelude::*;
use bevy_ecs::reflect::ReflectResource;
use bevy_ecs::schedule::common_conditions::{not, resource_exists};
use bevy_ecs::system::SystemParam;
use bevy_log::error;
use bevy_reflect::Reflect;
use bevy_state::prelude::*;
use hex_assets::{PerceptionSettings, SubstanceTable};
use hex_core::{
    ExteriorIllumination, GameplayLight, GameplaySetupFailure, Headroom, HexSpan, HexTile,
    IlluminationLevel, InteriorRegions, LightDomain, LocalMapKnowledge, PausableSystems,
    PerceptionSystems, RunBottom, Screen, SubstanceId, TerrainReady, TilePos, TraversalBlockers,
    UnitId,
};
use hex_units::{
    AuthoredObjectOccupancy, AuthoredObjectOccupancySystems, Body, Downed, Faction,
    MovementSystems, StandsOn, TerrainOccupancy, TerrainOccupancySystems,
};

use crate::{
    apply_observations, resolve_observations_with_authored_objects, FactionMapKnowledge,
    FactionObservations, LightSourceSnapshot, ObservedUnit, ResolvedIllumination, SurfaceSnapshot,
    SurfaceSnapshots,
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
        Has<Downed>,
    ),
    With<Body>,
>;

/// Local-light inputs captured with the surface frame they illuminated.
///
/// Observation of deleted remembered positions must use the same public light
/// snapshot as current surfaces. Keeping it between the ordered stages prevents an
/// unrelated system from moving a light halfway through one perception update.
#[derive(Resource, Reflect, Debug, Default)]
#[reflect(Resource)]
struct PerceptionFrame {
    lights: Vec<LightSourceSnapshot>,
}

/// Recompute counters exposed to the development inspector and benchmarks.
///
/// These counters are session-scoped diagnostics. An unchanged gameplay frame
/// increments only `frames_checked`; the remaining counters prove that cached
/// surface, illumination, observation, and knowledge projections were reused.
#[derive(Resource, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[reflect(Resource)]
pub struct PerceptionRuntimeStats {
    /// Gameplay update frames inspected for input changes.
    pub frames_checked: u64,
    /// Full exact-surface snapshots built from ECS terrain.
    pub surface_rebuilds: u64,
    /// Objective illumination maps resolved.
    pub illumination_resolutions: u64,
    /// Pooled faction observations resolved.
    pub observation_resolutions: u64,
    /// Faction and local knowledge projections published.
    pub knowledge_publications: u64,
}

#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq, Eq)]
#[reflect(Resource)]
struct PerceptionInvalidation {
    surfaces: bool,
    illumination: bool,
    observation: bool,
    knowledge: bool,
}

impl PerceptionInvalidation {
    const fn all() -> Self {
        Self {
            surfaces: true,
            illumination: true,
            observation: true,
            knowledge: true,
        }
    }

    fn invalidate_surfaces(&mut self) {
        self.surfaces = true;
        self.illumination = true;
        self.observation = true;
        self.knowledge = true;
    }

    fn invalidate_illumination(&mut self) {
        self.illumination = true;
        self.observation = true;
        self.knowledge = true;
    }

    fn invalidate_observation(&mut self) {
        self.observation = true;
        self.knowledge = true;
    }
}

impl Default for PerceptionInvalidation {
    fn default() -> Self {
        Self::all()
    }
}

/// Adds authoritative illumination, faction sight, and session knowledge systems.
pub fn plugin(app: &mut App) {
    app.register_type::<GameplayLight>()
        .register_type::<LightDomain>()
        .register_type::<IlluminationLevel>()
        .register_type::<SurfaceSnapshots>()
        .register_type::<ResolvedIllumination>()
        .register_type::<FactionObservations>()
        .register_type::<FactionMapKnowledge>()
        .register_type::<PerceptionFrame>()
        .register_type::<PerceptionInvalidation>()
        .register_type::<PerceptionRuntimeStats>()
        .init_resource::<PerceptionInvalidation>()
        .init_resource::<PerceptionRuntimeStats>()
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
            detect_perception_input_changes
                .in_set(PerceptionSystems::ResolveIllumination)
                .after(MovementSystems::Reconcile)
                .after(TerrainOccupancySystems::Publish)
                .after(AuthoredObjectOccupancySystems::Publish)
                .before(resolve_illumination)
                .run_if(in_state(Screen::Gameplay))
                .run_if(resource_exists::<TerrainReady>)
                .run_if(not(resource_exists::<GameplaySetupFailure>)),
        )
        .add_systems(
            Update,
            resolve_illumination
                .in_set(PerceptionSystems::ResolveIllumination)
                .in_set(PausableSystems)
                .after(MovementSystems::Reconcile)
                .run_if(in_state(Screen::Gameplay))
                .run_if(resource_exists::<TerrainReady>)
                .run_if(not(resource_exists::<GameplaySetupFailure>)),
        )
        .add_systems(
            Update,
            resolve_observation
                .in_set(PerceptionSystems::ResolveObservation)
                .in_set(PausableSystems)
                .after(MovementSystems::Reconcile)
                .run_if(in_state(Screen::Gameplay))
                .run_if(resource_exists::<TerrainReady>)
                .run_if(not(resource_exists::<GameplaySetupFailure>)),
        )
        .add_systems(
            Update,
            publish_knowledge
                .in_set(PerceptionSystems::PublishKnowledge)
                .in_set(PausableSystems)
                .run_if(in_state(Screen::Gameplay))
                .run_if(resource_exists::<TerrainReady>)
                .run_if(not(resource_exists::<GameplaySetupFailure>)),
        )
        .add_systems(OnExit(Screen::Gameplay), clear_session);
}

#[derive(SystemParam)]
struct PerceptionInputChanges<'w, 's> {
    tile_entities: Query<'w, 's, (), With<HexTile>>,
    light_entities: Query<'w, 's, (), With<GameplayLight>>,
    changed_tiles: Query<
        'w,
        's,
        (),
        (
            With<HexTile>,
            Or<(
                Changed<HexTile>,
                Changed<TilePos>,
                Changed<HexSpan>,
                Changed<SubstanceId>,
                Changed<Headroom>,
            )>,
        ),
    >,
    changed_lights: Query<
        'w,
        's,
        (),
        (
            With<GameplayLight>,
            Or<(Changed<GameplayLight>, Changed<TilePos>)>,
        ),
    >,
    changed_run_bottoms: Query<'w, 's, (), (With<HexTile>, Changed<RunBottom>)>,
    changed_units: Query<
        'w,
        's,
        (),
        (
            With<Body>,
            Or<(
                Changed<Body>,
                Changed<UnitId>,
                Changed<Faction>,
                Changed<StandsOn>,
                Changed<Downed>,
            )>,
        ),
    >,
    removed_tiles: RemovedComponents<'w, 's, HexTile>,
    removed_positions: RemovedComponents<'w, 's, TilePos>,
    removed_spans: RemovedComponents<'w, 's, HexSpan>,
    removed_substances: RemovedComponents<'w, 's, SubstanceId>,
    removed_headroom: RemovedComponents<'w, 's, Headroom>,
    removed_run_bottoms: RemovedComponents<'w, 's, RunBottom>,
    removed_lights: RemovedComponents<'w, 's, GameplayLight>,
    removed_bodies: RemovedComponents<'w, 's, Body>,
    removed_unit_ids: RemovedComponents<'w, 's, UnitId>,
    removed_factions: RemovedComponents<'w, 's, Faction>,
    removed_standing: RemovedComponents<'w, 's, StandsOn>,
    removed_downed: RemovedComponents<'w, 's, Downed>,
    table: Option<Res<'w, SubstanceTable>>,
    exterior: Option<Res<'w, ExteriorIllumination>>,
    interiors: Option<Res<'w, InteriorRegions>>,
    blockers: Option<Res<'w, TraversalBlockers>>,
    settings: Option<Res<'w, PerceptionSettings>>,
    terrain_ready: Option<Res<'w, TerrainReady>>,
    occupancy: Option<Res<'w, TerrainOccupancy>>,
    authored_objects: Option<Res<'w, AuthoredObjectOccupancy>>,
}

fn detect_perception_input_changes(
    mut inputs: PerceptionInputChanges,
    mut invalidation: ResMut<PerceptionInvalidation>,
    mut stats: ResMut<PerceptionRuntimeStats>,
) {
    stats.frames_checked = stats.frames_checked.saturating_add(1);

    let removed_positions = inputs.removed_positions.read().collect::<Vec<_>>();
    let tiles_removed = inputs.removed_tiles.read().count() != 0;
    let spans_removed = inputs.removed_spans.read().count() != 0;
    let substances_removed = inputs.removed_substances.read().count() != 0;
    let headroom_removed = inputs.removed_headroom.read().count() != 0;
    let lights_removed = inputs.removed_lights.read().count() != 0;
    let bodies_removed = inputs.removed_bodies.read().count() != 0;
    let unit_ids_removed = inputs.removed_unit_ids.read().count() != 0;
    let factions_removed = inputs.removed_factions.read().count() != 0;
    let standing_removed = inputs.removed_standing.read().count() != 0;
    let downed_removed = inputs.removed_downed.read().count() != 0;
    let mut run_bottom_removed = false;
    for entity in inputs.removed_run_bottoms.read() {
        run_bottom_removed |= inputs.tile_entities.contains(entity);
    }
    let tile_position_removed = removed_positions
        .iter()
        .any(|entity| inputs.tile_entities.contains(*entity));
    let light_position_removed = removed_positions
        .iter()
        .any(|entity| inputs.light_entities.contains(*entity));
    let surfaces_changed = !inputs.changed_tiles.is_empty()
        || tiles_removed
        || tile_position_removed
        || spans_removed
        || substances_removed
        || headroom_removed
        || inputs
            .table
            .as_ref()
            .is_some_and(|resource| resource.is_changed())
        || inputs
            .interiors
            .as_ref()
            .is_some_and(|resource| resource.is_changed())
        || inputs
            .blockers
            .as_ref()
            .is_some_and(|resource| resource.is_changed())
        || inputs
            .terrain_ready
            .as_ref()
            .is_some_and(|resource| resource.is_changed());
    if surfaces_changed {
        invalidation.invalidate_surfaces();
    }

    let illumination_changed = !inputs.changed_lights.is_empty()
        || lights_removed
        || light_position_removed
        || inputs
            .exterior
            .as_ref()
            .is_some_and(|resource| resource.is_changed())
        || inputs
            .interiors
            .as_ref()
            .is_some_and(|resource| resource.is_changed());
    if illumination_changed {
        invalidation.invalidate_illumination();
    }

    let observation_changed = !inputs.changed_units.is_empty()
        || !inputs.changed_run_bottoms.is_empty()
        || run_bottom_removed
        || bodies_removed
        || unit_ids_removed
        || factions_removed
        || standing_removed
        || downed_removed
        || inputs
            .settings
            .as_ref()
            .is_some_and(|resource| resource.is_changed())
        || inputs
            .occupancy
            .as_ref()
            .is_none_or(|resource| resource.is_changed())
        || inputs
            .authored_objects
            .as_ref()
            .is_none_or(|resource| resource.is_changed());
    if observation_changed {
        invalidation.invalidate_observation();
    }
}

fn resolve_illumination(
    mut commands: Commands,
    tiles: TileProjectionQuery,
    light_entities: LightProjectionQuery,
    cached_surfaces: Option<Res<SurfaceSnapshots>>,
    table: Option<Res<SubstanceTable>>,
    exterior: Option<Res<ExteriorIllumination>>,
    interiors: Option<Res<InteriorRegions>>,
    blockers: Option<Res<TraversalBlockers>>,
    mut invalidation: ResMut<PerceptionInvalidation>,
    mut stats: ResMut<PerceptionRuntimeStats>,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    if !invalidation.surfaces && !invalidation.illumination {
        return;
    }

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

    let rebuilt_surfaces = if invalidation.surfaces {
        match snapshot_surfaces(&tiles, &table, interiors.as_deref(), blockers.as_deref()) {
            Ok(surfaces) if !surfaces.is_empty() => Some(surfaces),
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
        }
    } else {
        None
    };
    let Some(surfaces) = rebuilt_surfaces.as_ref().or(cached_surfaces.as_deref()) else {
        fail(
            &mut commands,
            &mut next_screen,
            "Perception illumination started without a cached surface frame.",
        );
        return;
    };
    let lights = match snapshot_lights(&light_entities, interiors.as_deref()) {
        Ok(lights) => lights,
        Err(reason) => {
            fail(&mut commands, &mut next_screen, reason);
            return;
        }
    };
    let illumination = match ResolvedIllumination::from_surfaces(surfaces, *exterior, &lights) {
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

    if let Some(surfaces) = rebuilt_surfaces {
        commands.insert_resource(surfaces);
        stats.surface_rebuilds = stats.surface_rebuilds.saturating_add(1);
    }
    commands.insert_resource(PerceptionFrame { lights });
    commands.insert_resource(illumination);
    stats.illumination_resolutions = stats.illumination_resolutions.saturating_add(1);
    invalidation.surfaces = false;
    invalidation.illumination = false;
    invalidation.observation = true;
    invalidation.knowledge = true;
}

fn resolve_observation(
    mut commands: Commands,
    units: UnitProjectionQuery,
    surfaces: Option<Res<SurfaceSnapshots>>,
    illumination: Option<Res<ResolvedIllumination>>,
    frame: Option<Res<PerceptionFrame>>,
    exterior: Option<Res<ExteriorIllumination>>,
    settings: Option<Res<PerceptionSettings>>,
    terrain: Option<Res<TerrainOccupancy>>,
    authored_objects: Option<Res<AuthoredObjectOccupancy>>,
    prior_knowledge: Option<Res<FactionMapKnowledge>>,
    mut invalidation: ResMut<PerceptionInvalidation>,
    mut stats: ResMut<PerceptionRuntimeStats>,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    if !invalidation.observation {
        return;
    }

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
    let Some(terrain) = terrain.filter(|terrain| !terrain.is_empty()) else {
        fail(
            &mut commands,
            &mut next_screen,
            "Perception observation started without authoritative terrain occupancy.",
        );
        return;
    };
    let Some(authored_objects) = authored_objects else {
        fail(
            &mut commands,
            &mut next_screen,
            "Perception observation started without authoritative authored-object occupancy.",
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
    let observations = match resolve_observations_with_authored_objects(
        units,
        &illumination,
        prior,
        *exterior,
        &frame.lights,
        settings.active_profile(),
        &terrain,
        &authored_objects,
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
    stats.observation_resolutions = stats.observation_resolutions.saturating_add(1);
    invalidation.observation = false;
    invalidation.knowledge = true;
}

fn publish_knowledge(
    mut commands: Commands,
    surfaces: Option<Res<SurfaceSnapshots>>,
    observations: Option<Res<FactionObservations>>,
    knowledge: Option<ResMut<FactionMapKnowledge>>,
    local: Option<ResMut<LocalMapKnowledge>>,
    mut invalidation: ResMut<PerceptionInvalidation>,
    mut stats: ResMut<PerceptionRuntimeStats>,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    if !invalidation.knowledge {
        return;
    }

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
    stats.knowledge_publications = stats.knowledge_publications.saturating_add(1);
    invalidation.knowledge = false;
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
    for (id, faction, standing, downed) in units {
        let (Some(id), Some(faction), Some(standing)) = (id, faction, standing) else {
            return Err("A Body is missing UnitId, Faction, or StandsOn.".to_owned());
        };
        snapshots.push(ObservedUnit {
            id: *id,
            faction: *faction,
            pos: standing.0.pos,
            provides_sight: !downed,
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
    // A failure must withdraw every player-facing spatial authorization before the
    // final gameplay presentation pass. Otherwise a previously observed hostile can
    // remain disclosed for one stale frame while the state transition is pending.
    commands.remove_resource::<FactionObservations>();
    commands.remove_resource::<FactionMapKnowledge>();
    commands.remove_resource::<LocalMapKnowledge>();
    commands.insert_resource(GameplaySetupFailure::new(reason));
    next_screen.set(Screen::Title);
}

fn clear_session(
    mut commands: Commands,
    mut invalidation: ResMut<PerceptionInvalidation>,
    mut stats: ResMut<PerceptionRuntimeStats>,
) {
    commands.remove_resource::<PerceptionFrame>();
    commands.remove_resource::<SurfaceSnapshots>();
    commands.remove_resource::<ResolvedIllumination>();
    commands.remove_resource::<FactionObservations>();
    commands.remove_resource::<FactionMapKnowledge>();
    commands.remove_resource::<LocalMapKnowledge>();
    *invalidation = PerceptionInvalidation::all();
    *stats = PerceptionRuntimeStats::default();
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;
    use std::collections::{BTreeMap, BTreeSet};
    use std::time::{Duration, Instant};

    use bevy_ecs::reflect::{AppTypeRegistry, ReflectResource};
    use bevy_platform::collections::HashMap;
    use hex_assets::{
        ArtPalette, PaletteSwatch, SightPreset, SrgbColor, Substance, SubstanceFile, SwatchId,
    };
    use hex_core::{
        AuthoredObjectVoxelRun, AuthoredObjectVoxelRuns, GameplaySetup, HexCoord, InteriorRegionId,
        KnowledgeState, SightProfile, TraversalProfile,
    };
    use hex_test_app::HeadlessAppBuilder;
    use hex_units::Standing;

    use super::*;
    #[derive(Clone, Copy)]
    struct TestSubstances {
        stone: SubstanceId,
        water: SubstanceId,
    }

    #[derive(Resource)]
    struct PerceptionUpdatesEnabled(bool);

    fn test_table() -> (SubstanceTable, TestSubstances) {
        let swatch_id = SwatchId::new("test/gray").expect("the fixture swatch id should be valid");
        let swatch = PaletteSwatch::new(
            "Test Gray",
            SrgbColor::new(0.5, 0.5, 0.5).expect("the fixture color should be valid"),
            BTreeSet::from(["test".to_owned()]),
        )
        .expect("the fixture swatch should be valid");
        let palette = ArtPalette::new(BTreeMap::from([(swatch_id.clone(), swatch)]))
            .expect("the fixture palette should be valid");
        let substances = HashMap::from_iter([
            ("air".to_owned(), Substance::invisible(false, false)),
            (
                "stone".to_owned(),
                Substance::from_swatch(swatch_id.clone(), true, true),
            ),
            (
                "water".to_owned(),
                Substance::from_swatch(swatch_id, false, true),
            ),
        ]);
        let table = SubstanceTable::from_file(&SubstanceFile { substances }, &palette)
            .expect("the fixture substances should resolve through their palette");
        let ids = TestSubstances {
            stone: table.id("stone").expect("stone fixture"),
            water: table.id("water").expect("water fixture"),
        };
        (table, ids)
    }

    fn runtime_app(exterior: IlluminationLevel) -> (App, TestSubstances) {
        let mut builder = HeadlessAppBuilder::new()
            .with_state_plugin()
            .with_gameplay_sets();
        builder.app_mut().init_state::<Screen>();
        builder.app_mut().configure_sets(
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
        builder
            .app_mut()
            .insert_resource(PerceptionUpdatesEnabled(true));
        builder.app_mut().configure_sets(
            Update,
            PausableSystems.run_if(|enabled: Res<PerceptionUpdatesEnabled>| enabled.0),
        );
        builder.app_mut().configure_sets(
            OnEnter(Screen::Gameplay),
            (
                PerceptionSystems::PublishAmbient,
                PerceptionSystems::ResolveIllumination,
                PerceptionSystems::ResolveObservation,
                PerceptionSystems::PublishKnowledge,
                PerceptionSystems::ApplyPresentation,
            )
                .chain()
                .in_set(GameplaySetup::Perception),
        );
        let (table, substances) = test_table();
        builder.app_mut().insert_resource(table);
        builder
            .app_mut()
            .insert_resource(PerceptionSettings::default());
        builder
            .app_mut()
            .insert_resource(ExteriorIllumination::new(exterior));
        builder.app_mut().insert_resource(InteriorRegions::new());
        builder.app_mut().insert_resource(TraversalBlockers::new());
        builder.app_mut().insert_resource(TerrainReady);
        hex_units::authored_object_occupancy::plugin(builder.app_mut());
        builder.app_mut().add_plugins(plugin);
        (builder.build(), substances)
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
        spawn_tile_run(app, position, position.level, substance, headroom)
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "test terrain levels stay far inside f32's exact integer range"
    )]
    fn spawn_tile_run(
        app: &mut App,
        position: TilePos,
        bottom: i32,
        substance: SubstanceId,
        headroom: i32,
    ) -> Entity {
        let run_span = HexSpan::new(bottom as f32, (position.level + 1) as f32);
        let entity = app
            .world_mut()
            .spawn((
                HexTile,
                position,
                RunBottom(bottom),
                run_span,
                substance,
                Headroom(headroom),
            ))
            .id();
        let runs = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<(&TilePos, &RunBottom), With<HexTile>>();
            query
                .iter(world)
                .map(|(&top, &bottom)| (top, bottom))
                .collect::<Vec<_>>()
        };
        app.insert_resource(
            TerrainOccupancy::from_runs(runs).expect("test fixture terrain runs must be valid"),
        );
        entity
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
    fn full_run_one_level_ridge_is_observed_through_the_runtime_pipeline() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        let observer = pos(0, 0, 0);
        let before_ridge = pos(1, 0, 0);
        let ridge = pos(2, 0, 1);
        let target = pos(3, 0, 0);
        for position in [observer, before_ridge, ridge, target] {
            spawn_tile_run(&mut app, position, -4, substances.stone, 3);
        }
        spawn_unit(&mut app, 0, Faction::Player, observer);
        spawn_unit(&mut app, 1, Faction::Hostile, target);

        enter(&mut app, Screen::Gameplay);

        let observations = app.world().resource::<FactionObservations>();
        assert!(observations.faction(Faction::Player).observes(target));
        assert_eq!(
            observations.faction(Faction::Player).unit(UnitId(1)),
            Some(ObservedUnit {
                id: UnitId(1),
                faction: Faction::Hostile,
                pos: target,
                provides_sight: true,
            })
        );
        assert_eq!(
            app.world()
                .resource::<FactionMapKnowledge>()
                .faction(Faction::Player)
                .state(target),
            KnowledgeState::Observed
        );
        assert!(app
            .world()
            .resource::<FactionMapKnowledge>()
            .faction(Faction::Player)
            .unit(UnitId(1))
            .is_some());
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
    fn missing_authoritative_terrain_occupancy_fails_closed() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        spawn_tile(&mut app, pos(0, 0, 5), substances.stone, 2);
        app.world_mut().remove_resource::<TerrainOccupancy>();

        enter(&mut app, Screen::Gameplay);

        let failure = app.world().resource::<GameplaySetupFailure>();
        assert!(failure.reason.contains("authoritative terrain occupancy"));
        assert!(!app.world().contains_resource::<FactionObservations>());
        assert!(!app.world().contains_resource::<FactionMapKnowledge>());
    }

    #[test]
    fn missing_authoritative_object_occupancy_fails_closed_even_when_empty_is_valid() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        spawn_tile(&mut app, pos(0, 0, 5), substances.stone, 2);
        enter(&mut app, Screen::Gameplay);
        assert!(app.world().contains_resource::<AuthoredObjectOccupancy>());

        app.world_mut().remove_resource::<AuthoredObjectOccupancy>();
        app.update();

        let failure = app.world().resource::<GameplaySetupFailure>();
        assert!(failure
            .reason
            .contains("authoritative authored-object occupancy"));
        assert!(!app.world().contains_resource::<FactionObservations>());
        assert!(!app.world().contains_resource::<FactionMapKnowledge>());
    }

    #[test]
    fn authored_object_change_hides_and_reveals_a_hostile_in_the_same_update() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        let player = pos(0, 0, 0);
        let hostile = pos(4, 0, 0);
        spawn_tile(&mut app, player, substances.stone, 3);
        spawn_tile(&mut app, hostile, substances.stone, 3);
        spawn_unit(&mut app, 0, Faction::Player, player);
        spawn_unit(&mut app, 1, Faction::Hostile, hostile);
        let source = app
            .world_mut()
            .spawn(AuthoredObjectVoxelRuns::default())
            .id();
        enter(&mut app, Screen::Gameplay);
        assert!(app
            .world()
            .resource::<FactionObservations>()
            .faction(Faction::Player)
            .observes(hostile));
        let before_blocking = *app.world().resource::<PerceptionRuntimeStats>();

        app.world_mut()
            .entity_mut(source)
            .insert(AuthoredObjectVoxelRuns::new((1..=3).flat_map(|q| {
                (-2..=2).map(move |r| AuthoredObjectVoxelRun::new(pos(q, r, 1), 0))
            })));
        app.update();
        assert!(!app
            .world()
            .resource::<FactionObservations>()
            .faction(Faction::Player)
            .observes(hostile));
        assert!(app
            .world()
            .resource::<FactionMapKnowledge>()
            .faction(Faction::Player)
            .unit(UnitId(1))
            .is_none());
        let after_blocking = *app.world().resource::<PerceptionRuntimeStats>();
        assert_eq!(
            after_blocking.surface_rebuilds,
            before_blocking.surface_rebuilds
        );
        assert_eq!(
            after_blocking.illumination_resolutions,
            before_blocking.illumination_resolutions
        );
        assert_eq!(
            after_blocking.observation_resolutions,
            before_blocking.observation_resolutions + 1
        );
        assert_eq!(
            after_blocking.knowledge_publications,
            before_blocking.knowledge_publications + 1
        );

        app.world_mut()
            .entity_mut(source)
            .remove::<AuthoredObjectVoxelRuns>();
        app.update();
        assert!(app
            .world()
            .resource::<FactionObservations>()
            .faction(Faction::Player)
            .observes(hostile));
        let after_removal = *app.world().resource::<PerceptionRuntimeStats>();
        assert_eq!(
            after_removal.surface_rebuilds,
            after_blocking.surface_rebuilds
        );
        assert_eq!(
            after_removal.illumination_resolutions,
            after_blocking.illumination_resolutions
        );
        assert_eq!(
            after_removal.observation_resolutions,
            after_blocking.observation_resolutions + 1
        );
        assert_eq!(
            after_removal.knowledge_publications,
            after_blocking.knowledge_publications + 1
        );
    }

    #[test]
    fn malformed_authored_source_withdraws_authority_and_spatial_knowledge_same_frame() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        let player = pos(0, 0, 0);
        let hostile = pos(2, 0, 0);
        spawn_tile(&mut app, player, substances.stone, 3);
        spawn_tile(&mut app, hostile, substances.stone, 3);
        spawn_unit(&mut app, 0, Faction::Player, player);
        spawn_unit(&mut app, 1, Faction::Hostile, hostile);
        let source = app
            .world_mut()
            .spawn(AuthoredObjectVoxelRuns::default())
            .id();
        enter(&mut app, Screen::Gameplay);
        assert!(app
            .world()
            .resource::<FactionMapKnowledge>()
            .faction(Faction::Player)
            .unit(UnitId(1))
            .is_some());

        app.world_mut()
            .entity_mut(source)
            .insert(AuthoredObjectVoxelRuns::new([AuthoredObjectVoxelRun::new(
                pos(1, 0, 3),
                4,
            )]));
        app.update();

        assert!(!app.world().contains_resource::<AuthoredObjectOccupancy>());
        assert!(app
            .world()
            .resource::<GameplaySetupFailure>()
            .reason
            .contains("authoritative authored-object occupancy"));
        assert!(!app.world().contains_resource::<FactionObservations>());
        assert!(!app.world().contains_resource::<FactionMapKnowledge>());
        assert!(!app.world().contains_resource::<LocalMapKnowledge>());
    }

    #[test]
    fn malformed_authored_source_fails_initial_perception_setup_closed() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        let player = pos(0, 0, 0);
        spawn_tile(&mut app, player, substances.stone, 3);
        spawn_unit(&mut app, 0, Faction::Player, player);
        app.world_mut()
            .spawn(AuthoredObjectVoxelRuns::new([AuthoredObjectVoxelRun::new(
                pos(1, 0, 3),
                4,
            )]));

        enter(&mut app, Screen::Gameplay);

        assert!(!app.world().contains_resource::<AuthoredObjectOccupancy>());
        assert!(app
            .world()
            .resource::<GameplaySetupFailure>()
            .reason
            .contains("authoritative authored-object occupancy"));
        assert!(!app.world().contains_resource::<FactionObservations>());
        assert!(!app.world().contains_resource::<FactionMapKnowledge>());
        assert!(!app.world().contains_resource::<LocalMapKnowledge>());
    }

    #[test]
    fn withdrawn_occupancy_clears_previously_published_knowledge_before_failure() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        let player = pos(0, 0, 5);
        let hostile = pos(2, 0, 5);
        spawn_tile(&mut app, player, substances.stone, 2);
        spawn_tile(&mut app, hostile, substances.stone, 2);
        spawn_unit(&mut app, 0, Faction::Player, player);
        spawn_unit(&mut app, 1, Faction::Hostile, hostile);

        enter(&mut app, Screen::Gameplay);
        assert!(app
            .world()
            .resource::<FactionMapKnowledge>()
            .faction(Faction::Player)
            .unit(UnitId(1))
            .is_some());

        app.world_mut().remove_resource::<TerrainOccupancy>();
        app.update();

        assert!(app
            .world()
            .resource::<GameplaySetupFailure>()
            .reason
            .contains("authoritative terrain occupancy"));
        assert!(!app.world().contains_resource::<FactionObservations>());
        assert!(!app.world().contains_resource::<FactionMapKnowledge>());
        assert!(!app.world().contains_resource::<LocalMapKnowledge>());
    }

    #[test]
    fn owned_runtime_resources_are_registered_for_inspection() {
        let (app, _) = runtime_app(IlluminationLevel::Bright);
        let registry = app.world().resource::<AppTypeRegistry>().read();

        for type_id in [
            TypeId::of::<SurfaceSnapshots>(),
            TypeId::of::<ResolvedIllumination>(),
            TypeId::of::<FactionObservations>(),
            TypeId::of::<FactionMapKnowledge>(),
            TypeId::of::<PerceptionRuntimeStats>(),
        ] {
            assert!(
                registry.get_type_data::<ReflectResource>(type_id).is_some(),
                "owned runtime resource is missing ReflectResource registration"
            );
        }
    }

    #[test]
    fn unchanged_frames_reuse_every_cached_projection() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        let player = pos(0, 0, 5);
        spawn_tile(&mut app, player, substances.stone, 2);
        spawn_unit(&mut app, 0, Faction::Player, player);

        enter(&mut app, Screen::Gameplay);
        let before = *app.world().resource::<PerceptionRuntimeStats>();
        app.update();
        let after = *app.world().resource::<PerceptionRuntimeStats>();

        assert_eq!(after.frames_checked, before.frames_checked + 1);
        assert_eq!(after.surface_rebuilds, before.surface_rebuilds);
        assert_eq!(
            after.illumination_resolutions,
            before.illumination_resolutions
        );
        assert_eq!(
            after.observation_resolutions,
            before.observation_resolutions
        );
        assert_eq!(after.knowledge_publications, before.knowledge_publications);
    }

    #[test]
    fn radius_40_idle_frames_do_not_recompute_full_map_projections() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        for coord in HexCoord::ORIGIN.within_radius(40) {
            spawn_tile(&mut app, TilePos::new(coord, 15), substances.stone, 8);
        }
        let player = TilePos::new(HexCoord::ORIGIN, 15);
        spawn_unit(&mut app, 0, Faction::Player, player);

        enter(&mut app, Screen::Gameplay);
        let before = *app.world().resource::<PerceptionRuntimeStats>();
        for _ in 0..8 {
            app.update();
        }
        let after = *app.world().resource::<PerceptionRuntimeStats>();

        assert_eq!(after.frames_checked, before.frames_checked + 8);
        assert_eq!(after.surface_rebuilds, before.surface_rebuilds);
        assert_eq!(
            after.illumination_resolutions,
            before.illumination_resolutions
        );
        assert_eq!(
            after.observation_resolutions,
            before.observation_resolutions
        );
        assert_eq!(after.knowledge_publications, before.knowledge_publications);
    }

    #[test]
    #[ignore = "manual 10,000-frame radius-40 lifecycle stress gate"]
    fn radius_40_ten_thousand_idle_frames_reuse_every_projection() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        for coord in HexCoord::ORIGIN.within_radius(40) {
            spawn_tile(&mut app, TilePos::new(coord, 15), substances.stone, 8);
        }
        let player = TilePos::new(HexCoord::ORIGIN, 15);
        spawn_unit(&mut app, 0, Faction::Player, player);

        enter(&mut app, Screen::Gameplay);
        let before = *app.world().resource::<PerceptionRuntimeStats>();
        for _ in 0..10_000 {
            app.update();
        }
        let after = *app.world().resource::<PerceptionRuntimeStats>();

        assert_eq!(after.frames_checked, before.frames_checked + 10_000);
        assert_eq!(after.surface_rebuilds, before.surface_rebuilds);
        assert_eq!(
            after.illumination_resolutions,
            before.illumination_resolutions
        );
        assert_eq!(
            after.observation_resolutions,
            before.observation_resolutions
        );
        assert_eq!(after.knowledge_publications, before.knowledge_publications);
    }

    #[test]
    fn unit_changes_restart_at_observation_without_rebuilding_the_map() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        let start = pos(0, 0, 5);
        let destination = pos(1, 0, 5);
        spawn_tile(&mut app, start, substances.stone, 2);
        spawn_tile(&mut app, destination, substances.stone, 2);
        let unit = spawn_unit(&mut app, 0, Faction::Player, start);

        enter(&mut app, Screen::Gameplay);
        let before = *app.world().resource::<PerceptionRuntimeStats>();
        app.world_mut().entity_mut(unit).insert(StandsOn(Standing {
            pos: destination,
            span: span(destination.level),
        }));
        app.update();
        let after = *app.world().resource::<PerceptionRuntimeStats>();

        assert_eq!(after.surface_rebuilds, before.surface_rebuilds);
        assert_eq!(
            after.illumination_resolutions,
            before.illumination_resolutions
        );
        assert_eq!(
            after.observation_resolutions,
            before.observation_resolutions + 1
        );
        assert_eq!(
            after.knowledge_publications,
            before.knowledge_publications + 1
        );

        app.world_mut().resource_mut::<PerceptionSettings>().active = SightPreset::Tight;
        app.update();
        let after_settings = *app.world().resource::<PerceptionRuntimeStats>();
        assert_eq!(after_settings.surface_rebuilds, after.surface_rebuilds);
        assert_eq!(
            after_settings.illumination_resolutions,
            after.illumination_resolutions
        );
        assert_eq!(
            after_settings.observation_resolutions,
            after.observation_resolutions + 1
        );
        assert_eq!(
            after_settings.knowledge_publications,
            after.knowledge_publications + 1
        );
    }

    #[test]
    fn downed_changes_republish_visibility_without_removing_the_visible_unit() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        let active = pos(0, 0, 5);
        let nearby = pos(1, 0, 5);
        spawn_tile(&mut app, active, substances.stone, 2);
        spawn_tile(&mut app, nearby, substances.stone, 2);
        spawn_unit(&mut app, 0, Faction::Player, active);
        let incapacitated = spawn_unit(&mut app, 1, Faction::Player, nearby);

        enter(&mut app, Screen::Gameplay);
        let before = *app.world().resource::<PerceptionRuntimeStats>();
        app.world_mut().entity_mut(incapacitated).insert(Downed);
        app.update();
        let after_down = *app.world().resource::<PerceptionRuntimeStats>();

        assert_eq!(after_down.surface_rebuilds, before.surface_rebuilds);
        assert_eq!(
            after_down.illumination_resolutions,
            before.illumination_resolutions
        );
        assert_eq!(
            after_down.observation_resolutions,
            before.observation_resolutions + 1
        );
        assert_eq!(
            after_down.knowledge_publications,
            before.knowledge_publications + 1
        );
        let visible = app
            .world()
            .resource::<FactionMapKnowledge>()
            .faction(Faction::Player)
            .unit(UnitId(1))
            .expect("the active nearby ally still observes the downed unit");
        assert!(
            !visible.provides_sight,
            "downed units remain visible but cannot extend faction sight"
        );

        app.world_mut().entity_mut(incapacitated).remove::<Downed>();
        app.update();
        let after_revival = *app.world().resource::<PerceptionRuntimeStats>();
        assert_eq!(
            after_revival.observation_resolutions,
            after_down.observation_resolutions + 1
        );
        assert_eq!(
            after_revival.knowledge_publications,
            after_down.knowledge_publications + 1
        );
        assert!(
            app.world()
                .resource::<FactionMapKnowledge>()
                .faction(Faction::Player)
                .unit(UnitId(1))
                .expect("the revived unit remains visible")
                .provides_sight
        );
    }

    #[test]
    fn ambient_changes_reuse_surfaces_and_restart_at_illumination() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        let player = pos(0, 0, 5);
        spawn_tile(&mut app, player, substances.stone, 2);
        spawn_unit(&mut app, 0, Faction::Player, player);

        enter(&mut app, Screen::Gameplay);
        let before = *app.world().resource::<PerceptionRuntimeStats>();
        *app.world_mut().resource_mut::<ExteriorIllumination>() =
            ExteriorIllumination::new(IlluminationLevel::Dim);
        app.update();
        let after = *app.world().resource::<PerceptionRuntimeStats>();

        assert_eq!(after.surface_rebuilds, before.surface_rebuilds);
        assert_eq!(
            after.illumination_resolutions,
            before.illumination_resolutions + 1
        );
        assert_eq!(
            after.observation_resolutions,
            before.observation_resolutions + 1
        );
        assert_eq!(
            after.knowledge_publications,
            before.knowledge_publications + 1
        );

        app.update();
        let settled = *app.world().resource::<PerceptionRuntimeStats>();
        assert_eq!(settled.frames_checked, after.frames_checked + 1);
        assert_eq!(settled.surface_rebuilds, after.surface_rebuilds);
        assert_eq!(
            settled.illumination_resolutions,
            after.illumination_resolutions
        );
        assert_eq!(
            settled.observation_resolutions,
            after.observation_resolutions
        );
        assert_eq!(settled.knowledge_publications, after.knowledge_publications);
    }

    #[test]
    fn removed_light_reuses_surfaces_and_recomputes_illumination() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Dark);
        let player = pos(0, 0, 5);
        spawn_tile(&mut app, player, substances.stone, 2);
        spawn_unit(&mut app, 0, Faction::Player, player);
        let light = app
            .world_mut()
            .spawn((player, GameplayLight::new(IlluminationLevel::Bright, 1)))
            .id();

        enter(&mut app, Screen::Gameplay);
        let before = *app.world().resource::<PerceptionRuntimeStats>();
        app.world_mut().despawn(light);
        app.update();
        let after = *app.world().resource::<PerceptionRuntimeStats>();

        assert_eq!(after.surface_rebuilds, before.surface_rebuilds);
        assert_eq!(
            after.illumination_resolutions,
            before.illumination_resolutions + 1
        );
        assert_eq!(
            after.observation_resolutions,
            before.observation_resolutions + 1
        );
        assert_eq!(
            after.knowledge_publications,
            before.knowledge_publications + 1
        );
    }

    #[test]
    fn blocker_changes_rebuild_surfaces_and_every_downstream_projection() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        let player = pos(0, 0, 5);
        spawn_tile(&mut app, player, substances.stone, 2);
        spawn_unit(&mut app, 0, Faction::Player, player);

        enter(&mut app, Screen::Gameplay);
        let before = *app.world().resource::<PerceptionRuntimeStats>();
        app.world_mut()
            .resource_mut::<TraversalBlockers>()
            .insert(player);
        app.update();
        let after = *app.world().resource::<PerceptionRuntimeStats>();

        assert_eq!(after.surface_rebuilds, before.surface_rebuilds + 1);
        assert_eq!(
            after.illumination_resolutions,
            before.illumination_resolutions + 1
        );
        assert_eq!(
            after.observation_resolutions,
            before.observation_resolutions + 1
        );
        assert_eq!(
            after.knowledge_publications,
            before.knowledge_publications + 1
        );
        assert!(
            app.world()
                .resource::<SurfaceSnapshots>()
                .get(player)
                .expect("player surface")
                .blocked
        );
    }

    #[test]
    fn published_run_bottom_change_recomputes_observation_in_the_same_frame() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        hex_units::terrain_occupancy::plugin(&mut app);
        let player = pos(0, 0, 5);
        let wall_top = pos(2, 0, 7);
        let hostile_pos = pos(4, 0, 5);
        spawn_tile(&mut app, player, substances.stone, 2);
        let wall = spawn_tile(&mut app, wall_top, substances.stone, 2);
        spawn_tile(&mut app, hostile_pos, substances.stone, 2);
        spawn_unit(&mut app, 0, Faction::Player, player);
        spawn_unit(&mut app, 1, Faction::Hostile, hostile_pos);

        enter(&mut app, Screen::Gameplay);
        assert!(
            app.world()
                .resource::<FactionMapKnowledge>()
                .faction(Faction::Player)
                .unit(UnitId(1))
                .is_some(),
            "the single high wall voxel starts above the sight line"
        );
        let before = *app.world().resource::<PerceptionRuntimeStats>();

        *app.world_mut()
            .entity_mut(wall)
            .get_mut::<RunBottom>()
            .expect("wall run bottom") = RunBottom(6);
        app.update();

        let after = *app.world().resource::<PerceptionRuntimeStats>();
        assert_eq!(after.surface_rebuilds, before.surface_rebuilds);
        assert_eq!(
            after.illumination_resolutions,
            before.illumination_resolutions
        );
        assert_eq!(
            after.observation_resolutions,
            before.observation_resolutions + 1
        );
        assert_eq!(
            after.knowledge_publications,
            before.knowledge_publications + 1
        );
        assert_eq!(
            app.world()
                .resource::<FactionMapKnowledge>()
                .faction(Faction::Player)
                .state(hostile_pos),
            KnowledgeState::Remembered
        );
        assert!(
            app.world()
                .resource::<FactionMapKnowledge>()
                .faction(Faction::Player)
                .unit(UnitId(1))
                .is_none(),
            "the newly extended wall must hide the hostile in the same frame"
        );
    }

    #[test]
    fn paused_perception_records_changes_without_resolving_them() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        let player = pos(0, 0, 5);
        spawn_tile(&mut app, player, substances.stone, 2);
        spawn_unit(&mut app, 0, Faction::Player, player);

        enter(&mut app, Screen::Gameplay);
        app.world_mut().resource_mut::<PerceptionUpdatesEnabled>().0 = false;
        let before = *app.world().resource::<PerceptionRuntimeStats>();
        *app.world_mut().resource_mut::<ExteriorIllumination>() =
            ExteriorIllumination::new(IlluminationLevel::Dim);
        app.update();
        let paused = *app.world().resource::<PerceptionRuntimeStats>();
        assert_eq!(paused.frames_checked, before.frames_checked + 1);
        assert_eq!(paused.surface_rebuilds, before.surface_rebuilds);
        assert_eq!(
            paused.illumination_resolutions,
            before.illumination_resolutions
        );
        assert_eq!(
            paused.observation_resolutions,
            before.observation_resolutions
        );
        assert_eq!(paused.knowledge_publications, before.knowledge_publications);

        app.world_mut().resource_mut::<PerceptionUpdatesEnabled>().0 = true;
        app.update();
        let resumed = *app.world().resource::<PerceptionRuntimeStats>();
        assert_eq!(
            resumed.illumination_resolutions,
            before.illumination_resolutions + 1
        );
    }

    #[test]
    fn removals_during_pause_invalidate_cached_perception_for_resume() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Dark);
        let player = pos(0, 0, 5);
        spawn_tile(&mut app, player, substances.stone, 2);
        spawn_unit(&mut app, 0, Faction::Player, player);
        let light = app
            .world_mut()
            .spawn((player, GameplayLight::new(IlluminationLevel::Bright, 1)))
            .id();

        enter(&mut app, Screen::Gameplay);
        assert_eq!(
            app.world()
                .resource::<ResolvedIllumination>()
                .get(player)
                .expect("player illumination")
                .level,
            IlluminationLevel::Bright
        );

        app.world_mut().resource_mut::<PerceptionUpdatesEnabled>().0 = false;
        let before = *app.world().resource::<PerceptionRuntimeStats>();
        app.world_mut().despawn(light);
        app.update();
        let paused = *app.world().resource::<PerceptionRuntimeStats>();
        assert_eq!(
            paused.illumination_resolutions,
            before.illumination_resolutions
        );

        app.world_mut().resource_mut::<PerceptionUpdatesEnabled>().0 = true;
        app.update();
        let resumed = *app.world().resource::<PerceptionRuntimeStats>();
        assert_eq!(
            resumed.illumination_resolutions,
            before.illumination_resolutions + 1
        );
        assert_eq!(
            app.world()
                .resource::<ResolvedIllumination>()
                .get(player)
                .expect("player illumination")
                .level,
            IlluminationLevel::Dark
        );
    }

    #[test]
    fn unit_on_filtered_surface_is_an_explicit_setup_failure() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        let valid = pos(0, 0, 5);
        let buried = pos(1, 0, 5);
        spawn_tile(&mut app, valid, substances.stone, 2);
        spawn_tile(&mut app, buried, substances.stone, 0);
        spawn_unit(&mut app, 0, Faction::Player, buried);

        enter(&mut app, Screen::Gameplay);

        let failure = app.world().resource::<GameplaySetupFailure>();
        assert!(failure.reason.contains("occupies no exposed surface"));
        assert!(!app.world().contains_resource::<FactionObservations>());
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
        app.world_mut()
            .spawn((observer, GameplayLight::new(IlluminationLevel::Bright, 4)));
        app.insert_resource(MoveLightOnce {
            destination: exterior,
            move_now: false,
        });
        app.add_systems(
            Update,
            reconcile_test_light_move
                .in_set(MovementSystems::Reconcile)
                .run_if(in_state(Screen::Gameplay)),
        );

        enter(&mut app, Screen::Gameplay);
        assert_eq!(
            app.world()
                .resource::<ResolvedIllumination>()
                .get(cave_target)
                .expect("cave target")
                .level,
            IlluminationLevel::Bright
        );

        app.world_mut().resource_mut::<MoveLightOnce>().move_now = true;
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
    struct MoveLightOnce {
        destination: TilePos,
        move_now: bool,
    }

    fn reconcile_test_light_move(
        mut request: ResMut<MoveLightOnce>,
        mut lights: Query<&mut TilePos, With<GameplayLight>>,
    ) {
        if !request.move_now {
            return;
        }
        for mut position in &mut lights {
            *position = request.destination;
        }
        request.move_now = false;
    }

    #[derive(Resource)]
    struct RebuildOnce {
        old: Entity,
        replacement: TilePos,
        substance: SubstanceId,
        rebuild_now: bool,
    }

    fn rebuild_before_perception(
        mut commands: Commands,
        mut rebuild: ResMut<RebuildOnce>,
        tiles: Query<(Entity, &TilePos, &RunBottom), With<HexTile>>,
        mut terrain: ResMut<TerrainOccupancy>,
    ) {
        if !rebuild.rebuild_now {
            return;
        }
        let mut runs = tiles
            .iter()
            .filter(|(entity, _, _)| *entity != rebuild.old)
            .map(|(_, &top, &bottom)| (top, bottom))
            .collect::<Vec<_>>();
        runs.push((rebuild.replacement, RunBottom(rebuild.replacement.level)));
        *terrain = TerrainOccupancy::from_runs(runs).expect("replacement run must be valid");
        commands.entity(rebuild.old).despawn();
        commands.spawn((
            HexTile,
            rebuild.replacement,
            RunBottom(rebuild.replacement.level),
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
            rebuild_before_perception
                .before(detect_perception_input_changes)
                .before(PerceptionSystems::ResolveIllumination),
        );

        enter(&mut app, Screen::Gameplay);
        let before = *app.world().resource::<PerceptionRuntimeStats>();
        app.world_mut().resource_mut::<RebuildOnce>().rebuild_now = true;
        app.update();
        let after = *app.world().resource::<PerceptionRuntimeStats>();

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
        assert_eq!(after.surface_rebuilds, before.surface_rebuilds + 1);
        assert_eq!(
            after.illumination_resolutions,
            before.illumination_resolutions + 1
        );
        assert_eq!(
            after.observation_resolutions,
            before.observation_resolutions + 1
        );
        assert_eq!(
            after.knowledge_publications,
            before.knowledge_publications + 1
        );

        app.update();
        let idle = *app.world().resource::<PerceptionRuntimeStats>();
        assert_eq!(idle.frames_checked, after.frames_checked + 1);
        assert_eq!(idle.surface_rebuilds, after.surface_rebuilds);
        assert_eq!(
            idle.illumination_resolutions,
            after.illumination_resolutions
        );
        assert_eq!(idle.observation_resolutions, after.observation_resolutions);
        assert_eq!(idle.knowledge_publications, after.knowledge_publications);
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
    fn one_hundred_gameplay_cycles_leave_exact_perception_state() {
        let (mut app, substances) = runtime_app(IlluminationLevel::Bright);
        let player = pos(0, 0, 5);
        let adjacent = pos(1, 0, 5);
        spawn_tile(&mut app, player, substances.stone, 2);
        spawn_tile(&mut app, adjacent, substances.stone, 2);
        spawn_unit(&mut app, 0, Faction::Player, player);
        let object_voxel = pos(4, -2, 9);
        app.world_mut()
            .spawn(AuthoredObjectVoxelRuns::new([AuthoredObjectVoxelRun::new(
                object_voxel,
                7,
            )]));
        let expected_entities = app.world().entities().len();

        for cycle in 0..100 {
            enter(&mut app, Screen::Gameplay);
            assert_eq!(
                app.world().entities().len(),
                expected_entities,
                "perception spawned or leaked an entity on gameplay cycle {cycle}"
            );
            assert_eq!(
                app.world().resource::<SurfaceSnapshots>().len(),
                2,
                "cycle {cycle} did not rebuild the exact surface projection"
            );
            assert!(app.world().contains_resource::<ResolvedIllumination>());
            assert!(app.world().contains_resource::<FactionObservations>());
            assert!(app.world().contains_resource::<FactionMapKnowledge>());
            assert!(app.world().contains_resource::<LocalMapKnowledge>());
            assert!(app
                .world()
                .resource::<AuthoredObjectOccupancy>()
                .contains(object_voxel));

            enter(&mut app, Screen::Title);
            assert_eq!(
                app.world().entities().len(),
                expected_entities,
                "perception leaked an entity while leaving gameplay cycle {cycle}"
            );
            assert!(!app.world().contains_resource::<PerceptionFrame>());
            assert!(!app.world().contains_resource::<SurfaceSnapshots>());
            assert!(!app.world().contains_resource::<ResolvedIllumination>());
            assert!(!app.world().contains_resource::<FactionObservations>());
            assert!(!app.world().contains_resource::<FactionMapKnowledge>());
            assert!(!app.world().contains_resource::<LocalMapKnowledge>());
            assert!(!app.world().contains_resource::<AuthoredObjectOccupancy>());
            assert_eq!(
                *app.world().resource::<PerceptionRuntimeStats>(),
                PerceptionRuntimeStats::default()
            );
            assert_eq!(
                *app.world().resource::<PerceptionInvalidation>(),
                PerceptionInvalidation::all()
            );
        }
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
                provides_sight: true,
            },
            ObservedUnit {
                id: UnitId(1),
                faction: Faction::Hostile,
                pos: TilePos::new(HexCoord::from_axial(40, 0), 15),
                provides_sight: true,
            },
        ];
        let terrain = TerrainOccupancy::from_runs(
            snapshots
                .iter()
                .map(|(position, _)| (position, RunBottom(position.level))),
        )
        .expect("flat radius-40 occupancy");
        let authored_objects = AuthoredObjectOccupancy::default();
        let mut samples = Vec::new();
        for _ in 0..12 {
            let started = Instant::now();
            let illumination = ResolvedIllumination::from_surfaces(&snapshots, ambient, &[])
                .expect("benchmark illumination");
            let mut knowledge = FactionMapKnowledge::new();
            let observations = resolve_observations_with_authored_objects(
                units,
                &illumination,
                &knowledge,
                ambient,
                &[],
                SightProfile::DEFAULT,
                &terrain,
                &authored_objects,
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

    #[test]
    #[ignore = "manual dense-wall, six-observer perception recomputation benchmark"]
    fn radius_40_dense_walls_and_six_observers_benchmark() {
        let floor_level = 15;
        let wall_level = 18;
        let surface_level = |coord: HexCoord| {
            if coord.x().rem_euclid(4) == 0 {
                wall_level
            } else {
                floor_level
            }
        };
        let snapshots = SurfaceSnapshots::try_from_iter(
            HexCoord::ORIGIN.within_radius(40).into_iter().map(|coord| {
                let level = surface_level(coord);
                let position = TilePos::new(coord, level);
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
        .expect("radius-40 dense-wall surfaces");
        let terrain = TerrainOccupancy::from_runs(
            snapshots
                .iter()
                .map(|(position, _)| (position, RunBottom(floor_level))),
        )
        .expect("dense wall occupancy");
        let observer_coords = [
            HexCoord::from_axial(-1, 0),
            HexCoord::from_axial(-1, 1),
            HexCoord::from_axial(-1, 2),
            HexCoord::from_axial(1, 0),
            HexCoord::from_axial(1, -1),
            HexCoord::from_axial(1, -2),
        ];
        let units = observer_coords
            .into_iter()
            .enumerate()
            .map(|(index, coord)| ObservedUnit {
                id: UnitId(u64::try_from(index).expect("six observer indices fit u64")),
                faction: Faction::Player,
                pos: TilePos::new(coord, surface_level(coord)),
                provides_sight: true,
            });
        let units = units.collect::<Vec<_>>();
        let ambient = ExteriorIllumination::new(IlluminationLevel::Bright);
        let illumination = ResolvedIllumination::from_surfaces(&snapshots, ambient, &[])
            .expect("dense-wall illumination");
        let authored_objects = AuthoredObjectOccupancy::default();
        let mut samples = Vec::new();
        for _ in 0..12 {
            let started = Instant::now();
            let mut knowledge = FactionMapKnowledge::new();
            let observations = resolve_observations_with_authored_objects(
                units.iter().copied(),
                &illumination,
                &knowledge,
                ambient,
                &[],
                SightProfile::DEFAULT,
                &terrain,
                &authored_objects,
            )
            .expect("dense-wall observations");
            apply_observations(&mut knowledge, &snapshots, &observations);
            std::hint::black_box((observations, knowledge));
            samples.push(started.elapsed());
        }
        samples.sort_unstable();
        let median = samples
            .get(samples.len() / 2)
            .copied()
            .expect("twelve samples");
        let p95 = samples.last().copied().expect("twelve samples");
        eprintln!("radius-40 dense-wall six-observer recomputation: median={median:?} p95={p95:?}");
        let budget = if cfg!(debug_assertions) {
            Duration::from_millis(750)
        } else {
            Duration::from_millis(150)
        };
        assert!(median < budget && p95 < budget);
    }
}
