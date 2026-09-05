//! Player-facing tactical shroud derived from authoritative faction observation.
//!
//! The terrain itself remains the current, pickable map. This adapter adds a dark
//! presentation cap to every surface the player faction does not currently observe.
//! Caps are disconnected geometry batched by resident chunk and cutaway owner. The
//! adapter contributes only the composable [`PresentationOcclusionReason::Fog`]
//! reason to hidden hostile roots. Neither path feeds renderer state back into gameplay.

use std::collections::{BTreeMap, BTreeSet};

use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::light::NotShadowCaster;
use bevy::mesh::Indices;
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use hex_core::config::{HEX_CIRCUMRADIUS, HEX_SMALL_DIAMETER};
use hex_core::{
    CutawayOccluder, Headroom, HexSpan, HexTile, InteriorRegionId, KnowledgeState,
    PerceptionSystems, PresentationOcclusion, PresentationOcclusionReason, Screen, TilePos, UnitId,
};
use hex_map::terrain_chunk_key;
use hex_perception::FactionMapKnowledge;
use hex_units::{Enemy, Faction};

pub(super) const FOG_CAP_THICKNESS: f32 = 0.02;
pub(super) const FOG_CAP_INSET: f32 = 0.84;
pub(super) const FOG_CAP_LIFT: f32 = 0.08;
pub(super) const FOG_CAP_DEPTH_BIAS: f32 = 8.0;
const FOG_CAP_COLOR: Color = Color::srgba(0.07, 0.09, 0.18, 0.84);
#[cfg(any(feature = "map-review", test))]
const DIMMED_FOG_CAP_COLOR: Color = Color::srgba(0.09, 0.095, 0.10, 0.58);
#[cfg(any(feature = "map-review", test))]
const OBSERVED_ONLY_FOG_CAP_COLOR: Color = Color::srgba(0.004, 0.006, 0.012, 0.985);
#[cfg(any(feature = "map-review", test))]
const SOFTENED_NEAR_FOG_CAP_COLOR: Color = Color::srgba(0.07, 0.09, 0.18, 0.30);
#[cfg(any(feature = "map-review", test))]
const SOFTENED_FAR_FOG_CAP_COLOR: Color = Color::srgba(0.07, 0.09, 0.18, 0.56);

/// Development/review terrain-shroud treatment.
///
/// Ordinary game builds always retain [`Self::Current`]. The map-review adapter may
/// replace this resource before gameplay starts. Development exploration temporarily
/// disables terrain shading while its pawn is outside tactical perception. Every mode
/// leaves hostile [`PresentationOcclusionReason::Fog`] ownership unchanged.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum FogPresentationMode {
    #[default]
    Current,
    #[cfg(any(feature = "dev", feature = "map-review", test))]
    NoTerrainShading,
    #[cfg(any(feature = "map-review", test))]
    Dimmed,
    #[cfg(any(feature = "map-review", test))]
    ObservedOnlyApproximation,
    #[cfg(any(feature = "map-review", test))]
    SoftenedTwoBand,
}

impl FogPresentationMode {
    #[cfg(feature = "map-review")]
    pub(super) fn parse_review(value: &str) -> Result<Self, String> {
        match value {
            "current" => Ok(Self::Current),
            "none" => Ok(Self::NoTerrainShading),
            "dimmed" => Ok(Self::Dimmed),
            "observed-only" => Ok(Self::ObservedOnlyApproximation),
            "softened" => Ok(Self::SoftenedTwoBand),
            _ => Err(format!(
                "must be current, none, dimmed, observed-only, or softened; got {value:?}"
            )),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FogOverlayBand {
    #[default]
    Core,
    #[cfg(any(feature = "map-review", test))]
    FarBoundary,
    #[cfg(any(feature = "map-review", test))]
    NearBoundary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FogBatchKey {
    chunk_q: i32,
    chunk_r: i32,
    cutaway: Option<InteriorRegionId>,
    band: FogOverlayBand,
}

#[derive(Component, Debug, Clone, PartialEq)]
struct FogOverlayBatch {
    key: FogBatchKey,
    positions: Vec<TilePos>,
    spans: Vec<HexSpan>,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct FogBatchProjection {
    positions: Vec<TilePos>,
    spans: Vec<HexSpan>,
}

impl FogBatchProjection {
    fn push(&mut self, position: TilePos, span: HexSpan) {
        self.positions.push(position);
        self.spans.push(span);
    }
}

#[derive(Resource, Debug, Default)]
struct FogPresentationState {
    material_mode: Option<FogPresentationMode>,
    materials: BTreeMap<FogOverlayBand, Handle<StandardMaterial>>,
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
        &'static mut FogOverlayBatch,
        &'static Mesh3d,
        &'static mut MeshMaterial3d<StandardMaterial>,
        Option<&'static CutawayOccluder>,
    ),
>;

#[derive(SystemParam)]
struct RemovedTileProjection<'w, 's> {
    tiles: RemovedComponents<'w, 's, HexTile>,
    positions: RemovedComponents<'w, 's, TilePos>,
    spans: RemovedComponents<'w, 's, HexSpan>,
    headroom: RemovedComponents<'w, 's, Headroom>,
    cutaways: RemovedComponents<'w, 's, CutawayOccluder>,
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<FogPresentationMode>()
        .init_resource::<FogPresentationState>()
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
    mode: Res<FogPresentationMode>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
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
    mut removed: RemovedTileProjection,
    mut overlays: OverlayQuery,
    added_hostiles: Query<(), Added<Enemy>>,
    mut hostiles: Query<(&UnitId, &mut PresentationOcclusion), With<Enemy>>,
    mut state: ResMut<FogPresentationState>,
) {
    let tiles_removed = removed.tiles.read().count() != 0;
    let mut positions_removed = false;
    for entity in removed.positions.read() {
        positions_removed |= tile_entities.contains(entity);
    }
    let mut spans_removed = false;
    for entity in removed.spans.read() {
        spans_removed |= tile_entities.contains(entity);
    }
    let mut headroom_removed = false;
    for entity in removed.headroom.read() {
        headroom_removed |= tile_entities.contains(entity);
    }
    let mut cutaways_removed = false;
    for entity in removed.cutaways.read() {
        cutaways_removed |= tile_entities.contains(entity);
    }
    let tile_projection_removed =
        positions_removed || spans_removed || headroom_removed || cutaways_removed;
    let knowledge_changed = knowledge
        .as_ref()
        .is_none_or(|knowledge| knowledge.is_changed());
    let inputs_changed = !state.initialized
        || mode.is_changed()
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

    let (Some(meshes), Some(materials)) = (meshes.as_mut(), materials.as_mut()) else {
        // Unit concealment remains fail-closed even if renderer assets are not ready.
        return;
    };

    if state.material_mode != Some(*mode) {
        for material in state.materials.values() {
            drop(materials.remove(material.id()));
        }
        state.materials.clear();
        state.material_mode = Some(*mode);
    }

    let surfaces = collect_current_surfaces(&tiles);
    let desired = desired_shaded_surfaces(knowledge.as_deref(), surfaces.keys().copied());
    let desired_bands = desired_fog_bands(*mode, &desired, surfaces.keys().copied());
    let desired_batches = batch_shaded_surfaces(&desired_bands, &surfaces);
    let desired_materials = desired_batches
        .keys()
        .map(|key| key.band)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|band| {
            let material = state
                .materials
                .entry(band)
                .or_insert_with(|| materials.add(fog_material(*mode, band)))
                .clone();
            (band, material)
        })
        .collect::<BTreeMap<_, _>>();
    let mut existing = BTreeMap::new();
    for (entity, mut batch, mesh, mut material, cutaway) in &mut overlays {
        let Some(projection) = desired_batches.get(&batch.key) else {
            drop(meshes.remove(mesh.0.id()));
            commands.entity(entity).despawn();
            continue;
        };
        if existing.insert(batch.key, entity).is_some() {
            drop(meshes.remove(mesh.0.id()));
            commands.entity(entity).despawn();
            continue;
        }

        if batch.positions != projection.positions || batch.spans != projection.spans {
            let replacement = meshes.add(fog_batch_mesh(projection));
            drop(meshes.remove(mesh.0.id()));
            batch.positions.clone_from(&projection.positions);
            batch.spans.clone_from(&projection.spans);
            commands.entity(entity).insert(Mesh3d(replacement));
        }
        let Some(desired_material) = desired_materials.get(&batch.key.band) else {
            drop(meshes.remove(mesh.0.id()));
            commands.entity(entity).despawn();
            continue;
        };
        if material.0 != *desired_material {
            material.0 = desired_material.clone();
        }
        reconcile_cutaway(
            &mut commands,
            entity,
            cutaway.copied(),
            batch.key.cutaway.map(CutawayOccluder),
        );
    }

    for (key, projection) in desired_batches {
        if existing.contains_key(&key) {
            continue;
        }
        let Some(material) = desired_materials.get(&key.band).cloned() else {
            continue;
        };
        let mesh = meshes.add(fog_batch_mesh(&projection));
        let mut overlay = commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material.clone()),
            Transform::default(),
            Visibility::default(),
            Pickable::IGNORE,
            NotShadowCaster,
            PresentationOcclusion::default(),
            FogOverlayBatch {
                key,
                positions: projection.positions,
                spans: projection.spans,
            },
            Name::new(format!("FogOverlayBatch[{},{}]", key.chunk_q, key.chunk_r)),
        ));
        if let Some(cutaway) = key.cutaway {
            overlay.insert(CutawayOccluder(cutaway));
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

fn desired_fog_bands(
    _mode: FogPresentationMode,
    desired: &BTreeSet<TilePos>,
    _surfaces: impl IntoIterator<Item = TilePos>,
) -> BTreeMap<TilePos, FogOverlayBand> {
    #[cfg(any(feature = "dev", feature = "map-review", test))]
    if _mode == FogPresentationMode::NoTerrainShading {
        return BTreeMap::new();
    }
    #[cfg(any(feature = "map-review", test))]
    {
        if _mode == FogPresentationMode::SoftenedTwoBand {
            // This is intentionally a presentation-space approximation over exposed
            // surfaces. Stacked surfaces at one horizontal coordinate share a boundary
            // distance, which prevents a bridge deck and the ground beneath it from
            // producing contradictory rings in the same rendered column.
            let observed_coords = _surfaces
                .into_iter()
                .filter(|position| !desired.contains(position))
                .map(|position| position.coord)
                .collect::<BTreeSet<_>>();
            return desired
                .iter()
                .copied()
                .map(|position| {
                    let neighbours = position.coord.neighbors();
                    let band = if neighbours
                        .iter()
                        .any(|neighbour| observed_coords.contains(neighbour))
                    {
                        FogOverlayBand::NearBoundary
                    } else if neighbours.iter().any(|neighbour| {
                        neighbour
                            .neighbors()
                            .iter()
                            .any(|second| observed_coords.contains(second))
                    }) {
                        FogOverlayBand::FarBoundary
                    } else {
                        FogOverlayBand::Core
                    };
                    (position, band)
                })
                .collect();
        }
    }
    desired
        .iter()
        .copied()
        .map(|position| (position, FogOverlayBand::Core))
        .collect()
}

fn batch_shaded_surfaces(
    desired: &BTreeMap<TilePos, FogOverlayBand>,
    surfaces: &BTreeMap<TilePos, CurrentSurface>,
) -> BTreeMap<FogBatchKey, FogBatchProjection> {
    let mut batches = BTreeMap::<FogBatchKey, FogBatchProjection>::new();
    for (&position, &band) in desired {
        let Some(surface) = surfaces.get(&position) else {
            continue;
        };
        let (chunk_q, chunk_r) = terrain_chunk_key(position.coord);
        batches
            .entry(FogBatchKey {
                chunk_q,
                chunk_r,
                cutaway: surface.cutaway.map(|cutaway| cutaway.0),
                band,
            })
            .or_default()
            .push(position, surface.span);
    }
    batches
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

fn fog_batch_mesh(projection: &FogBatchProjection) -> Mesh {
    debug_assert_eq!(projection.positions.len(), projection.spans.len());
    let inradius = 0.5 * HEX_SMALL_DIAMETER;
    let corners = [
        Vec3::new(0.0, 0.0, -HEX_CIRCUMRADIUS),
        Vec3::new(-inradius, 0.0, -0.5 * HEX_CIRCUMRADIUS),
        Vec3::new(-inradius, 0.0, 0.5 * HEX_CIRCUMRADIUS),
        Vec3::new(0.0, 0.0, HEX_CIRCUMRADIUS),
        Vec3::new(inradius, 0.0, 0.5 * HEX_CIRCUMRADIUS),
        Vec3::new(inradius, 0.0, -0.5 * HEX_CIRCUMRADIUS),
    ];
    let mut local_positions = Vec::with_capacity(14);
    local_positions.push(Vec3::new(0.0, 0.5, 0.0));
    local_positions.extend(corners.map(|corner| corner + Vec3::Y * 0.5));
    local_positions.push(Vec3::new(0.0, -0.5, 0.0));
    local_positions.extend(corners.map(|corner| corner - Vec3::Y * 0.5));

    let mut local_indices = vec![0_u32, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 5, 0, 5, 6, 0, 6, 1];
    for index in 0_u32..6 {
        let next = (index + 1) % 6;
        let top = 1 + index;
        let top_next = 1 + next;
        let bottom = 8 + index;
        let bottom_next = 8 + next;
        local_indices.extend([7, bottom_next, bottom]);
        local_indices.extend([top, bottom, bottom_next, top, bottom_next, top_next]);
    }
    let mut vertices = Vec::with_capacity(
        projection
            .positions
            .len()
            .saturating_mul(local_positions.len()),
    );
    let mut normals = Vec::with_capacity(vertices.capacity());
    let mut indices = Vec::with_capacity(
        projection
            .positions
            .len()
            .saturating_mul(local_indices.len()),
    );

    for (&position, &span) in projection.positions.iter().zip(&projection.spans) {
        let Ok(base) = u32::try_from(vertices.len()) else {
            break;
        };
        let transform = fog_transform(position, span);
        vertices.extend(
            local_positions
                .iter()
                .copied()
                .map(|point| transform.transform_point(point).to_array()),
        );
        normals.extend(std::iter::repeat_n(
            Vec3::Y.to_array(),
            local_positions.len(),
        ));
        indices.extend(local_indices.iter().copied().map(|index| base + index));
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
}

fn fog_material(mode: FogPresentationMode, band: FogOverlayBand) -> StandardMaterial {
    let base_color = match (mode, band) {
        (FogPresentationMode::Current, _) => FOG_CAP_COLOR,
        #[cfg(any(feature = "map-review", test))]
        (FogPresentationMode::SoftenedTwoBand, FogOverlayBand::Core) => FOG_CAP_COLOR,
        #[cfg(any(feature = "dev", feature = "map-review", test))]
        (FogPresentationMode::NoTerrainShading, _) => Color::NONE,
        #[cfg(any(feature = "map-review", test))]
        (FogPresentationMode::Dimmed, _) => DIMMED_FOG_CAP_COLOR,
        #[cfg(any(feature = "map-review", test))]
        (FogPresentationMode::ObservedOnlyApproximation, _) => OBSERVED_ONLY_FOG_CAP_COLOR,
        #[cfg(any(feature = "map-review", test))]
        (FogPresentationMode::SoftenedTwoBand, FogOverlayBand::NearBoundary) => {
            SOFTENED_NEAR_FOG_CAP_COLOR
        }
        #[cfg(any(feature = "map-review", test))]
        (FogPresentationMode::SoftenedTwoBand, FogOverlayBand::FarBoundary) => {
            SOFTENED_FAR_FOG_CAP_COLOR
        }
    };
    StandardMaterial {
        // Keep a dark tactical shroud while bounding the presentation floor over
        // real canyon shadows. The previous 72%-black composite could drive
        // shadowed water to near-void RGB values even though its geometry was
        // intact; this more opaque blue floor compresses that contrast without
        // changing observation, lighting, or the underlying PBR surface.
        base_color,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        depth_bias: FOG_CAP_DEPTH_BIAS,
        ..default()
    }
}

fn clear_fog_presentation(
    mut commands: Commands,
    overlays: Query<(Entity, &Mesh3d), With<FogOverlayBatch>>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut occlusions: Query<&mut PresentationOcclusion>,
    mut state: ResMut<FogPresentationState>,
) {
    for (entity, mesh) in &overlays {
        if let Some(meshes) = meshes.as_mut() {
            drop(meshes.remove(mesh.0.id()));
        }
        commands.entity(entity).despawn();
    }
    for mut occlusion in &mut occlusions {
        occlusion.remove(PresentationOcclusionReason::Fog);
    }
    state.initialized = false;
}

/// Exact fog-cap positions exposed only to crate-owned composition tests.
#[cfg(all(test, feature = "test-support"))]
pub(crate) fn fog_overlay_positions(world: &mut World) -> BTreeSet<TilePos> {
    let mut overlays = world.query::<&FogOverlayBatch>();
    overlays
        .iter(world)
        .flat_map(|overlay| overlay.positions.iter().copied())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use hex_assets::ObjectInstance;
    use hex_core::{
        AuthoredObjectVoxelRuns, ExactGridPoint, HexCoord, SubstanceId, TraversalProfile,
    };
    use hex_perception::{
        apply_observations, FactionObservation, FactionObservations, ObservedUnit, SurfaceSnapshot,
        SurfaceSnapshots,
    };
    use hex_test_support::TestAppBuilder;
    use hex_units::{
        authored_object_sight_segment_is_clear, AuthoredObjectOccupancy, Body, Downed, Player,
        Standing, StandsOn,
    };

    use crate::scenarios::tests::{
        crystal_heart_blocked_sight_pair, crystal_heart_occupancy_snapshot, enter_screen,
        procedural_gameplay_app_with_combat,
    };

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
        app.init_resource::<FogPresentationMode>()
            .init_resource::<FogPresentationState>()
            .insert_resource(Assets::<Mesh>::default())
            .insert_resource(Assets::<StandardMaterial>::default())
            .add_systems(Update, (clear_removed_hostile_fog, reconcile_fog));
        if let Some(knowledge) = knowledge {
            app.insert_resource(knowledge);
        }
        app
    }

    fn fog_state_app(knowledge: Option<FactionMapKnowledge>) -> App {
        let mut builder = TestAppBuilder::new().with_fixed_step(Duration::ZERO);
        let app = builder.app_mut();
        app.init_resource::<FogPresentationMode>()
            .init_resource::<FogPresentationState>()
            .insert_resource(Assets::<Mesh>::default())
            .insert_resource(Assets::<StandardMaterial>::default());
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
        let mut query = world.query::<&FogOverlayBatch>();
        query
            .iter(world)
            .map(|overlay| overlay.positions.len())
            .sum()
    }

    fn overlay_batch_count(app: &mut App) -> usize {
        let world = app.world_mut();
        let mut query = world.query::<&FogOverlayBatch>();
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
        assert_eq!(FogPresentationMode::default(), FogPresentationMode::Current);
        let material = fog_material(FogPresentationMode::Current, FogOverlayBand::Core);
        assert!(material.unlit);
        assert_eq!(material.alpha_mode, AlphaMode::Blend);
        assert_eq!(material.base_color, FOG_CAP_COLOR);
        assert!((material.depth_bias - FOG_CAP_DEPTH_BIAS).abs() < f32::EPSILON);
        let transform = fog_transform(pos(3), HexSpan::new(1.0, 2.0));
        assert!((transform.scale.x - FOG_CAP_INSET).abs() < f32::EPSILON);
        assert!((transform.scale.y - FOG_CAP_THICKNESS).abs() < f32::EPSILON);
        assert!(
            (transform.translation.y - (2.0 + FOG_CAP_LIFT + FOG_CAP_THICKNESS * 0.5)).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn review_fog_materials_are_exact_and_softened_bands_are_deterministic() {
        assert_eq!(
            fog_material(FogPresentationMode::NoTerrainShading, FogOverlayBand::Core).base_color,
            Color::NONE
        );
        assert_eq!(
            fog_material(FogPresentationMode::Dimmed, FogOverlayBand::Core).base_color,
            DIMMED_FOG_CAP_COLOR
        );
        assert_eq!(
            fog_material(
                FogPresentationMode::ObservedOnlyApproximation,
                FogOverlayBand::Core,
            )
            .base_color,
            OBSERVED_ONLY_FOG_CAP_COLOR
        );
        assert_eq!(
            fog_material(
                FogPresentationMode::SoftenedTwoBand,
                FogOverlayBand::NearBoundary,
            )
            .base_color,
            SOFTENED_NEAR_FOG_CAP_COLOR
        );
        assert_eq!(
            fog_material(
                FogPresentationMode::SoftenedTwoBand,
                FogOverlayBand::FarBoundary,
            )
            .base_color,
            SOFTENED_FAR_FOG_CAP_COLOR
        );

        let observed = pos(0);
        let near = pos(1);
        let far = pos(2);
        let core = pos(3);
        assert_eq!(
            desired_fog_bands(
                FogPresentationMode::SoftenedTwoBand,
                &BTreeSet::from([near, far, core]),
                [observed, near, far, core],
            ),
            BTreeMap::from([
                (near, FogOverlayBand::NearBoundary),
                (far, FogOverlayBand::FarBoundary),
                (core, FogOverlayBand::Core),
            ])
        );
    }

    #[test]
    fn no_terrain_shading_still_conceals_an_unobserved_hostile() {
        let position = pos(0);
        let mut app = fog_app(None);
        app.insert_resource(FogPresentationMode::NoTerrainShading);
        spawn_surface(&mut app, position, None);
        let hostile = app
            .world_mut()
            .spawn((Enemy, UnitId(7), PresentationOcclusion::default()))
            .id();

        app.update();

        assert_eq!(overlay_count(&mut app), 0);
        assert!(app
            .world()
            .get::<PresentationOcclusion>(hostile)
            .expect("hostile presentation state")
            .contains(PresentationOcclusionReason::Fog));
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
                &FogOverlayBatch,
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
                            overlay.positions.clone(),
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
        assert!(overlays.iter().any(|(_, positions, _, _, marker, _)| {
            positions.as_slice() == [first] && *marker == Some(cutaway)
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

        let positions = {
            let world = app.world_mut();
            let mut query = world.query::<(&FogOverlayBatch, Option<&CutawayOccluder>)>();
            query
                .iter(world)
                .flat_map(|(overlay, cutaway)| {
                    overlay
                        .positions
                        .iter()
                        .copied()
                        .map(move |position| (position, cutaway.copied()))
                })
                .collect::<Vec<_>>()
        };
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
        assert_eq!(overlay_batch_count(&mut app), 1);
        assert_eq!(
            app.world()
                .resource::<FogPresentationState>()
                .reconciliations,
            reconciliations,
            "all replacement removal cursors must be drained in the rebuild frame"
        );
    }

    #[test]
    fn fog_caps_batch_by_euclidean_chunk_and_exact_cutaway_region() {
        let cutaway = CutawayOccluder(InteriorRegionId(9));
        let plain = [-17, -16, -1, 0, 15, 16]
            .into_iter()
            .map(pos)
            .collect::<BTreeSet<_>>();
        let cutaway_position = pos(-15);
        let mut app = fog_app(Some(FactionMapKnowledge::new()));
        for &position in &plain {
            spawn_surface(&mut app, position, None);
        }
        spawn_surface(&mut app, cutaway_position, Some(cutaway));

        app.update();

        let (keys, positions, material_handles, pickable_and_shadow) = {
            let world = app.world_mut();
            let mut query = world.query::<(
                &FogOverlayBatch,
                &MeshMaterial3d<StandardMaterial>,
                &Pickable,
                Has<NotShadowCaster>,
                Option<&CutawayOccluder>,
            )>();
            let mut keys = BTreeSet::new();
            let mut positions = BTreeSet::new();
            let mut material_handles = BTreeSet::new();
            let mut pickable_and_shadow = Vec::new();
            for (batch, material, pickable, no_shadow, marker) in query.iter(world) {
                assert_eq!(batch.key.cutaway.map(CutawayOccluder), marker.copied());
                assert!(keys.insert(batch.key), "duplicate fog batch key");
                for &position in &batch.positions {
                    assert!(positions.insert(position), "duplicate logical fog cap");
                }
                material_handles.insert(material.0.id());
                pickable_and_shadow.push((*pickable, no_shadow));
            }
            (keys, positions, material_handles, pickable_and_shadow)
        };
        assert_eq!(
            keys,
            BTreeSet::from([
                FogBatchKey {
                    chunk_q: -2,
                    chunk_r: 0,
                    cutaway: None,
                    band: FogOverlayBand::Core,
                },
                FogBatchKey {
                    chunk_q: -1,
                    chunk_r: 0,
                    cutaway: None,
                    band: FogOverlayBand::Core,
                },
                FogBatchKey {
                    chunk_q: -1,
                    chunk_r: 0,
                    cutaway: Some(cutaway.0),
                    band: FogOverlayBand::Core,
                },
                FogBatchKey {
                    chunk_q: 0,
                    chunk_r: 0,
                    cutaway: None,
                    band: FogOverlayBand::Core,
                },
                FogBatchKey {
                    chunk_q: 1,
                    chunk_r: 0,
                    cutaway: None,
                    band: FogOverlayBand::Core,
                },
            ])
        );
        let mut expected = plain;
        expected.insert(cutaway_position);
        assert_eq!(positions, expected);
        assert_eq!(material_handles.len(), 1, "fog retains one shared material");
        assert!(pickable_and_shadow
            .iter()
            .all(|(pickable, no_shadow)| *pickable == Pickable::IGNORE && *no_shadow));

        app.world_mut()
            .run_system_once(clear_fog_presentation)
            .expect("fog teardown should run");
        assert_eq!(overlay_count(&mut app), 0);
        assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 0);
    }

    #[test]
    fn changed_surface_geometry_rebuilds_one_batch_then_returns_to_zero_idle_churn() {
        let position = pos(0);
        let mut app = fog_app(Some(FactionMapKnowledge::new()));
        let tile = spawn_surface(&mut app, position, None);
        app.update();

        let (batch_entity, first_mesh) = {
            let world = app.world_mut();
            let mut query = world.query::<(Entity, &Mesh3d, &FogOverlayBatch)>();
            let (entity, mesh, batch) = query.single(world).expect("one fog batch");
            assert_eq!(batch.positions, vec![position]);
            (entity, mesh.0.id())
        };
        app.world_mut()
            .entity_mut(tile)
            .insert(HexSpan::new(1.0, 5.0));
        app.update();

        let second_mesh = {
            let world = app.world_mut();
            let mut query = world.query::<(Entity, &Mesh3d, &FogOverlayBatch)>();
            let (entity, mesh, batch) = query.single(world).expect("one rebuilt fog batch");
            assert_eq!(
                entity, batch_entity,
                "stable batch keys retain their entity"
            );
            assert_eq!(batch.spans, vec![HexSpan::new(1.0, 5.0)]);
            mesh.0.id()
        };
        assert_ne!(second_mesh, first_mesh);
        assert_eq!(
            app.world().resource::<Assets<Mesh>>().len(),
            1,
            "replaced batch meshes must not leak"
        );
        let reconciliations = app
            .world()
            .resource::<FogPresentationState>()
            .reconciliations;

        app.update();

        assert_eq!(
            app.world()
                .resource::<FogPresentationState>()
                .reconciliations,
            reconciliations,
            "an unchanged frame must not rebuild or reconcile fog"
        );
        let unchanged_mesh = {
            let world = app.world_mut();
            let mut query = world.query::<&Mesh3d>();
            query.single(world).expect("one unchanged batch").0.id()
        };
        assert_eq!(unchanged_mesh, second_mesh);
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

    #[test]
    fn shipped_cathedral_heart_drives_seven_ray_knowledge_and_hostile_fog() {
        let mut app = procedural_gameplay_app_with_combat("Crystal Ascent", false);
        plugin(&mut app);
        enter_screen(&mut app, Screen::Gameplay);

        let (heart, heart_runs, expected_occupancy) = crystal_heart_occupancy_snapshot(&mut app);
        let (observer, target) = crystal_heart_blocked_sight_pair(&app, heart.origin());
        let (observer_span, target_span) = {
            let surfaces = app.world().resource::<SurfaceSnapshots>();
            (
                surfaces
                    .get(observer)
                    .expect("heart fixture observer should be an exposed surface")
                    .span,
                surfaces
                    .get(target)
                    .expect("heart fixture target should be an exposed surface")
                    .span,
            )
        };

        let player_entities = {
            let world = app.world_mut();
            let mut players = world.query_filtered::<Entity, (With<Player>, With<Body>)>();
            players.iter(world).collect::<Vec<_>>()
        };
        let (&observer_entity, remaining_players) = player_entities
            .split_first()
            .expect("Crystal Ascent should roster the standard party");
        for player in remaining_players {
            app.world_mut().entity_mut(*player).insert(Downed);
        }
        app.world_mut()
            .entity_mut(observer_entity)
            .remove::<Downed>()
            .insert(StandsOn(Standing {
                pos: observer,
                span: observer_span,
            }));

        let hostile_id = UnitId(9_999);
        let hostile = app
            .world_mut()
            .spawn((
                Enemy,
                hostile_id,
                Faction::Hostile,
                Body::new(TraversalProfile::WALKER),
                StandsOn(Standing {
                    pos: target,
                    span: target_span,
                }),
                Transform::default(),
                Visibility::default(),
                PresentationOcclusion::default(),
            ))
            .id();
        app.update();

        assert!(
            app.world()
                .resource::<FactionMapKnowledge>()
                .faction(Faction::Player)
                .unit(hostile_id)
                .is_none(),
            "the exact heart volume should withhold hostile identity"
        );
        assert_ne!(
            app.world()
                .resource::<FactionMapKnowledge>()
                .faction(Faction::Player)
                .state(target),
            KnowledgeState::Observed,
        );
        assert!(app
            .world()
            .get::<PresentationOcclusion>(hostile)
            .expect("hostile should carry composable presentation authority")
            .contains(PresentationOcclusionReason::Fog));
        assert!(
            app.world_mut()
                .query::<&FogOverlayBatch>()
                .iter(app.world())
                .any(|overlay| overlay.positions.contains(&target)),
            "the heart-obscured hostile surface should retain its shroud cap"
        );

        let heart_root = {
            let world = app.world_mut();
            let mut sources = world.query::<(Entity, &ObjectInstance, &AuthoredObjectVoxelRuns)>();
            sources
                .iter(world)
                .find_map(|(entity, instance, _)| {
                    (instance.object_id().as_str() == "prop/crystal-cathedral-heart")
                        .then_some(entity)
                })
                .expect("the shipped heart source should still be live")
        };
        app.world_mut()
            .entity_mut(heart_root)
            .remove::<AuthoredObjectVoxelRuns>();
        app.update();

        assert!(app.world().resource::<AuthoredObjectOccupancy>().is_empty());
        assert!(
            app.world()
                .resource::<FactionMapKnowledge>()
                .faction(Faction::Player)
                .unit(hostile_id)
                .is_some(),
            "withdrawing the heart volume should reveal the hostile in the same update"
        );
        assert_eq!(
            app.world()
                .resource::<FactionMapKnowledge>()
                .faction(Faction::Player)
                .state(target),
            KnowledgeState::Observed,
        );
        assert!(!app
            .world()
            .get::<PresentationOcclusion>(hostile)
            .expect("hostile should retain its presentation reason set")
            .contains(PresentationOcclusionReason::Fog));
        assert!(
            !app.world_mut()
                .query::<&FogOverlayBatch>()
                .iter(app.world())
                .any(|overlay| overlay.positions.contains(&target)),
            "revealing the target should remove its fog cap"
        );

        app.world_mut()
            .entity_mut(heart_root)
            .insert(heart_runs.clone());
        app.update();
        assert_eq!(
            app.world().resource::<AuthoredObjectOccupancy>(),
            &expected_occupancy,
            "restoring the exact source should rebuild the blueprint-derived volume"
        );
        assert!(app
            .world()
            .resource::<FactionMapKnowledge>()
            .faction(Faction::Player)
            .unit(hostile_id)
            .is_none());
        assert!(app
            .world()
            .get::<PresentationOcclusion>(hostile)
            .expect("hostile should retain its presentation reason set")
            .contains(PresentationOcclusionReason::Fog));

        let peak = heart_runs
            .iter()
            .max_by_key(|run| run.top.level)
            .expect("the cathedral heart should occupy at least one voxel")
            .top;
        let tangent_source = ExactGridPoint::voxel_top_center(TilePos::new(
            HexCoord::from_axial(peak.coord.x().saturating_sub(2), peak.coord.y()),
            peak.level,
        ));
        let tangent_target = ExactGridPoint::voxel_top_center(TilePos::new(
            HexCoord::from_axial(peak.coord.x().saturating_add(2), peak.coord.y()),
            peak.level,
        ));
        assert!(
            authored_object_sight_segment_is_clear(
                tangent_source,
                tangent_target,
                &expected_occupancy,
            ),
            "an exact tangent across the shipped heart's upper face must remain clear"
        );
        assert_eq!(
            authored_object_sight_segment_is_clear(
                tangent_source,
                tangent_target,
                &expected_occupancy,
            ),
            authored_object_sight_segment_is_clear(
                tangent_target,
                tangent_source,
                &expected_occupancy,
            ),
            "the shipped-heart tangency must remain direction symmetric"
        );
        assert!(
            !authored_object_sight_segment_is_clear(
                ExactGridPoint::voxel_center(TilePos::new(
                    HexCoord::from_axial(peak.coord.x().saturating_sub(2), peak.coord.y(),),
                    peak.level,
                )),
                ExactGridPoint::voxel_center(TilePos::new(
                    HexCoord::from_axial(peak.coord.x().saturating_add(2), peak.coord.y(),),
                    peak.level,
                )),
                &expected_occupancy,
            ),
            "lowering the same segment into the shipped heart interior must block"
        );
    }
}
