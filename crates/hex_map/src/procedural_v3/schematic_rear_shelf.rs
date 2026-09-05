//! Sparse snowy vegetation on the connected eastern upper-lake apron.
//!
//! `prepare` records exact terrain before placement. `author` runs after ordinary
//! vegetation; it changes only new features and their exact blocked roots.

use super::*;

const SEED_OWNER: HexCoord = HexCoord::from_axial(7, -6);
const NORTH_WITNESS_OWNER: HexCoord = HexCoord::from_axial(7, -7);
const MAXIMUM_NEIGHBOR_STEP: u32 = 2;
// This approved terrace ceiling is below the accepted ecology's level-200
// snow-cap band, not the higher level-260 geometric peak summit contract.
const SHELF_CEILING: Level = 200;
const MINIMUM_TREE_SPACING: u32 = 3;
const MINIMUM_CLUSTER_SPACING: u32 = 12;
const CLUSTER_RADIUS: u32 = 4;
const CANOPY_PERCENT: usize = 4;
const FEATURE_BASE: u32 = WORLD_NAMESPACE | 0x0006_0000;

/// A connected terrain component sealed before route/canopy exclusions fragment
/// its planting sites. Fields remain private so the treeline exception cannot
/// be manufactured from a broad biome mask.
pub(super) struct RearShelfSite {
    mask: BTreeSet<HexCoord>,
    owners: BTreeMap<HexCoord, PatchId>,
    columns: BTreeMap<HexCoord, VolumeColumn>,
    metadata: BTreeMap<TilePos, SurfaceMetadata>,
    supports: BTreeMap<HexCoord, TilePos>,
    exclusions: BTreeSet<HexCoord>,
    crystal: BTreeSet<HexCoord>,
    seed: TilePos,
}

impl RearShelfSite {
    /// Exact region for the coordinator's mask report before authoring trees.
    pub(super) fn surfaces(&self) -> &BTreeMap<HexCoord, TilePos> {
        &self.supports
    }

    fn validate_terrain(&self, world: &GeneratedWorldPlan) -> Result<(), V3GenerationError> {
        for (coord, expected) in &self.columns {
            if world.volume.columns.get(coord) != Some(expected)
                || self.owners.get(coord).is_none_or(|owner| {
                    world
                        .layout
                        .patches
                        .get(owner)
                        .is_none_or(|patch| !patch.mask.contains(coord))
                })
            {
                return Err(schematic_contract(format!(
                    "rear shelf terrain or ownership changed at {coord:?}"
                )));
            }
        }
        let metadata = world
            .volume
            .surfaces
            .iter()
            .filter(|(position, _)| self.mask.contains(&position.coord))
            .map(|(position, metadata)| (*position, *metadata))
            .collect::<BTreeMap<_, _>>();
        if metadata != self.metadata {
            return Err(schematic_contract(
                "rear shelf changed its original surface access or interior metadata",
            ));
        }
        Ok(())
    }
}

fn connected_component(eligible: &BTreeSet<HexCoord>, seed: HexCoord) -> BTreeSet<HexCoord> {
    if !eligible.contains(&seed) {
        return BTreeSet::new();
    }
    let mut result = BTreeSet::from([seed]);
    let mut frontier = VecDeque::from([seed]);
    while let Some(coord) = frontier.pop_front() {
        for neighbor in coord.neighbors() {
            if eligible.contains(&neighbor) && result.insert(neighbor) {
                frontier.push_back(neighbor);
            }
        }
    }
    result
}

fn dry_natural_tops(world: &GeneratedWorldPlan) -> BTreeMap<HexCoord, TilePos> {
    let wet = world
        .volume
        .fill_runs_by_top()
        .keys()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    world
        .volume
        .mask
        .iter()
        .filter_map(|coord| {
            let (surface, metadata) = world.volume.top_surface_at_coord(*coord)?;
            (!wet.contains(coord)
                && metadata.interior.is_none()
                && metadata.access != SurfaceAccess::NonStandable
                && matches!(
                    solid_material_at(&world.volume, surface),
                    Some(
                        SolidMaterialRole::Stone
                            | SolidMaterialRole::Dirt
                            | SolidMaterialRole::Grass
                            | SolidMaterialRole::Gravel
                            | SolidMaterialRole::Snow
                    )
                ))
            .then_some((*coord, surface))
        })
        .collect()
}

fn is_low_slope_terrace(
    coord: HexCoord,
    tops: &BTreeMap<HexCoord, TilePos>,
    footprint: &BTreeSet<HexCoord>,
    exclusions: &BTreeSet<HexCoord>,
) -> bool {
    let Some(surface) = tops.get(&coord) else {
        return false;
    };
    !exclusions.contains(&coord)
        && (UPPER_REGION_THRESHOLD..SHELF_CEILING).contains(&surface.level)
        && coord
            .neighbors()
            .into_iter()
            .filter(|neighbor| footprint.contains(neighbor))
            .all(|neighbor| {
                !exclusions.contains(&neighbor)
                    && tops.get(&neighbor).is_some_and(|other| {
                        surface.level.abs_diff(other.level) <= MAXIMUM_NEIGHBOR_STEP
                    })
            })
}

/// Resolve and report the physical apron without mutating world state. The
/// supplied exclusion contains exact crest/other immutable terrain authority.
pub(super) fn prepare(
    plan: &SchematicPlanV1,
    world: &GeneratedWorldPlan,
    crystal: &BTreeSet<HexCoord>,
    crest_exclusion: &BTreeSet<HexCoord>,
) -> Result<Option<RearShelfSite>, V3GenerationError> {
    let east_peak_q = plan
        .cells
        .iter()
        .filter(|cell| has_overlay(cell, SchematicFeature::PeakRing))
        .map(|cell| cell.coord.q())
        .max()
        .ok_or_else(|| {
            schematic_contract("rear shelf cannot resolve its eastern PeakRing boundary")
        })?;
    let coarse = plan
        .cells
        .iter()
        .filter(|cell| {
            cell.coord.q() > east_peak_q
                && cell.facts.surface == SurfaceKind::Land
                && cell.facts.landform == LandformKind::Mountain
                && cell.facts.climate == ClimateKind::Alpine
                && cell.facts.overlays.is_empty()
        })
        .map(|cell| {
            (
                HexCoord::from_axial(cell.coord.q(), cell.coord.r()),
                PatchId(u32::from(cell.id.get())),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let Some(seed_owner) = coarse.get(&SEED_OWNER).copied() else {
        bevy::log::info!("rear shelf deferred: owner(7,-6) does not belong to this seed's eastern Mountain/Alpine envelope");
        return Ok(None);
    };
    let coarse_component = connected_component(&coarse.keys().copied().collect(), SEED_OWNER);
    let mut owners = BTreeMap::new();
    for coord in coarse_component {
        let owner = coarse
            .get(&coord)
            .copied()
            .ok_or_else(|| schematic_contract("rear shelf lost a coarse owner"))?;
        let patch = world
            .layout
            .patches
            .get(&owner)
            .ok_or_else(|| schematic_contract("rear shelf owner has no fine patch"))?;
        for fine in &patch.mask {
            if owners.insert(*fine, owner).is_some() {
                return Err(schematic_contract(
                    "rear shelf has overlapping semantic owners",
                ));
            }
        }
    }
    let mut exclusions = crest_exclusion
        .union(crystal)
        .copied()
        .collect::<BTreeSet<_>>();
    exclusions.extend(
        world
            .volume
            .fill_runs_by_top()
            .keys()
            .map(|position| position.coord),
    );
    let tops = dry_natural_tops(world);
    let eligible = owners
        .keys()
        .copied()
        .filter(|coord| is_low_slope_terrace(*coord, &tops, &world.volume.mask, &exclusions))
        .collect::<BTreeSet<_>>();
    let pitch = i32::try_from(V3_SCHEMATIC_CELL_PITCH)
        .map_err(|error| schematic_contract(error.to_string()))?;
    let center = HexCoord::from_axial(SEED_OWNER.x() * pitch, SEED_OWNER.y() * pitch);
    let Some(seed_coord) = eligible
        .iter()
        .copied()
        .filter(|coord| owners.get(coord) == Some(&seed_owner))
        .min_by_key(|coord| (center.distance(*coord), tops.get(coord).copied()))
    else {
        bevy::log::info!("rear shelf deferred: owner128 has no eligible low-slope upper terrace; bounds unchanged");
        return Ok(None);
    };
    let seed = tops
        .get(&seed_coord)
        .copied()
        .ok_or_else(|| schematic_contract("rear shelf seed has no exact support"))?;
    let mask = connected_component(&eligible, seed_coord);
    let supports = tops
        .into_iter()
        .filter(|(coord, _)| mask.contains(coord))
        .collect::<BTreeMap<_, _>>();
    owners.retain(|coord, _| mask.contains(coord));
    let mut counts = BTreeMap::<PatchId, usize>::new();
    for owner in owners.values() {
        *counts.entry(*owner).or_default() += 1;
    }
    let q_bounds = (
        mask.iter().map(|coord| coord.x()).min(),
        mask.iter().map(|coord| coord.x()).max(),
    );
    let r_bounds = (
        mask.iter().map(|coord| coord.y()).min(),
        mask.iter().map(|coord| coord.y()).max(),
    );
    let levels = (
        supports.values().map(|surface| surface.level).min(),
        supports.values().map(|surface| surface.level).max(),
    );
    let northern_owner = coarse.get(&NORTH_WITNESS_OWNER);
    let reaches_north_owner = northern_owner.is_some_and(|owner| counts.contains_key(owner));
    let world_edge_distance = mask
        .iter()
        .filter_map(|coord| {
            let radius = HexCoord::ORIGIN.distance(*coord);
            world.layout.grid_radius.checked_sub(radius)
        })
        .min();
    bevy::log::info!("rear shelf prepared seed {seed:?}, displaced {} hexes (center eligible={}): {} cells, owners {counts:?}, q{q_bounds:?}, r{r_bounds:?}, levels{levels:?}, reaches127={reaches_north_owner}, nearest world-edge distance={world_edge_distance:?}; no trees authored",
        center.distance(seed_coord), eligible.contains(&center), mask.len());
    let site = RearShelfSite {
        columns: world
            .volume
            .columns
            .iter()
            .filter(|(coord, _)| mask.contains(coord))
            .map(|(coord, column)| (*coord, column.clone()))
            .collect(),
        metadata: world
            .volume
            .surfaces
            .iter()
            .filter(|(surface, _)| mask.contains(&surface.coord))
            .map(|(surface, metadata)| (*surface, *metadata))
            .collect(),
        mask,
        owners,
        supports,
        exclusions,
        crystal: crystal.clone(),
        seed,
    };
    site.validate_terrain(world)?;
    Ok(Some(site))
}

/// Exact new tree identities, rather than a blanket treeline exemption.
pub(super) struct RearShelfTrees {
    site: RearShelfSite,
    trees: BTreeMap<FeatureId, PlannedFeature>,
    clusters: BTreeMap<HexCoord, BTreeSet<FeatureId>>,
    prior_blockers: BTreeSet<TilePos>,
    prior_canopy: BTreeSet<HexCoord>,
    added_blockers: BTreeSet<TilePos>,
}

impl RearShelfTrees {
    /// Final ecology admits only these exact authored features above treeline.
    pub(super) fn permits_tree(&self, id: FeatureId, tree: &PlannedFeature) -> bool {
        tree.kind == FeatureKind::Tree
            && self.site.supports.get(&tree.root.coord) == Some(&tree.root)
            && self.trees.get(&id) == Some(tree)
    }

    pub(super) fn validate(
        &self,
        plan: &SchematicPlanV1,
        catalog: &RuntimeArtCatalog,
        world: &GeneratedWorldPlan,
    ) -> Result<(), V3GenerationError> {
        self.site.validate_terrain(world)?;
        let expected_blockers = self
            .prior_blockers
            .union(&self.added_blockers)
            .copied()
            .collect::<BTreeSet<_>>();
        if world.blockers != expected_blockers {
            return Err(schematic_contract(
                "rear shelf final blockers differ from the exact added roots",
            ));
        }
        let (supports, reserved) = planting_context(plan, world, &self.site, &self.prior_blockers);
        let existing =
            existing_feature_volume(catalog, world, &self.trees.keys().copied().collect())?;
        if existing
            .canopy
            .intersection(&self.site.mask)
            .copied()
            .collect::<BTreeSet<_>>()
            != self.prior_canopy
        {
            return Err(schematic_contract(
                "rear shelf preexisting canopy changed after authoring",
            ));
        }
        let mut occupied = existing.volume;
        let snowy =
            SnowyVegetationSet::resolve(catalog, "Grand rear shelf").map_err(schematic_contract)?;
        let mut blockers = self.prior_blockers.clone();
        let mut canopy = self.prior_canopy.clone();
        for (id, tree) in &self.trees {
            if world.features.by_id.get(id) != Some(tree) || !self.permits_tree(*id, tree) {
                return Err(schematic_contract(
                    "rear shelf tree identity or exact root changed",
                ));
            }
            let object = [&snowy.small_broadleaf, &snowy.tall_narrow]
                .into_iter()
                .find(|object| object.id == tree.object_id)
                .ok_or_else(|| {
                    schematic_contract("rear shelf tree lost its accepted snowy asset")
                })?;
            if !object.projection_is_clear(
                tree.root,
                tree.rotation,
                &supports,
                &reserved,
                &occupied,
                &blockers,
            ) {
                return Err(schematic_contract(format!(
                    "rear shelf full tree geometry or canopy is obstructed at {:?}",
                    tree.root
                )));
            }
            let projected = object
                .project_blockers(tree.root, tree.rotation, &supports)
                .ok_or_else(|| {
                    schematic_contract("rear shelf tree cannot project exact blockers")
                })?;
            if projected != tree.blocker_footprint {
                return Err(schematic_contract(
                    "rear shelf tree blocker projection changed",
                ));
            }
            let visual = object
                .project_visual_volume(tree.root, tree.rotation)
                .ok_or_else(|| schematic_contract("rear shelf tree cannot project foliage"))?;
            canopy.extend(visual.cells.iter().map(|voxel| voxel.coord));
            occupied.extend(visual.cells);
            blockers.extend(projected);
        }
        let clustered = self
            .clusters
            .values()
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>();
        if canopy.len()
            > (self.site.mask.len().saturating_mul(CANOPY_PERCENT) / 100)
                .max(self.prior_canopy.len())
            || clustered != self.trees.keys().copied().collect()
            || self.clusters.iter().any(|(center, members)| {
                !(2..=4).contains(&members.len())
                    || members.iter().any(|id| {
                        self.trees
                            .get(id)
                            .is_none_or(|tree| center.distance(tree.root.coord) > CLUSTER_RADIUS)
                    })
                    || self.clusters.keys().any(|other| {
                        other != center && other.distance(*center) < MINIMUM_CLUSTER_SPACING
                    })
            })
        {
            return Err(schematic_contract("rear shelf lost its sparse two-to-four-tree clusters or exceeded its canopy ceiling"));
        }
        Ok(())
    }
}

fn planting_context(
    plan: &SchematicPlanV1,
    world: &GeneratedWorldPlan,
    site: &RearShelfSite,
    prior_blockers: &BTreeSet<TilePos>,
) -> (BTreeMap<HexCoord, TilePos>, BTreeSet<HexCoord>) {
    let supports = world
        .volume
        .mask
        .iter()
        .filter_map(|coord| {
            world
                .volume
                .top_surface_at_coord(*coord)
                .map(|(surface, _)| (*coord, surface))
        })
        .collect();
    let mut reserved = schematic_vegetation_reserved(plan, world, &site.crystal, prior_blockers);
    reserved.extend(site.exclusions.iter().copied());
    // The complete occupied tree, including overhanging leaves, stays inside
    // the recorded terrace. A root-only region test would leak over its cliffs.
    reserved.extend(world.volume.mask.difference(&site.mask).copied());
    (supports, reserved)
}

struct ExistingVegetation {
    volume: BTreeSet<TilePos>,
    roots: BTreeSet<HexCoord>,
    canopy: BTreeSet<HexCoord>,
}

fn existing_feature_volume(
    catalog: &RuntimeArtCatalog,
    world: &GeneratedWorldPlan,
    omit: &BTreeSet<FeatureId>,
) -> Result<ExistingVegetation, V3GenerationError> {
    let mut objects = BTreeMap::new();
    let mut volume = BTreeSet::new();
    let mut tree_roots = BTreeSet::new();
    let mut canopy = BTreeSet::new();
    for (id, feature) in &world.features.by_id {
        if omit.contains(id) {
            continue;
        }
        if let std::collections::btree_map::Entry::Vacant(entry) =
            objects.entry(feature.object_id.clone())
        {
            let blueprint = catalog.object(&feature.object_id).ok_or_else(|| {
                schematic_contract("rear shelf cannot resolve an existing feature's geometry")
            })?;
            let object = VegetationObjectSpec::resolve(
                catalog,
                feature.object_id.as_str(),
                blueprint.category,
                blueprint.blocker_footprint.len(),
                "Grand rear shelf existing feature",
            )
            .map_err(schematic_contract)?;
            entry.insert(object);
        }
        let object = objects
            .get(&feature.object_id)
            .ok_or_else(|| schematic_contract("rear shelf lost a resolved object"))?;
        let projected = object
            .project_visual_volume(feature.root, feature.rotation)
            .ok_or_else(|| {
                schematic_contract("rear shelf existing feature cannot project its full geometry")
            })?;
        if feature.kind == FeatureKind::Tree {
            tree_roots.insert(feature.root.coord);
            canopy.extend(projected.cells.iter().map(|voxel| voxel.coord));
        }
        volume.extend(projected.cells);
    }
    Ok(ExistingVegetation {
        volume,
        roots: tree_roots,
        canopy,
    })
}

/// Add sparse trees by consuming the exact prepared terrain mask.
/// Ordinary vegetation must already be complete so all existing canopies and
/// blockers participate in the placement preflight.
pub(super) fn author(
    site: RearShelfSite,
    plan: &SchematicPlanV1,
    seed: u64,
    catalog: &RuntimeArtCatalog,
    world: &mut GeneratedWorldPlan,
) -> Result<RearShelfTrees, V3GenerationError> {
    site.validate_terrain(world)?;
    let prior_blockers = world.blockers.clone();
    let (supports, reserved) = planting_context(plan, world, &site, &prior_blockers);
    let existing = existing_feature_volume(catalog, world, &BTreeSet::new())?;
    let mut occupied = existing.volume;
    let mut tree_roots = existing.roots;
    let prior_canopy = existing
        .canopy
        .intersection(&site.mask)
        .copied()
        .collect::<BTreeSet<_>>();
    let snowy =
        SnowyVegetationSet::resolve(catalog, "Grand rear shelf").map_err(schematic_contract)?;
    let objects = [&snowy.small_broadleaf, &snowy.tall_narrow];
    let zero_rotation =
        HexObjectRotation::new(0).map_err(|error| schematic_contract(error.to_string()))?;
    let minimum_cluster_cover = objects
        .iter()
        .filter_map(|object| {
            object
                .project_visual_volume(TilePos::new(HexCoord::ORIGIN, 0), zero_rotation)
                .map(|visual| {
                    visual
                        .cells
                        .iter()
                        .map(|voxel| voxel.coord)
                        .collect::<BTreeSet<_>>()
                        .len()
                })
        })
        .min()
        .unwrap_or(usize::MAX)
        .saturating_mul(2);
    let mut roots = site.supports.values().copied().collect::<Vec<_>>();
    roots.sort_unstable_by_key(|root| {
        (named_sample(seed, "rear_shelf_roots/v1", root.coord), *root)
    });
    let target = site.mask.len().saturating_mul(CANOPY_PERCENT) / 100;
    let mut covered = prior_canopy.clone();
    let mut planned = BTreeMap::new();
    let mut blockers = prior_blockers.clone();
    let mut next = FEATURE_BASE;
    let mut cluster_centers = BTreeSet::<HexCoord>::new();
    let mut clusters = BTreeMap::new();
    for cluster in roots {
        if target.saturating_sub(covered.len()) < minimum_cluster_cover {
            break;
        }
        if reserved.contains(&cluster.coord)
            || cluster_centers
                .iter()
                .any(|other| other.distance(cluster.coord) < MINIMUM_CLUSTER_SPACING)
        {
            continue;
        }
        let cluster_target = 2 + usize::try_from(
            named_sample(seed, "rear_shelf_cluster_size/v1", cluster.coord) % 3,
        )
        .unwrap_or(0);
        let mut near = cluster
            .coord
            .within_radius(CLUSTER_RADIUS)
            .into_iter()
            .filter_map(|coord| site.supports.get(&coord).copied())
            .collect::<Vec<_>>();
        near.sort_unstable_by_key(|root| {
            (
                cluster.coord.distance(root.coord),
                named_sample(seed, "rear_shelf_cluster_member/v1", root.coord),
                *root,
            )
        });
        let mut members = Vec::<(PlannedFeature, BTreeSet<TilePos>)>::new();
        let mut cluster_cover = BTreeSet::new();
        for root in near {
            if members.len() >= cluster_target {
                break;
            }
            if reserved.contains(&root.coord)
                || tree_roots
                    .iter()
                    .any(|other| other.distance(root.coord) < MINIMUM_TREE_SPACING)
            {
                continue;
            }
            let family =
                usize::try_from(named_sample(seed, "rear_shelf_family/v1", root.coord) % 2)
                    .unwrap_or(0);
            let rotation_start =
                u8::try_from(named_sample(seed, "rear_shelf_rotation/v1", root.coord) % 6)
                    .unwrap_or(0);
            let mut accepted = None;
            for family_offset in 0..2 {
                let Some(object) = objects.get((family + family_offset) % 2).copied() else {
                    continue;
                };
                for rotation_offset in 0..6_u8 {
                    let rotation = HexObjectRotation::new((rotation_start + rotation_offset) % 6)
                        .map_err(|error| schematic_contract(error.to_string()))?;
                    if !object.projection_is_clear(
                        root, rotation, &supports, &reserved, &occupied, &blockers,
                    ) {
                        continue;
                    }
                    let Some(projected_blockers) =
                        object.project_blockers(root, rotation, &supports)
                    else {
                        continue;
                    };
                    let Some(visual) = object.project_visual_volume(root, rotation) else {
                        continue;
                    };
                    let expanded_cover = covered
                        .iter()
                        .copied()
                        .chain(cluster_cover.iter().copied())
                        .chain(visual.cells.iter().map(|voxel| voxel.coord))
                        .collect::<BTreeSet<_>>();
                    if expanded_cover.len() > target {
                        continue;
                    }
                    accepted = Some((object, rotation, projected_blockers, visual.cells));
                    break;
                }
                if accepted.is_some() {
                    break;
                }
            }
            let Some((object, rotation, projected_blockers, visual)) = accepted else {
                continue;
            };
            occupied.extend(visual.iter().copied());
            blockers.extend(projected_blockers.iter().copied());
            tree_roots.insert(root.coord);
            cluster_cover.extend(visual.iter().map(|voxel| voxel.coord));
            members.push((
                PlannedFeature {
                    root,
                    kind: FeatureKind::Tree,
                    object_id: object.id.clone(),
                    rotation,
                    blocker_footprint: projected_blockers,
                },
                visual,
            ));
        }
        // Do not turn a failed cluster into isolated peppering. Projections
        // were disjoint from prior occupancy, so this rollback removes only
        // temporary cluster claims and never changes the original world.
        if members.len() < 2 {
            for (feature, visual) in members {
                for voxel in visual {
                    occupied.remove(&voxel);
                }
                for blocker in feature.blocker_footprint {
                    blockers.remove(&blocker);
                }
                tree_roots.remove(&feature.root.coord);
            }
            continue;
        }
        cluster_centers.insert(cluster.coord);
        covered.extend(cluster_cover);
        let mut member_ids = BTreeSet::new();
        for (feature, _) in members {
            let id = FeatureId(next);
            if world.features.by_id.contains_key(&id) {
                return Err(schematic_contract(
                    "rear shelf feature namespace is already occupied",
                ));
            }
            next = next
                .checked_add(1)
                .ok_or_else(|| schematic_contract("rear shelf feature namespace exhausted"))?;
            planned.insert(id, feature);
            member_ids.insert(id);
        }
        clusters.insert(cluster.coord, member_ids);
    }
    let authority = RearShelfTrees {
        added_blockers: blockers.difference(&prior_blockers).copied().collect(),
        prior_blockers,
        prior_canopy,
        site,
        trees: planned,
        clusters,
    };
    // All features are planned against the original world before the first write.
    world.features.by_id.extend(
        authority
            .trees
            .iter()
            .map(|(id, feature)| (*id, feature.clone())),
    );
    world.blockers = blockers;
    authority.validate(plan, catalog, world)?;
    bevy::log::info!("rear shelf authored {} clusters, {} snowy trees and {} exact blockers on {} terrace cells from {:?}; canopy {}/{target} maximum (zero clusters means deferred placement)",
        cluster_centers.len(), authority.trees.len(), authority.added_blockers.len(), authority.site.mask.len(), authority.site.seed, covered.len());
    Ok(authority)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (SchematicPlanV1, GeneratedWorldPlan) {
        let plan = hex_schematic::reference_plan(
            &hex_schematic::grand_v3_reference_template().expect("template"),
            0,
        )
        .expect("plan")
        .plan;
        let settings = ProceduralV3Settings {
            layout: V3LayoutSettings::Schematic(V3SchematicLayoutSettings {
                template: V3SchematicTemplate::GrandV3,
                template_revision: V3_GRAND_V3_TEMPLATE_REVISION,
                cell_pitch: 22,
                terrain_profile: V3SchematicTerrainProfile::GrandV3BasicV1(
                    V3GrandV3BasicTerrainProfile::canonical(),
                ),
            }),
        };
        let layout = resolve_layout(V3_SCHEMATIC_GRID_RADIUS, &settings).expect("layout");
        let mut volume = VolumePlan::new(layout.footprint.clone());
        // A literal terrace with a steep transverse break; the source fixture
        // is independent of both component selection and the real generator.
        for q in 140..=185 {
            for r in -166..=-105 {
                let coord = HexCoord::from_axial(q, r);
                if !volume.mask.contains(&coord) {
                    continue;
                }
                let level = if r == -145 { 170 } else { 133 };
                volume.columns.insert(
                    coord,
                    VolumeColumn {
                        elements: vec![VolumeElement::Solid(SolidMass {
                            levels: LevelInterval::new(0, level + 1),
                            material: SolidMaterialRole::Gravel,
                            cutaway_for: None,
                        })],
                    },
                );
                volume.surfaces.insert(
                    TilePos::new(coord, level),
                    SurfaceMetadata {
                        access: SurfaceAccess::Ordinary,
                        interior: None,
                    },
                );
            }
        }
        let world = GeneratedWorldPlan {
            source_schematic_fingerprint: Some(plan.semantic_fingerprint),
            layout,
            volume,
            liquids: LiquidPlan::default(),
            features: FeaturePlan::default(),
            structures: StructurePlan::default(),
            blockers: BTreeSet::new(),
            lights: BTreeMap::new(),
            biome_regions: BTreeMap::new(),
            interiors: InteriorPlan::default(),
            anchors: BTreeMap::new(),
            observation_anchors: BTreeMap::new(),
            view_hint: MapViewHint::new((0.0, 10.0, 10.0), (0.0, 0.0, 0.0)),
        };
        (plan, world)
    }

    #[test]
    fn rear_shelf_preparation_is_read_only_and_stops_at_physical_breaks() {
        let (plan, world) = fixture();
        let before = world.clone();
        let excluded = BTreeSet::from([HexCoord::from_axial(170, -132)]);
        let site = prepare(&plan, &world, &BTreeSet::new(), &excluded)
            .expect("shelf preparation")
            .expect("connected shelf");
        assert_eq!(world.volume, before.volume);
        assert_eq!(world.features, before.features);
        assert_eq!(world.blockers, before.blockers);
        assert!(site.mask.len() > 500);
        assert_eq!(connected_component(&site.mask, site.seed.coord), site.mask);
        assert!(site.mask.iter().all(|coord| coord.y() > -145));
        assert!(
            !site.mask.contains(&HexCoord::from_axial(154, -154)),
            "do not jump to the other seed's disconnected shelf"
        );
        assert!(site.mask.is_disjoint(&excluded));
        let cells = plan
            .cells
            .iter()
            .map(|cell| (PatchId(u32::from(cell.id.get())), cell))
            .collect::<BTreeMap<_, _>>();
        assert!(site
            .owners
            .values()
            .all(
                |owner| cells.get(owner).is_some_and(|cell| cell.coord.q() > 6
                    && cell.facts.landform == LandformKind::Mountain
                    && cell.facts.climate == ClimateKind::Alpine)
            ));
        assert!(site
            .owners
            .values()
            .any(|owner| cells.get(owner).is_some_and(
                |cell| cell.coord.q() == 8 && cell.facts.access == AccessIntent::Ordinary
            )));
        assert!(site
            .surfaces()
            .values()
            .all(|surface| (121..200).contains(&surface.level)));
    }

    #[test]
    fn rear_shelf_terrace_bounds_reject_low_high_steep_wet_and_other_components() {
        let center = HexCoord::ORIGIN;
        let footprint = center.within_radius(3).into_iter().collect::<BTreeSet<_>>();
        let mut tops = footprint
            .iter()
            .map(|coord| (*coord, TilePos::new(*coord, 133)))
            .collect::<BTreeMap<_, _>>();
        assert!(is_low_slope_terrace(
            center,
            &tops,
            &footprint,
            &BTreeSet::new()
        ));
        for (level, admitted) in [(120, false), (121, true), (199, true), (200, false)] {
            for surface in tops.values_mut() {
                surface.level = level;
            }
            assert_eq!(
                is_low_slope_terrace(center, &tops, &footprint, &BTreeSet::new()),
                admitted
            );
        }
        for surface in tops.values_mut() {
            surface.level = 133;
        }
        let neighbor = HexCoord::from_axial(1, 0);
        tops.insert(neighbor, TilePos::new(neighbor, 136));
        assert!(!is_low_slope_terrace(
            center,
            &tops,
            &footprint,
            &BTreeSet::new()
        ));
        tops.remove(&neighbor);
        assert!(!is_low_slope_terrace(
            center,
            &tops,
            &footprint,
            &BTreeSet::new()
        ));
        tops.insert(neighbor, TilePos::new(neighbor, 133));
        assert!(!is_low_slope_terrace(
            center,
            &tops,
            &footprint,
            &BTreeSet::from([neighbor])
        ));
        let two_pieces = BTreeSet::from([center, neighbor, HexCoord::from_axial(5, 0)]);
        assert_eq!(
            connected_component(&two_pieces, center),
            BTreeSet::from([center, neighbor])
        );
    }

    #[test]
    fn rear_shelf_final_trees_preserve_terrain_and_full_canopy_reservations() {
        let (plan, mut world) = fixture();
        let catalog = super::super::super::vegetation::tests::runtime_art_catalog();
        let route_coord = HexCoord::from_axial(167, -124);
        let route_surface = TilePos::new(route_coord, 133);
        world.features.protected_routes.insert(
            "test.route".to_owned(),
            ProtectedFeatureRoute {
                centerline: vec![route_surface],
                surfaces: BTreeSet::from([route_surface]),
            },
        );
        let temperate =
            TemperateVegetationSet::resolve(catalog, "existing test tree").expect("temperate");
        let prior_root = TilePos::new(HexCoord::from_axial(176, -119), 133);
        world.features.by_id.insert(
            FeatureId(1),
            PlannedFeature {
                root: prior_root,
                kind: FeatureKind::Tree,
                object_id: temperate.small_broadleaf.id.clone(),
                rotation: HexObjectRotation::new(0).expect("rotation"),
                blocker_footprint: BTreeSet::from([prior_root]),
            },
        );
        world.blockers.insert(prior_root);
        let site = prepare(&plan, &world, &BTreeSet::new(), &BTreeSet::new())
            .expect("shelf preparation")
            .expect("shelf");
        let before = world.clone();
        let trees = author(site, &plan, 7, catalog, &mut world).expect("sparse snowy trees");
        assert!(!trees.trees.is_empty());
        assert_eq!(world.volume, before.volume);
        assert_eq!(world.layout, before.layout);
        assert_eq!(
            world.features.protected_routes,
            before.features.protected_routes
        );
        assert_eq!(world.features.clearings, before.features.clearings);
        assert_eq!(world.anchors, before.anchors);
        assert_eq!(world.structures, before.structures);
        assert_eq!(world.interiors, before.interiors);
        for (id, original) in &before.features.by_id {
            assert_eq!(world.features.by_id.get(id), Some(original));
        }
        assert_eq!(trees.added_blockers.len(), trees.trees.len());
        assert!(trees
            .clusters
            .values()
            .all(|members| (2..=4).contains(&members.len())));
        let snowy = SnowyVegetationSet::resolve(catalog, "test").expect("snowy");
        for (id, tree) in &trees.trees {
            assert!(trees.permits_tree(*id, tree));
            assert!(!trees.permits_tree(FeatureId(0), tree));
            let object = [&snowy.small_broadleaf, &snowy.tall_narrow]
                .into_iter()
                .find(|object| object.id == tree.object_id)
                .expect("accepted snowy tree");
            let visual = object
                .project_visual_volume(tree.root, tree.rotation)
                .expect("foliage");
            assert!(visual
                .cells
                .iter()
                .all(|voxel| trees.site.mask.contains(&voxel.coord)
                    && voxel.coord.distance(route_coord) > 2));
        }
        let stable = world.clone();
        let expected_features = stable.features.clone();
        trees.validate(&plan, catalog, &world).expect("final guard");
        let tree = trees.trees.values().next().expect("new tree");
        world.features.protected_routes.insert(
            "test.new-obstruction".to_owned(),
            ProtectedFeatureRoute {
                centerline: vec![tree.root],
                surfaces: BTreeSet::from([tree.root]),
            },
        );
        assert!(trees.validate(&plan, catalog, &world).is_err());
        world = stable.clone();
        world.features.by_id.insert(FeatureId(2), tree.clone());
        assert!(
            trees.validate(&plan, catalog, &world).is_err(),
            "an unrecorded overlapping canopy must fail"
        );
        world = stable.clone();
        world.blockers.remove(&tree.root);
        assert!(trees.validate(&plan, catalog, &world).is_err());
        world = stable;
        let column = world
            .volume
            .columns
            .get_mut(&tree.root.coord)
            .expect("root column");
        let Some(VolumeElement::Solid(mass)) = column.elements.first_mut() else {
            panic!("solid fixture");
        };
        mass.material = SolidMaterialRole::Metal;
        assert!(trees.validate(&plan, catalog, &world).is_err());
        let mut repeat = before;
        let repeat_site = prepare(&plan, &repeat, &BTreeSet::new(), &BTreeSet::new())
            .expect("repeat preparation")
            .expect("repeat shelf");
        author(repeat_site, &plan, 7, catalog, &mut repeat).expect("repeat trees");
        assert_eq!(repeat.features, expected_features);
    }
}
