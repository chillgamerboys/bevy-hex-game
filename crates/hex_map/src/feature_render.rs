//! Shared low-poly presentation for generated surface features.
//!
//! Feature placement and traversal effects remain semantic V3 map data. This
//! module only turns the retained exact roots into non-pickable tree and grass
//! entities. Programmatic meshes and materials are shared by every instance and
//! visual variation is a pure function of stable feature identity.

use std::collections::BTreeSet;
use std::f32::consts::{PI, TAU};
use std::fmt;

use bevy::{
    asset::RenderAssetUsages,
    light::NotShadowCaster,
    mesh::Indices,
    prelude::*,
    render::render_resource::{Face, PrimitiveTopology},
};
use hex_assets::{to_color, Rgb, SubstanceTable};
use hex_core::config::HEX_CIRCUMRADIUS;
#[cfg(test)]
use hex_core::TilePos;
use hex_core::{CanopyOccluder, PresentationOcclusion};

use crate::procedural_v3::{FeatureId, FeatureKind, MapPresentationProjection, PlannedFeature};

const TRUNK_RADIUS: f32 = 0.16 * HEX_CIRCUMRADIUS;
const TRUNK_HEIGHT_LEVELS: f32 = 4.5;
const CANOPY_RADIUS: f32 = 0.80 * HEX_CIRCUMRADIUS;
const CANOPY_HALF_HEIGHT_LEVELS: f32 = 2.5;
const CANOPY_CENTRE_LEVELS: f32 = 5.625;
const TALL_TRUNK_HEIGHT_LEVELS: f32 = 7.5;
const TALL_CANOPY_RADIUS: f32 = 1.05 * HEX_CIRCUMRADIUS;
const TALL_CANOPY_HALF_HEIGHT_LEVELS: f32 = 3.2;
const TALL_CANOPY_CENTRE_LEVELS: f32 = 9.5;
const TALL_TREE_EXEMPLAR_LIMIT: usize = 3;
const GRASS_RADIUS: f32 = 0.325 * HEX_CIRCUMRADIUS;
const GRASS_HEIGHT_LEVELS: f32 = 1.875;
const VISUAL_ROTATION_STEPS: u32 = 24;
const VISUAL_SCALE_STEPS: u32 = 9;

const TRUNK_SWATCH: &str = "plant/trunk";
const CANOPY_SWATCHES: [&str; 3] = [
    "plant/foliage-dark",
    "plant/foliage-mid",
    "plant/foliage-light",
];
const GRASS_SWATCHES: [&str; 2] = ["plant/grass-dark", "plant/grass-light"];
const FEATURE_SWATCHES: [&str; 6] = [
    TRUNK_SWATCH,
    CANOPY_SWATCHES[0],
    CANOPY_SWATCHES[1],
    CANOPY_SWATCHES[2],
    GRASS_SWATCHES[0],
    GRASS_SWATCHES[1],
];

/// Shared renderer-owned handles for every generated surface feature.
///
/// The map plugin initializes the three meshes once. The first generated feature
/// presentation resolves the accepted palette snapshot into six shared materials;
/// later rebuilds reuse those handles and update them in place if the accepted
/// palette changed.
#[derive(Resource, Clone, Debug)]
pub(crate) struct FeaturePresentationAssets {
    trunk_mesh: Handle<Mesh>,
    canopy_mesh: Handle<Mesh>,
    grass_mesh: Handle<Mesh>,
    materials: Option<FeatureMaterials>,
}

#[derive(Clone, Debug)]
struct FeatureMaterials {
    palette: FeaturePalette,
    trunk_material: Handle<StandardMaterial>,
    canopy_materials: [Handle<StandardMaterial>; 3],
    grass_materials: [Handle<StandardMaterial>; 2],
}

impl FromWorld for FeaturePresentationAssets {
    fn from_world(world: &mut World) -> Self {
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        let (trunk_mesh, canopy_mesh, grass_mesh) = {
            let mut meshes = world.resource_mut::<Assets<Mesh>>();
            (
                meshes.add(trunk_geometry().into_mesh()),
                meshes.add(canopy_geometry().into_mesh()),
                meshes.add(grass_geometry().into_mesh()),
            )
        };
        Self {
            trunk_mesh,
            canopy_mesh,
            grass_mesh,
            materials: None,
        }
    }
}

/// Creates the shared programmatic assets used by [`spawn_presentations`].
pub(crate) fn register_assets(app: &mut App) {
    app.init_resource::<FeaturePresentationAssets>();
}

/// Resolves every Forest presentation swatch before any map entities are queued.
///
/// This uses the palette snapshot accepted with [`SubstanceTable`], so a rejected
/// cross-file reload cannot mix new colours with the previous substance semantics.
pub(crate) fn prepare_materials(
    assets: &mut FeaturePresentationAssets,
    materials: &mut Assets<StandardMaterial>,
    table: &SubstanceTable,
    projection: Option<&MapPresentationProjection>,
) -> Result<(), FeaturePresentationError> {
    let Some(projection) = projection else {
        return Ok(());
    };
    if projection.features().is_empty() {
        return Ok(());
    }
    assets.apply_palette(FeaturePalette::resolve(table)?, materials)
}

impl FeaturePresentationAssets {
    fn apply_palette(
        &mut self,
        palette: FeaturePalette,
        materials: &mut Assets<StandardMaterial>,
    ) -> Result<(), FeaturePresentationError> {
        let Some(shared) = self.materials.as_mut() else {
            self.materials = Some(FeatureMaterials::create(palette, materials));
            return Ok(());
        };
        if shared.palette == palette {
            return Ok(());
        }
        shared.update(palette, materials)
    }
}

impl FeatureMaterials {
    fn create(palette: FeaturePalette, materials: &mut Assets<StandardMaterial>) -> Self {
        Self {
            palette,
            trunk_material: materials.add(feature_material(to_color(palette.trunk), false)),
            canopy_materials: palette
                .canopy
                .map(|color| materials.add(feature_material(to_color(color), false))),
            grass_materials: palette
                .grass
                .map(|color| materials.add(feature_material(to_color(color), true))),
        }
    }

    fn update(
        &mut self,
        palette: FeaturePalette,
        materials: &mut Assets<StandardMaterial>,
    ) -> Result<(), FeaturePresentationError> {
        update_material(
            materials,
            &self.trunk_material,
            palette.trunk,
            false,
            TRUNK_SWATCH,
        )?;
        for ((handle, color), swatch) in self
            .canopy_materials
            .iter()
            .zip(palette.canopy)
            .zip(CANOPY_SWATCHES)
        {
            update_material(materials, handle, color, false, swatch)?;
        }
        for ((handle, color), swatch) in self
            .grass_materials
            .iter()
            .zip(palette.grass)
            .zip(GRASS_SWATCHES)
        {
            update_material(materials, handle, color, true, swatch)?;
        }
        self.palette = palette;
        Ok(())
    }
}

fn update_material(
    materials: &mut Assets<StandardMaterial>,
    handle: &Handle<StandardMaterial>,
    color: Rgb,
    double_sided: bool,
    swatch: &'static str,
) -> Result<(), FeaturePresentationError> {
    let Some(mut material) = materials.get_mut(handle) else {
        return Err(FeaturePresentationError::MissingSharedMaterial { swatch });
    };
    *material = feature_material(to_color(color), double_sided);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct FeaturePalette {
    trunk: Rgb,
    canopy: [Rgb; 3],
    grass: [Rgb; 2],
}

impl FeaturePalette {
    fn resolve(table: &SubstanceTable) -> Result<Self, FeaturePresentationError> {
        Ok(Self {
            trunk: required_palette_color(table, TRUNK_SWATCH)?,
            canopy: [
                required_palette_color(table, CANOPY_SWATCHES[0])?,
                required_palette_color(table, CANOPY_SWATCHES[1])?,
                required_palette_color(table, CANOPY_SWATCHES[2])?,
            ],
            grass: [
                required_palette_color(table, GRASS_SWATCHES[0])?,
                required_palette_color(table, GRASS_SWATCHES[1])?,
            ],
        })
    }
}

fn required_palette_color(
    table: &SubstanceTable,
    swatch: &'static str,
) -> Result<Rgb, FeaturePresentationError> {
    table
        .palette_color(swatch)
        .ok_or(FeaturePresentationError::MissingPaletteSwatch { swatch })
}

/// Private identity on the transform root of one generated feature.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GeneratedFeatureRoot {
    pub(crate) id: FeatureId,
    pub(crate) kind: FeatureKind,
}

/// Renderer-private tree silhouette; semantic roots remain [`FeatureKind::Tree`].
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
enum TreePresentationArchetype {
    Standard,
    Tall,
}

/// Failure to turn retained feature semantics into finite world transforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FeaturePresentationError {
    InvalidLevelHeight,
    NonFiniteTransform { id: FeatureId },
    MissingPaletteSwatch { swatch: &'static str },
    MissingSharedMaterial { swatch: &'static str },
}

impl fmt::Display for FeaturePresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLevelHeight => {
                write!(
                    formatter,
                    "feature presentation level height must be positive and finite"
                )
            }
            Self::NonFiniteTransform { id } => {
                write!(
                    formatter,
                    "feature {id:?} produced a non-finite presentation transform"
                )
            }
            Self::MissingPaletteSwatch { swatch } => {
                write!(
                    formatter,
                    "feature presentation requires missing accepted palette swatch {swatch:?}"
                )
            }
            Self::MissingSharedMaterial { swatch } => {
                write!(
                    formatter,
                    "feature presentation lost the shared material for palette swatch {swatch:?}"
                )
            }
        }
    }
}

impl std::error::Error for FeaturePresentationError {}

/// Spawns generated feature roots and their mesh-bearing children.
///
/// Returned root entities are intended to become direct children of `HexGrid`.
/// Relationship-linked despawning then removes their visual descendants on every
/// terrain rebuild and gameplay teardown.
pub(crate) fn spawn_presentations(
    commands: &mut Commands,
    assets: &FeaturePresentationAssets,
    level_height: f32,
    projection: Option<&MapPresentationProjection>,
) -> Result<Vec<Entity>, FeaturePresentationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(FeaturePresentationError::InvalidLevelHeight);
    }
    let Some(projection) = projection else {
        return Ok(Vec::new());
    };
    if projection.features().is_empty() {
        return Ok(Vec::new());
    }
    let Some(materials) = assets.materials.as_ref() else {
        return Err(FeaturePresentationError::MissingSharedMaterial {
            swatch: FEATURE_SWATCHES[0],
        });
    };

    let tall_tree_ids = tall_tree_exemplar_ids(projection);
    let mut roots = Vec::with_capacity(projection.features().len());
    for (id, feature) in projection.features() {
        let instance = FeatureInstance::new(*id, *feature, level_height)?;
        let root = commands
            .spawn((
                instance.root_transform,
                Visibility::default(),
                GeneratedFeatureRoot {
                    id: instance.id,
                    kind: instance.feature.kind,
                },
                Name::new(match instance.feature.kind {
                    FeatureKind::Tree => "GeneratedTree",
                    FeatureKind::TallGrass => "GeneratedTallGrass",
                }),
            ))
            .id();

        match instance.feature.kind {
            FeatureKind::Tree => {
                let archetype = if tall_tree_ids.contains(&instance.id) {
                    TreePresentationArchetype::Tall
                } else {
                    TreePresentationArchetype::Standard
                };
                commands.entity(root).insert(archetype);
                let canopy_material = materials
                    .canopy_materials
                    .get(
                        instance
                            .variant
                            .material_index(materials.canopy_materials.len()),
                    )
                    .cloned()
                    .ok_or(FeaturePresentationError::MissingSharedMaterial {
                        swatch: CANOPY_SWATCHES[0],
                    })?;
                let trunk = commands
                    .spawn((
                        Mesh3d(assets.trunk_mesh.clone()),
                        MeshMaterial3d(materials.trunk_material.clone()),
                        tree_trunk_transform(level_height, archetype),
                        Pickable::IGNORE,
                        PresentationOcclusion::default(),
                        CanopyOccluder(instance.feature.root),
                        Name::new("TreeTrunk"),
                    ))
                    .id();
                let canopy = commands
                    .spawn((
                        Mesh3d(assets.canopy_mesh.clone()),
                        MeshMaterial3d(canopy_material),
                        tree_canopy_transform(level_height, archetype),
                        Pickable::IGNORE,
                        PresentationOcclusion::default(),
                        CanopyOccluder(instance.feature.root),
                        Name::new("TreeCanopy"),
                    ))
                    .id();
                commands.entity(root).add_children(&[trunk, canopy]);
            }
            FeatureKind::TallGrass => {
                let grass_material = materials
                    .grass_materials
                    .get(
                        instance
                            .variant
                            .material_index(materials.grass_materials.len()),
                    )
                    .cloned()
                    .ok_or(FeaturePresentationError::MissingSharedMaterial {
                        swatch: GRASS_SWATCHES[0],
                    })?;
                let tuft = commands
                    .spawn((
                        Mesh3d(assets.grass_mesh.clone()),
                        MeshMaterial3d(grass_material),
                        grass_transform(level_height),
                        Pickable::IGNORE,
                        NotShadowCaster,
                        Name::new("TallGrassTuft"),
                    ))
                    .id();
                commands.entity(root).add_child(tuft);
            }
        }
        roots.push(root);
    }
    Ok(roots)
}

fn tall_tree_exemplar_ids(projection: &MapPresentationProjection) -> BTreeSet<FeatureId> {
    let mut ranked = projection
        .features()
        .iter()
        .filter(|(_, feature)| feature.kind == FeatureKind::Tree)
        .map(|(id, feature)| (VisualVariant::for_feature(*id, *feature).hash, *id))
        .collect::<Vec<_>>();
    ranked.sort_unstable();
    ranked
        .into_iter()
        .take(TALL_TREE_EXEMPLAR_LIMIT)
        .map(|(_, id)| id)
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct FeatureInstance {
    id: FeatureId,
    feature: PlannedFeature,
    variant: VisualVariant,
    root_transform: Transform,
}

impl FeatureInstance {
    fn new(
        id: FeatureId,
        feature: PlannedFeature,
        level_height: f32,
    ) -> Result<Self, FeaturePresentationError> {
        let variant = VisualVariant::for_feature(id, feature);
        let root_transform = Transform {
            translation: feature.root.coord.to_world(level_to_world_height(
                feature.root.level.saturating_add(1),
                level_height,
            )),
            rotation: Quat::from_rotation_y(variant.rotation()),
            scale: Vec3::splat(variant.scale()),
        };
        if !root_transform.translation.is_finite()
            || !root_transform.rotation.is_finite()
            || !root_transform.scale.is_finite()
        {
            return Err(FeaturePresentationError::NonFiniteTransform { id });
        }
        Ok(Self {
            id,
            feature,
            variant,
            root_transform,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VisualVariant {
    hash: u64,
}

impl VisualVariant {
    fn for_feature(id: FeatureId, feature: PlannedFeature) -> Self {
        let mut hash = 0x243f_6a88_85a3_08d3_u64;
        for value in [
            id.0,
            u32::from_le_bytes(feature.root.coord.x().to_le_bytes()),
            u32::from_le_bytes(feature.root.coord.y().to_le_bytes()),
            u32::from_le_bytes(feature.root.level.to_le_bytes()),
            match feature.kind {
                FeatureKind::Tree => 0,
                FeatureKind::TallGrass => 1,
            },
        ] {
            hash = mix64(hash ^ u64::from(value));
        }
        Self { hash }
    }

    fn material_index(self, count: usize) -> usize {
        let count = u64::try_from(count).unwrap_or(u64::MAX).max(1);
        usize::try_from((self.hash >> 16) % count).unwrap_or_default()
    }

    fn rotation(self) -> f32 {
        let step = u32::try_from(self.hash % u64::from(VISUAL_ROTATION_STEPS)).unwrap_or_default();
        #[expect(
            clippy::cast_precision_loss,
            reason = "the visual rotation uses only 24 exactly representable steps"
        )]
        {
            TAU * (step as f32) / (VISUAL_ROTATION_STEPS as f32)
        }
    }

    fn scale(self) -> f32 {
        let step =
            u32::try_from((self.hash >> 32) % u64::from(VISUAL_SCALE_STEPS)).unwrap_or_default();
        #[expect(
            clippy::cast_precision_loss,
            reason = "the visual scale uses only nine exactly representable steps"
        )]
        {
            0.92 + (step as f32) * 0.02
        }
    }
}

fn mix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn level_to_world_height(level: i32, level_height: f32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "playable voxel levels are small integers and exact in f32"
    )]
    {
        (level as f32) * level_height
    }
}

fn tree_trunk_transform(level_height: f32, archetype: TreePresentationArchetype) -> Transform {
    let height_levels = match archetype {
        TreePresentationArchetype::Standard => TRUNK_HEIGHT_LEVELS,
        TreePresentationArchetype::Tall => TALL_TRUNK_HEIGHT_LEVELS,
    };
    let height = height_levels * level_height;
    Transform::from_translation(Vec3::Y * height.mul_add(0.5, 0.0)).with_scale(Vec3::new(
        TRUNK_RADIUS,
        height,
        TRUNK_RADIUS,
    ))
}

fn tree_canopy_transform(level_height: f32, archetype: TreePresentationArchetype) -> Transform {
    let (radius, half_height_levels, centre_levels) = match archetype {
        TreePresentationArchetype::Standard => (
            CANOPY_RADIUS,
            CANOPY_HALF_HEIGHT_LEVELS,
            CANOPY_CENTRE_LEVELS,
        ),
        TreePresentationArchetype::Tall => (
            TALL_CANOPY_RADIUS,
            TALL_CANOPY_HALF_HEIGHT_LEVELS,
            TALL_CANOPY_CENTRE_LEVELS,
        ),
    };
    Transform::from_translation(Vec3::Y * (centre_levels * level_height)).with_scale(Vec3::new(
        radius,
        half_height_levels * level_height,
        radius,
    ))
}

fn grass_transform(level_height: f32) -> Transform {
    Transform::from_scale(Vec3::new(
        GRASS_RADIUS,
        GRASS_HEIGHT_LEVELS * level_height,
        GRASS_RADIUS,
    ))
}

fn feature_material(color: Color, double_sided: bool) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        perceptual_roughness: 0.9,
        reflectance: 0.18,
        alpha_mode: AlphaMode::Opaque,
        double_sided,
        cull_mode: if double_sided { None } else { Some(Face::Back) },
        ..default()
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

    #[cfg(test)]
    fn is_finite(&self) -> bool {
        self.positions
            .iter()
            .flatten()
            .chain(self.normals.iter().flatten())
            .chain(self.uvs.iter().flatten())
            .all(|component| component.is_finite())
    }

    fn push_triangle(&mut self, a: Vec3, b: Vec3, c: Vec3, normal: Vec3) {
        let base = u32::try_from(self.positions.len()).unwrap_or(u32::MAX.saturating_sub(2));
        self.positions
            .extend([a.to_array(), b.to_array(), c.to_array()]);
        self.normals.extend([normal.to_array(); 3]);
        self.uvs.extend([[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]]);
        self.indices.extend([base, base + 1, base + 2]);
    }

    fn push_outward_triangle(&mut self, a: Vec3, mut b: Vec3, mut c: Vec3) {
        let mut normal = (b - a).cross(c - a).normalize();
        if normal.dot((a + b + c) / 3.0) < 0.0 {
            std::mem::swap(&mut b, &mut c);
            normal = -normal;
        }
        self.push_triangle(a, b, c, normal);
    }
}

fn trunk_geometry() -> RawMesh {
    let mut mesh = RawMesh::default();
    let bottom = -0.5;
    let top = 0.5;
    for side in 0..6 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "the trunk has exactly six authored sides"
        )]
        let angle_a = (side as f32) * TAU / 6.0;
        #[expect(
            clippy::cast_precision_loss,
            reason = "the trunk has exactly six authored sides"
        )]
        let angle_b = ((side + 1) as f32) * TAU / 6.0;
        let a0 = Vec3::new(angle_a.cos(), bottom, angle_a.sin());
        let a1 = Vec3::new(angle_a.cos(), top, angle_a.sin());
        let b0 = Vec3::new(angle_b.cos(), bottom, angle_b.sin());
        let b1 = Vec3::new(angle_b.cos(), top, angle_b.sin());
        let normal = Vec3::new(
            ((angle_a + angle_b) * 0.5).cos(),
            0.0,
            ((angle_a + angle_b) * 0.5).sin(),
        );
        mesh.push_triangle(a0, b0, b1, normal);
        mesh.push_triangle(a0, b1, a1, normal);
        mesh.push_triangle(Vec3::Y * top, a1, b1, Vec3::Y);
        mesh.push_triangle(Vec3::Y * bottom, b0, a0, Vec3::NEG_Y);
    }
    mesh
}

fn canopy_geometry() -> RawMesh {
    let phi = (1.0 + 5.0_f32.sqrt()) * 0.5;
    let vertices = [
        Vec3::new(-1.0, phi, 0.0),
        Vec3::new(1.0, phi, 0.0),
        Vec3::new(-1.0, -phi, 0.0),
        Vec3::new(1.0, -phi, 0.0),
        Vec3::new(0.0, -1.0, phi),
        Vec3::new(0.0, 1.0, phi),
        Vec3::new(0.0, -1.0, -phi),
        Vec3::new(0.0, 1.0, -phi),
        Vec3::new(phi, 0.0, -1.0),
        Vec3::new(phi, 0.0, 1.0),
        Vec3::new(-phi, 0.0, -1.0),
        Vec3::new(-phi, 0.0, 1.0),
    ]
    .map(Vec3::normalize);
    let faces = [
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    let mut mesh = RawMesh::default();
    for [a, b, c] in faces {
        let (Some(a), Some(b), Some(c)) = (vertices.get(a), vertices.get(b), vertices.get(c))
        else {
            continue;
        };
        mesh.push_outward_triangle(*a, *b, *c);
    }
    mesh
}

fn grass_geometry() -> RawMesh {
    let mut mesh = RawMesh::default();
    for angle in [0.0, PI * 0.5] {
        let direction = Vec3::new(angle.cos(), 0.0, angle.sin());
        let bottom_left = -direction;
        let bottom_right = direction;
        let top_right = direction * 0.62 + Vec3::Y;
        let top_left = -direction * 0.62 + Vec3::Y;
        let normal = direction.cross(Vec3::Y).normalize();
        mesh.push_triangle(bottom_left, bottom_right, top_right, normal);
        mesh.push_triangle(bottom_left, top_right, top_left, normal);
    }
    mesh
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use bevy::ecs::world::CommandQueue;
    use bevy::platform::collections::HashMap;
    use hex_assets::{ArtPalette, PaletteSwatch, SrgbColor, Substance, SubstanceFile, SwatchId};
    use hex_core::HexCoord;

    use super::*;

    fn feature_table(omitted: Option<&str>, trunk: Rgb) -> SubstanceTable {
        let colors = [
            (TRUNK_SWATCH, trunk),
            (CANOPY_SWATCHES[0], (0.12, 0.34, 0.12)),
            (CANOPY_SWATCHES[1], (0.18, 0.42, 0.14)),
            (CANOPY_SWATCHES[2], (0.25, 0.48, 0.16)),
            (GRASS_SWATCHES[0], (0.34, 0.52, 0.14)),
            (GRASS_SWATCHES[1], (0.45, 0.62, 0.18)),
        ];
        let swatches = colors
            .into_iter()
            .filter(|(id, _color)| omitted != Some(*id))
            .map(|(id, (red, green, blue))| {
                (
                    SwatchId::new(id).expect("fixture swatch ids should be valid"),
                    PaletteSwatch::new(
                        format!("Fixture {id}"),
                        SrgbColor::new(red, green, blue)
                            .expect("fixture swatch colors should be valid"),
                        BTreeSet::from(["test".to_owned()]),
                    )
                    .expect("fixture swatches should be valid"),
                )
            })
            .collect();
        let palette = ArtPalette::new(swatches).expect("fixture palette should be valid");
        let substances = HashMap::from([("air".to_owned(), Substance::invisible(false, false))]);
        SubstanceTable::from_file(&SubstanceFile { substances }, &palette)
            .expect("fixture substances should accept the fixture palette")
    }

    fn feature(x: i32, y: i32, level: i32, kind: FeatureKind) -> PlannedFeature {
        PlannedFeature {
            root: TilePos::new(HexCoord::from_axial(x, y), level),
            kind,
        }
    }

    #[test]
    fn shared_assets_are_bounded_and_initialization_is_idempotent() {
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();

        world.init_resource::<FeaturePresentationAssets>();
        let first = world.resource::<FeaturePresentationAssets>().clone();
        world.init_resource::<FeaturePresentationAssets>();
        let repeated = world.resource::<FeaturePresentationAssets>();

        assert_eq!(world.resource::<Assets<Mesh>>().len(), 3);
        assert_eq!(world.resource::<Assets<StandardMaterial>>().len(), 0);
        assert_eq!(first.trunk_mesh.id(), repeated.trunk_mesh.id());
        assert_eq!(first.canopy_mesh.id(), repeated.canopy_mesh.id());
        assert_eq!(first.grass_mesh.id(), repeated.grass_mesh.id());
    }

    #[test]
    fn accepted_palette_drives_six_bounded_shared_materials() {
        let projection = MapPresentationProjection::with_test_features([(
            FeatureId(0),
            feature(0, 0, 15, FeatureKind::Tree),
        )]);
        let mut world = World::new();
        world.init_resource::<FeaturePresentationAssets>();
        let mut assets = world.resource::<FeaturePresentationAssets>().clone();
        let original = feature_table(None, (0.28, 0.15, 0.07));
        {
            let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
            prepare_materials(&mut assets, &mut materials, &original, Some(&projection))
                .expect("the complete accepted palette should prepare feature materials");
        }
        let shared = assets
            .materials
            .as_ref()
            .expect("preparation should install shared materials");
        let trunk_id = shared.trunk_material.id();
        assert_eq!(world.resource::<Assets<StandardMaterial>>().len(), 6);
        assert_eq!(
            world
                .resource::<Assets<StandardMaterial>>()
                .get(&shared.trunk_material)
                .expect("the shared trunk material should exist")
                .base_color,
            to_color(
                original
                    .palette_color(TRUNK_SWATCH)
                    .expect("the fixture should retain its trunk swatch")
            )
        );

        let changed = feature_table(None, (0.71, 0.22, 0.09));
        {
            let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
            prepare_materials(&mut assets, &mut materials, &changed, Some(&projection))
                .expect("a new accepted palette should update shared materials");
            prepare_materials(&mut assets, &mut materials, &changed, Some(&projection))
                .expect("repeated preparation should be idempotent");
        }
        let shared = assets
            .materials
            .as_ref()
            .expect("the updated materials should remain installed");
        assert_eq!(shared.trunk_material.id(), trunk_id);
        assert_eq!(world.resource::<Assets<StandardMaterial>>().len(), 6);
        assert_eq!(
            world
                .resource::<Assets<StandardMaterial>>()
                .get(&shared.trunk_material)
                .expect("the updated trunk material should exist")
                .base_color,
            to_color(
                changed
                    .palette_color(TRUNK_SWATCH)
                    .expect("the changed fixture should retain its trunk swatch")
            )
        );
    }

    #[test]
    fn every_required_feature_swatch_fails_before_material_allocation() {
        let projection = MapPresentationProjection::with_test_features([(
            FeatureId(0),
            feature(0, 0, 15, FeatureKind::Tree),
        )]);
        for missing in FEATURE_SWATCHES {
            let mut world = World::new();
            world.init_resource::<FeaturePresentationAssets>();
            let mut assets = world.resource::<FeaturePresentationAssets>().clone();
            let table = feature_table(Some(missing), (0.28, 0.15, 0.07));
            let error = {
                let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
                prepare_materials(&mut assets, &mut materials, &table, Some(&projection))
                    .expect_err("a missing required swatch should reject feature presentation")
            };

            assert_eq!(
                error,
                FeaturePresentationError::MissingPaletteSwatch { swatch: missing }
            );
            assert_eq!(
                world.resource::<Assets<StandardMaterial>>().len(),
                0,
                "missing {missing} allocated a partial material set"
            );
        }
    }

    #[test]
    fn visual_variation_is_stable_and_uses_exact_identity() {
        let tree = feature(-3, 7, 15, FeatureKind::Tree);
        let first = VisualVariant::for_feature(FeatureId(4), tree);
        let repeated = VisualVariant::for_feature(FeatureId(4), tree);
        let moved = VisualVariant::for_feature(FeatureId(4), feature(-2, 7, 15, FeatureKind::Tree));
        let renumbered = VisualVariant::for_feature(FeatureId(5), tree);

        assert_eq!(first, repeated);
        assert_ne!(first, moved);
        assert_ne!(first, renumbered);
        assert!((0.92..=1.08).contains(&first.scale()));
        assert!((0.0..TAU).contains(&first.rotation()));
    }

    #[test]
    fn tall_tree_selection_is_bounded_stable_and_tree_only() {
        let features = [
            (FeatureId(8), feature(-3, 1, 15, FeatureKind::Tree)),
            (FeatureId(2), feature(-2, 1, 15, FeatureKind::TallGrass)),
            (FeatureId(7), feature(-1, 1, 15, FeatureKind::Tree)),
            (FeatureId(6), feature(0, 1, 15, FeatureKind::Tree)),
            (FeatureId(5), feature(1, 1, 15, FeatureKind::Tree)),
            (FeatureId(4), feature(2, 1, 15, FeatureKind::Tree)),
        ];
        let forward = MapPresentationProjection::with_test_features(features);
        let reverse = MapPresentationProjection::with_test_features(features.into_iter().rev());

        let selected = tall_tree_exemplar_ids(&forward);
        assert_eq!(selected.len(), 3);
        assert_eq!(selected, tall_tree_exemplar_ids(&reverse));
        assert!(!selected.contains(&FeatureId(2)));

        let two_trees = MapPresentationProjection::with_test_features([
            (FeatureId(0), feature(0, 0, 15, FeatureKind::Tree)),
            (FeatureId(1), feature(1, 0, 15, FeatureKind::Tree)),
        ]);
        assert_eq!(tall_tree_exemplar_ids(&two_trees).len(), 2);
    }

    #[test]
    fn tall_tree_archetype_is_finite_and_visibly_taller() {
        let standard_trunk = tree_trunk_transform(0.4, TreePresentationArchetype::Standard);
        let tall_trunk = tree_trunk_transform(0.4, TreePresentationArchetype::Tall);
        let standard_canopy = tree_canopy_transform(0.4, TreePresentationArchetype::Standard);
        let tall_canopy = tree_canopy_transform(0.4, TreePresentationArchetype::Tall);

        for transform in [standard_trunk, tall_trunk, standard_canopy, tall_canopy] {
            assert!(transform.translation.is_finite());
            assert!(transform.rotation.is_finite());
            assert!(transform.scale.is_finite());
        }
        assert!(tall_trunk.scale.y > standard_trunk.scale.y);
        assert!(tall_canopy.translation.y > standard_canopy.translation.y);
        assert!(tall_canopy.scale.y > standard_canopy.scale.y);
    }

    #[test]
    fn spawned_tall_exemplars_preserve_tree_roots_and_canopy_occlusion() {
        let projection = MapPresentationProjection::with_test_features([
            (FeatureId(0), feature(-2, 0, 15, FeatureKind::Tree)),
            (FeatureId(1), feature(-1, 0, 15, FeatureKind::Tree)),
            (FeatureId(2), feature(0, 0, 15, FeatureKind::Tree)),
            (FeatureId(3), feature(1, 0, 15, FeatureKind::Tree)),
            (FeatureId(4), feature(2, 0, 15, FeatureKind::Tree)),
            (FeatureId(5), feature(3, 0, 15, FeatureKind::TallGrass)),
        ]);
        let expected_tall = tall_tree_exemplar_ids(&projection);
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<FeaturePresentationAssets>();
        let mut assets = world.resource::<FeaturePresentationAssets>().clone();
        let table = feature_table(None, (0.28, 0.15, 0.07));
        {
            let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
            prepare_materials(&mut assets, &mut materials, &table, Some(&projection))
                .expect("the fixture palette should prepare feature materials");
        }
        let mut queue = CommandQueue::default();
        let roots = {
            let mut commands = Commands::new(&mut queue, &world);
            spawn_presentations(&mut commands, &assets, 0.4, Some(&projection))
                .expect("feature presentation should spawn")
        };
        queue.apply(&mut world);

        assert_eq!(roots.len(), 6);
        let mut tree_roots =
            world.query::<(&GeneratedFeatureRoot, &TreePresentationArchetype, &Name)>();
        let presented = tree_roots
            .iter(&world)
            .map(|(root, archetype, name)| {
                assert_eq!(root.kind, FeatureKind::Tree);
                assert_eq!(name.as_str(), "GeneratedTree");
                (root.id, *archetype)
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(presented.len(), 5);
        assert_eq!(
            presented
                .iter()
                .filter(|(_, archetype)| **archetype == TreePresentationArchetype::Tall)
                .map(|(id, _)| *id)
                .collect::<BTreeSet<_>>(),
            expected_tall
        );

        let mut canopies = world.query::<(&CanopyOccluder, &PresentationOcclusion, &Name)>();
        assert_eq!(
            canopies
                .iter(&world)
                .filter(|(_, _, name)| name.as_str() == "TreeCanopy")
                .count(),
            5
        );
        assert_eq!(
            canopies
                .iter(&world)
                .filter(|(_, _, name)| name.as_str() == "TreeTrunk")
                .count(),
            5,
            "tree trunks must participate in the same local obstruction cutaway"
        );
    }

    #[test]
    fn shared_feature_assets_initialize_their_bevy_asset_stores() {
        let mut world = World::new();
        world.init_resource::<FeaturePresentationAssets>();

        assert!(world.contains_resource::<Assets<Mesh>>());
        assert!(world.contains_resource::<Assets<StandardMaterial>>());
    }

    #[test]
    fn roots_use_the_exact_surface_top_height() {
        let root = feature(2, -1, 15, FeatureKind::TallGrass);
        let instance =
            FeatureInstance::new(FeatureId(7), root, 0.4).expect("the transform is finite");
        let expected = root.root.coord.to_world(6.4);

        assert_eq!(instance.root_transform.translation, expected);
        assert!(instance.root_transform.scale.x >= 0.92);
        let zero_height = FeatureInstance::new(FeatureId(7), root, 0.0)
            .expect("pure instance construction assumes validated settings")
            .root_transform
            .translation
            .y;
        assert!(zero_height.abs() <= f32::EPSILON);
    }

    #[test]
    fn feature_meshes_are_finite_low_poly_geometry() {
        let trunk = trunk_geometry();
        let canopy = canopy_geometry();
        let grass = grass_geometry();

        assert!(trunk.is_finite());
        assert!(canopy.is_finite());
        assert!(grass.is_finite());
        assert_eq!(trunk.indices.len(), 72);
        assert_eq!(canopy.indices.len(), 60);
        assert_eq!(grass.indices.len(), 12);
    }

    #[test]
    fn invalid_level_height_is_rejected_before_spawning() {
        let projection = MapPresentationProjection::with_test_features(BTreeMap::from([(
            FeatureId(0),
            feature(0, 0, 0, FeatureKind::Tree),
        )]));
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<FeaturePresentationAssets>();
        let assets = world.resource::<FeaturePresentationAssets>().clone();
        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);

        assert_eq!(
            spawn_presentations(&mut commands, &assets, f32::NAN, Some(&projection)),
            Err(FeaturePresentationError::InvalidLevelHeight)
        );
    }
}
