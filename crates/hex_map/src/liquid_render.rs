//! Opaque, non-interactive presentation geometry for liquid voxel runs.
//!
//! The ordinary voxel prisms remain the authoritative volume, pick target, and
//! shadow caster. This module adds only a biased horizontal cap to each exposed
//! water or lava run, a combined vertical curtain for semantic V3 falls, and
//! deterministic landing-splash geometry for lava falls.

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
use crate::voxel::{runs, SubstanceRun, VoxelMap};

const LIQUID_SHADER_PATH: &str = "shaders/liquid.wgsl";
const LIQUID_FOAM_SWATCH: &str = "liquid/foam";
const PHASE_WRAP_SECONDS: f32 = 400.0;
const CURRENT_FLOW_SPEED: f32 = 0.22;
const RAPID_FLOW_SPEED: f32 = 0.55;
const FALL_FLOW_SPEED: f32 = 0.85;
const LAVA_STILL_PULSE_RATE: f32 = 0.05;
const LAVA_CURRENT_PULSE_RATE: f32 = 0.10;
const LAVA_RAPID_PULSE_RATE: f32 = 0.25;
const LAVA_FALL_PULSE_RATE: f32 = 0.40;
#[cfg(test)]
const SECONDARY_WAVE_PHASE_RATE: f32 = 0.025;
const LIQUID_CAP_BIAS_RATIO: f32 = 0.02;
const LIQUID_CAP_BIAS_MAX: f32 = 0.002 * HEX_CIRCUMRADIUS;
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

#[derive(Debug)]
struct PresentationPlan {
    surfaces: Vec<LiquidSurface>,
    falls: BTreeMap<FillMaterialRole, RawMesh>,
    roles: BTreeSet<FillMaterialRole>,
}

/// Spawns non-pickable liquid caps and combined fall curtains.
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
    let cap = meshes.add(cap_geometry().into_mesh());
    let mut material_sets = Vec::with_capacity(role_colors.len());
    let mut registered_handles = Vec::with_capacity(role_colors.len().saturating_mul(4));
    for (role, color) in role_colors {
        let set = MaterialSet::create(role, color, foam, phase_seconds, materials);
        set.extend_registry(&mut registered_handles);
        material_sets.push(set);
    }

    let mut entities = Vec::with_capacity(plan.surfaces.len().saturating_add(plan.falls.len()));
    for surface in plan.surfaces {
        let style = match surface.flow {
            LiquidFlowState::Still => MaterialStyle::Still,
            LiquidFlowState::Current => MaterialStyle::Current,
            LiquidFlowState::Rapid | LiquidFlowState::Fall => MaterialStyle::Rapid,
        };
        let material = material_handle(&material_sets, surface.role, style);
        let direction = match surface.flow {
            LiquidFlowState::Still => None,
            LiquidFlowState::Current | LiquidFlowState::Rapid | LiquidFlowState::Fall => surface
                .downstream
                .and_then(|downstream| side_between(surface.position.coord, downstream.coord)),
        };
        let transform = cap_transform(surface.position, direction, level_height);
        let entity = commands
            .spawn((
                Mesh3d(cap.clone()),
                MeshMaterial3d(material),
                transform,
                Pickable::IGNORE,
                NotShadowCaster,
                Name::new("LiquidCap"),
            ))
            .id();
        entities.push(entity);
    }

    for (role, geometry) in plan.falls {
        let mesh = meshes.add(geometry.into_mesh());
        let material = material_handle(&material_sets, role, MaterialStyle::Fall);
        let entity = commands
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::default(),
                Pickable::IGNORE,
                NotShadowCaster,
                Name::new("LiquidFallCurtain"),
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
    let falls = build_fall_meshes(&surfaces, level_height)?;
    let roles = surfaces.iter().map(|surface| surface.role).collect();
    Ok(PresentationPlan {
        surfaces,
        falls,
        roles,
    })
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

fn build_fall_meshes(
    surfaces: &[LiquidSurface],
    level_height: f32,
) -> Result<BTreeMap<FillMaterialRole, RawMesh>, LiquidPresentationError> {
    let surface_by_position: BTreeMap<_, _> = surfaces
        .iter()
        .map(|surface| (surface.position, *surface))
        .collect();
    let mut falls = BTreeMap::<FillMaterialRole, Vec<FallGeometry>>::new();
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
        falls.entry(source.role).or_default().push(FallGeometry {
            source: source.position,
            downstream,
            side,
        });
    }

    falls
        .into_iter()
        .map(|(role, strips)| {
            let mut geometry = curtain_geometry(&strips, level_height)?;
            if role == FillMaterialRole::Lava {
                append_lava_landing_splashes(&mut geometry, &strips, level_height)?;
            }
            geometry.validate_finite()?;
            Ok((role, geometry))
        })
        .collect()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MaterialStyle {
    Still,
    Current,
    Rapid,
    Fall,
}

#[derive(Debug, Clone, Copy)]
struct LiquidMaterialProfile {
    flow_velocity: Vec2,
    modulation: Vec4,
    emission: Vec4,
    double_sided: bool,
}

impl LiquidMaterialProfile {
    fn new(role: FillMaterialRole, style: MaterialStyle) -> Self {
        let foam_scale = match role {
            FillMaterialRole::Water => 1.0,
            FillMaterialRole::Lava => 0.0,
        };
        let (flow_velocity, modulation, double_sided) = match style {
            MaterialStyle::Still => (Vec2::ZERO, Vec4::new(0.08, 0.0, 0.04, 0.65), false),
            MaterialStyle::Current => (
                Vec2::new(0.0, CURRENT_FLOW_SPEED),
                Vec4::new(0.18, 0.05 * foam_scale, 0.08, 0.75),
                false,
            ),
            MaterialStyle::Rapid => (
                Vec2::new(0.0, RAPID_FLOW_SPEED),
                Vec4::new(0.28, 0.32 * foam_scale, 0.12, 0.95),
                false,
            ),
            MaterialStyle::Fall => (
                Vec2::new(0.0, FALL_FLOW_SPEED),
                Vec4::new(0.34, 0.48 * foam_scale, 0.14, 1.25),
                true,
            ),
        };
        let emission = match (role, style) {
            (FillMaterialRole::Water, _) => Vec4::ZERO,
            (FillMaterialRole::Lava, MaterialStyle::Still) => {
                Vec4::new(0.20, 0.10, LAVA_STILL_PULSE_RATE, 0.0)
            }
            (FillMaterialRole::Lava, MaterialStyle::Current) => {
                Vec4::new(0.26, 0.10, LAVA_CURRENT_PULSE_RATE, 0.0)
            }
            (FillMaterialRole::Lava, MaterialStyle::Rapid) => {
                Vec4::new(0.34, 0.14, LAVA_RAPID_PULSE_RATE, 0.0)
            }
            (FillMaterialRole::Lava, MaterialStyle::Fall) => {
                Vec4::new(0.52, 0.18, LAVA_FALL_PULSE_RATE, 0.0)
            }
        };
        Self {
            flow_velocity,
            modulation,
            emission,
            double_sided,
        }
    }
}

#[derive(Debug, Clone)]
struct MaterialSet {
    role: FillMaterialRole,
    still: Handle<LiquidMaterial>,
    current: Handle<LiquidMaterial>,
    rapid: Handle<LiquidMaterial>,
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
            still: add(MaterialStyle::Still),
            current: add(MaterialStyle::Current),
            rapid: add(MaterialStyle::Rapid),
            fall: add(MaterialStyle::Fall),
        }
    }

    fn extend_registry(&self, registry: &mut Vec<Handle<LiquidMaterial>>) {
        registry.extend([
            self.still.clone(),
            self.current.clone(),
            self.rapid.clone(),
            self.fall.clone(),
        ]);
    }

    fn handle(&self, style: MaterialStyle) -> Handle<LiquidMaterial> {
        match style {
            MaterialStyle::Still => self.still.clone(),
            MaterialStyle::Current => self.current.clone(),
            MaterialStyle::Rapid => self.rapid.clone(),
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
            perceptual_roughness: 0.28,
            reflectance: 0.72,
            alpha_mode: AlphaMode::Opaque,
            opaque_render_method: OpaqueRendererMethod::Forward,
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

#[derive(Debug, Clone, Copy)]
struct FallGeometry {
    source: TilePos,
    downstream: TilePos,
    side: HexSide,
}

fn curtain_geometry(
    falls: &[FallGeometry],
    level_height: f32,
) -> Result<RawMesh, LiquidPresentationError> {
    let mut mesh = RawMesh::default();
    for fall in falls {
        let base = u32::try_from(mesh.positions.len())
            .map_err(|_error| LiquidPresentationError::MeshIndexOverflow)?;
        let top_y = surface_y(fall.source.level, level_height);
        let bottom_y = surface_y(fall.downstream.level, level_height);
        let height = top_y - bottom_y;
        let rotation = side_rotation(fall.side);
        let center = fall.source.coord.to_world(0.0);
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
    falls: &[FallGeometry],
    level_height: f32,
) -> Result<(), LiquidPresentationError> {
    for fall in falls {
        let base = u32::try_from(mesh.positions.len())
            .map_err(|_error| LiquidPresentationError::MeshIndexOverflow)?;
        let rotation = side_rotation(fall.side);
        let center = fall.downstream.coord.to_world(0.0);
        let y = surface_y(fall.downstream.level, level_height) + LAVA_SPLASH_SURFACE_BIAS;
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

fn cap_transform(position: TilePos, direction: Option<HexSide>, level_height: f32) -> Transform {
    let translation = position
        .coord
        .to_world(surface_y(position.level, level_height));
    let rotation = direction.map_or(Quat::IDENTITY, side_rotation);
    Transform::from_translation(translation).with_rotation(rotation)
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
            CURRENT_FLOW_SPEED,
            RAPID_FLOW_SPEED,
            FALL_FLOW_SPEED,
            LAVA_STILL_PULSE_RATE,
            LAVA_CURRENT_PULSE_RATE,
            LAVA_RAPID_PULSE_RATE,
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

        assert_eq!(handles.len(), 4);
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
    fn cap_mesh_has_upward_triangles_and_downstream_uv_axis() {
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
        let Some([_west_cross, west]) = mesh.uvs.get(3).copied() else {
            unreachable!("cap contains its west UV")
        };
        let Some([_east_cross, east]) = mesh.uvs.get(5).copied() else {
            unreachable!("cap contains its east UV")
        };
        assert!(west < east, "local +V must point East/downstream");
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
            &[FallGeometry {
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
    fn lava_profiles_pulse_slowly_when_still_and_brighten_when_falling() {
        let water = LiquidMaterialProfile::new(FillMaterialRole::Water, MaterialStyle::Still);
        assert_eq!(water.emission, Vec4::ZERO);

        let still = LiquidMaterialProfile::new(FillMaterialRole::Lava, MaterialStyle::Still);
        let fall = LiquidMaterialProfile::new(FillMaterialRole::Lava, MaterialStyle::Fall);
        assert_eq!(still.flow_velocity, Vec2::ZERO);
        assert_f32_near(still.emission.z, LAVA_STILL_PULSE_RATE);
        assert!(still.emission.y > 0.0);
        assert_f32_near(fall.flow_velocity.y, FALL_FLOW_SPEED);
        assert!(fall.flow_velocity.y > RAPID_FLOW_SPEED);
        assert!(fall.emission.x > still.emission.x);
        assert!(fall.emission.y > still.emission.y);
        assert!(fall.emission.z > still.emission.z);
        assert!(fall.double_sided);
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

        let water = build_fall_meshes(&surfaces(FillMaterialRole::Water), 0.4)
            .expect("water curtain should remain valid");
        let water = water
            .get(&FillMaterialRole::Water)
            .expect("water curtain should exist");
        assert_eq!(water.positions.len(), 4);
        assert_eq!(water.indices.len(), 6);

        let first = build_fall_meshes(&surfaces(FillMaterialRole::Lava), 0.4)
            .expect("lava fall effect should be valid");
        let second = build_fall_meshes(&surfaces(FillMaterialRole::Lava), 0.4)
            .expect("lava fall effect should be repeatable");
        assert_eq!(first, second);
        let lava = first
            .get(&FillMaterialRole::Lava)
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
            build_fall_meshes(&[source], 0.4),
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
        assert!(plan.falls.is_empty());
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
