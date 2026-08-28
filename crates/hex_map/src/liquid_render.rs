//! Opaque, non-interactive presentation geometry for liquid voxel runs.
//!
//! The ordinary voxel prisms remain the authoritative volume, pick target, and
//! shadow caster. This module adds only chunk-batched biased horizontal caps for
//! exposed water or lava runs, combined vertical curtains for exposed liquid
//! height edges, and deterministic landing-splash geometry for semantic lava
//! falls.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use bevy::{
    asset::RenderAssetUsages,
    light::NotShadowCaster,
    material::OpaqueRendererMethod,
    mesh::Indices,
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, Face, PrimitiveTopology, ShaderType},
    shader::ShaderRef,
};
use hex_assets::{to_color, SubstanceTable};
use hex_core::config::{HEX_CIRCUMRADIUS, HEX_SMALL_DIAMETER};
use hex_core::{HexCoord, Level, PausableSystems, Screen, SubstanceId, TilePos};

use crate::procedural_v3::{FillMaterialRole, HexSide, LiquidFlowState, MapPresentationProjection};
use crate::voxel::{runs, terrain_chunk_coord, SubstanceRun, TerrainChunkCoord, VoxelMap};

const LIQUID_SHADER_PATH: &str = "shaders/liquid.wgsl";
const LIQUID_FOAM_SWATCH: &str = "liquid/foam";
const PHASE_WRAP_SECONDS: f32 = 400.0;
const FALL_FLOW_SPEED: f32 = 0.85;
const LAVA_STILL_PULSE_RATE: f32 = 0.05;
const LAVA_FALL_PULSE_RATE: f32 = 0.40;
/// Keeps broad, flat water caps from becoming a saturated planar mirror under
/// the noon directional light. Bevy maps this reflectance to roughly 2% F0,
/// matching ordinary water rather than the legacy 8.3% highlight.
const WATER_SURFACE_ROUGHNESS: f32 = 0.60;
const WATER_SURFACE_REFLECTANCE: f32 = 0.35;
const LEGACY_LIQUID_ROUGHNESS: f32 = 0.28;
const LEGACY_LIQUID_REFLECTANCE: f32 = 0.72;
#[cfg(test)]
const SECONDARY_WAVE_PHASE_RATE: f32 = 0.025;
const LIQUID_CAP_BIAS_RATIO: f32 = 0.02;
const LIQUID_CAP_BIAS_MAX: f32 = 0.002 * HEX_CIRCUMRADIUS;
/// Stable render ordering above the authoritative opaque voxel surface.
///
/// The physical lift remains deliberately tiny so the visual cap cannot alter
/// the apparent water level. A small pipeline bias keeps that cap in front at
/// Grand-map camera distances where depth precision is coarser than the lift.
const LIQUID_PRESENTATION_DEPTH_BIAS: f32 = 1.0;
const LIQUID_CURTAIN_EDGE_BIAS: f32 = 0.002 * HEX_CIRCUMRADIUS;
const LAVA_SPLASH_SURFACE_BIAS: f32 = 0.001 * HEX_CIRCUMRADIUS;
const HEX_INRADIUS: f32 = 0.5 * HEX_SMALL_DIAMETER;

/// Uniform shared by one visual liquid flow class.
#[derive(Clone, Copy, Debug, Reflect, ShaderType)]
struct LiquidMaterialParams {
    /// `xy`: UV velocity, `z`: visual phase, `w`: base UV scale.
    flow_phase_scale: Vec4,
    /// Highlight, foam, roughness reduction, and cross-wave frequency.
    modulation: Vec4,
    /// Base emission, pulse amplitude, pulse rate, and reserved future control.
    emission: Vec4,
    /// Canonical palette-backed foam colour in linear RGB.
    foam_color: Vec4,
}

/// PBR extension used only by map-owned liquid presentation geometry.
#[derive(Asset, AsBindGroup, Clone, Debug, Reflect)]
pub(crate) struct LiquidExtension {
    // StandardMaterial owns the low binding numbers. Bevy's extension examples
    // reserve binding 100 and above for extension uniforms.
    #[uniform(100)]
    params: LiquidMaterialParams,
}

impl MaterialExtension for LiquidExtension {
    fn fragment_shader() -> ShaderRef {
        LIQUID_SHADER_PATH.into()
    }
}

pub(crate) type LiquidMaterial = ExtendedMaterial<StandardMaterial, LiquidExtension>;

/// Session-only visual phase. It never affects topology or map fingerprints.
#[derive(Resource, Reflect, Clone, Debug, Default)]
#[reflect(Resource)]
pub struct LiquidVisualTime {
    phase_seconds: f32,
    frozen_phase_seconds: Option<f32>,
}

impl LiquidVisualTime {
    /// Builds a deterministically frozen clock, rejecting non-finite review input.
    #[must_use]
    pub fn frozen_at(phase_seconds: f32) -> Option<Self> {
        phase_seconds.is_finite().then(|| Self {
            phase_seconds: 0.0,
            frozen_phase_seconds: Some(wrap_phase(phase_seconds)),
        })
    }

    /// Freezes animation at a deterministic phase.
    ///
    /// Returns `false` without changing the current clock when the phase is not
    /// finite.
    pub fn freeze(&mut self, phase_seconds: f32) -> bool {
        if !phase_seconds.is_finite() {
            return false;
        }
        self.frozen_phase_seconds = Some(wrap_phase(phase_seconds));
        true
    }

    /// Returns animation control to the session clock.
    pub fn unfreeze(&mut self) {
        self.frozen_phase_seconds = None;
    }

    /// Current wrapped phase, including a frozen review override.
    #[must_use]
    pub fn phase_seconds(&self) -> f32 {
        self.frozen_phase_seconds.unwrap_or(self.phase_seconds)
    }

    /// Whether the phase is currently frozen for deterministic review.
    #[must_use]
    pub const fn is_frozen(&self) -> bool {
        self.frozen_phase_seconds.is_some()
    }

    fn advance(&mut self, delta_seconds: f32) -> f32 {
        if let Some(frozen) = self.frozen_phase_seconds {
            return frozen;
        }
        if !self.phase_seconds.is_finite() {
            self.phase_seconds = 0.0;
        }
        let delta = if delta_seconds.is_finite() {
            delta_seconds.max(0.0)
        } else {
            0.0
        };
        self.phase_seconds = wrap_phase(self.phase_seconds + delta);
        self.phase_seconds
    }

    fn reset_for_gameplay_entry(&mut self) {
        if self.frozen_phase_seconds.is_none() {
            self.phase_seconds = 0.0;
        }
    }
}

fn wrap_phase(phase_seconds: f32) -> f32 {
    phase_seconds.rem_euclid(PHASE_WRAP_SECONDS)
}

/// The bounded set of shared materials whose phase uniform changes each frame.
#[derive(Resource, Clone, Debug, Default)]
struct LiquidMaterialHandles {
    handles: Vec<Handle<LiquidMaterial>>,
}

/// Registers the material extension and its single visual-clock system.
pub(crate) fn plugin(app: &mut App) {
    app.add_plugins(MaterialPlugin::<LiquidMaterial>::default())
        .init_resource::<LiquidVisualTime>()
        .register_type::<LiquidVisualTime>()
        .add_systems(
            OnEnter(Screen::Gameplay),
            reset_liquid_visual_time_for_gameplay,
        )
        .add_systems(
            Update,
            advance_liquid_visual_time
                .in_set(PausableSystems)
                .run_if(in_state(Screen::Gameplay)),
        );
}

/// Removes presentation material ownership during map teardown.
pub(crate) fn clear_material_cache(commands: &mut Commands) {
    commands.remove_resource::<LiquidMaterialHandles>();
}

fn advance_liquid_visual_time(
    time: Res<Time>,
    mut visual_time: ResMut<LiquidVisualTime>,
    handles: Option<Res<LiquidMaterialHandles>>,
    mut materials: ResMut<Assets<LiquidMaterial>>,
) {
    let phase = visual_time.advance(time.delta_secs());
    let Some(handles) = handles else {
        return;
    };
    for handle in &handles.handles {
        if let Some(mut material) = materials.get_mut(handle) {
            material.extension.params.flow_phase_scale.z = phase;
        }
    }
}

fn reset_liquid_visual_time_for_gameplay(mut visual_time: ResMut<LiquidVisualTime>) {
    visual_time.reset_for_gameplay_entry();
}

/// A renderer-only disagreement between voxels and retained liquid semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LiquidPresentationError {
    InvalidLevelHeight,
    MissingSubstance {
        role: FillMaterialRole,
    },
    MissingPaletteSwatch {
        swatch: &'static str,
    },
    AmbiguousSubstances {
        id: SubstanceId,
    },
    MissingProjectionVoxel {
        position: TilePos,
    },
    OrphanProjectionVoxel {
        position: TilePos,
    },
    ProjectionMaterialMismatch {
        position: TilePos,
        expected: FillMaterialRole,
        actual: FillMaterialRole,
    },
    InconsistentRunProjection {
        top: TilePos,
    },
    MovingFlowWithoutDownstream {
        source: TilePos,
        flow: LiquidFlowState,
    },
    NonAdjacentDownstream {
        source: TilePos,
        downstream: TilePos,
    },
    MissingFallLanding {
        source: TilePos,
        downstream: TilePos,
    },
    FallMaterialMismatch {
        source: TilePos,
        downstream: TilePos,
    },
    ShallowFall {
        source: TilePos,
        downstream: TilePos,
    },
    MeshIndexOverflow,
    NonFiniteGeometry,
}

impl fmt::Display for LiquidPresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLevelHeight => {
                write!(
                    formatter,
                    "liquid presentation level height must be positive and finite"
                )
            }
            Self::MissingSubstance { role } => {
                write!(formatter, "liquid role {role:?} has no live substance")
            }
            Self::MissingPaletteSwatch { swatch } => {
                write!(
                    formatter,
                    "liquid presentation requires art-palette swatch '{swatch}'"
                )
            }
            Self::AmbiguousSubstances { id } => {
                write!(
                    formatter,
                    "water and lava resolve to the same substance id {id:?}"
                )
            }
            Self::MissingProjectionVoxel { position } => {
                write!(
                    formatter,
                    "V3 liquid projection omits occupied voxel {position:?}"
                )
            }
            Self::OrphanProjectionVoxel { position } => {
                write!(
                    formatter,
                    "V3 liquid projection contains non-liquid voxel {position:?}"
                )
            }
            Self::ProjectionMaterialMismatch {
                position,
                expected,
                actual,
            } => write!(
                formatter,
                "V3 liquid voxel {position:?} is {actual:?}, but live substance requires {expected:?}"
            ),
            Self::InconsistentRunProjection { top } => {
                write!(
                    formatter,
                    "liquid run ending at {top:?} has inconsistent V3 metadata"
                )
            }
            Self::MovingFlowWithoutDownstream { source, flow } => {
                write!(
                    formatter,
                    "{flow:?} liquid at {source:?} has no exact downstream"
                )
            }
            Self::NonAdjacentDownstream { source, downstream } => write!(
                formatter,
                "liquid at {source:?} points to non-adjacent downstream {downstream:?}"
            ),
            Self::MissingFallLanding { source, downstream } => write!(
                formatter,
                "fall at {source:?} has no exposed exact landing at {downstream:?}"
            ),
            Self::FallMaterialMismatch { source, downstream } => write!(
                formatter,
                "fall at {source:?} and landing {downstream:?} use different materials"
            ),
            Self::ShallowFall { source, downstream } => write!(
                formatter,
                "fall at {source:?} does not drop at least two levels to {downstream:?}"
            ),
            Self::MeshIndexOverflow => {
                write!(formatter, "liquid presentation mesh exceeds u32 indices")
            }
            Self::NonFiniteGeometry => write!(
                formatter,
                "liquid presentation produced non-finite geometry"
            ),
        }
    }
}

impl std::error::Error for LiquidPresentationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LiquidSurface {
    position: TilePos,
    role: FillMaterialRole,
    flow: LiquidFlowState,
    downstream: Option<TilePos>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct LiquidCapBatchKey {
    chunk: TerrainChunkCoord,
    role: FillMaterialRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct LiquidCurtainBatchKey {
    role: FillMaterialRole,
    style: MaterialStyle,
}

#[derive(Component, Debug, Clone, PartialEq, Eq)]
struct LiquidCapBatch {
    key: LiquidCapBatchKey,
    surfaces: Vec<LiquidSurface>,
}

#[derive(Debug)]
struct PresentationPlan {
    surfaces: Vec<LiquidSurface>,
    curtains: BTreeMap<LiquidCurtainBatchKey, RawMesh>,
    roles: BTreeSet<FillMaterialRole>,
}

/// Spawns non-pickable, chunk-batched liquid caps and combined side curtains.
///
/// Validation and mesh construction complete before commands or assets are
/// changed, so an error never leaves a partially spawned presentation.
pub(crate) fn spawn_presentations(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<LiquidMaterial>,
    map: &VoxelMap,
    table: &SubstanceTable,
    level_height: f32,
    phase_seconds: f32,
    projection: Option<&MapPresentationProjection>,
) -> Result<Vec<Entity>, LiquidPresentationError> {
    let plan = build_presentation_plan(map, table, level_height, projection)?;
    if plan.surfaces.is_empty() {
        clear_material_cache(commands);
        return Ok(Vec::new());
    }
    let cap_batches = batch_liquid_caps(&plan.surfaces)
        .into_iter()
        .map(|(key, surfaces)| {
            cap_batch_geometry(&surfaces, level_height).map(|geometry| (key, surfaces, geometry))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let role_colors = plan
        .roles
        .iter()
        .copied()
        .map(|role| role_color(role, table).map(|color| (role, color)))
        .collect::<Result<Vec<_>, _>>()?;
    let foam = table
        .palette_color(LIQUID_FOAM_SWATCH)
        .map(to_color)
        .ok_or(LiquidPresentationError::MissingPaletteSwatch {
            swatch: LIQUID_FOAM_SWATCH,
        })?;
    let mut material_sets = Vec::with_capacity(role_colors.len());
    let mut registered_handles = Vec::with_capacity(role_colors.len().saturating_mul(2));
    for (role, color) in role_colors {
        let set = MaterialSet::create(role, color, foam, phase_seconds, materials);
        set.extend_registry(&mut registered_handles);
        material_sets.push(set);
    }

    let mut entities = Vec::with_capacity(cap_batches.len().saturating_add(plan.curtains.len()));
    for (key, surfaces, geometry) in cap_batches {
        let mesh = meshes.add(geometry.into_mesh());
        let material = material_handle(&material_sets, key.role, MaterialStyle::Surface);
        let entity = commands
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::default(),
                Pickable::IGNORE,
                NotShadowCaster,
                LiquidCapBatch { key, surfaces },
                Name::new("LiquidCap"),
            ))
            .id();
        entities.push(entity);
    }

    for (key, geometry) in plan.curtains {
        let mesh = meshes.add(geometry.into_mesh());
        let material = material_handle(&material_sets, key.role, key.style);
        let name = match key.style {
            MaterialStyle::Surface => "LiquidSideCurtain",
            MaterialStyle::Fall => "LiquidFallCurtain",
        };
        let entity = commands
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::default(),
                Pickable::IGNORE,
                NotShadowCaster,
                Name::new(name),
            ))
            .id();
        entities.push(entity);
    }

    commands.insert_resource(LiquidMaterialHandles {
        handles: registered_handles,
    });
    Ok(entities)
}

fn build_presentation_plan(
    map: &VoxelMap,
    table: &SubstanceTable,
    level_height: f32,
    projection: Option<&MapPresentationProjection>,
) -> Result<PresentationPlan, LiquidPresentationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(LiquidPresentationError::InvalidLevelHeight);
    }
    let substances = LiquidSubstances::resolve(table)?;
    let mut consumed_projection = BTreeSet::new();
    let mut surfaces = Vec::new();
    let mut coordinates: Vec<_> = map.columns().collect();
    coordinates.sort_by_key(|(coord, _column)| *coord);

    for (coord, column) in coordinates {
        for run in runs(column) {
            let Some(role) = substances.role(run.substance) else {
                continue;
            };
            let descriptor =
                descriptor_for_run(coord, run, role, projection, &mut consumed_projection)?;
            if column.get(run.top).is_air() {
                surfaces.push(LiquidSurface {
                    position: TilePos::new(coord, run.top.saturating_sub(1)),
                    role,
                    flow: descriptor.flow,
                    downstream: descriptor.downstream,
                });
            }
        }
    }

    if let Some(projection) = projection {
        if let Some(position) = projection
            .liquids()
            .keys()
            .find(|position| !consumed_projection.contains(position))
        {
            return Err(LiquidPresentationError::OrphanProjectionVoxel {
                position: *position,
            });
        }
    }

    validate_surface_directions(&surfaces)?;
    let curtains = build_curtain_meshes(&surfaces, level_height)?;
    let roles = surfaces.iter().map(|surface| surface.role).collect();
    Ok(PresentationPlan {
        surfaces,
        curtains,
        roles,
    })
}

fn batch_liquid_caps(
    surfaces: &[LiquidSurface],
) -> BTreeMap<LiquidCapBatchKey, Vec<LiquidSurface>> {
    let mut batches = BTreeMap::<LiquidCapBatchKey, Vec<LiquidSurface>>::new();
    for &surface in surfaces {
        batches
            .entry(LiquidCapBatchKey {
                chunk: terrain_chunk_coord(surface.position.coord),
                role: surface.role,
            })
            .or_default()
            .push(surface);
    }
    for batch in batches.values_mut() {
        batch.sort_by_key(|surface| surface.position);
    }
    batches
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RenderDescriptor {
    flow: LiquidFlowState,
    downstream: Option<TilePos>,
}

fn descriptor_for_run(
    coord: HexCoord,
    run: SubstanceRun,
    role: FillMaterialRole,
    projection: Option<&MapPresentationProjection>,
    consumed_projection: &mut BTreeSet<TilePos>,
) -> Result<RenderDescriptor, LiquidPresentationError> {
    let Some(projection) = projection else {
        return Ok(RenderDescriptor {
            flow: LiquidFlowState::Still,
            downstream: None,
        });
    };

    let mut descriptor = None;
    for level in run.bottom..run.top {
        let position = TilePos::new(coord, level);
        let projected = projection
            .liquids()
            .get(&position)
            .ok_or(LiquidPresentationError::MissingProjectionVoxel { position })?;
        if projected.material != role {
            return Err(LiquidPresentationError::ProjectionMaterialMismatch {
                position,
                expected: role,
                actual: projected.material,
            });
        }
        let next = RenderDescriptor {
            flow: projected.flow,
            downstream: projected.downstream,
        };
        if descriptor.is_some_and(|current| current != next) {
            return Err(LiquidPresentationError::InconsistentRunProjection {
                top: TilePos::new(coord, run.top.saturating_sub(1)),
            });
        }
        descriptor = Some(next);
        consumed_projection.insert(position);
    }
    descriptor.ok_or(LiquidPresentationError::MissingProjectionVoxel {
        position: TilePos::new(coord, run.bottom),
    })
}

fn validate_surface_directions(surfaces: &[LiquidSurface]) -> Result<(), LiquidPresentationError> {
    for surface in surfaces {
        match (surface.flow, surface.downstream) {
            (LiquidFlowState::Still, _) => {}
            (LiquidFlowState::Current | LiquidFlowState::Rapid | LiquidFlowState::Fall, None) => {
                return Err(LiquidPresentationError::MovingFlowWithoutDownstream {
                    source: surface.position,
                    flow: surface.flow,
                });
            }
            (
                LiquidFlowState::Current | LiquidFlowState::Rapid | LiquidFlowState::Fall,
                Some(downstream),
            ) => {
                if side_between(surface.position.coord, downstream.coord).is_none() {
                    return Err(LiquidPresentationError::NonAdjacentDownstream {
                        source: surface.position,
                        downstream,
                    });
                }
            }
        }
    }
    Ok(())
}

fn build_curtain_meshes(
    surfaces: &[LiquidSurface],
    level_height: f32,
) -> Result<BTreeMap<LiquidCurtainBatchKey, RawMesh>, LiquidPresentationError> {
    curtain_strips(surfaces)?
        .into_iter()
        .map(|(key, strips)| {
            let mut geometry = curtain_geometry(&strips, level_height)?;
            if key.role == FillMaterialRole::Lava && key.style == MaterialStyle::Fall {
                append_lava_landing_splashes(&mut geometry, &strips, level_height)?;
            }
            geometry.validate_finite()?;
            Ok((key, geometry))
        })
        .collect()
}

/// Resolves every visible vertical liquid boundary exactly once.
///
/// Semantic downstream falls retain the animated Fall material. Other exposed
/// steps between neighboring surfaces use the same role-wide material as the
/// horizontal caps, covering the authoritative voxel side without inventing a
/// second flow direction.
fn curtain_strips(
    surfaces: &[LiquidSurface],
) -> Result<BTreeMap<LiquidCurtainBatchKey, Vec<CurtainStrip>>, LiquidPresentationError> {
    let surface_by_position: BTreeMap<_, _> = surfaces
        .iter()
        .map(|surface| (surface.position, *surface))
        .collect();
    let mut strips = BTreeMap::<LiquidCurtainBatchKey, BTreeSet<CurtainStrip>>::new();
    let mut semantic_falls = BTreeSet::new();
    for source in surfaces
        .iter()
        .filter(|surface| surface.flow == LiquidFlowState::Fall)
    {
        let Some(downstream) = source.downstream else {
            return Err(LiquidPresentationError::MovingFlowWithoutDownstream {
                source: source.position,
                flow: source.flow,
            });
        };
        let Some(landing) = surface_by_position.get(&downstream) else {
            return Err(LiquidPresentationError::MissingFallLanding {
                source: source.position,
                downstream,
            });
        };
        if source.role != landing.role {
            return Err(LiquidPresentationError::FallMaterialMismatch {
                source: source.position,
                downstream,
            });
        }
        if source.position.level.saturating_sub(downstream.level) < 2 {
            return Err(LiquidPresentationError::ShallowFall {
                source: source.position,
                downstream,
            });
        }
        let Some(side) = side_between(source.position.coord, downstream.coord) else {
            return Err(LiquidPresentationError::NonAdjacentDownstream {
                source: source.position,
                downstream,
            });
        };
        let strip = CurtainStrip {
            source: source.position,
            downstream,
            side,
        };
        semantic_falls.insert((source.position, downstream));
        strips
            .entry(LiquidCurtainBatchKey {
                role: source.role,
                style: MaterialStyle::Fall,
            })
            .or_default()
            .insert(strip);
    }

    // Presentation can encounter more than one exposed run in a column in
    // hand-authored fixtures. Only the highest surface can expose an outer side;
    // lower runs are hidden behind that column's upper presentation.
    let mut highest_by_coord_role = BTreeMap::<(HexCoord, FillMaterialRole), LiquidSurface>::new();
    for &surface in surfaces {
        highest_by_coord_role
            .entry((surface.position.coord, surface.role))
            .and_modify(|current| {
                if surface.position.level > current.position.level {
                    *current = surface;
                }
            })
            .or_insert(surface);
    }
    for &source in highest_by_coord_role.values() {
        for side in HexSide::ALL {
            let Some(&downstream) =
                highest_by_coord_role.get(&(side.neighbor(source.position.coord), source.role))
            else {
                continue;
            };
            if source.position.level <= downstream.position.level {
                continue;
            }
            let style = if semantic_falls.contains(&(source.position, downstream.position)) {
                MaterialStyle::Fall
            } else {
                MaterialStyle::Surface
            };
            strips
                .entry(LiquidCurtainBatchKey {
                    role: source.role,
                    style,
                })
                .or_default()
                .insert(CurtainStrip {
                    source: source.position,
                    downstream: downstream.position,
                    side,
                });
        }
    }

    Ok(strips
        .into_iter()
        .map(|(key, strips)| (key, strips.into_iter().collect()))
        .collect())
}

#[derive(Debug, Clone, Copy)]
struct LiquidSubstances {
    water: Option<SubstanceId>,
    lava: Option<SubstanceId>,
}

impl LiquidSubstances {
    fn resolve(table: &SubstanceTable) -> Result<Self, LiquidPresentationError> {
        let water = table.id("water");
        let lava = table.id("lava");
        if let (Some(water), Some(lava)) = (water, lava) {
            if water == lava {
                return Err(LiquidPresentationError::AmbiguousSubstances { id: water });
            }
        }
        Ok(Self { water, lava })
    }

    fn role(self, substance: SubstanceId) -> Option<FillMaterialRole> {
        if self.water.is_some() && self.water == Some(substance) {
            Some(FillMaterialRole::Water)
        } else if self.lava.is_some() && self.lava == Some(substance) {
            Some(FillMaterialRole::Lava)
        } else {
            None
        }
    }
}

fn role_color(
    role: FillMaterialRole,
    table: &SubstanceTable,
) -> Result<Color, LiquidPresentationError> {
    let name = match role {
        FillMaterialRole::Water => "water",
        FillMaterialRole::Lava => "lava",
    };
    table
        .id(name)
        .and_then(|id| table.get(id))
        .map(|substance| to_color(substance.color))
        .ok_or(LiquidPresentationError::MissingSubstance { role })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MaterialStyle {
    Surface,
    Fall,
}

#[derive(Debug, Clone, Copy)]
struct LiquidMaterialProfile {
    flow_velocity: Vec2,
    modulation: Vec4,
    emission: Vec4,
    perceptual_roughness: f32,
    reflectance: f32,
    double_sided: bool,
}

impl LiquidMaterialProfile {
    fn new(role: FillMaterialRole, style: MaterialStyle) -> Self {
        let foam_scale = match role {
            FillMaterialRole::Water => 1.0,
            FillMaterialRole::Lava => 0.0,
        };
        let (flow_velocity, modulation, double_sided) = match style {
            MaterialStyle::Surface => (Vec2::ZERO, Vec4::new(0.08, 0.0, 0.04, 0.65), false),
            MaterialStyle::Fall => (
                Vec2::new(0.0, FALL_FLOW_SPEED),
                Vec4::new(0.34, 0.48 * foam_scale, 0.14, 1.25),
                true,
            ),
        };
        let emission = match (role, style) {
            (FillMaterialRole::Water, _) => Vec4::ZERO,
            (FillMaterialRole::Lava, MaterialStyle::Surface) => {
                Vec4::new(0.20, 0.10, LAVA_STILL_PULSE_RATE, 0.0)
            }
            (FillMaterialRole::Lava, MaterialStyle::Fall) => {
                Vec4::new(0.52, 0.18, LAVA_FALL_PULSE_RATE, 0.0)
            }
        };
        let (perceptual_roughness, reflectance) = match (role, style) {
            (FillMaterialRole::Water, MaterialStyle::Surface) => {
                (WATER_SURFACE_ROUGHNESS, WATER_SURFACE_REFLECTANCE)
            }
            _ => (LEGACY_LIQUID_ROUGHNESS, LEGACY_LIQUID_REFLECTANCE),
        };
        Self {
            flow_velocity,
            modulation,
            emission,
            perceptual_roughness,
            reflectance,
            double_sided,
        }
    }
}

#[derive(Debug, Clone)]
struct MaterialSet {
    role: FillMaterialRole,
    surface: Handle<LiquidMaterial>,
    fall: Handle<LiquidMaterial>,
}

impl MaterialSet {
    fn create(
        role: FillMaterialRole,
        color: Color,
        foam: Color,
        phase_seconds: f32,
        materials: &mut Assets<LiquidMaterial>,
    ) -> Self {
        let mut add = |style| {
            materials.add(liquid_material(
                color,
                phase_seconds,
                foam,
                LiquidMaterialProfile::new(role, style),
            ))
        };
        Self {
            role,
            surface: add(MaterialStyle::Surface),
            fall: add(MaterialStyle::Fall),
        }
    }

    fn extend_registry(&self, registry: &mut Vec<Handle<LiquidMaterial>>) {
        registry.extend([self.surface.clone(), self.fall.clone()]);
    }

    fn handle(&self, style: MaterialStyle) -> Handle<LiquidMaterial> {
        match style {
            MaterialStyle::Surface => self.surface.clone(),
            MaterialStyle::Fall => self.fall.clone(),
        }
    }
}

fn material_handle(
    sets: &[MaterialSet],
    role: FillMaterialRole,
    style: MaterialStyle,
) -> Handle<LiquidMaterial> {
    sets.iter()
        .find(|set| set.role == role)
        .map_or_else(Handle::default, |set| set.handle(style))
}

fn liquid_material(
    color: Color,
    phase_seconds: f32,
    foam: Color,
    profile: LiquidMaterialProfile,
) -> LiquidMaterial {
    let foam = foam.to_linear();
    LiquidMaterial {
        base: StandardMaterial {
            base_color: color,
            perceptual_roughness: profile.perceptual_roughness,
            reflectance: profile.reflectance,
            alpha_mode: AlphaMode::Opaque,
            opaque_render_method: OpaqueRendererMethod::Forward,
            depth_bias: LIQUID_PRESENTATION_DEPTH_BIAS,
            cull_mode: if profile.double_sided {
                None
            } else {
                Some(Face::Back)
            },
            double_sided: profile.double_sided,
            ..default()
        },
        extension: LiquidExtension {
            params: LiquidMaterialParams {
                flow_phase_scale: Vec4::new(
                    profile.flow_velocity.x,
                    profile.flow_velocity.y,
                    wrap_phase(phase_seconds),
                    3.0,
                ),
                modulation: profile.modulation,
                emission: profile.emission,
                foam_color: Vec4::new(foam.red, foam.green, foam.blue, 1.0),
            },
        },
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
struct RawMesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl RawMesh {
    fn into_mesh(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
        .with_inserted_indices(Indices::U32(self.indices))
    }

    fn validate_finite(&self) -> Result<(), LiquidPresentationError> {
        let finite = self
            .positions
            .iter()
            .flatten()
            .chain(self.normals.iter().flatten())
            .chain(self.uvs.iter().flatten())
            .all(|component| component.is_finite());
        finite
            .then_some(())
            .ok_or(LiquidPresentationError::NonFiniteGeometry)
    }
}

fn cap_geometry() -> RawMesh {
    let positions = vec![
        [0.0, 0.0, 0.0],
        [0.0, 0.0, -HEX_CIRCUMRADIUS],
        [-HEX_INRADIUS, 0.0, -0.5 * HEX_CIRCUMRADIUS],
        [-HEX_INRADIUS, 0.0, 0.5 * HEX_CIRCUMRADIUS],
        [0.0, 0.0, HEX_CIRCUMRADIUS],
        [HEX_INRADIUS, 0.0, 0.5 * HEX_CIRCUMRADIUS],
        [HEX_INRADIUS, 0.0, -0.5 * HEX_CIRCUMRADIUS],
    ];
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    let uvs = positions
        .iter()
        .map(|[x, _y, z]| {
            [
                0.5 + z / (2.0 * HEX_CIRCUMRADIUS),
                0.5 + x / (2.0 * HEX_INRADIUS),
            ]
        })
        .collect();
    RawMesh {
        positions,
        normals,
        uvs,
        indices: vec![0, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 5, 0, 5, 6, 0, 6, 1],
    }
}

fn cap_batch_geometry(
    surfaces: &[LiquidSurface],
    level_height: f32,
) -> Result<RawMesh, LiquidPresentationError> {
    let cap = cap_geometry();
    let mut batch = RawMesh {
        positions: Vec::with_capacity(surfaces.len().saturating_mul(cap.positions.len())),
        normals: Vec::with_capacity(surfaces.len().saturating_mul(cap.normals.len())),
        uvs: Vec::with_capacity(surfaces.len().saturating_mul(cap.uvs.len())),
        indices: Vec::with_capacity(surfaces.len().saturating_mul(cap.indices.len())),
    };
    for surface in surfaces {
        let transform = cap_transform(surface.position, level_height);
        let transformed_positions = cap
            .positions
            .iter()
            .copied()
            .map(Vec3::from_array)
            .map(|position| transform.transform_point(position))
            .collect::<Vec<_>>();
        let base = u32::try_from(batch.positions.len())
            .map_err(|_error| LiquidPresentationError::MeshIndexOverflow)?;
        batch.positions.extend(
            transformed_positions
                .iter()
                .map(|position| position.to_array()),
        );
        batch.normals.extend(
            cap.normals
                .iter()
                .copied()
                .map(Vec3::from_array)
                .map(|normal| (transform.rotation * normal).to_array()),
        );
        batch
            .uvs
            .extend(transformed_positions.iter().copied().map(continuous_cap_uv));
        for &index in &cap.indices {
            batch.indices.push(
                base.checked_add(index)
                    .ok_or(LiquidPresentationError::MeshIndexOverflow)?,
            );
        }
    }
    batch.validate_finite()?;
    Ok(batch)
}

/// Projects every horizontal cap from one absolute world-space UV chart.
///
/// The old per-prism `0..=1` UVs restarted the analytic wave at every hex, making
/// rivers look like independently animated tiles. A role-wide material plus
/// absolute coordinates gives both vertices of every shared edge identical UVs,
/// including at flow turns, state changes, and chunk boundaries.
fn continuous_cap_uv(world_position: Vec3) -> [f32; 2] {
    [
        world_position.z / (2.0 * HEX_CIRCUMRADIUS),
        world_position.x / (2.0 * HEX_INRADIUS),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CurtainStrip {
    source: TilePos,
    downstream: TilePos,
    side: HexSide,
}

fn curtain_geometry(
    strips: &[CurtainStrip],
    level_height: f32,
) -> Result<RawMesh, LiquidPresentationError> {
    let mut mesh = RawMesh::default();
    for strip in strips {
        let base = u32::try_from(mesh.positions.len())
            .map_err(|_error| LiquidPresentationError::MeshIndexOverflow)?;
        let top_y = surface_y(strip.source.level, level_height);
        let bottom_y = surface_y(strip.downstream.level, level_height);
        let height = top_y - bottom_y;
        let rotation = side_rotation(strip.side);
        let center = strip.source.coord.to_world(0.0);
        let normal = rotation * Vec3::X;
        for (x, y, z, uv) in [
            (
                HEX_INRADIUS + LIQUID_CURTAIN_EDGE_BIAS,
                top_y,
                -0.5 * HEX_CIRCUMRADIUS,
                [0.0, 0.0],
            ),
            (
                HEX_INRADIUS + LIQUID_CURTAIN_EDGE_BIAS,
                top_y,
                0.5 * HEX_CIRCUMRADIUS,
                [1.0, 0.0],
            ),
            (
                HEX_INRADIUS + LIQUID_CURTAIN_EDGE_BIAS,
                bottom_y,
                0.5 * HEX_CIRCUMRADIUS,
                [1.0, height / HEX_CIRCUMRADIUS],
            ),
            (
                HEX_INRADIUS + LIQUID_CURTAIN_EDGE_BIAS,
                bottom_y,
                -0.5 * HEX_CIRCUMRADIUS,
                [0.0, height / HEX_CIRCUMRADIUS],
            ),
        ] {
            let world = center + rotation * Vec3::new(x, 0.0, z) + Vec3::Y * y;
            mesh.positions.push(world.to_array());
            mesh.normals.push(normal.to_array());
            mesh.uvs.push(uv);
        }
        mesh.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    mesh.validate_finite()?;
    Ok(mesh)
}

fn append_lava_landing_splashes(
    mesh: &mut RawMesh,
    strips: &[CurtainStrip],
    level_height: f32,
) -> Result<(), LiquidPresentationError> {
    for strip in strips {
        let base = u32::try_from(mesh.positions.len())
            .map_err(|_error| LiquidPresentationError::MeshIndexOverflow)?;
        let rotation = side_rotation(strip.side);
        let center = strip.downstream.coord.to_world(0.0);
        let y = surface_y(strip.downstream.level, level_height) + LAVA_SPLASH_SURFACE_BIAS;
        for (x, z, uv) in [
            (
                -HEX_INRADIUS + LIQUID_CURTAIN_EDGE_BIAS,
                -0.46 * HEX_CIRCUMRADIUS,
                [0.0, 0.0],
            ),
            (
                -HEX_INRADIUS + LIQUID_CURTAIN_EDGE_BIAS,
                0.46 * HEX_CIRCUMRADIUS,
                [1.0, 0.0],
            ),
            (-0.12 * HEX_INRADIUS, 0.68 * HEX_CIRCUMRADIUS, [1.0, 1.0]),
            (-0.12 * HEX_INRADIUS, -0.68 * HEX_CIRCUMRADIUS, [0.0, 1.0]),
        ] {
            let world = center + rotation * Vec3::new(x, 0.0, z) + Vec3::Y * y;
            mesh.positions.push(world.to_array());
            mesh.normals.push(Vec3::Y.to_array());
            mesh.uvs.push(uv);
        }
        mesh.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Ok(())
}

fn cap_transform(position: TilePos, level_height: f32) -> Transform {
    let translation = position
        .coord
        .to_world(surface_y(position.level, level_height));
    Transform::from_translation(translation)
}

fn surface_y(level: Level, level_height: f32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "playable voxel levels are exactly representable in f32"
    )]
    let boundary = level.saturating_add(1) as f32 * level_height;
    boundary + cap_bias(level_height)
}

fn cap_bias(level_height: f32) -> f32 {
    (level_height * LIQUID_CAP_BIAS_RATIO).min(LIQUID_CAP_BIAS_MAX)
}

const fn side_yaw(side: HexSide) -> f32 {
    match side {
        HexSide::East => 0.0,
        HexSide::SouthEast => -std::f32::consts::PI / 3.0,
        HexSide::SouthWest => -2.0 * std::f32::consts::PI / 3.0,
        HexSide::West => std::f32::consts::PI,
        HexSide::NorthWest => 2.0 * std::f32::consts::PI / 3.0,
        HexSide::NorthEast => std::f32::consts::PI / 3.0,
    }
}

fn side_rotation(side: HexSide) -> Quat {
    Quat::from_rotation_y(side_yaw(side))
}

fn side_between(source: HexCoord, target: HexCoord) -> Option<HexSide> {
    HexSide::ALL
        .into_iter()
        .find(|side| side.neighbor(source) == target)
}

#[cfg(test)]
mod tests {
    use bevy::ecs::world::CommandQueue;
    use bevy::platform::collections::HashMap;
    use hex_assets::{ArtPalette, PaletteSwatch, SrgbColor, Substance, SubstanceFile, SwatchId};
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;

    fn coord(x: i32, y: i32, z: i32) -> HexCoord {
        HexCoord::new_cubic(x, y, z)
    }

    fn assert_vec3_near(actual: Vec3, expected: Vec3) {
        assert!(
            actual.abs_diff_eq(expected, 1.0e-5),
            "{actual:?} != {expected:?}"
        );
    }

    fn assert_f32_near(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 1.0e-6, "{actual} != {expected}");
    }

    fn liquid_table() -> SubstanceTable {
        let swatches = [
            ("terrain/stone", "Stone", (0.5, 0.5, 0.5)),
            ("liquid/water", "Water", (0.08, 0.32, 0.65)),
            ("liquid/lava", "Lava", (0.9, 0.2, 0.04)),
            (
                LIQUID_FOAM_SWATCH,
                "Water Foam",
                (0.896_243_8, 0.959_346_6, 0.991_156_4),
            ),
        ]
        .into_iter()
        .map(|(id, name, (red, green, blue))| {
            let id = SwatchId::new(id).expect("fixture swatch id should be valid");
            let swatch = PaletteSwatch::new(
                name,
                SrgbColor::new(red, green, blue).expect("fixture color should be valid"),
                BTreeSet::from(["test".to_owned()]),
            )
            .expect("fixture swatch should be valid");
            (id, swatch)
        })
        .collect::<BTreeMap<_, _>>();
        let palette = ArtPalette::new(swatches).expect("fixture palette should be valid");

        let mut substances = HashMap::default();
        substances.insert("air".to_owned(), Substance::invisible(false, false));
        for (name, swatch, solid) in [
            ("stone", "terrain/stone", true),
            ("water", "liquid/water", false),
            ("lava", "liquid/lava", false),
        ] {
            substances.insert(
                name.to_owned(),
                Substance::from_swatch(
                    SwatchId::new(swatch).expect("fixture swatch id should be valid"),
                    solid,
                    true,
                ),
            );
        }
        SubstanceTable::from_file(&SubstanceFile { substances }, &palette)
            .expect("fixture substances should resolve through the fixture palette")
    }

    #[test]
    fn frozen_phase_rejects_non_finite_input_and_wraps() {
        assert!(LiquidVisualTime::frozen_at(f32::NAN).is_none());
        assert!(LiquidVisualTime::frozen_at(f32::INFINITY).is_none());

        let Some(mut time) = LiquidVisualTime::frozen_at(PHASE_WRAP_SECONDS + 0.25) else {
            unreachable!("finite review phase must be admitted")
        };
        assert!(time.is_frozen());
        assert!((time.advance(100.0) - 0.25).abs() < f32::EPSILON);
        assert!(!time.freeze(f32::NEG_INFINITY));
        assert!((time.phase_seconds() - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn phase_wrap_is_a_common_period_for_every_authored_rate() {
        for rate in [
            FALL_FLOW_SPEED,
            LAVA_STILL_PULSE_RATE,
            LAVA_FALL_PULSE_RATE,
            SECONDARY_WAVE_PHASE_RATE,
        ] {
            let cycles = rate * PHASE_WRAP_SECONDS;
            assert_f32_near(cycles, cycles.round());
        }
    }

    #[test]
    fn replacement_materials_start_at_the_current_wrapped_phase() {
        let mut materials = Assets::<LiquidMaterial>::default();
        let phase = PHASE_WRAP_SECONDS + 17.25;
        let set = MaterialSet::create(
            FillMaterialRole::Water,
            Color::srgb(0.08, 0.32, 0.65),
            Color::srgb(0.896_243_8, 0.959_346_6, 0.991_156_4),
            phase,
            &mut materials,
        );
        let mut handles = Vec::new();
        set.extend_registry(&mut handles);

        assert_eq!(handles.len(), 2);
        for handle in handles {
            let material = materials
                .get(&handle)
                .expect("new liquid material must remain present");
            assert_f32_near(
                material.extension.params.flow_phase_scale.z,
                wrap_phase(phase),
            );
            for (actual, previous) in material
                .extension
                .params
                .foam_color
                .to_array()
                .into_iter()
                .zip([0.78_f32, 0.91, 0.98, 1.0])
            {
                assert!(
                    actual.to_bits().abs_diff(previous.to_bits()) <= 2,
                    "the palette swatch must choose the nearest representable sRGB round-trip"
                );
            }
        }
    }

    #[test]
    fn live_phase_ignores_invalid_or_negative_delta_and_wraps() {
        let mut time = LiquidVisualTime::default();
        assert_f32_near(time.advance(f32::NAN), 0.0);
        assert_f32_near(time.advance(-1.0), 0.0);
        assert_f32_near(time.advance(PHASE_WRAP_SECONDS + 0.5), 0.5);
        time.unfreeze();
        assert!(!time.is_frozen());
    }

    #[test]
    fn gameplay_reentry_resets_live_time_but_preserves_review_phase() {
        let mut live = LiquidVisualTime::default();
        assert_f32_near(live.advance(3.5), 3.5);
        live.reset_for_gameplay_entry();
        assert_f32_near(live.phase_seconds(), 0.0);

        let Some(mut frozen) = LiquidVisualTime::frozen_at(PHASE_WRAP_SECONDS + 0.75) else {
            unreachable!("finite review phase must be admitted")
        };
        frozen.reset_for_gameplay_entry();
        assert_f32_near(frozen.phase_seconds(), 0.75);
        assert!(frozen.is_frozen());
    }

    #[test]
    fn cap_mesh_has_upward_triangles_and_complete_vertex_attributes() {
        let mesh = cap_geometry();
        assert_eq!(mesh.positions.len(), 7);
        assert_eq!(mesh.normals, vec![[0.0, 1.0, 0.0]; 7]);
        assert_eq!(mesh.indices.len(), 18);
        for triangle in mesh.indices.chunks_exact(3) {
            let [a_index, b_index, c_index] = triangle else {
                unreachable!("chunks_exact(3) always yields three indices")
            };
            let Some(a) = mesh.positions.get(*a_index as usize).copied() else {
                unreachable!("cap indices must name cap vertices")
            };
            let Some(b) = mesh.positions.get(*b_index as usize).copied() else {
                unreachable!("cap indices must name cap vertices")
            };
            let Some(c) = mesh.positions.get(*c_index as usize).copied() else {
                unreachable!("cap indices must name cap vertices")
            };
            let a = Vec3::from_array(a);
            let b = Vec3::from_array(b);
            let c = Vec3::from_array(c);
            assert!((b - a).cross(c - a).y > 0.0);
        }
        assert_eq!(mesh.uvs.len(), mesh.positions.len());
    }

    #[test]
    fn liquid_caps_batch_by_euclidean_chunk_and_role_only() {
        let still = |q, role| LiquidSurface {
            position: TilePos::new(HexCoord::from_axial(q, 0), 4),
            role,
            flow: LiquidFlowState::Still,
            downstream: None,
        };
        let moving = |q, level, role, flow| LiquidSurface {
            position: TilePos::new(HexCoord::from_axial(q, 0), level),
            role,
            flow,
            downstream: Some(TilePos::new(
                HexCoord::from_axial(q + 1, 0),
                level.saturating_sub(4),
            )),
        };
        let surfaces = vec![
            still(-17, FillMaterialRole::Water),
            still(-16, FillMaterialRole::Water),
            still(-1, FillMaterialRole::Water),
            moving(0, 4, FillMaterialRole::Water, LiquidFlowState::Current),
            moving(2, 8, FillMaterialRole::Water, LiquidFlowState::Rapid),
            moving(4, 8, FillMaterialRole::Water, LiquidFlowState::Fall),
            still(6, FillMaterialRole::Lava),
        ];

        let batches = batch_liquid_caps(&surfaces);
        assert_eq!(batches.len(), 4);
        assert_eq!(
            batches
                .keys()
                .map(|key| (key.chunk.q, key.chunk.r, key.role))
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                (-2, 0, FillMaterialRole::Water),
                (-1, 0, FillMaterialRole::Water),
                (0, 0, FillMaterialRole::Lava),
                (0, 0, FillMaterialRole::Water),
            ])
        );
        assert_eq!(
            batches
                .get(&LiquidCapBatchKey {
                    chunk: TerrainChunkCoord { q: 0, r: 0 },
                    role: FillMaterialRole::Water,
                })
                .map(Vec::len),
            Some(3),
            "Current, Rapid, and Fall semantics must not split the opaque base surface"
        );
        let logical = batches
            .values()
            .flatten()
            .map(|surface| surface.position)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            logical.len(),
            surfaces.len(),
            "logical caps may not duplicate"
        );
        assert_eq!(
            logical,
            surfaces
                .iter()
                .map(|surface| surface.position)
                .collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn disconnected_cap_batch_preserves_each_surface_and_flow_orientation() {
        let east = TilePos::new(HexCoord::from_axial(1, 0), 4);
        let surfaces = [
            LiquidSurface {
                position: TilePos::new(HexCoord::ORIGIN, 4),
                role: FillMaterialRole::Water,
                flow: LiquidFlowState::Current,
                downstream: Some(east),
            },
            LiquidSurface {
                position: TilePos::new(HexCoord::from_axial(15, 15), 7),
                role: FillMaterialRole::Water,
                flow: LiquidFlowState::Still,
                downstream: None,
            },
        ];
        let mesh = cap_batch_geometry(&surfaces, 0.4).expect("valid disconnected cap batch");
        assert_eq!(mesh.positions.len(), surfaces.len() * 7);
        assert_eq!(mesh.normals.len(), surfaces.len() * 7);
        assert_eq!(mesh.uvs.len(), surfaces.len() * 7);
        assert_eq!(mesh.indices.len(), surfaces.len() * 18);
        mesh.validate_finite().expect("batch geometry is finite");

        let expected_first_center =
            cap_transform(surfaces[0].position, 0.4).transform_point(Vec3::ZERO);
        assert_vec3_near(Vec3::from_array(mesh.positions[0]), expected_first_center);
        let expected_second_center =
            cap_transform(surfaces[1].position, 0.4).transform_point(Vec3::ZERO);
        assert_vec3_near(Vec3::from_array(mesh.positions[7]), expected_second_center);
    }

    #[test]
    fn adjacent_caps_share_world_uvs_across_every_turn_and_chunk_boundary() {
        for origin in [HexCoord::ORIGIN, HexCoord::from_axial(15, 0)] {
            for (index, side) in HexSide::ALL.into_iter().enumerate() {
                let source = TilePos::new(origin, 4);
                let target = TilePos::new(side.neighbor(origin), 4);
                let Some(turn) = HexSide::ALL.get((index + 1) % HexSide::ALL.len()).copied() else {
                    unreachable!("a wrapped side index stays inside the six-side table")
                };
                let surfaces = [
                    LiquidSurface {
                        position: source,
                        role: FillMaterialRole::Water,
                        flow: LiquidFlowState::Current,
                        downstream: Some(target),
                    },
                    LiquidSurface {
                        position: target,
                        role: FillMaterialRole::Water,
                        flow: LiquidFlowState::Rapid,
                        downstream: Some(TilePos::new(turn.neighbor(target.coord), target.level)),
                    },
                ];
                let mesh = cap_batch_geometry(&surfaces, 0.4)
                    .expect("adjacent turning flow caps are valid");
                let mut shared_vertices = 0;
                for first in 0..7 {
                    for second in 7..14 {
                        let first_position = Vec3::from_array(mesh.positions[first]);
                        let second_position = Vec3::from_array(mesh.positions[second]);
                        if !first_position.abs_diff_eq(second_position, 1.0e-5) {
                            continue;
                        }
                        shared_vertices += 1;
                        assert!(Vec2::from_array(mesh.uvs[first])
                            .abs_diff_eq(Vec2::from_array(mesh.uvs[second]), 1.0e-5));
                    }
                }
                assert_eq!(shared_vertices, 2, "one shared hex edge has two vertices");
            }
        }
    }

    #[test]
    fn spawned_cap_batches_retain_exact_logical_coverage_and_teardown_cleanly() {
        let table = liquid_table();
        let Some(water) = table.id("water") else {
            unreachable!("test table contains water")
        };
        let Some(lava) = table.id("lava") else {
            unreachable!("test table contains lava")
        };
        let positions = [
            (TilePos::new(HexCoord::from_axial(-17, 0), 0), water),
            (TilePos::new(HexCoord::from_axial(-16, 0), 0), water),
            (TilePos::new(HexCoord::from_axial(-1, 0), 0), water),
            (TilePos::new(HexCoord::from_axial(0, 0), 0), water),
            (TilePos::new(HexCoord::from_axial(16, 0), 0), water),
            (TilePos::new(HexCoord::from_axial(0, 1), 0), lava),
        ];
        let mut map = VoxelMap::new();
        for &(position, substance) in &positions {
            map.set(position, substance);
        }
        let mut world = World::new();
        let mut queue = CommandQueue::default();
        let mut meshes = Assets::<Mesh>::default();
        let mut materials = Assets::<LiquidMaterial>::default();
        let entities = {
            let mut commands = Commands::new(&mut queue, &world);
            spawn_presentations(
                &mut commands,
                &mut meshes,
                &mut materials,
                &map,
                &table,
                0.4,
                0.0,
                None,
            )
            .expect("valid liquid batches should spawn")
        };
        queue.apply(&mut world);

        assert_eq!(
            entities.len(),
            5,
            "five chunk/role cap batches are expected and no curtains are needed"
        );
        assert_eq!(meshes.len(), entities.len());
        assert_eq!(
            materials.len(),
            4,
            "water and lava each retain one shared surface and one fall material"
        );
        assert_eq!(
            world.resource::<LiquidMaterialHandles>().handles.len(),
            materials.len(),
            "the teardown registry owns every role-wide material handle"
        );
        let mut logical = BTreeSet::new();
        let mut keys = BTreeSet::new();
        let mut query =
            world.query::<(&LiquidCapBatch, &Pickable, Has<NotShadowCaster>, &Transform)>();
        for (batch, pickable, no_shadow, transform) in query.iter(&world) {
            assert!(keys.insert(batch.key), "duplicate liquid batch key");
            assert_eq!(*pickable, Pickable::IGNORE);
            assert!(no_shadow);
            assert_eq!(*transform, Transform::default());
            for surface in &batch.surfaces {
                assert!(
                    logical.insert(surface.position),
                    "duplicate logical liquid cap"
                );
                assert_eq!(surface.flow, LiquidFlowState::Still);
            }
        }
        assert_eq!(
            logical,
            positions
                .into_iter()
                .map(|(position, _substance)| position)
                .collect::<BTreeSet<_>>()
        );

        for entity in entities {
            assert!(world.despawn(entity));
        }
        assert_eq!(query.iter(&world).count(), 0);
    }

    #[test]
    fn side_rotations_match_exact_neighbor_world_directions() {
        for side in HexSide::ALL {
            let source = HexCoord::ORIGIN.to_world(0.0);
            let target = side.neighbor(HexCoord::ORIGIN).to_world(0.0);
            let expected = (target - source).normalize();
            assert_vec3_near(side_rotation(side) * Vec3::X, expected);
        }
    }

    #[test]
    fn cap_bias_stays_above_surface_and_below_thin_level() {
        assert_f32_near(cap_bias(0.4), LIQUID_CAP_BIAS_MAX);
        assert!(cap_bias(0.01) > 0.0);
        assert!(cap_bias(0.01) < 0.01);
    }

    #[test]
    fn curtain_geometry_uses_exact_shared_edge_and_world_scale_v() {
        let source = TilePos::new(HexCoord::ORIGIN, 8);
        let downstream = TilePos::new(coord(1, 0, -1), 4);
        let mesh = curtain_geometry(
            &[CurtainStrip {
                source,
                downstream,
                side: HexSide::East,
            }],
            0.4,
        )
        .expect("valid fall geometry");

        assert_eq!(mesh.positions.len(), 4);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
        let top_y = surface_y(source.level, 0.4);
        let bottom_y = surface_y(downstream.level, 0.4);
        let Some([first_x, first_y, _first_z]) = mesh.positions.first().copied() else {
            unreachable!("curtain has a first vertex")
        };
        let Some([_third_x, third_y, _third_z]) = mesh.positions.get(2).copied() else {
            unreachable!("curtain has a third vertex")
        };
        let Some([_third_u, third_v]) = mesh.uvs.get(2).copied() else {
            unreachable!("curtain has a third UV")
        };
        assert_f32_near(first_y, top_y);
        assert_f32_near(third_y, bottom_y);
        assert_f32_near(third_v, top_y - bottom_y);
        assert!(first_x > HEX_INRADIUS);
        for normal in mesh.normals {
            assert_vec3_near(Vec3::from_array(normal), Vec3::X);
        }
    }

    #[test]
    fn every_same_role_exposed_height_edge_gets_one_deduplicated_curtain() {
        let generic_source = TilePos::new(HexCoord::ORIGIN, 8);
        let generic_lower = TilePos::new(HexSide::East.neighbor(generic_source.coord), 4);
        let fall_source = TilePos::new(HexCoord::from_axial(10, 0), 8);
        let fall_lower = TilePos::new(HexSide::East.neighbor(fall_source.coord), 4);
        let equal_source = TilePos::new(HexCoord::from_axial(20, 0), 5);
        let equal_neighbor = TilePos::new(HexSide::East.neighbor(equal_source.coord), 5);
        let unlike_source = TilePos::new(HexCoord::from_axial(30, 0), 8);
        let unlike_lower = TilePos::new(HexSide::East.neighbor(unlike_source.coord), 4);
        let surfaces = [
            LiquidSurface {
                position: generic_source,
                role: FillMaterialRole::Water,
                flow: LiquidFlowState::Still,
                downstream: None,
            },
            LiquidSurface {
                position: generic_lower,
                role: FillMaterialRole::Water,
                flow: LiquidFlowState::Still,
                downstream: None,
            },
            LiquidSurface {
                position: fall_source,
                role: FillMaterialRole::Water,
                flow: LiquidFlowState::Fall,
                downstream: Some(fall_lower),
            },
            LiquidSurface {
                position: fall_lower,
                role: FillMaterialRole::Water,
                flow: LiquidFlowState::Still,
                downstream: None,
            },
            LiquidSurface {
                position: equal_source,
                role: FillMaterialRole::Water,
                flow: LiquidFlowState::Still,
                downstream: None,
            },
            LiquidSurface {
                position: equal_neighbor,
                role: FillMaterialRole::Water,
                flow: LiquidFlowState::Still,
                downstream: None,
            },
            LiquidSurface {
                position: unlike_source,
                role: FillMaterialRole::Water,
                flow: LiquidFlowState::Still,
                downstream: None,
            },
            LiquidSurface {
                position: unlike_lower,
                role: FillMaterialRole::Lava,
                flow: LiquidFlowState::Still,
                downstream: None,
            },
        ];

        let strips = curtain_strips(&surfaces).expect("valid exposed liquid sides");
        assert_eq!(strips.len(), 2);
        assert_eq!(
            strips[&LiquidCurtainBatchKey {
                role: FillMaterialRole::Water,
                style: MaterialStyle::Surface,
            }],
            vec![CurtainStrip {
                source: generic_source,
                downstream: generic_lower,
                side: HexSide::East,
            }]
        );
        assert_eq!(
            strips[&LiquidCurtainBatchKey {
                role: FillMaterialRole::Water,
                style: MaterialStyle::Fall,
            }],
            vec![CurtainStrip {
                source: fall_source,
                downstream: fall_lower,
                side: HexSide::East,
            }]
        );

        let meshes = build_curtain_meshes(&surfaces, 0.4)
            .expect("every exact exposed edge should materialize");
        assert!(meshes
            .values()
            .all(|mesh| mesh.positions.len() == 4 && mesh.indices.len() == 6));
    }

    #[test]
    fn role_wide_surface_profiles_stay_subtle_and_falls_brighten() {
        let water = LiquidMaterialProfile::new(FillMaterialRole::Water, MaterialStyle::Surface);
        assert_eq!(water.emission, Vec4::ZERO);
        assert_f32_near(water.perceptual_roughness, WATER_SURFACE_ROUGHNESS);
        assert_f32_near(water.reflectance, WATER_SURFACE_REFLECTANCE);
        assert!(
            water.perceptual_roughness - water.modulation.z >= 0.55,
            "animated ripples must not turn a flat water cap back into a broad planar mirror"
        );

        let still = LiquidMaterialProfile::new(FillMaterialRole::Lava, MaterialStyle::Surface);
        let fall = LiquidMaterialProfile::new(FillMaterialRole::Lava, MaterialStyle::Fall);
        assert_eq!(still.flow_velocity, Vec2::ZERO);
        assert_f32_near(still.emission.z, LAVA_STILL_PULSE_RATE);
        assert!(still.emission.y > 0.0);
        assert_f32_near(fall.flow_velocity.y, FALL_FLOW_SPEED);
        assert!(fall.flow_velocity.y > still.flow_velocity.y);
        assert!(fall.emission.x > still.emission.x);
        assert!(fall.emission.y > still.emission.y);
        assert!(fall.emission.z > still.emission.z);
        assert!(fall.double_sided);
        assert_f32_near(still.perceptual_roughness, LEGACY_LIQUID_ROUGHNESS);
        assert_f32_near(still.reflectance, LEGACY_LIQUID_REFLECTANCE);
        assert_f32_near(fall.perceptual_roughness, LEGACY_LIQUID_ROUGHNESS);
        assert_f32_near(fall.reflectance, LEGACY_LIQUID_REFLECTANCE);
    }

    #[test]
    fn liquid_material_uses_depth_precedence_above_the_opaque_voxel_surface() {
        let material = liquid_material(
            Color::srgb(0.08, 0.32, 0.65),
            0.0,
            Color::srgb(0.90, 0.96, 0.99),
            LiquidMaterialProfile::new(FillMaterialRole::Water, MaterialStyle::Surface),
        );
        assert!(LIQUID_PRESENTATION_DEPTH_BIAS > 0.0);
        assert!(cap_bias(0.4) > 0.0 && cap_bias(0.4) < 0.4);
        assert_f32_near(material.base.depth_bias, LIQUID_PRESENTATION_DEPTH_BIAS);
        assert_f32_near(material.base.perceptual_roughness, WATER_SURFACE_ROUGHNESS);
        assert_f32_near(material.base.reflectance, WATER_SURFACE_REFLECTANCE);
        assert_eq!(material.base.alpha_mode, AlphaMode::Opaque);
        assert_eq!(
            material.base.opaque_render_method,
            OpaqueRendererMethod::Forward
        );
    }

    #[test]
    fn lava_falls_add_deterministic_landing_splashes_without_changing_water() {
        let source = TilePos::new(HexCoord::ORIGIN, 8);
        let downstream = TilePos::new(coord(1, 0, -1), 4);
        let surfaces = |role| {
            [
                LiquidSurface {
                    position: source,
                    role,
                    flow: LiquidFlowState::Fall,
                    downstream: Some(downstream),
                },
                LiquidSurface {
                    position: downstream,
                    role,
                    flow: LiquidFlowState::Still,
                    downstream: None,
                },
            ]
        };

        let water = build_curtain_meshes(&surfaces(FillMaterialRole::Water), 0.4)
            .expect("water curtain should remain valid");
        let water = water
            .get(&LiquidCurtainBatchKey {
                role: FillMaterialRole::Water,
                style: MaterialStyle::Fall,
            })
            .expect("water curtain should exist");
        assert_eq!(water.positions.len(), 4);
        assert_eq!(water.indices.len(), 6);

        let first = build_curtain_meshes(&surfaces(FillMaterialRole::Lava), 0.4)
            .expect("lava fall effect should be valid");
        let second = build_curtain_meshes(&surfaces(FillMaterialRole::Lava), 0.4)
            .expect("lava fall effect should be repeatable");
        assert_eq!(first, second);
        let lava = first
            .get(&LiquidCurtainBatchKey {
                role: FillMaterialRole::Lava,
                style: MaterialStyle::Fall,
            })
            .expect("lava fall effect should exist");
        assert_eq!(lava.positions.len(), 8);
        assert_eq!(lava.indices.len(), 12);
        let [_, splash_y, _] = *lava
            .positions
            .get(4)
            .expect("lava effect should append splash vertices");
        assert!(splash_y > surface_y(downstream.level, 0.4));
        let splash_normals = lava
            .normals
            .get(4..)
            .expect("lava effect should append splash normals");
        assert!(splash_normals
            .iter()
            .all(|normal| Vec3::from_array(*normal).abs_diff_eq(Vec3::Y, 1.0e-6)));
    }

    #[test]
    fn fall_contract_requires_an_exposed_exact_landing() {
        let source = LiquidSurface {
            position: TilePos::new(HexCoord::ORIGIN, 8),
            role: FillMaterialRole::Water,
            flow: LiquidFlowState::Fall,
            downstream: Some(TilePos::new(coord(1, 0, -1), 4)),
        };
        assert!(matches!(
            build_curtain_meshes(&[source], 0.4),
            Err(LiquidPresentationError::MissingFallLanding { .. })
        ));
    }

    #[test]
    fn legacy_liquid_runs_are_inferred_as_still_caps() {
        let table = liquid_table();
        let Some(water) = table.id("water") else {
            unreachable!("test table contains water")
        };
        let mut map = VoxelMap::new();
        map.set(TilePos::new(HexCoord::ORIGIN, 0), water);
        map.set(TilePos::new(HexCoord::ORIGIN, 1), water);

        let plan = build_presentation_plan(&map, &table, 0.4, None)
            .expect("legacy liquid presentation should be valid");
        assert_eq!(
            plan.surfaces,
            vec![LiquidSurface {
                position: TilePos::new(HexCoord::ORIGIN, 1),
                role: FillMaterialRole::Water,
                flow: LiquidFlowState::Still,
                downstream: None,
            }]
        );
        assert!(plan.curtains.is_empty());
    }

    #[test]
    fn present_v3_projection_is_authoritative_over_legacy_inference() {
        let table = liquid_table();
        let Some(water) = table.id("water") else {
            unreachable!("test table contains water")
        };
        let mut map = VoxelMap::new();
        let position = TilePos::new(HexCoord::ORIGIN, 0);
        map.set(position, water);
        let projection = MapPresentationProjection::default();

        assert_eq!(
            build_presentation_plan(&map, &table, 0.4, Some(&projection))
                .expect_err("empty V3 projection must not synthesize legacy metadata"),
            LiquidPresentationError::MissingProjectionVoxel { position }
        );
    }

    #[test]
    fn shader_preserves_opaque_forward_pbr_contract() {
        let shader = include_str!("../../../assets/shaders/liquid.wgsl");
        let flow = shader
            .find("flow_phase_scale: vec4<f32>")
            .expect("shader must declare flow and phase");
        let modulation = shader
            .find("modulation: vec4<f32>")
            .expect("shader must declare modulation");
        let emission = shader
            .find("emission: vec4<f32>")
            .expect("shader must declare emission");
        let foam = shader
            .find("foam_color: vec4<f32>")
            .expect("shader must declare palette-backed foam");
        assert!(flow < modulation && modulation < emission && emission < foam);
        assert!(shader.contains("@binding(100)"));
        assert!(shader.contains("pbr_input_from_standard_material"));
        assert!(shader.contains("apply_pbr_lighting"));
        assert!(shader.contains("main_pass_post_lighting_processing"));
        assert!(shader.contains("pbr_input.material.base_color = vec4<f32>"));
        assert!(!shader.contains("pbr_input.material.base_color.rgb ="));
        assert!(shader.contains("liquid.foam_color.rgb"));
        assert!(shader.contains("pbr_input.material.emissive = vec4<f32>"));
        assert!(shader.contains("liquid.emission.y"));
        assert!(!shader.contains("vec3<f32>(0.78, 0.91, 0.98)"));
        assert!(shader.contains(&format!(
            "liquid.flow_phase_scale.z * {SECONDARY_WAVE_PHASE_RATE}"
        )));
        assert!(shader.contains("out.color.a = 1.0"));
    }
}
