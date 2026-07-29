//! Materialization boundary for validated procedural V3 worlds.
//!
//! Candidate planning remains semantic and ordered. This module is the only place
//! where an admitted plan becomes voxel storage and public runtime resources. It
//! retains map-owned presentation descriptors in ordered collections for later
//! terrain spawning.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use bevy::prelude::Resource;
use hex_core::{
    BiomeRegionId, BiomeRegions, Headroom, HexCoord, IlluminationLevel, InteriorRegionId,
    InteriorRegions, MapAnchorId, MapAnchors, MapViewHint, SpecialMovementRegion,
    SpecialMovementRegions, SubstanceId, TilePos, TraversalBlockers, TraversalProfile,
};

use super::fingerprint::{
    encode_light_presentation, semantic_plan_fingerprint, FingerprintEncoder,
};
use super::liquid::{LiquidFlowState, LiquidPlan};
use super::selection::ValidatedWorldPlan;
use super::volume::{
    FillMaterialRole, MaterializedVolume, SurfaceAccess, VolumeMaterializationError,
};
use super::world::{
    FeatureId, FeatureKind, GeneratedWorldPlan, LightId, PlannedFeature, PlannedGameplayLight,
    PlannedStructure, StructureId, StructureKind,
};
use crate::terrain::TerrainPalette;
use crate::voxel::VoxelMap;

/// Runtime resources and deterministic identities produced from one admitted plan.
#[derive(Debug)]
pub(crate) struct MaterializedV3World {
    pub(crate) map: VoxelMap,
    pub(crate) anchors: MapAnchors,
    pub(crate) special_regions: SpecialMovementRegions,
    pub(crate) interiors: InteriorRegions,
    pub(crate) blockers: TraversalBlockers,
    pub(crate) biome_regions: BiomeRegions,
    pub(crate) view_hint: MapViewHint,
    pub(crate) semantic_fingerprint: u64,
    pub(crate) materialized_fingerprint: u64,
    pub(crate) presentation: MapPresentationProjection,
}

/// Ordered map-owned descriptors retained for later presentation spawning.
///
/// Gameplay receives only the public exact consequences on [`MaterializedV3World`].
/// These semantic descriptors remain private to the V3 map pipeline.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct MapPresentationProjection {
    liquids: BTreeMap<TilePos, MaterializedLiquidVoxel>,
    features: BTreeMap<FeatureId, PlannedFeature>,
    structures: BTreeMap<StructureId, PlannedStructure>,
    lights: BTreeMap<LightId, PlannedGameplayLight>,
}

impl MapPresentationProjection {
    /// Returns the exact ordered liquid presentation projection.
    #[must_use]
    pub(crate) const fn liquids(&self) -> &BTreeMap<TilePos, MaterializedLiquidVoxel> {
        &self.liquids
    }

    /// Returns exact surface features in stable map-local identity order.
    #[must_use]
    pub(crate) const fn features(&self) -> &BTreeMap<FeatureId, PlannedFeature> {
        &self.features
    }

    /// Returns generated gameplay-light descriptors in stable map-local order.
    #[must_use]
    pub(crate) const fn lights(&self) -> &BTreeMap<LightId, PlannedGameplayLight> {
        &self.lights
    }

    /// Retains feature presentations whose exact authored support remains valid.
    ///
    /// Terrain edits may remove presentation-only features such as tall grass.
    /// Blocking structures use the separate conservative edit guard instead.
    pub(crate) fn retain_features(&mut self, mut retain: impl FnMut(&PlannedFeature) -> bool) {
        self.features.retain(|_id, feature| retain(feature));
    }

    /// Iterates exact liquid voxels in deterministic [`TilePos`] order.
    #[cfg(test)]
    pub(crate) fn iter_liquids(
        &self,
    ) -> impl ExactSizeIterator<Item = (&TilePos, &MaterializedLiquidVoxel)> {
        self.liquids.iter()
    }

    /// Returns the presentation descriptor for one exact liquid voxel.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn liquid_at(&self, position: TilePos) -> Option<&MaterializedLiquidVoxel> {
        self.liquids.get(&position)
    }

    /// Reports whether editing a voxel would remove, undercut, or bury authored liquid.
    ///
    /// V3 liquid topology cannot currently be rebuilt after a terrain edit. An
    /// edit is therefore protected when the same column contains authored liquid
    /// at that level or above it, or immediately below it.
    #[must_use]
    pub(crate) fn protects_liquid_edit(&self, position: TilePos) -> bool {
        self.liquids.keys().any(|liquid| {
            liquid.coord == position.coord && liquid.level.saturating_add(1) >= position.level
        })
    }

    /// Whether one exact voxel is occupied by an authored liquid fill.
    ///
    /// Terrain-edit consequence projection uses this to retain the biome identity
    /// of a non-standable liquid bed while still removing ordinary surfaces buried
    /// by newly placed solid terrain.
    #[must_use]
    pub(crate) fn contains_liquid(&self, position: TilePos) -> bool {
        self.liquids.contains_key(&position)
    }

    /// Reports whether an edit would intersect an authored surface feature.
    ///
    /// V3 features are static projections until feature impacts and semantic
    /// reprojection exist. Rejecting edits at or above a root prevents a voxel
    /// from replacing the feature's support or being built through its visual
    /// volume. Edits below the exact root remain ordinary terrain edits.
    #[must_use]
    pub(crate) fn protects_feature_edit(&self, position: TilePos) -> bool {
        self.features.values().any(|feature| {
            feature.kind == FeatureKind::Tree
                && feature.root.coord == position.coord
                && position.level >= feature.root.level
        })
    }

    /// Reports whether an edit would invalidate a generated static light source.
    ///
    /// Until light-bearing objects can be reprojected after terrain edits, the
    /// complete source column is conservative map-owned geometry. This prevents
    /// digging out its footing as well as building through its future crystal mesh.
    #[must_use]
    pub(crate) fn protects_light_edit(&self, position: TilePos) -> bool {
        self.lights
            .values()
            .any(|light| light.origin.coord == position.coord)
    }

    #[cfg(test)]
    pub(crate) fn with_test_features(
        features: impl IntoIterator<Item = (FeatureId, PlannedFeature)>,
    ) -> Self {
        Self {
            features: features.into_iter().collect(),
            ..Self::default()
        }
    }
}

/// Presentation metadata expanded to every occupied voxel in one liquid run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MaterializedLiquidVoxel {
    pub(crate) material: FillMaterialRole,
    pub(crate) flow: LiquidFlowState,
    pub(crate) downstream: Option<TilePos>,
}

/// Failure after selection admitted a semantic plan but before runtime publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MaterializationError {
    /// The supposedly immutable type-state payload no longer matches its identity.
    SemanticFingerprintMismatch { expected: u64, actual: u64 },
    /// Semantic material roles did not resolve to compatible live substances.
    Volume(VolumeMaterializationError),
    /// An exact semantic consequence did not survive voxelization or projection.
    Projection(String),
    /// A deterministic materialized fingerprint could not encode a value.
    Fingerprint(String),
}

impl fmt::Display for MaterializationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SemanticFingerprintMismatch { expected, actual } => write!(
                formatter,
                "validated V3 semantic fingerprint changed from {expected} to {actual}"
            ),
            Self::Volume(error) => write!(formatter, "cannot materialize V3 volume: {error}"),
            Self::Projection(reason) => {
                write!(formatter, "invalid V3 runtime projection: {reason}")
            }
            Self::Fingerprint(reason) => {
                write!(
                    formatter,
                    "cannot fingerprint materialized V3 world: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for MaterializationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Volume(error) => Some(error),
            Self::SemanticFingerprintMismatch { .. }
            | Self::Projection(_)
            | Self::Fingerprint(_) => None,
        }
    }
}

/// Consumes an admitted semantic plan and publishes only validated exact resources.
pub(crate) fn materialize(
    validated: ValidatedWorldPlan,
    palette: &TerrainPalette,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<MaterializedV3World, MaterializationError> {
    let ValidatedWorldPlan {
        plan,
        semantic_fingerprint,
    } = validated;
    let actual_semantic =
        semantic_plan_fingerprint(&plan).map_err(MaterializationError::Fingerprint)?;
    if actual_semantic != semantic_fingerprint {
        return Err(MaterializationError::SemanticFingerprintMismatch {
            expected: semantic_fingerprint,
            actual: actual_semantic,
        });
    }

    let MaterializedVolume {
        map,
        interiors,
        special_regions,
    } = plan
        .volume
        .materialize(palette, is_solid)
        .map_err(MaterializationError::Volume)?;

    let materialized_liquids = project_liquids(&plan.liquids, &plan.volume)?;
    verify_materialized_consequences(
        &plan,
        &map,
        &interiors,
        &special_regions,
        &materialized_liquids,
        is_solid,
    )?;

    let anchors = project_anchors(&plan.anchors);
    let blockers = project_blockers(&plan.blockers);
    let biome_regions = project_biomes(&plan.biome_regions);
    verify_public_resources(&plan, &anchors, &blockers, &biome_regions)?;

    let materialized_fingerprint = fingerprint_materialized(&plan, &map, &materialized_liquids)
        .map_err(MaterializationError::Fingerprint)?;
    let view_hint = plan.view_hint;
    let GeneratedWorldPlan {
        features,
        structures,
        lights,
        ..
    } = plan;
    let presentation = MapPresentationProjection {
        liquids: materialized_liquids,
        features: features.by_id,
        structures: structures.by_id,
        lights,
    };

    Ok(MaterializedV3World {
        map,
        anchors,
        special_regions,
        interiors,
        blockers,
        biome_regions,
        view_hint,
        semantic_fingerprint,
        materialized_fingerprint,
        presentation,
    })
}

fn project_anchors(source: &BTreeMap<String, TilePos>) -> MapAnchors {
    source
        .iter()
        .map(|(name, position)| (MapAnchorId::from(name.as_str()), *position))
        .collect()
}

fn project_blockers(source: &BTreeSet<TilePos>) -> TraversalBlockers {
    let mut projected = TraversalBlockers::new();
    for position in source {
        let _inserted = projected.insert(*position);
    }
    projected
}

fn project_biomes(source: &BTreeMap<TilePos, BiomeRegionId>) -> BiomeRegions {
    let mut projected = BiomeRegions::new();
    for (position, region) in source {
        let _previous = projected.insert(*position, *region);
    }
    projected
}

fn project_liquids(
    liquids: &LiquidPlan,
    volume: &super::volume::VolumePlan,
) -> Result<BTreeMap<TilePos, MaterializedLiquidVoxel>, MaterializationError> {
    let fill_runs = volume.fill_runs_by_top();
    let mut projected = BTreeMap::new();
    let mut projected_runs = BTreeSet::new();

    for (body_id, body) in &liquids.bodies {
        for (run_top, node) in &body.nodes {
            if !projected_runs.insert(*run_top) {
                return Err(MaterializationError::Projection(format!(
                    "multiple liquid bodies own fill run {run_top:?}"
                )));
            }
            let fill = fill_runs.get(run_top).ok_or_else(|| {
                MaterializationError::Projection(format!(
                    "liquid body {body_id:?} node {run_top:?} has no owning fill run"
                ))
            })?;
            if fill.material != body.material {
                return Err(MaterializationError::Projection(format!(
                    "liquid body {body_id:?} node {run_top:?} material {:?} disagrees with its {:?} fill",
                    body.material, fill.material
                )));
            }
            if let Some(downstream) = node.downstream {
                if run_top.coord.distance(downstream.coord) != 1 {
                    return Err(MaterializationError::Projection(format!(
                        "liquid body {body_id:?} node {run_top:?} has non-adjacent downstream {downstream:?}"
                    )));
                }
            }
            let descriptor = MaterializedLiquidVoxel {
                material: body.material,
                flow: node.state,
                downstream: node.downstream,
            };

            for level in fill.levels.bottom..fill.levels.top {
                let position = TilePos::new(run_top.coord, level);
                if projected.insert(position, descriptor).is_some() {
                    return Err(MaterializationError::Projection(format!(
                        "multiple liquid nodes project onto occupied voxel {position:?}"
                    )));
                }
            }
        }
    }
    if projected_runs.len() != fill_runs.len() {
        if let Some(missing) = fill_runs
            .keys()
            .find(|position| !projected_runs.contains(position))
        {
            return Err(MaterializationError::Projection(format!(
                "fill run {missing:?} has no liquid node"
            )));
        }
        return Err(MaterializationError::Projection(
            "liquid run projection cardinality disagrees with occupied volume".to_owned(),
        ));
    }

    Ok(projected)
}
fn verify_materialized_consequences(
    plan: &GeneratedWorldPlan,
    map: &VoxelMap,
    interiors: &InteriorRegions,
    special_regions: &SpecialMovementRegions,
    liquids: &BTreeMap<TilePos, MaterializedLiquidVoxel>,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<(), MaterializationError> {
    for position in plan.volume.surfaces.keys().copied() {
        verify_surface(plan, map, position, is_solid)?;
    }

    for (name, position) in &plan.anchors {
        verify_walker_surface(plan, map, *position, is_solid, &format!("anchor {name:?}"))?;
    }
    for position in &plan.blockers {
        verify_walker_surface(plan, map, *position, is_solid, "traversal blocker")?;
    }
    for (position, metadata) in &plan.volume.surfaces {
        if matches!(metadata.access, SurfaceAccess::SpecialMovement(_)) {
            verify_walker_surface(plan, map, *position, is_solid, "special-movement surface")?;
        }
    }

    for (position, liquid) in liquids {
        let substance = map.get(*position);
        if substance.is_air() || is_solid(substance) {
            return Err(MaterializationError::Projection(format!(
                "liquid {liquid:?} at {position:?} did not resolve to occupied non-solid volume"
            )));
        }
    }
    for (id, feature) in &plan.features.by_id {
        if !plan.volume.surfaces.contains_key(&feature.root) {
            return Err(MaterializationError::Projection(format!(
                "feature {id:?} lost its exact root surface {:?}",
                feature.root
            )));
        }
    }
    for (id, structure) in &plan.structures.by_id {
        for position in &structure.voxels {
            if !is_solid(map.get(*position)) {
                return Err(MaterializationError::Projection(format!(
                    "structure {id:?} voxel {position:?} is not solid after materialization"
                )));
            }
        }
    }
    for (id, light) in &plan.lights {
        verify_surface(plan, map, light.origin, is_solid).map_err(|error| {
            MaterializationError::Projection(format!(
                "gameplay light {id:?} has an invalid materialized origin: {error}"
            ))
        })?;
    }

    verify_special_regions(plan, special_regions)?;
    verify_interiors(plan, interiors, map, is_solid)?;
    Ok(())
}

fn verify_surface(
    plan: &GeneratedWorldPlan,
    map: &VoxelMap,
    position: TilePos,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<Headroom, MaterializationError> {
    if !plan.volume.surfaces.contains_key(&position) {
        return Err(MaterializationError::Projection(format!(
            "{position:?} is not an exact semantic surface"
        )));
    }
    let substance = map.get(position);
    if !is_solid(substance) {
        return Err(MaterializationError::Projection(format!(
            "semantic surface {position:?} is not solid after materialization"
        )));
    }
    let materialized = map
        .column(position.coord)
        .ok_or_else(|| {
            MaterializationError::Projection(format!(
                "semantic surface {position:?} has no materialized column"
            ))
        })?
        .headroom_above(position.level.saturating_add(1));
    let semantic = plan.volume.surface_headroom(position).ok_or_else(|| {
        MaterializationError::Projection(format!(
            "semantic surface {position:?} has no planned headroom"
        ))
    })?;
    if materialized != semantic {
        return Err(MaterializationError::Projection(format!(
            "surface {position:?} headroom changed from {} to {} during materialization",
            semantic.0, materialized.0
        )));
    }
    Ok(materialized)
}

fn verify_walker_surface(
    plan: &GeneratedWorldPlan,
    map: &VoxelMap,
    position: TilePos,
    is_solid: &dyn Fn(SubstanceId) -> bool,
    kind: &str,
) -> Result<(), MaterializationError> {
    let headroom = verify_surface(plan, map, position, is_solid)?;
    if !TraversalProfile::WALKER.admits_surface(is_solid(map.get(position)), headroom) {
        return Err(MaterializationError::Projection(format!(
            "{kind} {position:?} is not ordinary walker footing after materialization"
        )));
    }
    Ok(())
}

fn verify_special_regions(
    plan: &GeneratedWorldPlan,
    actual: &SpecialMovementRegions,
) -> Result<(), MaterializationError> {
    let expected: BTreeMap<_, _> = plan
        .volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| match metadata.access {
            SurfaceAccess::SpecialMovement(region) => Some((*position, region)),
            SurfaceAccess::Ordinary | SurfaceAccess::NonStandable => None,
        })
        .collect();
    let materialized: BTreeMap<_, _> = actual.iter().collect();
    if materialized != expected {
        return Err(MaterializationError::Projection(
            "special-movement resource disagrees with exact surface metadata".to_owned(),
        ));
    }
    Ok(())
}

fn verify_interiors(
    plan: &GeneratedWorldPlan,
    actual: &InteriorRegions,
    map: &VoxelMap,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<(), MaterializationError> {
    let expected_floors: BTreeMap<_, _> = plan
        .interiors
        .by_id
        .iter()
        .flat_map(|(region, interior)| {
            interior
                .floors
                .iter()
                .copied()
                .map(|position| (position, *region))
        })
        .collect();
    let expected_roofs: BTreeMap<_, _> = plan
        .interiors
        .by_id
        .iter()
        .flat_map(|(region, interior)| {
            interior
                .roof_voxels
                .iter()
                .copied()
                .map(|position| (position, *region))
        })
        .collect();
    let materialized_floors: BTreeMap<_, _> = actual.surfaces().collect();
    let materialized_roofs: BTreeMap<_, _> = actual.roof_voxels().collect();
    if materialized_floors != expected_floors || materialized_roofs != expected_roofs {
        return Err(MaterializationError::Projection(
            "interior resource disagrees with exact floor or roof metadata".to_owned(),
        ));
    }
    for position in expected_floors.keys().copied() {
        verify_surface(plan, map, position, is_solid)?;
    }
    for position in expected_roofs.keys() {
        if !is_solid(map.get(*position)) {
            return Err(MaterializationError::Projection(format!(
                "interior roof voxel {position:?} is not solid after materialization"
            )));
        }
    }
    Ok(())
}

fn verify_public_resources(
    plan: &GeneratedWorldPlan,
    anchors: &MapAnchors,
    blockers: &TraversalBlockers,
    biome_regions: &BiomeRegions,
) -> Result<(), MaterializationError> {
    if anchors.len() != plan.anchors.len()
        || plan.anchors.iter().any(|(name, position)| {
            anchors.get(&MapAnchorId::from(name.as_str())) != Some(*position)
        })
    {
        return Err(MaterializationError::Projection(
            "anchor resource disagrees with the ordered semantic anchors".to_owned(),
        ));
    }

    let materialized_blockers: BTreeSet<_> = blockers.iter().collect();
    if materialized_blockers != plan.blockers {
        return Err(MaterializationError::Projection(
            "traversal blocker resource disagrees with semantic blockers".to_owned(),
        ));
    }
    let materialized_biomes: BTreeMap<_, _> = biome_regions.iter().collect();
    if materialized_biomes != plan.biome_regions {
        return Err(MaterializationError::Projection(
            "biome resource disagrees with exact semantic membership".to_owned(),
        ));
    }
    Ok(())
}

fn fingerprint_materialized(
    plan: &GeneratedWorldPlan,
    map: &VoxelMap,
    liquids: &BTreeMap<TilePos, MaterializedLiquidVoxel>,
) -> Result<u64, String> {
    let mut encoder = FingerprintEncoder::new();
    encoder.u32(3);

    encoder.tag(0);
    encode_voxel_map(&mut encoder, map)?;

    encoder.tag(1);
    encoder.collection_count(plan.anchors.len())?;
    for (name, position) in &plan.anchors {
        encoder.str(name)?;
        encoder.tile_pos(*position);
    }

    encoder.tag(2);
    let special: BTreeMap<_, _> = plan
        .volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| match metadata.access {
            SurfaceAccess::SpecialMovement(region) => Some((*position, region)),
            SurfaceAccess::Ordinary | SurfaceAccess::NonStandable => None,
        })
        .collect();
    encode_special_regions(&mut encoder, &special)?;

    encoder.tag(3);
    encode_projected_interiors(&mut encoder, plan)?;

    encoder.tag(4);
    encode_tile_set(&mut encoder, &plan.blockers)?;

    encoder.tag(5);
    encoder.collection_count(plan.biome_regions.len())?;
    for (position, region) in &plan.biome_regions {
        encoder.tile_pos(*position);
        encoder.u32(region.0);
    }

    encoder.tag(6);
    encode_view_hint(&mut encoder, plan.view_hint)?;

    encoder.tag(7);
    encode_liquids(&mut encoder, liquids)?;
    encoder.tag(8);
    encode_features(&mut encoder, &plan.features.by_id)?;
    encoder.tag(9);
    encode_structures(&mut encoder, &plan.structures.by_id)?;
    encoder.tag(10);
    encode_lights(&mut encoder, &plan.lights)?;

    Ok(encoder.finish_materialized_world())
}

fn encode_voxel_map(encoder: &mut FingerprintEncoder, map: &VoxelMap) -> Result<(), String> {
    let columns: BTreeMap<HexCoord, _> = map.columns().collect();
    encoder.collection_count(columns.len())?;
    for (coord, column) in columns {
        encoder.hex_coord(coord);
        encoder.collection_count(column.iter().len())?;
        for (index, substance) in column.iter().enumerate() {
            let level = i32::try_from(index)
                .map_err(|source| format!("voxel level index {index} exceeds i32: {source}"))?;
            encoder.tile_pos(TilePos::new(coord, level));
            encoder.u16(substance.0);
        }
    }
    Ok(())
}

fn encode_special_regions(
    encoder: &mut FingerprintEncoder,
    regions: &BTreeMap<TilePos, SpecialMovementRegion>,
) -> Result<(), String> {
    encoder.collection_count(regions.len())?;
    for (position, region) in regions {
        encoder.tile_pos(*position);
        encoder.u32(region.0);
    }
    Ok(())
}

fn encode_projected_interiors(
    encoder: &mut FingerprintEncoder,
    plan: &GeneratedWorldPlan,
) -> Result<(), String> {
    let floors: BTreeMap<_, _> = plan
        .interiors
        .by_id
        .iter()
        .flat_map(|(region, interior)| {
            interior
                .floors
                .iter()
                .copied()
                .map(|position| (position, *region))
        })
        .collect();
    let roofs: BTreeMap<_, _> = plan
        .interiors
        .by_id
        .iter()
        .flat_map(|(region, interior)| {
            interior
                .roof_voxels
                .iter()
                .copied()
                .map(|position| (position, *region))
        })
        .collect();
    encode_interior_membership(encoder, &floors)?;
    encode_interior_membership(encoder, &roofs)
}

fn encode_interior_membership(
    encoder: &mut FingerprintEncoder,
    membership: &BTreeMap<TilePos, InteriorRegionId>,
) -> Result<(), String> {
    encoder.collection_count(membership.len())?;
    for (position, region) in membership {
        encoder.tile_pos(*position);
        encoder.u32(region.0);
    }
    Ok(())
}

fn encode_tile_set(
    encoder: &mut FingerprintEncoder,
    positions: &BTreeSet<TilePos>,
) -> Result<(), String> {
    encoder.collection_count(positions.len())?;
    for position in positions {
        encoder.tile_pos(*position);
    }
    Ok(())
}

fn encode_view_hint(
    encoder: &mut FingerprintEncoder,
    view_hint: MapViewHint,
) -> Result<(), String> {
    for value in [
        view_hint.eye.0,
        view_hint.eye.1,
        view_hint.eye.2,
        view_hint.focus.0,
        view_hint.focus.1,
        view_hint.focus.2,
    ] {
        encoder.finite_f32(value)?;
    }
    Ok(())
}

fn encode_liquids(
    encoder: &mut FingerprintEncoder,
    liquids: &BTreeMap<TilePos, MaterializedLiquidVoxel>,
) -> Result<(), String> {
    encoder.collection_count(liquids.len())?;
    for (position, liquid) in liquids {
        encoder.tile_pos(*position);
        encoder.tag(match liquid.material {
            FillMaterialRole::Water => 0,
            FillMaterialRole::Lava => 1,
        });
        encoder.tag(match liquid.flow {
            LiquidFlowState::Still => 0,
            LiquidFlowState::Current => 1,
            LiquidFlowState::Rapid => 2,
            LiquidFlowState::Fall => 3,
        });
        match liquid.downstream {
            None => encoder.tag(0),
            Some(downstream) => {
                encoder.tag(1);
                encoder.tile_pos(downstream);
            }
        }
    }
    Ok(())
}

fn encode_features(
    encoder: &mut FingerprintEncoder,
    features: &BTreeMap<FeatureId, PlannedFeature>,
) -> Result<(), String> {
    encoder.collection_count(features.len())?;
    for (id, feature) in features {
        encoder.u32(id.0);
        encoder.tile_pos(feature.root);
        encoder.tag(match feature.kind {
            FeatureKind::Tree => 0,
            FeatureKind::TallGrass => 1,
        });
    }
    Ok(())
}

fn encode_structures(
    encoder: &mut FingerprintEncoder,
    structures: &BTreeMap<StructureId, PlannedStructure>,
) -> Result<(), String> {
    encoder.collection_count(structures.len())?;
    for (id, structure) in structures {
        encoder.u32(id.0);
        encoder.tag(match structure.kind {
            StructureKind::Bridge => 0,
            StructureKind::Wall => 1,
            StructureKind::Stair => 2,
            StructureKind::Tower => 3,
            StructureKind::Gate => 4,
            StructureKind::Keep => 5,
        });
        encode_tile_set(encoder, &structure.voxels)?;
    }
    Ok(())
}

fn encode_lights(
    encoder: &mut FingerprintEncoder,
    lights: &BTreeMap<LightId, PlannedGameplayLight>,
) -> Result<(), String> {
    encoder.collection_count(lights.len())?;
    for (id, light) in lights {
        encoder.u32(id.0);
        encoder.tile_pos(light.origin);
        encoder.tag(match light.level {
            IlluminationLevel::Dark => 0,
            IlluminationLevel::Dim => 1,
            IlluminationLevel::Bright => 2,
        });
        encoder.u32(light.radius);
        encode_light_presentation(encoder, light.presentation);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procedural_v3::layout::{
        HexSide, LayoutKind, PatchId, ResolvedEdgeReference, ResolvedLayoutPlan, ResolvedPatch,
    };
    use crate::procedural_v3::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidNode};
    use crate::procedural_v3::volume::{
        FillMaterialRole, LevelInterval, NonSolidFill, SolidMass, SolidMaterialRole,
        SurfaceMetadata, VolumeElement, VolumePlan,
    };
    use crate::procedural_v3::world::{FeaturePlan, InteriorPlan, PlannedInterior, StructurePlan};

    fn palette() -> TerrainPalette {
        TerrainPalette {
            bedrock: SubstanceId(1),
            stone: SubstanceId(2),
            dirt: SubstanceId(3),
            grass: SubstanceId(4),
            gravel: SubstanceId(5),
            water: SubstanceId(6),
            metal: SubstanceId(7),
            snow: SubstanceId(8),
            ice: SubstanceId(9),
            basalt: SubstanceId(10),
            lava: SubstanceId(11),
        }
    }

    fn is_solid(substance: SubstanceId) -> bool {
        matches!(substance.0, 1 | 2 | 3 | 4 | 5 | 7 | 8 | 9 | 10)
    }

    #[test]
    fn expands_a_fall_run_and_retains_its_exact_downstream_position() {
        let source_coord = HexCoord::ORIGIN;
        let target_coord = HexSide::East.neighbor(source_coord);
        let source = TilePos::new(source_coord, 5);
        let target = TilePos::new(target_coord, 3);
        let mut volume = VolumePlan::new(BTreeSet::from([source_coord, target_coord]));
        volume.columns.get_mut(&source_coord).unwrap().elements =
            vec![VolumeElement::Fill(NonSolidFill {
                levels: LevelInterval::new(1, 6),
                material: FillMaterialRole::Water,
            })];
        volume.columns.get_mut(&target_coord).unwrap().elements =
            vec![VolumeElement::Fill(NonSolidFill {
                levels: LevelInterval::new(3, 4),
                material: FillMaterialRole::Water,
            })];
        let liquids = LiquidPlan {
            bodies: BTreeMap::from([(
                LiquidBodyId(4),
                LiquidBodyPlan {
                    material: FillMaterialRole::Water,
                    nodes: BTreeMap::from([
                        (
                            source,
                            LiquidNode {
                                state: LiquidFlowState::Fall,
                                downstream: Some(target),
                            },
                        ),
                        (
                            target,
                            LiquidNode {
                                state: LiquidFlowState::Still,
                                downstream: None,
                            },
                        ),
                    ]),
                },
            )]),
        };

        let projected =
            project_liquids(&liquids, &volume).expect("valid liquid runs project exactly");
        assert_eq!(projected.len(), 6);
        for level in 1..=5 {
            assert_eq!(
                projected.get(&TilePos::new(source_coord, level)),
                Some(&MaterializedLiquidVoxel {
                    material: FillMaterialRole::Water,
                    flow: LiquidFlowState::Fall,
                    downstream: Some(target),
                })
            );
        }
        assert_eq!(
            projected.get(&target),
            Some(&MaterializedLiquidVoxel {
                material: FillMaterialRole::Water,
                flow: LiquidFlowState::Still,
                downstream: None,
            })
        );
    }

    #[test]
    fn projection_rejects_non_adjacent_downstream_coordinates() {
        let source_coord = HexCoord::ORIGIN;
        let target_coord = HexCoord::new_cubic(2, 0, -2);
        let source = TilePos::new(source_coord, 5);
        let target = TilePos::new(target_coord, 3);
        let mut volume = VolumePlan::new(BTreeSet::from([source_coord, target_coord]));
        volume
            .columns
            .get_mut(&source_coord)
            .expect("the source is in the mask")
            .elements = vec![VolumeElement::Fill(NonSolidFill {
            levels: LevelInterval::new(5, 6),
            material: FillMaterialRole::Water,
        })];
        volume
            .columns
            .get_mut(&target_coord)
            .expect("the target is in the mask")
            .elements = vec![VolumeElement::Fill(NonSolidFill {
            levels: LevelInterval::new(3, 4),
            material: FillMaterialRole::Water,
        })];
        let liquids = LiquidPlan {
            bodies: BTreeMap::from([(
                LiquidBodyId(4),
                LiquidBodyPlan {
                    material: FillMaterialRole::Water,
                    nodes: BTreeMap::from([
                        (
                            source,
                            LiquidNode {
                                state: LiquidFlowState::Fall,
                                downstream: Some(target),
                            },
                        ),
                        (
                            target,
                            LiquidNode {
                                state: LiquidFlowState::Still,
                                downstream: None,
                            },
                        ),
                    ]),
                },
            )]),
        };

        let error = project_liquids(&liquids, &volume)
            .expect_err("a non-adjacent downstream cannot be projected");
        assert!(error.to_string().contains("non-adjacent downstream"));
    }

    #[test]
    fn all_six_adjacent_downstream_coordinates_are_retained_exactly() {
        for side in HexSide::ALL {
            let source = TilePos::new(HexCoord::ORIGIN, 5);
            let downstream = TilePos::new(side.neighbor(source.coord), 2);
            let mut volume = VolumePlan::new(BTreeSet::from([source.coord, downstream.coord]));
            for (position, bottom) in [(source, 5), (downstream, 2)] {
                volume
                    .columns
                    .get_mut(&position.coord)
                    .expect("both liquid columns are in the mask")
                    .elements = vec![VolumeElement::Fill(NonSolidFill {
                    levels: LevelInterval::new(bottom, bottom + 1),
                    material: FillMaterialRole::Water,
                })];
            }
            let liquids = LiquidPlan {
                bodies: BTreeMap::from([(
                    LiquidBodyId(4),
                    LiquidBodyPlan {
                        material: FillMaterialRole::Water,
                        nodes: BTreeMap::from([
                            (
                                source,
                                LiquidNode {
                                    state: LiquidFlowState::Fall,
                                    downstream: Some(downstream),
                                },
                            ),
                            (
                                downstream,
                                LiquidNode {
                                    state: LiquidFlowState::Still,
                                    downstream: None,
                                },
                            ),
                        ]),
                    },
                )]),
            };

            let projection =
                project_liquids(&liquids, &volume).expect("adjacent liquid nodes project");
            assert_eq!(
                projection.get(&source).map(|liquid| liquid.downstream),
                Some(Some(downstream))
            );
        }
    }

    #[test]
    fn presentation_accessors_are_exact_ordered_and_protect_liquid_support() {
        let coord = HexCoord::ORIGIN;
        let lower = TilePos::new(coord, 3);
        let upper = TilePos::new(coord, 5);
        let descriptor = MaterializedLiquidVoxel {
            material: FillMaterialRole::Water,
            flow: LiquidFlowState::Current,
            downstream: Some(TilePos::new(HexSide::East.neighbor(coord), 2)),
        };
        let projection = MapPresentationProjection {
            liquids: BTreeMap::from([(upper, descriptor), (lower, descriptor)]),
            ..Default::default()
        };

        assert_eq!(projection.liquids().len(), 2);
        assert_eq!(projection.liquid_at(lower), Some(&descriptor));
        assert_eq!(
            projection
                .iter_liquids()
                .map(|(position, _liquid)| *position)
                .collect::<Vec<_>>(),
            vec![lower, upper]
        );

        assert!(projection.protects_liquid_edit(TilePos::new(coord, 5)));
        assert!(projection.protects_liquid_edit(TilePos::new(coord, 4)));
        assert!(projection.protects_liquid_edit(TilePos::new(coord, 0)));
        assert!(projection.protects_liquid_edit(TilePos::new(coord, 6)));
        assert!(!projection.protects_liquid_edit(TilePos::new(coord, 7)));
        assert!(!projection.protects_liquid_edit(TilePos::new(HexSide::West.neighbor(coord), 0)));
    }

    #[test]
    fn materialized_liquid_identity_covers_exact_downstream_level() {
        fn fingerprint(
            descriptor: MaterializedLiquidVoxel,
            position: TilePos,
        ) -> Result<u64, String> {
            let mut encoder = FingerprintEncoder::new();
            encode_liquids(&mut encoder, &BTreeMap::from([(position, descriptor)]))?;
            Ok(encoder.finish_materialized_world())
        }

        let position = TilePos::new(HexCoord::ORIGIN, 5);
        let downstream_coord = HexSide::East.neighbor(position.coord);
        let baseline = MaterializedLiquidVoxel {
            material: FillMaterialRole::Water,
            flow: LiquidFlowState::Fall,
            downstream: Some(TilePos::new(downstream_coord, 3)),
        };
        let changed_level = MaterializedLiquidVoxel {
            downstream: Some(TilePos::new(downstream_coord, 2)),
            ..baseline
        };

        assert_ne!(
            fingerprint(baseline, position).expect("the baseline encodes"),
            fingerprint(changed_level, position).expect("the changed descriptor encodes")
        );
    }

    fn mass(
        bottom: i32,
        top: i32,
        material: SolidMaterialRole,
        cutaway_for: Option<InteriorRegionId>,
    ) -> VolumeElement {
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(bottom, top),
            material,
            cutaway_for,
        })
    }

    fn valid_plan(light_radius: u32) -> GeneratedWorldPlan {
        let mask: BTreeSet<_> = HexCoord::ORIGIN.within_radius(1).into_iter().collect();
        let mut positions = mask.iter().copied();
        let anchor_coord = positions.next().expect("the radius-one mask is nonempty");
        let tree_coord = positions
            .next()
            .expect("the radius-one mask has seven cells");
        let light_coord = positions
            .next()
            .expect("the radius-one mask has seven cells");
        let structure_coord = positions
            .next()
            .expect("the radius-one mask has seven cells");
        let water_coord = positions
            .next()
            .expect("the radius-one mask has seven cells");
        let special_coord = positions
            .next()
            .expect("the radius-one mask has seven cells");

        let mut volume = VolumePlan::new(mask.clone());
        for coord in &mask {
            volume
                .columns
                .get_mut(coord)
                .expect("the volume constructor covers its mask")
                .elements = vec![mass(0, 1, SolidMaterialRole::Stone, None)];
            volume.surfaces.insert(
                TilePos::new(*coord, 0),
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }

        let interior = InteriorRegionId(3);
        volume
            .columns
            .get_mut(&anchor_coord)
            .expect("the anchor column is masked")
            .elements = vec![
            mass(0, 1, SolidMaterialRole::Stone, None),
            mass(3, 4, SolidMaterialRole::Stone, Some(interior)),
        ];
        volume
            .surfaces
            .get_mut(&TilePos::new(anchor_coord, 0))
            .expect("the anchor surface was inserted")
            .interior = Some(interior);
        volume.surfaces.insert(
            TilePos::new(anchor_coord, 3),
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );

        volume
            .columns
            .get_mut(&water_coord)
            .expect("the water column is masked")
            .elements = vec![
            mass(0, 1, SolidMaterialRole::Gravel, None),
            VolumeElement::Fill(NonSolidFill {
                levels: LevelInterval::new(1, 2),
                material: FillMaterialRole::Water,
            }),
        ];
        volume
            .surfaces
            .get_mut(&TilePos::new(water_coord, 0))
            .expect("the waterbed surface was inserted")
            .access = SurfaceAccess::NonStandable;
        let special_region = SpecialMovementRegion(7);
        volume
            .surfaces
            .get_mut(&TilePos::new(special_coord, 0))
            .expect("the special surface was inserted")
            .access = SurfaceAccess::SpecialMovement(special_region);

        let biome_regions = volume
            .surfaces
            .keys()
            .copied()
            .map(|position| (position, BiomeRegionId(0)))
            .collect();
        let edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect();
        let patch = ResolvedPatch {
            biome_region: BiomeRegionId(0),
            mask: mask.clone(),
            edges,
        };
        let layout = ResolvedLayoutPlan {
            kind: LayoutKind::Single,
            grid_radius: 12,
            footprint: mask.clone(),
            patches: BTreeMap::from([(PatchId(0), patch)]),
            shared_edges: BTreeMap::new(),
        };
        let anchor = TilePos::new(anchor_coord, 0);
        let tree = TilePos::new(tree_coord, 0);
        let light = TilePos::new(light_coord, 0);
        let structure = TilePos::new(structure_coord, 0);
        let water = TilePos::new(water_coord, 1);
        let roof = TilePos::new(anchor_coord, 3);

        GeneratedWorldPlan {
            layout,
            volume,
            liquids: LiquidPlan {
                bodies: BTreeMap::from([(
                    LiquidBodyId(0),
                    LiquidBodyPlan {
                        material: FillMaterialRole::Water,
                        nodes: BTreeMap::from([(
                            water,
                            LiquidNode {
                                state: LiquidFlowState::Still,
                                downstream: None,
                            },
                        )]),
                    },
                )]),
            },
            features: FeaturePlan {
                by_id: BTreeMap::from([
                    (
                        FeatureId(0),
                        PlannedFeature {
                            root: tree,
                            kind: FeatureKind::Tree,
                        },
                    ),
                    (
                        FeatureId(1),
                        PlannedFeature {
                            root: light,
                            kind: FeatureKind::TallGrass,
                        },
                    ),
                ]),
                ..FeaturePlan::default()
            },
            structures: StructurePlan {
                by_id: BTreeMap::from([(
                    StructureId(0),
                    PlannedStructure {
                        kind: StructureKind::Bridge,
                        voxels: BTreeSet::from([structure]),
                    },
                )]),
            },
            blockers: BTreeSet::from([tree]),
            lights: BTreeMap::from([(
                LightId(0),
                PlannedGameplayLight {
                    origin: light,
                    level: IlluminationLevel::Dim,
                    radius: light_radius,
                    presentation: None,
                },
            )]),
            biome_regions,
            interiors: InteriorPlan {
                by_id: BTreeMap::from([(
                    interior,
                    PlannedInterior {
                        floors: BTreeSet::from([anchor]),
                        entrances: BTreeSet::from([anchor]),
                        roof_voxels: BTreeSet::from([roof]),
                    },
                )]),
            },
            anchors: BTreeMap::from([("party_start".to_owned(), anchor)]),
            view_hint: MapViewHint::new((0.0, 20.0, 20.0), (0.0, 1.0, 0.0)),
        }
    }

    fn validated(plan: GeneratedWorldPlan) -> ValidatedWorldPlan {
        assert!(
            plan.validate().is_empty(),
            "the materialization fixture must satisfy common validation"
        );
        let semantic_fingerprint =
            semantic_plan_fingerprint(&plan).expect("the fixture fingerprint is finite");
        ValidatedWorldPlan {
            plan,
            semantic_fingerprint,
        }
    }

    #[test]
    fn materializes_every_public_and_private_exact_projection() {
        let selected = validated(valid_plan(5));
        let expected_semantic = selected.semantic_fingerprint;
        let output =
            materialize(selected, &palette(), &is_solid).expect("the valid world materializes");

        assert_eq!(output.map.len(), 7);
        assert!(output
            .anchors
            .get(&MapAnchorId::from("party_start"))
            .is_some());
        assert_eq!(output.special_regions.len(), 1);
        assert_eq!(output.interiors.surfaces().count(), 1);
        assert_eq!(output.interiors.roof_voxels().count(), 1);
        assert_eq!(output.blockers.len(), 1);
        assert_eq!(output.biome_regions.len(), 8);
        assert!(output.view_hint.is_valid());
        assert_eq!(output.semantic_fingerprint, expected_semantic);
        assert_eq!(output.presentation.liquids.len(), 1);
        assert_eq!(output.presentation.features.len(), 2);
        assert_eq!(output.presentation.structures.len(), 1);
        assert_eq!(output.presentation.lights.len(), 1);
    }

    #[test]
    fn feature_projection_is_ordered_and_protects_the_authored_root_volume() {
        let output = materialize(validated(valid_plan(5)), &palette(), &is_solid)
            .expect("the valid world materializes");
        let projected: Vec<_> = output
            .presentation
            .features()
            .iter()
            .map(|(id, feature)| (*id, *feature))
            .collect();
        assert_eq!(
            projected
                .iter()
                .map(|(id, _feature)| id.0)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );

        let tree = projected
            .iter()
            .find_map(|(_id, feature)| (feature.kind == FeatureKind::Tree).then_some(feature.root))
            .expect("the fixture contains a tree");
        assert!(output.presentation.protects_feature_edit(tree));
        assert!(output
            .presentation
            .protects_feature_edit(TilePos::new(tree.coord, tree.level.saturating_add(12))));
        assert!(!output
            .presentation
            .protects_feature_edit(TilePos::new(tree.coord, tree.level.saturating_sub(1))));
        assert!(!output
            .presentation
            .protects_feature_edit(TilePos::new(HexCoord::from_axial(12, -12), tree.level)));

        let grass = projected
            .iter()
            .find_map(|(_id, feature)| {
                (feature.kind == FeatureKind::TallGrass).then_some(feature.root)
            })
            .expect("the fixture contains tall grass");
        assert!(
            !output.presentation.protects_feature_edit(grass),
            "presentation-only grass must not make its supporting terrain immutable"
        );
        assert!(!output
            .presentation
            .protects_feature_edit(TilePos::new(grass.coord, grass.level.saturating_add(12))));
    }

    #[test]
    fn gameplay_light_projection_is_ordered_and_protects_its_source_column() {
        let output = materialize(validated(valid_plan(5)), &palette(), &is_solid)
            .expect("the valid world materializes");
        let projected: Vec<_> = output
            .presentation
            .lights()
            .iter()
            .map(|(id, light)| (*id, *light))
            .collect();
        assert_eq!(projected.len(), 1);
        let source = projected
            .first()
            .map(|(_id, light)| light.origin)
            .expect("the fixture contains a gameplay light");
        assert!(output.presentation.protects_light_edit(source));
        assert!(output
            .presentation
            .protects_light_edit(TilePos::new(source.coord, source.level.saturating_add(20))));
        assert!(!output
            .presentation
            .protects_light_edit(TilePos::new(HexCoord::from_axial(12, -12), source.level)));
    }

    #[test]
    fn materialized_fingerprint_is_deterministic_and_covers_presented_lights() {
        let first = materialize(validated(valid_plan(5)), &palette(), &is_solid)
            .expect("the first world materializes");
        let repeated = materialize(validated(valid_plan(5)), &palette(), &is_solid)
            .expect("the repeated world materializes");
        let changed = materialize(validated(valid_plan(6)), &palette(), &is_solid)
            .expect("the changed world materializes");

        assert_eq!(
            first.materialized_fingerprint,
            repeated.materialized_fingerprint
        );
        assert_eq!(
            first.materialized_fingerprint, 15_501_428_346_321_951_035,
            "update only with an explicit materialized V3 fingerprint decision"
        );
        assert_ne!(
            first.materialized_fingerprint,
            changed.materialized_fingerprint
        );
    }

    #[test]
    fn body_identity_changes_semantics_without_changing_materialized_voxels() {
        let first = materialize(validated(valid_plan(5)), &palette(), &is_solid)
            .expect("the first world materializes");
        let mut renumbered = valid_plan(5);
        let body = renumbered
            .liquids
            .bodies
            .pop_first()
            .expect("the fixture has one liquid body")
            .1;
        renumbered.liquids.bodies.insert(LiquidBodyId(99), body);
        let changed = materialize(validated(renumbered), &palette(), &is_solid)
            .expect("the renumbered world materializes");

        assert_ne!(first.semantic_fingerprint, changed.semantic_fingerprint);
        assert_eq!(
            first.materialized_fingerprint,
            changed.materialized_fingerprint
        );
        assert_eq!(first.presentation.liquids, changed.presentation.liquids);
    }

    #[test]
    fn stale_validated_identity_is_rejected_before_projection() {
        let mut selected = validated(valid_plan(5));
        selected.semantic_fingerprint ^= 1;

        assert!(matches!(
            materialize(selected, &palette(), &is_solid),
            Err(MaterializationError::SemanticFingerprintMismatch { .. })
        ));
    }

    #[test]
    fn forged_unstandable_anchor_is_rejected_by_materialized_cross_check() {
        let mut plan = valid_plan(5);
        let waterbed = plan
            .volume
            .surfaces
            .iter()
            .find_map(|(position, metadata)| {
                (metadata.access == SurfaceAccess::NonStandable).then_some(*position)
            })
            .expect("the fixture contains a waterbed");
        plan.anchors.insert("party_start".to_owned(), waterbed);
        let semantic_fingerprint =
            semantic_plan_fingerprint(&plan).expect("the forged plan remains encodable");
        let selected = ValidatedWorldPlan {
            plan,
            semantic_fingerprint,
        };

        let error = materialize(selected, &palette(), &is_solid)
            .expect_err("an anchor without walker headroom must fail");
        assert!(error.to_string().contains("anchor"));
        assert!(error.to_string().contains("not ordinary walker footing"));
    }

    #[test]
    fn incompatible_live_substance_contract_stops_materialization() {
        let selected = validated(valid_plan(5));
        let never_solid = |_substance: SubstanceId| false;

        assert!(matches!(
            materialize(selected, &palette(), &never_solid),
            Err(MaterializationError::Volume(
                VolumeMaterializationError::MaterialContract { .. }
            ))
        ));
    }
}
