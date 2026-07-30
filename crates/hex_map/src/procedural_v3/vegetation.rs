//! Shared private authored-vegetation resolution and exact placement projection.
//!
//! The object catalog remains the visual source of truth. Recipes use this module
//! only to reserve the exact rotated volume and to publish separately authored
//! blocker footprints; visual occupancy never becomes gameplay collision by
//! implication.

use std::collections::{BTreeMap, BTreeSet};

use hex_assets::{
    HexObjectRotation, LocalAxialCoord, LocalVoxelCoord, ObjectAssetId, ObjectBlueprint,
    ObjectCategory, ObjectPart, PlantPart, RuntimeArtCatalog,
};
use hex_core::{HexCoord, TilePos};
use xxhash_rust::xxh3::xxh3_64;

use super::seed::SeedStream;
use super::world::{FeatureId, FeatureKind, FeaturePlan, PlannedFeature};
use crate::settings::V3EnvironmentSettings;

pub(super) const SMALL_BROADLEAF_ID: &str = "plant/small-broadleaf";
pub(super) const TALL_NARROW_ID: &str = "plant/tall-narrow";
pub(super) const OLD_GROWTH_ID: &str = "plant/old-growth";
pub(super) const GRASS_TUFT_ID: &str = "prop/grass-tuft";
pub(super) const SNOWY_SMALL_BROADLEAF_ID: &str = "plant/snowy-small-broadleaf";
pub(super) const SNOWY_TALL_NARROW_ID: &str = "plant/snowy-tall-narrow";
pub(super) const SNOWY_OLD_GROWTH_ID: &str = "plant/snowy-old-growth";
pub(super) const SNOWY_GRASS_TUFT_ID: &str = "prop/snowy-grass-tuft";
pub(super) const CAVE_MOSS_ID: &str = "prop/cave-moss";
pub(super) const CAVE_LICHEN_ID: &str = "prop/cave-lichen";

#[derive(Debug, Clone)]
pub(super) struct TemperateVegetationSet {
    pub(super) small_broadleaf: VegetationObjectSpec,
    pub(super) tall_narrow: VegetationObjectSpec,
    pub(super) old_growth: VegetationObjectSpec,
    pub(super) grass_tuft: VegetationObjectSpec,
}

impl TemperateVegetationSet {
    pub(super) fn resolve(catalog: &RuntimeArtCatalog, recipe: &str) -> Result<Self, String> {
        Ok(Self {
            small_broadleaf: VegetationObjectSpec::resolve(
                catalog,
                SMALL_BROADLEAF_ID,
                ObjectCategory::Plant,
                1,
                recipe,
            )?,
            tall_narrow: VegetationObjectSpec::resolve(
                catalog,
                TALL_NARROW_ID,
                ObjectCategory::Plant,
                1,
                recipe,
            )?,
            old_growth: VegetationObjectSpec::resolve(
                catalog,
                OLD_GROWTH_ID,
                ObjectCategory::Plant,
                7,
                recipe,
            )?,
            grass_tuft: VegetationObjectSpec::resolve(
                catalog,
                GRASS_TUFT_ID,
                ObjectCategory::Prop,
                0,
                recipe,
            )?,
        })
    }
}

/// Narrow authored authority for recipes that consume only the grass tuft.
#[derive(Debug, Clone)]
pub(super) struct GrassVegetationSpec {
    pub(super) grass_tuft: VegetationObjectSpec,
}

impl GrassVegetationSpec {
    pub(super) fn resolve(catalog: &RuntimeArtCatalog, recipe: &str) -> Result<Self, String> {
        Ok(Self {
            grass_tuft: VegetationObjectSpec::resolve(
                catalog,
                GRASS_TUFT_ID,
                ObjectCategory::Prop,
                0,
                recipe,
            )?,
        })
    }
}

/// Snow-covered counterparts resolved through the same exact authored contract.
#[derive(Debug, Clone)]
pub(super) struct SnowyVegetationSet {
    pub(super) small_broadleaf: VegetationObjectSpec,
    pub(super) tall_narrow: VegetationObjectSpec,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the snowy old-growth variant is catalogued for galleries; sparse frozen landforms deliberately use smaller trees"
        )
    )]
    pub(super) old_growth: VegetationObjectSpec,
    pub(super) grass_tuft: VegetationObjectSpec,
}

impl SnowyVegetationSet {
    pub(super) fn resolve(catalog: &RuntimeArtCatalog, recipe: &str) -> Result<Self, String> {
        Ok(Self {
            small_broadleaf: VegetationObjectSpec::resolve(
                catalog,
                SNOWY_SMALL_BROADLEAF_ID,
                ObjectCategory::Plant,
                1,
                recipe,
            )?,
            tall_narrow: VegetationObjectSpec::resolve(
                catalog,
                SNOWY_TALL_NARROW_ID,
                ObjectCategory::Plant,
                1,
                recipe,
            )?,
            old_growth: VegetationObjectSpec::resolve(
                catalog,
                SNOWY_OLD_GROWTH_ID,
                ObjectCategory::Plant,
                7,
                recipe,
            )?,
            grass_tuft: VegetationObjectSpec::resolve(
                catalog,
                SNOWY_GRASS_TUFT_ID,
                ObjectCategory::Prop,
                0,
                recipe,
            )?,
        })
    }
}

/// Exact temperate or snow-covered vegetation used by non-Forest landforms.
#[derive(Debug, Clone)]
pub(super) struct LandformVegetationSet {
    small_broadleaf: VegetationObjectSpec,
    tall_narrow: VegetationObjectSpec,
    grass_tuft: VegetationObjectSpec,
}

impl LandformVegetationSet {
    pub(super) fn resolve(
        catalog: &RuntimeArtCatalog,
        environment: V3EnvironmentSettings,
        recipe: &str,
    ) -> Result<Self, String> {
        match environment {
            V3EnvironmentSettings::TemperateGrassland => {
                let set = TemperateVegetationSet::resolve(catalog, recipe)?;
                Ok(Self {
                    small_broadleaf: set.small_broadleaf,
                    tall_narrow: set.tall_narrow,
                    grass_tuft: set.grass_tuft,
                })
            }
            V3EnvironmentSettings::Frozen => {
                let set = SnowyVegetationSet::resolve(catalog, recipe)?;
                Ok(Self {
                    small_broadleaf: set.small_broadleaf,
                    tall_narrow: set.tall_narrow,
                    grass_tuft: set.grass_tuft,
                })
            }
            unsupported => Err(format!(
                "{recipe} cannot resolve landform vegetation for {unsupported:?}"
            )),
        }
    }

    fn object(&self, id: &ObjectAssetId) -> Option<&VegetationObjectSpec> {
        [&self.small_broadleaf, &self.tall_narrow, &self.grass_tuft]
            .into_iter()
            .find(|object| object.id == *id)
    }

    fn tree_order(&self, hash: u64) -> [&VegetationObjectSpec; 2] {
        if hash.is_multiple_of(3) {
            [&self.tall_narrow, &self.small_broadleaf]
        } else {
            [&self.small_broadleaf, &self.tall_narrow]
        }
    }
}

/// Deterministic feature counts produced by one shared placement pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LandformVegetationMetrics {
    pub(super) trees: usize,
    pub(super) grass: usize,
}

/// One independently validated support layer and its horizontally reserved columns.
#[derive(Debug, Clone, Copy)]
pub(super) struct LandformVegetationDomain<'a> {
    pub(super) surfaces: &'a BTreeMap<HexCoord, TilePos>,
    pub(super) reserved: &'a BTreeSet<HexCoord>,
}

/// Reprojects every accepted authored object instead of trusting stored footprints.
///
/// Recipes remain responsible for their semantic density and root-eligibility
/// policies. This shared boundary proves the lower-level spatial invariants common
/// to every landform: exact support, complete visual clearance, reserved-column
/// clearance, pairwise object separation, and blocker equality.
pub(super) fn validate_landform_vegetation(
    recipe: &str,
    objects: &LandformVegetationSet,
    domains: &[LandformVegetationDomain<'_>],
    features: &FeaturePlan,
    nonvegetation_blockers: &BTreeSet<TilePos>,
    blockers: &BTreeSet<TilePos>,
) -> Result<LandformVegetationMetrics, Vec<String>> {
    let mut issues = Vec::new();
    let mut metrics = LandformVegetationMetrics { trees: 0, grass: 0 };
    let mut roots = BTreeSet::new();
    let mut occupied_visual = BTreeSet::new();
    let mut projected_blockers = BTreeSet::new();

    for (id, feature) in &features.by_id {
        let Some(object) = objects.object(&feature.object_id) else {
            issues.push(format!(
                "{recipe} feature {id:?} uses unsupported authored object '{}'",
                feature.object_id
            ));
            continue;
        };
        let expected_kind = if object.id == objects.grass_tuft.id {
            metrics.grass = metrics.grass.saturating_add(1);
            FeatureKind::TallGrass
        } else {
            metrics.trees = metrics.trees.saturating_add(1);
            FeatureKind::Tree
        };
        if feature.kind != expected_kind {
            issues.push(format!(
                "{recipe} feature {id:?} publishes {:?} for object '{}'; expected {expected_kind:?}",
                feature.kind, feature.object_id
            ));
        }
        if !roots.insert(feature.root) {
            issues.push(format!(
                "{recipe} feature {id:?} object '{}' reuses authored root {:?}",
                feature.object_id, feature.root
            ));
        }

        let mut matching_domains = domains.iter().filter(|domain| {
            domain.surfaces.get(&feature.root.coord).copied() == Some(feature.root)
        });
        let Some(domain) = matching_domains.next() else {
            issues.push(format!(
                "{recipe} feature {id:?} object '{}' root {:?} is not exact support in any accepted landform layer",
                feature.object_id, feature.root
            ));
            continue;
        };
        if matching_domains.next().is_some() {
            issues.push(format!(
                "{recipe} feature {id:?} object '{}' root {:?} ambiguously belongs to multiple support layers",
                feature.object_id, feature.root
            ));
            continue;
        }

        let Some(volume) = object.project_visual_volume(feature.root, feature.rotation) else {
            issues.push(format!(
                "{recipe} feature {id:?} cannot project object '{}' at {:?}",
                feature.object_id, feature.root
            ));
            continue;
        };
        if let Some(cell) = volume
            .cells
            .iter()
            .find(|cell| domain.reserved.contains(&cell.coord))
        {
            issues.push(format!(
                "{recipe} feature {id:?} object '{}' enters reserved column {:?} at {cell:?}",
                feature.object_id, cell.coord
            ));
        }
        if let Some(cell) = volume.cells.iter().find(|cell| {
            domain
                .surfaces
                .get(&cell.coord)
                .is_none_or(|support| cell.level <= support.level)
        }) {
            issues.push(format!(
                "{recipe} feature {id:?} object '{}' leaves or intersects its support layer at {cell:?}",
                feature.object_id
            ));
        }
        if let Some(cell) = volume.cells.intersection(&occupied_visual).next() {
            issues.push(format!(
                "{recipe} feature {id:?} object '{}' overlaps neighboring authored vegetation at {cell:?}",
                feature.object_id
            ));
        }
        occupied_visual.extend(volume.cells);

        let Some(exact_blockers) =
            object.project_blockers(feature.root, feature.rotation, domain.surfaces)
        else {
            issues.push(format!(
                "{recipe} feature {id:?} cannot project the exact blocker footprint for object '{}'",
                feature.object_id
            ));
            continue;
        };
        if exact_blockers != feature.blocker_footprint {
            issues.push(format!(
                "{recipe} feature {id:?} stores blocker footprint {:?}, but object '{}' projects {:?}",
                feature.blocker_footprint, feature.object_id, exact_blockers
            ));
        }
        if let Some(blocker) = exact_blockers
            .iter()
            .find(|blocker| domain.reserved.contains(&blocker.coord))
        {
            issues.push(format!(
                "{recipe} feature {id:?} object '{}' blocker {blocker:?} enters a reserved column",
                feature.object_id
            ));
        }
        if let Some(blocker) = exact_blockers.intersection(&projected_blockers).next() {
            issues.push(format!(
                "{recipe} feature {id:?} object '{}' blocker {blocker:?} overlaps a neighboring authored object",
                feature.object_id
            ));
        }
        if let Some(blocker) = exact_blockers.intersection(nonvegetation_blockers).next() {
            issues.push(format!(
                "{recipe} feature {id:?} object '{}' projects blocker {blocker:?} over non-vegetation blocker authority",
                feature.object_id
            ));
        }
        projected_blockers.extend(exact_blockers);
    }

    let expected_blockers = projected_blockers
        .union(nonvegetation_blockers)
        .copied()
        .collect::<BTreeSet<_>>();
    if expected_blockers != *blockers {
        issues.push(format!(
            "{recipe} blockers differ from the independently projected authored vegetation plus \
             declared non-vegetation authority (vegetation {projected_blockers:?}, \
             non-vegetation {nonvegetation_blockers:?}, published {blockers:?})"
        ));
    }
    if issues.is_empty() {
        Ok(metrics)
    } else {
        Err(issues)
    }
}

pub(super) fn landform_vegetation_metrics<'a>(
    recipe: &str,
    environment: V3EnvironmentSettings,
    features: impl IntoIterator<Item = &'a PlannedFeature>,
) -> Result<LandformVegetationMetrics, String> {
    let (tree_ids, grass_id) = match environment {
        V3EnvironmentSettings::TemperateGrassland => {
            ([SMALL_BROADLEAF_ID, TALL_NARROW_ID], GRASS_TUFT_ID)
        }
        V3EnvironmentSettings::Frozen => (
            [SNOWY_SMALL_BROADLEAF_ID, SNOWY_TALL_NARROW_ID],
            SNOWY_GRASS_TUFT_ID,
        ),
        _ => {
            let count = features.into_iter().count();
            return (count == 0)
                .then_some(LandformVegetationMetrics { trees: 0, grass: 0 })
                .ok_or_else(|| {
                    format!(
                        "{recipe} publishes {count} authored vegetation features in unsupported \
                         {environment:?}"
                    )
                });
        }
    };
    let mut metrics = LandformVegetationMetrics { trees: 0, grass: 0 };
    for feature in features {
        let id = feature.object_id.as_str();
        if tree_ids.contains(&id) && feature.kind == FeatureKind::Tree {
            metrics.trees = metrics.trees.saturating_add(1);
        } else if id == grass_id && feature.kind == FeatureKind::TallGrass {
            metrics.grass = metrics.grass.saturating_add(1);
        } else {
            return Err(format!(
                "{recipe} feature '{}' has the wrong environment or semantic kind",
                feature.object_id
            ));
        }
    }
    Ok(metrics)
}

/// Adds exact authored vegetation while preserving all pre-existing feature IDs.
///
/// Every selected tree is tried through all six rotations. A placement is admitted
/// only when its complete visual volume remains above known support, inside the
/// supplied surface domain, clear of reserved horizontal columns, and disjoint from
/// neighboring authored vegetation. Tree blocker footprints are projected from the
/// same accepted rotation.
#[expect(
    clippy::too_many_arguments,
    reason = "the shared placement boundary keeps each spatial authority explicit"
)]
pub(super) fn append_landform_vegetation(
    recipe: &str,
    objects: &LandformVegetationSet,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    tree_candidates: &BTreeSet<HexCoord>,
    grass_candidates: &BTreeSet<HexCoord>,
    reserved: &BTreeSet<HexCoord>,
    tree_target: usize,
    grass_target: usize,
    tree_stream: Option<SeedStream<'_>>,
    grass_stream: Option<SeedStream<'_>>,
    features: &mut FeaturePlan,
    blockers: &mut BTreeSet<TilePos>,
) -> Result<LandformVegetationMetrics, String> {
    let mut occupied_visual = BTreeSet::new();
    for feature in features.by_id.values() {
        let Some(object) = objects.object(&feature.object_id) else {
            continue;
        };
        let projected = object
            .project_visual_volume(feature.root, feature.rotation)
            .ok_or_else(|| {
                format!(
                    "{recipe} existing object '{}' cannot project its accepted rotation at {:?}",
                    feature.object_id, feature.root
                )
            })?;
        occupied_visual.extend(projected.cells);
    }
    let mut occupied_blockers = blockers.clone();
    let mut planned = Vec::with_capacity(tree_target.saturating_add(grass_target));

    let mut tree_roots = tree_candidates
        .iter()
        .filter_map(|coord| {
            (!reserved.contains(coord))
                .then(|| surfaces.get(coord).copied())
                .flatten()
        })
        .collect::<Vec<_>>();
    tree_roots.sort_unstable_by_key(|root| {
        (
            vegetation_priority(tree_stream, root.coord, 0),
            root.coord,
            root.level,
        )
    });
    for root in tree_roots {
        if planned
            .iter()
            .filter(|feature: &&PlannedFeature| feature.kind == FeatureKind::Tree)
            .count()
            >= tree_target
        {
            break;
        }
        let family_hash = vegetation_priority(tree_stream, root.coord, 17);
        let first_rotation = vegetation_rotation(tree_stream, root.coord, 29, recipe)?;
        let mut selected = None;
        for object in objects.tree_order(family_hash) {
            for offset in 0..6 {
                let rotation =
                    HexObjectRotation::new(first_rotation.steps().saturating_add(offset) % 6)
                        .map_err(|error| {
                            format!("{recipe} authored tree rotation failed: {error}")
                        })?;
                let Some((visual, projected_blockers)) = project_landform_object(
                    object,
                    root,
                    rotation,
                    surfaces,
                    reserved,
                    &occupied_visual,
                    &occupied_blockers,
                ) else {
                    continue;
                };
                selected = Some((object, rotation, visual, projected_blockers));
                break;
            }
            if selected.is_some() {
                break;
            }
        }
        let Some((object, rotation, visual, projected_blockers)) = selected else {
            continue;
        };
        occupied_visual.extend(visual);
        occupied_blockers.extend(projected_blockers.iter().copied());
        planned.push(PlannedFeature {
            root,
            kind: FeatureKind::Tree,
            object_id: object.id.clone(),
            rotation,
            blocker_footprint: projected_blockers,
        });
    }
    let tree_count = planned
        .iter()
        .filter(|feature| feature.kind == FeatureKind::Tree)
        .count();
    if tree_count != tree_target {
        return Err(format!(
            "{recipe} exact authored bounds can place only {tree_count} trees; expected {tree_target}"
        ));
    }

    let mut grass_roots = grass_candidates
        .iter()
        .filter_map(|coord| {
            (!reserved.contains(coord))
                .then(|| surfaces.get(coord).copied())
                .flatten()
        })
        .collect::<Vec<_>>();
    grass_roots.sort_unstable_by_key(|root| {
        (
            vegetation_priority(grass_stream, root.coord, 0),
            root.coord,
            root.level,
        )
    });
    let grass_start = planned.len();
    for root in grass_roots {
        if planned.len().saturating_sub(grass_start) >= grass_target {
            break;
        }
        let rotation = vegetation_rotation(grass_stream, root.coord, 41, recipe)?;
        let Some((visual, projected_blockers)) = project_landform_object(
            &objects.grass_tuft,
            root,
            rotation,
            surfaces,
            reserved,
            &occupied_visual,
            &occupied_blockers,
        ) else {
            continue;
        };
        if !projected_blockers.is_empty() {
            return Err(format!(
                "{recipe} grass '{}' unexpectedly projected blockers",
                objects.grass_tuft.id
            ));
        }
        occupied_visual.extend(visual);
        planned.push(PlannedFeature {
            root,
            kind: FeatureKind::TallGrass,
            object_id: objects.grass_tuft.id.clone(),
            rotation,
            blocker_footprint: BTreeSet::new(),
        });
    }
    let grass_count = planned.len().saturating_sub(grass_start);
    if grass_count != grass_target {
        return Err(format!(
            "{recipe} exact authored bounds can place only {grass_count} grass tufts; expected {grass_target}"
        ));
    }

    let first_id = features
        .by_id
        .keys()
        .next_back()
        .map_or(0, |id| id.0.saturating_add(1));
    for (offset, feature) in planned.into_iter().enumerate() {
        let id = FeatureId(first_id.saturating_add(u32::try_from(offset).unwrap_or(u32::MAX)));
        if features.by_id.insert(id, feature).is_some() {
            return Err(format!("{recipe} reused feature id {id:?}"));
        }
    }
    *blockers = occupied_blockers;
    Ok(LandformVegetationMetrics {
        trees: tree_count,
        grass: grass_count,
    })
}

fn project_landform_object(
    object: &VegetationObjectSpec,
    root: TilePos,
    rotation: HexObjectRotation,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    reserved: &BTreeSet<HexCoord>,
    occupied_visual: &BTreeSet<TilePos>,
    occupied_blockers: &BTreeSet<TilePos>,
) -> Option<(BTreeSet<TilePos>, BTreeSet<TilePos>)> {
    let projected_blockers = object.project_blockers(root, rotation, surfaces)?;
    if projected_blockers
        .iter()
        .any(|position| reserved.contains(&position.coord) || occupied_blockers.contains(position))
    {
        return None;
    }
    let volume = object.project_visual_volume(root, rotation)?;
    if !volume.cells.is_disjoint(occupied_visual)
        || volume.cells.iter().any(|cell| {
            reserved.contains(&cell.coord)
                || surfaces
                    .get(&cell.coord)
                    .is_none_or(|support| cell.level <= support.level)
        })
    {
        return None;
    }
    Some((volume.cells, projected_blockers))
}

fn vegetation_rotation(
    stream: Option<SeedStream<'_>>,
    coord: HexCoord,
    salt: u64,
    recipe: &str,
) -> Result<HexObjectRotation, String> {
    let steps = u8::try_from(vegetation_priority(stream, coord, salt) % 6).unwrap_or_default();
    HexObjectRotation::new(steps)
        .map_err(|error| format!("{recipe} authored vegetation rotation failed: {error}"))
}

fn vegetation_priority(stream: Option<SeedStream<'_>>, coord: HexCoord, salt: u64) -> u64 {
    stream.map_or_else(
        || {
            let mut bytes = Vec::with_capacity(56);
            bytes.extend_from_slice(b"bevy-hex-game/v3/landform-vegetation");
            bytes.extend_from_slice(&coord.x().to_le_bytes());
            bytes.extend_from_slice(&coord.y().to_le_bytes());
            bytes.extend_from_slice(&coord.z().to_le_bytes());
            bytes.extend_from_slice(&salt.to_le_bytes());
            xxh3_64(&bytes)
        },
        |stream| stream.sample_coord(coord, salt),
    )
}

/// Nonblocking cave vegetation resolved as an exact pair of authored props.
#[derive(Debug, Clone)]
pub(super) struct CaveVegetationSet {
    pub(super) moss: VegetationObjectSpec,
    pub(super) lichen: VegetationObjectSpec,
}

impl CaveVegetationSet {
    pub(super) fn resolve(catalog: &RuntimeArtCatalog, recipe: &str) -> Result<Self, String> {
        Ok(Self {
            moss: VegetationObjectSpec::resolve(
                catalog,
                CAVE_MOSS_ID,
                ObjectCategory::Prop,
                0,
                recipe,
            )?,
            lichen: VegetationObjectSpec::resolve(
                catalog,
                CAVE_LICHEN_ID,
                ObjectCategory::Prop,
                0,
                recipe,
            )?,
        })
    }

    pub(super) fn object(&self, id: &ObjectAssetId) -> Option<&VegetationObjectSpec> {
        [&self.moss, &self.lichen]
            .into_iter()
            .find(|object| object.id == *id)
    }
}

#[derive(Debug, Clone)]
pub(super) struct VegetationObjectSpec {
    pub(super) id: ObjectAssetId,
    origin: LocalVoxelCoord,
    blocker_footprint: Vec<LocalAxialCoord>,
    occupied: Vec<LocalVoxelCoord>,
    rigid: BTreeSet<LocalVoxelCoord>,
}

#[derive(Debug)]
pub(super) struct ProjectedVegetationVolume {
    pub(super) cells: BTreeSet<TilePos>,
    pub(super) structural_cells: BTreeSet<TilePos>,
}

impl VegetationObjectSpec {
    fn resolve(
        catalog: &RuntimeArtCatalog,
        raw_id: &str,
        expected_category: ObjectCategory,
        expected_blocker_cells: usize,
        recipe: &str,
    ) -> Result<Self, String> {
        let id = ObjectAssetId::new(raw_id).map_err(|error| {
            format!("{recipe} authored-object id {raw_id:?} is invalid: {error}")
        })?;
        let blueprint = catalog.object(&id).ok_or_else(|| {
            format!(
                "{recipe} requires authored object {raw_id:?}, but it is absent from the accepted \
                 catalog"
            )
        })?;
        validate_object(blueprint, expected_category, expected_blocker_cells, recipe)?;
        Ok(Self {
            id,
            origin: blueprint.origin,
            blocker_footprint: blueprint.blocker_footprint.clone(),
            occupied: blueprint
                .placements
                .iter()
                .map(|placement| placement.position)
                .collect(),
            rigid: blueprint
                .placements
                .iter()
                .filter(|placement| is_structural_part(blueprint.category, &placement.part))
                .map(|placement| placement.position)
                .collect(),
        })
    }

    pub(super) fn project_visual_volume(
        &self,
        root: TilePos,
        rotation: HexObjectRotation,
    ) -> Option<ProjectedVegetationVolume> {
        let visual_origin_level = root.level.checked_add(1)?;
        let mut cells = BTreeSet::new();
        let mut structural_cells = BTreeSet::new();
        for local in &self.occupied {
            let rotated = rotation.rotate_voxel(*local, self.origin)?;
            let coord = project_coord(root.coord, rotated.axial(), self.origin.axial())?;
            let relative_level = rotated.level.checked_sub(self.origin.level)?;
            let level = visual_origin_level.checked_add(relative_level)?;
            let position = TilePos::new(coord, level);
            cells.insert(position);
            if self.rigid.contains(local) {
                structural_cells.insert(position);
            }
        }
        (cells.len() == self.occupied.len()).then_some(ProjectedVegetationVolume {
            cells,
            structural_cells,
        })
    }

    pub(super) fn project_blockers(
        &self,
        root: TilePos,
        rotation: HexObjectRotation,
        surfaces: &BTreeMap<HexCoord, TilePos>,
    ) -> Option<BTreeSet<TilePos>> {
        let mut projected = BTreeSet::new();
        for local in &self.blocker_footprint {
            let rotated = rotation.rotate_axial(*local, self.origin.axial())?;
            let coord = project_coord(root.coord, rotated, self.origin.axial())?;
            let support = surfaces.get(&coord).copied()?;
            if support.level != root.level {
                return None;
            }
            projected.insert(support);
        }
        (projected.len() == self.blocker_footprint.len()).then_some(projected)
    }
}

fn is_structural_part(category: ObjectCategory, part: &ObjectPart) -> bool {
    category == ObjectCategory::Prop
        || matches!(
            part,
            ObjectPart::Plant(PlantPart::Root | PlantPart::Trunk | PlantPart::Branch)
        )
}

fn project_coord(
    world_origin: HexCoord,
    local: LocalAxialCoord,
    object_origin: LocalAxialCoord,
) -> Option<HexCoord> {
    let delta_q = local.q.checked_sub(object_origin.q)?;
    let delta_r = local.r.checked_sub(object_origin.r)?;
    Some(HexCoord::from_axial(
        world_origin.x().checked_add(delta_q)?,
        world_origin.y().checked_add(delta_r)?,
    ))
}

fn validate_object(
    blueprint: &ObjectBlueprint,
    expected_category: ObjectCategory,
    expected_blocker_cells: usize,
    recipe: &str,
) -> Result<(), String> {
    if blueprint.category != expected_category {
        return Err(format!(
            "{recipe} object '{}' is {:?}; expected {expected_category:?}",
            blueprint.id, blueprint.category
        ));
    }
    if blueprint.origin.level != 0 {
        return Err(format!(
            "{recipe} object '{}' must keep its authored origin at level zero",
            blueprint.id
        ));
    }
    if blueprint.blocker_footprint.len() != expected_blocker_cells {
        return Err(format!(
            "{recipe} object '{}' must define exactly {expected_blocker_cells} blocker cells; got \
             {}",
            blueprint.id,
            blueprint.blocker_footprint.len()
        ));
    }
    if expected_blocker_cells > 0
        && !blueprint
            .blocker_footprint
            .contains(&blueprint.origin.axial())
    {
        return Err(format!(
            "{recipe} tree '{}' must block its authored origin",
            blueprint.id
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::OnceLock;

    use super::*;
    use hex_assets::{ArtPalette, ObjectCatalogFile, VoxelStyleCatalog};

    pub(crate) fn runtime_art_catalog() -> &'static RuntimeArtCatalog {
        static CATALOG: OnceLock<RuntimeArtCatalog> = OnceLock::new();
        CATALOG.get_or_init(|| {
            let palette: ArtPalette = ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/art/palette.ron"
            )))
            .expect("tracked art palette should parse");
            let styles: VoxelStyleCatalog = ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/art/voxel_styles.ron"
            )))
            .expect("tracked voxel styles should parse");
            let manifest: ObjectCatalogFile = ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/art/object_catalog.ron"
            )))
            .expect("tracked object catalog should parse");
            let mut objects = BTreeMap::new();
            for source in [
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/plant/small-broadleaf.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/plant/tall-narrow.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/plant/old-growth.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/plant/snowy-old-growth.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/plant/snowy-small-broadleaf.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/plant/snowy-tall-narrow.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/cave-lichen.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/cave-moss.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/grass-tuft.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/snowy-grass-tuft.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/crystal-low-cluster.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/crystal-branched.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/crystal-spire.ron"
                )),
            ] {
                let blueprint: ObjectBlueprint =
                    ron::from_str(source).expect("tracked object blueprint should parse");
                objects.insert(blueprint.id.clone(), blueprint);
            }
            RuntimeArtCatalog::from_sources(&palette, &styles, &manifest, objects)
                .expect("tracked runtime art graph should resolve")
        })
    }

    pub(crate) fn grass_only_runtime_art_catalog() -> &'static RuntimeArtCatalog {
        static CATALOG: OnceLock<RuntimeArtCatalog> = OnceLock::new();
        CATALOG.get_or_init(|| {
            let palette: ArtPalette = ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/art/palette.ron"
            )))
            .expect("tracked art palette should parse");
            let styles: VoxelStyleCatalog = ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/art/voxel_styles.ron"
            )))
            .expect("tracked voxel styles should parse");
            let grass: ObjectBlueprint = ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/art/objects/prop/grass-tuft.ron"
            )))
            .expect("tracked grass blueprint should parse");
            let manifest = ObjectCatalogFile::new([grass.id.clone()])
                .expect("grass-only object manifest should validate");
            RuntimeArtCatalog::from_sources(
                &palette,
                &styles,
                &manifest,
                BTreeMap::from([(grass.id.clone(), grass)]),
            )
            .expect("grass-only runtime art graph should resolve")
        })
    }

    #[test]
    fn shared_environment_sets_resolve_exact_categories_and_blockers() {
        let snowy = SnowyVegetationSet::resolve(runtime_art_catalog(), "Frozen Hills")
            .expect("tracked snowy vegetation should resolve");
        assert_eq!(snowy.small_broadleaf.id.as_str(), SNOWY_SMALL_BROADLEAF_ID);
        assert_eq!(snowy.tall_narrow.id.as_str(), SNOWY_TALL_NARROW_ID);
        assert_eq!(snowy.old_growth.id.as_str(), SNOWY_OLD_GROWTH_ID);
        assert_eq!(snowy.grass_tuft.id.as_str(), SNOWY_GRASS_TUFT_ID);
        assert_eq!(snowy.small_broadleaf.blocker_footprint.len(), 1);
        assert_eq!(snowy.tall_narrow.blocker_footprint.len(), 1);
        assert_eq!(snowy.old_growth.blocker_footprint.len(), 7);
        assert!(snowy.grass_tuft.blocker_footprint.is_empty());

        let cave = CaveVegetationSet::resolve(runtime_art_catalog(), "Caves")
            .expect("tracked cave vegetation should resolve");
        assert_eq!(cave.moss.id.as_str(), CAVE_MOSS_ID);
        assert_eq!(cave.lichen.id.as_str(), CAVE_LICHEN_ID);
        assert!(cave.moss.blocker_footprint.is_empty());
        assert!(cave.lichen.blocker_footprint.is_empty());
    }

    #[test]
    fn woody_plant_support_and_complete_props_are_structural() {
        for part in [PlantPart::Root, PlantPart::Trunk, PlantPart::Branch] {
            assert!(is_structural_part(
                ObjectCategory::Plant,
                &ObjectPart::Plant(part)
            ));
        }
        assert!(!is_structural_part(
            ObjectCategory::Plant,
            &ObjectPart::Plant(PlantPart::Foliage)
        ));
        assert!(is_structural_part(
            ObjectCategory::Prop,
            &ObjectPart::Plant(PlantPart::Foliage)
        ));
    }

    #[test]
    fn exact_visual_projection_preserves_stack_levels_and_six_rotations() {
        let object = VegetationObjectSpec {
            id: ObjectAssetId::new("plant/test").expect("fixture id should be valid"),
            origin: LocalVoxelCoord::new(0, 0, 0),
            blocker_footprint: vec![LocalAxialCoord::new(0, 0)],
            occupied: vec![LocalVoxelCoord::new(0, 0, 0), LocalVoxelCoord::new(1, 0, 1)],
            rigid: BTreeSet::from([LocalVoxelCoord::new(0, 0, 0)]),
        };
        let root = TilePos::new(HexCoord::from_axial(4, -2), 12);
        for steps in 0..6 {
            let rotation = HexObjectRotation::new(steps).expect("fixture rotation should be valid");
            let cells = object
                .project_visual_volume(root, rotation)
                .expect("fixture cells should project");
            assert_eq!(cells.cells.len(), 2);
            assert_eq!(cells.structural_cells.len(), 1);
            assert!(cells.cells.contains(&TilePos::new(root.coord, 13)));
            assert!(cells.cells.iter().any(|cell| cell.level == 14));
        }
    }

    #[test]
    fn snowy_validation_reprojects_reserved_overlap_and_blocker_authority() {
        let objects = LandformVegetationSet::resolve(
            runtime_art_catalog(),
            V3EnvironmentSettings::Frozen,
            "Frozen validation fixture",
        )
        .expect("tracked snowy set should resolve");
        let root = TilePos::new(HexCoord::ORIGIN, 0);
        let rotation = HexObjectRotation::new(0).expect("zero rotation");
        let surfaces = HexCoord::ORIGIN
            .within_radius(3)
            .into_iter()
            .map(|coord| (coord, TilePos::new(coord, 0)))
            .collect::<BTreeMap<_, _>>();
        let blockers = objects
            .small_broadleaf
            .project_blockers(root, rotation, &surfaces)
            .expect("snowy tree blockers should project");
        let feature = PlannedFeature {
            root,
            kind: FeatureKind::Tree,
            object_id: objects.small_broadleaf.id.clone(),
            rotation,
            blocker_footprint: blockers.clone(),
        };
        let mut features = FeaturePlan::default();
        features.by_id.insert(FeatureId(0), feature.clone());
        let empty_reserved = BTreeSet::new();
        let empty_blockers = BTreeSet::new();
        let metrics = validate_landform_vegetation(
            "Frozen validation fixture",
            &objects,
            &[LandformVegetationDomain {
                surfaces: &surfaces,
                reserved: &empty_reserved,
            }],
            &features,
            &empty_blockers,
            &blockers,
        )
        .expect("the exact snowy feature should validate");
        assert_eq!(metrics, LandformVegetationMetrics { trees: 1, grass: 0 });

        let reserved_coord = objects
            .small_broadleaf
            .project_visual_volume(root, rotation)
            .expect("snowy tree volume should project")
            .cells
            .iter()
            .find(|cell| cell.coord != root.coord)
            .map(|cell| cell.coord)
            .expect("snowy tree canopy should leave its root column");
        let reserved = BTreeSet::from([reserved_coord]);
        let reserved_errors = validate_landform_vegetation(
            "Frozen validation fixture",
            &objects,
            &[LandformVegetationDomain {
                surfaces: &surfaces,
                reserved: &reserved,
            }],
            &features,
            &empty_blockers,
            &blockers,
        )
        .expect_err("a snowy canopy entering a reserved column must fail");
        assert!(reserved_errors.iter().any(|error| {
            error.contains(SNOWY_SMALL_BROADLEAF_ID) && error.contains("reserved column")
        }));

        let mut overlapping = features.clone();
        overlapping.by_id.insert(FeatureId(1), feature);
        let overlap_errors = validate_landform_vegetation(
            "Frozen validation fixture",
            &objects,
            &[LandformVegetationDomain {
                surfaces: &surfaces,
                reserved: &empty_reserved,
            }],
            &overlapping,
            &empty_blockers,
            &blockers,
        )
        .expect_err("neighboring authored volumes may not overlap");
        assert!(overlap_errors
            .iter()
            .any(|error| error.contains("overlaps neighboring authored vegetation")));

        let mut stale_footprint = features;
        stale_footprint
            .by_id
            .get_mut(&FeatureId(0))
            .expect("fixture feature should remain present")
            .blocker_footprint
            .clear();
        let blocker_errors = validate_landform_vegetation(
            "Frozen validation fixture",
            &objects,
            &[LandformVegetationDomain {
                surfaces: &surfaces,
                reserved: &empty_reserved,
            }],
            &stale_footprint,
            &empty_blockers,
            &blockers,
        )
        .expect_err("stored blockers must equal the accepted authored projection");
        assert!(blocker_errors
            .iter()
            .any(|error| error.contains("stores blocker footprint")));
    }
}
