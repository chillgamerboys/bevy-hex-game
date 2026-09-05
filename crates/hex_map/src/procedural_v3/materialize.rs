//! Materialization boundary for validated procedural V3 worlds.
//!
//! Candidate planning remains semantic and ordered. This module is the only place
//! where an admitted plan becomes voxel storage and public runtime resources. It
//! retains map-owned presentation descriptors in ordered collections for later
//! terrain spawning.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use bevy::prelude::Resource;
#[cfg(feature = "map-review")]
use hex_core::HexCoord;
use hex_core::{
    BiomeRegionId, BiomeRegions, Headroom, IlluminationLevel, InteriorRegions, MapAnchorId,
    MapAnchors, MapObservationAnchors, MapViewHint, SpecialMovementRegions, SubstanceId, TilePos,
    TraversalBlockers, TraversalProfile,
};

use super::fingerprint::{
    encode_light_presentation, semantic_plan_fingerprint, FingerprintEncoder,
};
use super::liquid::{LiquidFlowState, LiquidPlan};
use super::selection::ValidatedWorldPlan;
use super::volume::{
    FillMaterialRole, MaterializedVolume, SurfaceAccess, VolumeElement, VolumeMaterializationError,
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
    pub(crate) observation_anchors: MapObservationAnchors,
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
    #[cfg(feature = "map-review")]
    review_protected_routes: BTreeMap<String, BTreeSet<TilePos>>,
    #[cfg(feature = "map-review")]
    review_frozen_woods_mask: BTreeSet<HexCoord>,
    #[cfg(feature = "map-review")]
    review_garden_mask: BTreeSet<HexCoord>,
    #[cfg(feature = "map-review")]
    review_forced_summits: BTreeSet<TilePos>,
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

    /// Returns exact structures in stable map-local identity order.
    #[must_use]
    pub(crate) const fn structures(&self) -> &BTreeMap<StructureId, PlannedStructure> {
        &self.structures
    }

    /// Returns generated gameplay-light descriptors in stable map-local order.
    #[must_use]
    pub(crate) const fn lights(&self) -> &BTreeMap<LightId, PlannedGameplayLight> {
        &self.lights
    }

    /// Returns the generator's exact protected route footprints for disposable
    /// review projections. These surfaces are never published to gameplay and
    /// exist only while the `map-review` feature is compiled.
    #[cfg(feature = "map-review")]
    #[must_use]
    pub(crate) const fn review_protected_routes(&self) -> &BTreeMap<String, BTreeSet<TilePos>> {
        &self.review_protected_routes
    }

    /// Returns the exact union of authored Frozen-Woods ownership patches.
    #[cfg(feature = "map-review")]
    #[must_use]
    pub(crate) const fn review_frozen_woods_mask(&self) -> &BTreeSet<HexCoord> {
        &self.review_frozen_woods_mask
    }

    /// Returns the exact authored Lake-Island garden ownership patch.
    #[cfg(feature = "map-review")]
    #[must_use]
    pub(crate) const fn review_garden_mask(&self) -> &BTreeSet<HexCoord> {
        &self.review_garden_mask
    }

    /// Returns the exact final summit pins retained by Grand's highland authority.
    #[cfg(feature = "map-review")]
    #[must_use]
    pub(crate) const fn review_forced_summits(&self) -> &BTreeSet<TilePos> {
        &self.review_forced_summits
    }

    /// Rebuilds only the generator-neutral runtime consequences carried by a live
    /// world snapshot. Generator plans and structure recipe identities deliberately
    /// remain absent: their surviving voxel/public consequences are restored by the
    /// snapshot adapter instead.
    pub(crate) fn from_snapshot_parts(
        liquids: BTreeMap<TilePos, MaterializedLiquidVoxel>,
        features: BTreeMap<FeatureId, PlannedFeature>,
        lights: BTreeMap<LightId, PlannedGameplayLight>,
    ) -> Self {
        Self {
            liquids,
            features,
            structures: BTreeMap::new(),
            lights,
            #[cfg(feature = "map-review")]
            review_protected_routes: BTreeMap::new(),
            #[cfg(feature = "map-review")]
            review_frozen_woods_mask: BTreeSet::new(),
            #[cfg(feature = "map-review")]
            review_garden_mask: BTreeSet::new(),
            #[cfg(feature = "map-review")]
            review_forced_summits: BTreeSet::new(),
        }
    }

    /// Retains feature presentations whose exact authored support remains valid.
    ///
    /// Terrain edits may remove presentation-only features such as grass or cave flora.
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
                && feature.blocker_footprint.iter().any(|blocker| {
                    blocker.coord == position.coord && position.level >= blocker.level
                })
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
    let profile_started = std::time::Instant::now();
    let mut profile_previous = profile_started;
    let semantic_fingerprint = validated.semantic_fingerprint();
    let actual_semantic =
        semantic_plan_fingerprint(validated.plan()).map_err(MaterializationError::Fingerprint)?;
    if actual_semantic != semantic_fingerprint {
        return Err(MaterializationError::SemanticFingerprintMismatch {
            expected: semantic_fingerprint,
            actual: actual_semantic,
        });
    }
    materialization_profile_checkpoint(
        "semantic verification",
        profile_started,
        &mut profile_previous,
    );

    let MaterializedVolume {
        map,
        interiors,
        special_regions,
    } = validated
        .plan()
        .volume
        .materialize_admitted(validated.volume_admission(), palette, is_solid)
        .map_err(MaterializationError::Volume)?;
    materialization_profile_checkpoint("volume projection", profile_started, &mut profile_previous);

    let plan = validated.plan();
    let materialized_liquids = project_liquids(&plan.liquids, &plan.volume)?;
    verify_materialized_consequences(
        plan,
        &map,
        &interiors,
        &special_regions,
        &materialized_liquids,
        is_solid,
    )?;
    materialization_profile_checkpoint(
        "consequence verification",
        profile_started,
        &mut profile_previous,
    );

    let (anchors, observation_anchors) = project_anchors(&plan.anchors, &plan.observation_anchors)?;
    let blockers = project_blockers(&plan.blockers)?;
    let biome_regions = project_biomes(&plan.biome_regions)?;
    materialization_profile_checkpoint("public projection", profile_started, &mut profile_previous);

    let materialized_fingerprint = fingerprint_materialized(plan, &map, &materialized_liquids)
        .map_err(MaterializationError::Fingerprint)?;
    materialization_profile_checkpoint(
        "materialized fingerprint",
        profile_started,
        &mut profile_previous,
    );
    #[cfg(feature = "map-review")]
    let review_snow_exception_masks = validated
        .review_snow_exception_masks()
        .cloned()
        .unwrap_or_default();
    let (plan, admitted_semantic_fingerprint) = validated.into_parts();
    debug_assert_eq!(semantic_fingerprint, admitted_semantic_fingerprint);
    let view_hint = plan.view_hint;
    let GeneratedWorldPlan {
        features,
        structures,
        lights,
        ..
    } = plan;
    let features_by_id = features.by_id;
    #[cfg(feature = "map-review")]
    let review_protected_routes = features
        .protected_routes
        .into_iter()
        .map(|(name, route)| (name, route.surfaces))
        .collect();
    let presentation = MapPresentationProjection {
        liquids: materialized_liquids,
        features: features_by_id,
        structures: structures.by_id,
        lights,
        #[cfg(feature = "map-review")]
        review_protected_routes,
        #[cfg(feature = "map-review")]
        review_frozen_woods_mask: review_snow_exception_masks.frozen_woods,
        #[cfg(feature = "map-review")]
        review_garden_mask: review_snow_exception_masks.garden,
        #[cfg(feature = "map-review")]
        review_forced_summits: review_snow_exception_masks.forced_summits,
    };

    Ok(MaterializedV3World {
        map,
        anchors,
        observation_anchors,
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

fn materialization_profile_checkpoint(
    stage: &str,
    started: std::time::Instant,
    previous: &mut std::time::Instant,
) {
    if std::env::var_os("HEX_GRAND_PROFILE").is_some() {
        let now = std::time::Instant::now();
        eprintln!(
            "v3 materialization profile: {stage}: delta={:?} total={:?}",
            now.duration_since(*previous),
            now.duration_since(started)
        );
        *previous = now;
    }
}

fn project_anchors(
    walker: &BTreeMap<String, TilePos>,
    observation: &BTreeMap<String, TilePos>,
) -> Result<(MapAnchors, MapObservationAnchors), MaterializationError> {
    if walker.keys().any(|name| observation.contains_key(name)) {
        return Err(MaterializationError::Projection(
            "walker and observation anchor projections contain a duplicate stable identity"
                .to_owned(),
        ));
    }

    let mut projected_walker = MapAnchors::new();
    for (name, position) in walker {
        if projected_walker
            .insert(MapAnchorId::from(name.as_str()), *position)
            .is_some()
        {
            return Err(MaterializationError::Projection(
                "walker anchor projection contains a duplicate stable identity".to_owned(),
            ));
        }
    }
    let mut projected_observation = MapObservationAnchors::new();
    for (name, position) in observation {
        if projected_observation
            .insert(MapAnchorId::from(name.as_str()), *position)
            .is_some()
        {
            return Err(MaterializationError::Projection(
                "observation anchor projection contains a duplicate stable identity".to_owned(),
            ));
        }
    }
    if projected_walker.len() != walker.len() || projected_observation.len() != observation.len() {
        return Err(MaterializationError::Projection(
            "anchor projection cardinality disagrees with its ordered semantic namespaces"
                .to_owned(),
        ));
    }
    Ok((projected_walker, projected_observation))
}

fn project_blockers(source: &BTreeSet<TilePos>) -> Result<TraversalBlockers, MaterializationError> {
    let mut projected = TraversalBlockers::new();
    for position in source {
        if !projected.insert(*position) {
            return Err(MaterializationError::Projection(format!(
                "traversal blocker {position:?} was projected more than once"
            )));
        }
    }
    if projected.len() != source.len() {
        return Err(MaterializationError::Projection(
            "traversal blocker projection cardinality disagrees with semantic blockers".to_owned(),
        ));
    }
    Ok(projected)
}

fn project_biomes(
    source: &BTreeMap<TilePos, BiomeRegionId>,
) -> Result<BiomeRegions, MaterializationError> {
    let mut projected = BiomeRegions::new();
    for (position, region) in source {
        if projected.insert(*position, *region).is_some() {
            return Err(MaterializationError::Projection(format!(
                "biome surface {position:?} was projected more than once"
            )));
        }
    }
    if projected.len() != source.len() {
        return Err(MaterializationError::Projection(
            "biome projection cardinality disagrees with exact semantic membership".to_owned(),
        ));
    }
    Ok(projected)
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
    // This single ordered pass proves that every admitted semantic surface
    // survived voxelization with the same substance class and headroom. Later
    // consequence checks can therefore validate membership against the sealed
    // plan without repeating materialized column lookups.
    for (position, metadata) in &plan.volume.surfaces {
        let headroom = verify_materialized_surface(&plan.volume, map, *position, is_solid)?;
        if matches!(metadata.access, SurfaceAccess::SpecialMovement(_))
            && !TraversalProfile::WALKER.admits_surface(true, headroom)
        {
            return Err(MaterializationError::Projection(format!(
                "special-movement surface {position:?} is not ordinary walker footing after materialization"
            )));
        }
    }

    // Exact surface membership and materialized geometry were proven above;
    // these projections still have to prove their independent walker contract.
    for (name, position) in &plan.anchors {
        verify_planned_walker_surface(plan, *position, &format!("anchor {name:?}"))?;
    }
    for (name, position) in &plan.observation_anchors {
        if !plan.volume.surfaces.contains_key(position) {
            return Err(MaterializationError::Projection(format!(
                "observation anchor {name:?} at {position:?} is not an exact semantic surface"
            )));
        }
        let _headroom = verify_materialized_surface(&plan.volume, map, *position, is_solid)?;
    }
    for position in &plan.blockers {
        verify_planned_walker_surface(plan, *position, "traversal blocker")?;
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
        if !plan.volume.surfaces.contains_key(&light.origin) {
            return Err(MaterializationError::Projection(format!(
                "gameplay light {id:?} has an invalid materialized origin: {:?} is not an exact semantic surface",
                light.origin
            )));
        }
    }

    verify_special_regions(plan, special_regions)?;
    verify_interiors(plan, interiors, map, is_solid)?;
    Ok(())
}

fn verify_materialized_surface(
    volume: &super::volume::VolumePlan,
    map: &VoxelMap,
    position: TilePos,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<Headroom, MaterializationError> {
    let materialized_column = map.column(position.coord).ok_or_else(|| {
        MaterializationError::Projection(format!(
            "semantic surface {position:?} has no materialized column"
        ))
    })?;
    let substance = materialized_column.get(position.level);
    if !is_solid(substance) {
        return Err(MaterializationError::Projection(format!(
            "semantic surface {position:?} is not solid after materialization"
        )));
    }
    let materialized = materialized_column.headroom_above(position.level.saturating_add(1));
    let semantic = volume
        .columns
        .get(&position.coord)
        .map(|column| column.headroom_above(position.level.saturating_add(1)))
        .ok_or_else(|| {
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

fn verify_planned_walker_surface(
    plan: &GeneratedWorldPlan,
    position: TilePos,
    kind: &str,
) -> Result<(), MaterializationError> {
    let headroom = plan.volume.surface_headroom(position).ok_or_else(|| {
        MaterializationError::Projection(format!(
            "{kind} {position:?} is not an exact semantic surface"
        ))
    })?;
    if !TraversalProfile::WALKER.admits_surface(true, headroom) {
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
    let expected_count = plan
        .volume
        .surfaces
        .values()
        .filter(|metadata| matches!(metadata.access, SurfaceAccess::SpecialMovement(_)))
        .count();
    if actual.len() != expected_count
        || plan
            .volume
            .surfaces
            .iter()
            .any(|(position, metadata)| match metadata.access {
                SurfaceAccess::SpecialMovement(region) => actual.get(*position) != Some(region),
                SurfaceAccess::Ordinary | SurfaceAccess::NonStandable => false,
            })
    {
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
    let expected_floor_count = plan
        .interiors
        .by_id
        .values()
        .map(|interior| interior.floors.len())
        .sum::<usize>();
    let expected_roof_count = plan
        .interiors
        .by_id
        .values()
        .map(|interior| interior.roof_voxels.len())
        .sum::<usize>();
    if actual.surfaces().count() != expected_floor_count
        || actual.roof_voxels().count() != expected_roof_count
        || plan.interiors.by_id.iter().any(|(region, interior)| {
            interior
                .floors
                .iter()
                .any(|position| actual.get(*position) != Some(*region))
                || interior
                    .roof_voxels
                    .iter()
                    .any(|position| actual.roof_region(*position) != Some(*region))
        })
    {
        return Err(MaterializationError::Projection(
            "interior resource disagrees with exact floor or roof metadata".to_owned(),
        ));
    }
    for interior in plan.interiors.by_id.values() {
        for position in &interior.roof_voxels {
            if !is_solid(map.get(*position)) {
                return Err(MaterializationError::Projection(format!(
                    "interior roof voxel {position:?} is not solid after materialization"
                )));
            }
        }
    }
    Ok(())
}

fn fingerprint_materialized(
    plan: &GeneratedWorldPlan,
    map: &VoxelMap,
    liquids: &BTreeMap<TilePos, MaterializedLiquidVoxel>,
) -> Result<u64, String> {
    let capacity = materialized_fingerprint_capacity_hint(plan, map)?;
    let mut encoder = FingerprintEncoder::with_capacity(capacity);
    encoder.u32(3);

    // Conditional for legacy stability: only schematic-compiled worlds carry the
    // complete coarse-plan identity into their materialized map fingerprint.
    if let Some(source) = plan.source_schematic_fingerprint {
        encoder.tag(255);
        encoder.u64(source);
    }

    encoder.tag(0);
    encode_voxel_map(&mut encoder, map, &plan.volume)?;

    encoder.tag(1);
    encoder.collection_count(plan.anchors.len())?;
    for (name, position) in &plan.anchors {
        encoder.str(name)?;
        encoder.tile_pos(*position);
    }
    if !plan.observation_anchors.is_empty() {
        encoder.tag(254);
        encoder.collection_count(plan.observation_anchors.len())?;
        for (name, position) in &plan.observation_anchors {
            encoder.str(name)?;
            encoder.tile_pos(*position);
        }
    }

    encoder.tag(2);
    encode_projected_special_regions(&mut encoder, plan)?;

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

fn materialized_fingerprint_capacity_hint(
    plan: &GeneratedWorldPlan,
    map: &VoxelMap,
) -> Result<usize, String> {
    const ENCODED_COLUMN_BYTES: usize = 12 + 8;
    const ENCODED_VOXEL_BYTES: usize = 16 + 2;
    const ENCODED_BIOME_BYTES: usize = 16 + 4;

    // These are the dominant collections in a large schematic world. Reserving
    // them plus bounded headroom avoids repeatedly copying a tens-of-megabytes
    // payload as `Vec` grows. The remaining projections may use some or all of
    // the headroom; an underestimate affects capacity only, never encoded bytes.
    let dominant = map
        .columns()
        .try_fold(0_usize, |encoded, (_coord, column)| {
            let voxels = column
                .iter()
                .len()
                .checked_mul(ENCODED_VOXEL_BYTES)
                .ok_or_else(|| {
                    "materialized voxel fingerprint capacity exceeds usize".to_owned()
                })?;
            encoded
                .checked_add(ENCODED_COLUMN_BYTES)
                .and_then(|encoded| encoded.checked_add(voxels))
                .ok_or_else(|| "materialized voxel fingerprint capacity exceeds usize".to_owned())
        })?;
    let biome_bytes = plan
        .biome_regions
        .len()
        .checked_mul(ENCODED_BIOME_BYTES)
        .ok_or_else(|| "materialized biome fingerprint capacity exceeds usize".to_owned())?;
    let lower_bound = dominant
        .checked_add(biome_bytes)
        .and_then(|encoded| encoded.checked_add(256))
        .ok_or_else(|| "materialized fingerprint capacity exceeds usize".to_owned())?;
    lower_bound
        .checked_add(lower_bound / 4)
        .ok_or_else(|| "materialized fingerprint capacity headroom exceeds usize".to_owned())
}

fn encode_voxel_map(
    encoder: &mut FingerprintEncoder,
    map: &VoxelMap,
    volume: &super::volume::VolumePlan,
) -> Result<(), String> {
    if map.len() != volume.columns.len() {
        return Err(format!(
            "materialized voxel map has {} columns but the admitted volume has {}",
            map.len(),
            volume.columns.len()
        ));
    }

    encoder.collection_count(volume.columns.len())?;
    // The admitted volume supplies canonical coordinate order without rebuilding
    // the chunk-native map as a temporary `BTreeMap`. Equal cardinality plus an
    // exact lookup for every admitted key also proves that the live map has no
    // missing or extra column.
    for coord in volume.columns.keys().copied() {
        let column = map.column(coord).ok_or_else(|| {
            format!("materialized voxel map is missing admitted column {coord:?}")
        })?;
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

fn encode_projected_special_regions(
    encoder: &mut FingerprintEncoder,
    plan: &GeneratedWorldPlan,
) -> Result<(), String> {
    let count = plan
        .volume
        .surfaces
        .values()
        .filter(|metadata| matches!(metadata.access, SurfaceAccess::SpecialMovement(_)))
        .count();
    encoder.collection_count(count)?;
    for (position, metadata) in &plan.volume.surfaces {
        if let SurfaceAccess::SpecialMovement(region) = metadata.access {
            encoder.tile_pos(*position);
            encoder.u32(region.0);
        }
    }
    Ok(())
}

fn encode_projected_interiors(
    encoder: &mut FingerprintEncoder,
    plan: &GeneratedWorldPlan,
) -> Result<(), String> {
    let floor_count = plan
        .volume
        .surfaces
        .values()
        .filter(|metadata| metadata.interior.is_some())
        .count();
    encoder.collection_count(floor_count)?;
    for (position, metadata) in &plan.volume.surfaces {
        if let Some(region) = metadata.interior {
            encoder.tile_pos(*position);
            encoder.u32(region.0);
        }
    }

    let roof_count = plan
        .volume
        .columns
        .values()
        .flat_map(|column| &column.elements)
        .filter_map(|element| match *element {
            VolumeElement::Solid(mass) if mass.cutaway_for.is_some() => Some(mass.levels),
            VolumeElement::Solid(_) | VolumeElement::Fill(_) => None,
        })
        .try_fold(0_usize, |count, levels| {
            let run_length = levels.top.checked_sub(levels.bottom).ok_or_else(|| {
                format!(
                    "interior roof run {:?}..{:?} overflows its exact level difference",
                    levels.bottom, levels.top
                )
            })?;
            let run_count = usize::try_from(run_length)
                .map_err(|error| format!("interior roof run exceeds usize: {error}"))?;
            count
                .checked_add(run_count)
                .ok_or_else(|| "interior roof voxel count exceeds usize".to_owned())
        })?;
    encoder.collection_count(roof_count)?;
    for (coord, column) in &plan.volume.columns {
        for element in &column.elements {
            let VolumeElement::Solid(mass) = *element else {
                continue;
            };
            let Some(region) = mass.cutaway_for else {
                continue;
            };
            for level in mass.levels.bottom..mass.levels.top {
                encoder.tile_pos(TilePos::new(*coord, level));
                encoder.u32(region.0);
            }
        }
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
            FeatureKind::CaveVegetation => 2,
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
    use hex_core::{HexCoord, InteriorRegionId, SpecialMovementRegion};

    fn palette() -> TerrainPalette {
        TerrainPalette {
            bedrock: SubstanceId(1),
            stone: SubstanceId(2),
            dirt: SubstanceId(3),
            grass: SubstanceId(4),
            gravel: SubstanceId(5),
            sand: SubstanceId(13),
            water: SubstanceId(6),
            metal: SubstanceId(7),
            worked_stone: SubstanceId(12),
            snow: SubstanceId(8),
            ice: SubstanceId(9),
            basalt: SubstanceId(10),
            lava: SubstanceId(11),
        }
    }

    fn is_solid(substance: SubstanceId) -> bool {
        matches!(substance.0, 1 | 2 | 3 | 4 | 5 | 7 | 8 | 9 | 10 | 13)
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
            rotation_turns: 0,
            mask: mask.clone(),
            edges,
        };
        let layout = ResolvedLayoutPlan {
            kind: LayoutKind::Single,
            grid_radius: 12,
            footprint: mask.clone(),
            patches: BTreeMap::from([(PatchId(0), patch)]),
            shared_edges: BTreeMap::new(),
            boundary_liquid_outlets: BTreeMap::new(),
        };
        let anchor = TilePos::new(anchor_coord, 0);
        let tree = TilePos::new(tree_coord, 0);
        let light = TilePos::new(light_coord, 0);
        let structure = TilePos::new(structure_coord, 0);
        let water = TilePos::new(water_coord, 1);
        let roof = TilePos::new(anchor_coord, 3);

        GeneratedWorldPlan {
            source_schematic_fingerprint: None,
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
                            object_id: hex_assets::ObjectAssetId::new("plant/small-broadleaf")
                                .expect("fixture id should be valid"),
                            rotation: hex_assets::HexObjectRotation::ZERO,
                            blocker_footprint: BTreeSet::from([tree]),
                        },
                    ),
                    (
                        FeatureId(1),
                        PlannedFeature {
                            root: light,
                            kind: FeatureKind::TallGrass,
                            object_id: hex_assets::ObjectAssetId::new("prop/grass-tuft")
                                .expect("fixture id should be valid"),
                            rotation: hex_assets::HexObjectRotation::ZERO,
                            blocker_footprint: BTreeSet::new(),
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
            observation_anchors: BTreeMap::new(),
            view_hint: MapViewHint::new((0.0, 20.0, 20.0), (0.0, 1.0, 0.0)),
        }
    }

    fn validated(plan: GeneratedWorldPlan) -> ValidatedWorldPlan {
        ValidatedWorldPlan::validate_complete(plan)
            .and_then(super::super::selection::CompleteWorldAdmission::fingerprint)
            .expect("the materialization fixture must satisfy complete-world admission")
    }

    #[test]
    fn invalid_volume_cannot_obtain_materialization_admission() {
        let mut plan = valid_plan(5);
        let _removed = plan
            .volume
            .columns
            .pop_first()
            .expect("the fixture has a generated column");

        let error = ValidatedWorldPlan::validate_complete(plan)
            .expect_err("incomplete volume coverage must fail before materialization");
        assert!(error
            .to_string()
            .contains("volume columns do not exactly cover the mask"));
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
    fn observation_anchor_materializes_on_an_exact_nonstandable_surface() {
        let mut plan = valid_plan(5);
        let observation = plan
            .volume
            .surfaces
            .iter()
            .find_map(|(position, metadata)| {
                (metadata.access == SurfaceAccess::NonStandable).then_some(*position)
            })
            .expect("the fixture contains one exact nonstandable waterbed");
        plan.observation_anchors
            .insert("review.waterbed".to_owned(), observation);

        let output = materialize(validated(plan), &palette(), &is_solid)
            .expect("an exact observation surface materializes without walker admission");
        assert!(output
            .anchors
            .get(&MapAnchorId::from("review.waterbed"))
            .is_none());
        assert_eq!(
            output
                .observation_anchors
                .get(&MapAnchorId::from("review.waterbed")),
            Some(observation)
        );
        assert!(output
            .anchors
            .get(&MapAnchorId::from("party_start"))
            .is_some());
    }

    #[test]
    fn anchor_projection_keeps_both_namespaces_separate_and_rejects_collisions() {
        let walker = BTreeMap::from([("party_start".to_owned(), TilePos::ORIGIN)]);
        let observation = BTreeMap::from([(
            "review.target".to_owned(),
            TilePos::new(HexCoord::ORIGIN, 6),
        )]);
        let (projected_walker, projected_observation) = project_anchors(&walker, &observation)
            .expect("disjoint walker and observation names project separately");
        assert_eq!(projected_walker.len(), 1);
        assert_eq!(projected_observation.len(), 1);

        let collision =
            BTreeMap::from([("party_start".to_owned(), TilePos::new(HexCoord::ORIGIN, 6))]);
        assert!(matches!(
            project_anchors(&walker, &collision),
            Err(MaterializationError::Projection(message))
                if message.contains("duplicate stable identity")
        ));
    }

    #[test]
    fn feature_projection_is_ordered_and_protects_the_authored_root_volume() {
        let output = materialize(validated(valid_plan(5)), &palette(), &is_solid)
            .expect("the valid world materializes");
        let projected: Vec<_> = output
            .presentation
            .features()
            .iter()
            .map(|(id, feature)| (*id, feature.clone()))
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
    fn materialized_fingerprint_conditionally_covers_observation_anchors() {
        let baseline = materialize(validated(valid_plan(5)), &palette(), &is_solid)
            .expect("the baseline world materializes");
        let mut plan = valid_plan(5);
        let observation = plan
            .volume
            .surfaces
            .iter()
            .find_map(|(position, metadata)| {
                (metadata.access == SurfaceAccess::NonStandable).then_some(*position)
            })
            .expect("the fixture contains one exact nonstandable waterbed");
        plan.observation_anchors
            .insert("review.waterbed".to_owned(), observation);
        let observed = materialize(validated(plan), &palette(), &is_solid)
            .expect("the observation world materializes");

        assert_ne!(baseline.semantic_fingerprint, observed.semantic_fingerprint);
        assert_ne!(
            baseline.materialized_fingerprint,
            observed.materialized_fingerprint
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
        let plan = valid_plan(5);
        let waterbed = plan
            .volume
            .surfaces
            .iter()
            .find_map(|(position, metadata)| {
                (metadata.access == SurfaceAccess::NonStandable).then_some(*position)
            })
            .expect("the fixture contains a waterbed");
        let mut selected = validated(plan);
        selected
            .plan
            .anchors
            .insert("party_start".to_owned(), waterbed);
        selected.semantic_fingerprint = semantic_plan_fingerprint(&selected.plan)
            .expect("the deliberately forged test plan remains encodable");

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
