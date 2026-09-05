//! Grand-only snow on the existing Crystal crown and its existing tree voxels.
//! The standalone landmark, its geometry, and its placement streams stay authored.

use super::*;
use hex_assets::{ObjectBlueprint, ObjectPart, PlantPart};
use xxhash_rust::xxh3::xxh3_64_with_seed;

const SITE_RADIUS: u32 = 32;
const BARE_RIM_RADIUS: u32 = 11;
const SNOWY_RIM_RADIUS: u32 = 27;
const COVERAGE_SCALE: u32 = 10_000;

/// Exact post-ecology columns and foliage retained through final generation.
pub(super) struct CrystalCrownSnow {
    mask: BTreeSet<HexCoord>,
    columns: BTreeMap<HexCoord, VolumeColumn>,
    surfaces: BTreeMap<TilePos, SurfaceMetadata>,
    trees: BTreeMap<FeatureId, PlannedFeature>,
    blockers: BTreeSet<TilePos>,
    caps: BTreeMap<TilePos, SolidMaterialRole>,
}

impl CrystalCrownSnow {
    #[cfg(test)]
    pub(super) fn without_bare_caps_for_test() -> Self {
        Self {
            mask: BTreeSet::new(),
            columns: BTreeMap::new(),
            surfaces: BTreeMap::new(),
            trees: BTreeMap::new(),
            blockers: BTreeSet::new(),
            caps: BTreeMap::new(),
        }
    }

    /// Only a preflighted bare cap in the authored radial transition may keep
    /// Grass below the crown's snow edge; this never exempts its whole column.
    pub(super) fn permits_bare_cap(
        &self,
        surface: TilePos,
        material: Option<SolidMaterialRole>,
    ) -> bool {
        material == Some(SolidMaterialRole::Grass)
            && self.caps.get(&surface) == Some(&SolidMaterialRole::Grass)
    }

    pub(super) fn validate(&self, world: &GeneratedWorldPlan) -> Result<(), V3GenerationError> {
        for (coord, column) in &self.columns {
            if world.volume.columns.get(coord) != Some(column) {
                return Err(schematic_contract(format!(
                    "Crystal crown snow changed geometry or non-cap strata at {coord:?}"
                )));
            }
        }
        let surfaces = world
            .volume
            .surfaces
            .iter()
            .filter(|(surface, _)| self.mask.contains(&surface.coord))
            .map(|(surface, metadata)| (*surface, *metadata))
            .collect::<BTreeMap<_, _>>();
        let trees = world
            .features
            .by_id
            .iter()
            .filter(|(_, feature)| self.mask.contains(&feature.root.coord))
            .map(|(id, feature)| (*id, feature.clone()))
            .collect::<BTreeMap<_, _>>();
        let blockers = world
            .blockers
            .iter()
            .filter(|position| self.mask.contains(&position.coord))
            .copied()
            .collect::<BTreeSet<_>>();
        if surfaces != self.surfaces || trees != self.trees || blockers != self.blockers {
            return Err(schematic_contract(
                "Crystal crown snow changed its exact surface metadata, authored feature placement, or blockers",
            ));
        }
        if self.caps.iter().any(|(surface, material)| {
            solid_material_at(&world.volume, *surface) != Some(*material)
        }) {
            return Err(schematic_contract(
                "Crystal crown lost its exact radial snow caps",
            ));
        }
        Ok(())
    }
}

/// Selects snow-covered existing cap voxels, not additional vertical occupancy.
fn snow_coverage(radius: u32) -> u32 {
    let span = SNOWY_RIM_RADIUS - BARE_RIM_RADIUS;
    let x = radius.saturating_sub(BARE_RIM_RADIUS).min(span);
    let numerator = u64::from(COVERAGE_SCALE) * u64::from(x * x * (3 * span - 2 * x));
    u32::try_from(numerator / u64::from(span * span * span)).unwrap_or(COVERAGE_SCALE)
}

fn crown_cap_material(seed: u64, local: HexCoord) -> SolidMaterialRole {
    let radius = HexCoord::ORIGIN.distance(local);
    if radius <= BARE_RIM_RADIUS {
        return SolidMaterialRole::Grass;
    }
    if radius >= SNOWY_RIM_RADIUS {
        return SolidMaterialRole::Snow;
    }
    let span = SNOWY_RIM_RADIUS - BARE_RIM_RADIUS;
    let x = radius - BARE_RIM_RADIUS;
    // Coherent patches perturb the probability most in the middle of the
    // transition. Fine dither avoids a single hard snow boundary between them.
    let patch = crate::terrain_noise::coherent_level_offset(
        seed,
        b"grand-crystal-crown-snow-patches/v1",
        local,
        3,
        1_500,
    );
    let envelope = i32::try_from(4 * x * (span - x)).unwrap_or(0);
    let probability = (i32::try_from(snow_coverage(radius)).unwrap_or(0)
        + patch * envelope / i32::try_from(span * span).unwrap_or(1))
    .clamp(0, i32::try_from(COVERAGE_SCALE).unwrap_or(i32::MAX));
    let mut bytes = b"grand-crystal-crown-snow-dither/v1".to_vec();
    bytes.extend_from_slice(&local.x().to_le_bytes());
    bytes.extend_from_slice(&local.y().to_le_bytes());
    if xxh3_64_with_seed(&bytes, seed) % u64::from(COVERAGE_SCALE)
        < u64::try_from(probability).unwrap_or(0)
    {
        SolidMaterialRole::Snow
    } else {
        SolidMaterialRole::Grass
    }
}

/// The accepted snowy counterpart changes foliage styles only. Check the
/// complete asset contract before reusing the original root/rotation/blockers.
fn validate_tree_counterpart(
    original: &ObjectBlueprint,
    snowy: &ObjectBlueprint,
) -> Result<(), V3GenerationError> {
    let before = original
        .placements
        .iter()
        .map(|voxel| (voxel.position, voxel))
        .collect::<BTreeMap<_, _>>();
    let after = snowy
        .placements
        .iter()
        .map(|voxel| (voxel.position, voxel))
        .collect::<BTreeMap<_, _>>();
    let same_footprint = original.schema_version == snowy.schema_version
        && original.bounds == snowy.bounds
        && original.origin == snowy.origin
        && original.category == snowy.category
        && original.connectivity == snowy.connectivity
        && original.blocker_footprint.iter().collect::<BTreeSet<_>>()
            == snowy.blocker_footprint.iter().collect()
        && original.canopy_occluders.iter().collect::<BTreeSet<_>>()
            == snowy.canopy_occluders.iter().collect()
        && before.keys().eq(after.keys());
    let styles_only = before.iter().all(|(position, old)| {
        after.get(position).is_some_and(|new| {
            old.part == new.part
                && (old.style == new.style
                    || (old.part == ObjectPart::Plant(PlantPart::Foliage)
                        && new.style.as_str() == "plant/snow-cover"))
        })
    });
    let snow = snowy
        .placements
        .iter()
        .filter(|voxel| voxel.style.as_str() == "plant/snow-cover")
        .count();
    if !same_footprint || !styles_only || snow == 0 {
        return Err(schematic_contract(
            "Crystal crown snowy tree is not a foliage-only counterpart of its authored tree",
        ));
    }
    Ok(())
}

/// Run after general alpine caps and before general vegetation. The Crystal
/// recipe already owns every tree; this pass never chooses new planting sites.
pub(super) fn author(
    plan: &SchematicPlanV1,
    seed: u64,
    catalog: &RuntimeArtCatalog,
    world: &mut GeneratedWorldPlan,
) -> Result<CrystalCrownSnow, V3GenerationError> {
    let cells = plan
        .cells
        .iter()
        .filter(|cell| has_overlay(cell, SchematicFeature::CrystalAscent))
        .collect::<Vec<_>>();
    let [cell] = cells.as_slice() else {
        return Err(schematic_contract(
            "Crystal crown snow requires one exact Crystal owner",
        ));
    };
    let patch = world
        .layout
        .patches
        .get(&PatchId(u32::from(cell.id.get())))
        .ok_or_else(|| schematic_contract("Crystal crown snow lost its claimed patch"))?;
    let mask = patch.mask.clone();
    let center = exact_hex_disk_center(&mask, SITE_RADIUS).ok_or_else(|| {
        schematic_contract("Crystal crown snow requires the exact radius-32 claim")
    })?;
    let summit = world
        .anchors
        .get("crystal_ascent.upper_exit")
        .copied()
        .ok_or_else(|| schematic_contract("Crystal crown snow has no authored summit datum"))?
        .level;
    let outer_trail = super::super::crystal_ascent::macro_composite_upper_trail_coords(
        &mask,
        patch.rotation_turns,
    )
    .map_err(schematic_contract)?;
    let mut caps = BTreeMap::new();
    for coord in &mask {
        let radius = center.distance(*coord);
        if !outer_trail.contains(coord) && !(BARE_RIM_RADIUS..=SNOWY_RIM_RADIUS).contains(&radius) {
            continue;
        }
        // Select the authored summit surface itself: a preserved upper circuit
        // may occupy a higher stratum above the exact shell trail opening.
        let surface = TilePos::new(*coord, summit);
        let Some(metadata) = world.volume.surfaces.get(&surface) else {
            if outer_trail.contains(coord) {
                return Err(schematic_contract(
                    "Crystal crown snow lost an exact summit trail surface",
                ));
            }
            continue;
        };
        let material = solid_material_at(&world.volume, surface);
        if outer_trail.contains(coord)
            && (metadata.interior.is_some()
                || !matches!(
                    material,
                    Some(SolidMaterialRole::Grass | SolidMaterialRole::Snow)
                ))
        {
            return Err(schematic_contract(format!(
                "Crystal outer summit trail has no unchanged natural cap at {coord:?}"
            )));
        }
        if metadata.interior.is_some()
            || !matches!(
                material,
                Some(SolidMaterialRole::Grass | SolidMaterialRole::Snow)
            )
        {
            continue;
        }
        let column = world
            .volume
            .columns
            .get(coord)
            .ok_or_else(|| schematic_contract("Crystal crown cap has no column"))?;
        let cap = solid_mass_at_level(column, surface.level)
            .ok_or_else(|| schematic_contract("Crystal crown cap is not solid"))?;
        if cap.levels != LevelInterval::new(surface.level, surface.level.saturating_add(1)) {
            return Err(schematic_contract(
                "Crystal crown snow may recolour only an existing one-voxel natural cap",
            ));
        }
        let local = HexCoord::from_axial(coord.x() - center.x(), coord.y() - center.y());
        caps.insert(surface, crown_cap_material(seed, local));
    }
    if caps.is_empty()
        || !outer_trail
            .iter()
            .all(|coord| caps.contains_key(&TilePos::new(*coord, summit)))
    {
        return Err(schematic_contract(
            "Crystal crown snow has an incomplete crown or summit trail",
        ));
    }
    let temperate =
        super::super::vegetation::TemperateTreeSet::resolve(catalog, "Crystal crown snow")
            .map_err(schematic_contract)?;
    let frozen =
        SnowyVegetationSet::resolve(catalog, "Crystal crown snow").map_err(schematic_contract)?;
    let original = catalog
        .object(&temperate.small_broadleaf.id)
        .ok_or_else(|| schematic_contract("Crystal crown lost its accepted broadleaf asset"))?;
    let snowy = catalog.object(&frozen.small_broadleaf.id).ok_or_else(|| {
        schematic_contract("Crystal crown lost its accepted snowy broadleaf asset")
    })?;
    validate_tree_counterpart(original, snowy)?;
    let mut trees = world
        .features
        .by_id
        .iter()
        .filter(|(_, feature)| mask.contains(&feature.root.coord))
        .map(|(id, feature)| (*id, feature.clone()))
        .collect::<BTreeMap<_, _>>();
    for feature in trees
        .values_mut()
        .filter(|feature| feature.kind == FeatureKind::Tree)
    {
        if !caps.contains_key(&feature.root)
            || (feature.object_id != temperate.small_broadleaf.id
                && feature.object_id != frozen.small_broadleaf.id)
        {
            return Err(schematic_contract(
                "Crystal crown snow found an unauthored tree or a tree outside its summit ground",
            ));
        }
        feature.object_id = frozen.small_broadleaf.id.clone();
    }
    // All geometry, source caps, and asset equivalence are checked before the
    // first write. Every changed element is an existing material or object ID.
    for (surface, material) in &caps {
        for element in &mut world
            .volume
            .columns
            .get_mut(&surface.coord)
            .ok_or_else(|| schematic_contract("Crystal crown lost its preflighted column"))?
            .elements
        {
            if let VolumeElement::Solid(mass) = element {
                if mass.levels == LevelInterval::new(surface.level, surface.level.saturating_add(1))
                {
                    mass.material = *material;
                }
            }
        }
    }
    for (id, feature) in &trees {
        world.features.by_id.insert(*id, feature.clone());
    }
    let authority = CrystalCrownSnow {
        columns: world
            .volume
            .columns
            .iter()
            .filter(|(coord, _)| mask.contains(coord))
            .map(|(coord, column)| (*coord, column.clone()))
            .collect(),
        surfaces: world
            .volume
            .surfaces
            .iter()
            .filter(|(surface, _)| mask.contains(&surface.coord))
            .map(|(surface, metadata)| (*surface, *metadata))
            .collect(),
        blockers: world
            .blockers
            .iter()
            .filter(|position| mask.contains(&position.coord))
            .copied()
            .collect(),
        mask,
        trees,
        caps,
    };
    authority.validate(world)?;
    Ok(authority)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crown_fixture() -> (SchematicPlanV1, GeneratedWorldPlan) {
        let plan = hex_schematic::reference_plan(
            &hex_schematic::grand_v3_reference_template().expect("template"),
            0,
        )
        .expect("reference")
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
        let mut layout = resolve_layout(V3_SCHEMATIC_GRID_RADIUS, &settings).expect("layout");
        let claim = super::super::super::schematic_crystal::claim_site(&plan, &mut layout, 22)
            .expect("Crystal claim");
        let catalog = super::super::super::vegetation::tests::runtime_art_catalog();
        let fragment = super::super::super::schematic_crystal::construct_fragment(
            &layout,
            claim.patch_id(),
            0.4,
            0,
            catalog,
        )
        .expect("authored Crystal fragment");
        let mut world = GeneratedWorldPlan {
            source_schematic_fingerprint: Some(plan.semantic_fingerprint),
            volume: VolumePlan::new(layout.footprint.clone()),
            layout,
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
        super::super::super::schematic_crystal::merge_fragment(&mut world, fragment)
            .expect("Crystal merge");
        (plan, world)
    }

    #[test]
    fn crystal_crown_snow_preserves_final_geometry_strata_and_foliage() {
        let (plan, mut world) = crown_fixture();
        // A different owner's identical grass and tree are outside this pass.
        let crystal_cell = plan
            .cells
            .iter()
            .find(|cell| has_overlay(cell, SchematicFeature::CrystalAscent))
            .expect("Crystal owner");
        let crystal_mask = &world.layout.patches[&PatchId(u32::from(crystal_cell.id.get()))].mask;
        // VolumePlan::new preallocates all-air columns for the full footprint.
        // Find an empty site, with the complete tree outside the merged claim.
        let outside_coord = world
            .layout
            .footprint
            .iter()
            .copied()
            .filter(|coord| {
                world
                    .volume
                    .columns
                    .get(coord)
                    .is_some_and(|column| column.elements.is_empty())
                    && world.volume.surfaces_at_coord(*coord).next().is_none()
                    && coord.within_radius(1).into_iter().all(|neighbor| {
                        world.layout.footprint.contains(&neighbor)
                            && !crystal_mask.contains(&neighbor)
                    })
            })
            .min_by_key(|coord| (HexCoord::ORIGIN.distance(*coord), *coord))
            .expect("empty sentinel site outside Crystal");
        let outside = TilePos::new(outside_coord, 150);
        world.volume.columns.insert(
            outside.coord,
            VolumeColumn {
                elements: vec![VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(150, 151),
                    material: SolidMaterialRole::Grass,
                    cutaway_for: None,
                })],
            },
        );
        world.volume.surfaces.insert(
            outside,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );
        let mut outside_tree = world
            .features
            .by_id
            .values()
            .find(|feature| feature.kind == FeatureKind::Tree)
            .expect("authored tree")
            .clone();
        outside_tree.root = outside;
        outside_tree.blocker_footprint = BTreeSet::from([outside]);
        world
            .features
            .by_id
            .insert(FeatureId(u32::MAX), outside_tree);
        world.blockers.insert(outside);
        let before = world.clone();
        let catalog = super::super::super::vegetation::tests::runtime_art_catalog();
        let authority = author(&plan, 0, catalog, &mut world).expect("crown snow");
        let bare = *authority
            .caps
            .iter()
            .find(|(_, material)| **material == SolidMaterialRole::Grass)
            .expect("bare inner crown")
            .0;
        assert!(authority.permits_bare_cap(bare, Some(SolidMaterialRole::Grass)));
        assert!(!authority.permits_bare_cap(bare, Some(SolidMaterialRole::Dirt)));
        assert!(!authority.permits_bare_cap(
            TilePos::new(bare.coord, bare.level - 1),
            Some(SolidMaterialRole::Grass)
        ));
        assert!(!authority.permits_bare_cap(outside, Some(SolidMaterialRole::Grass)));
        let white = *authority
            .caps
            .iter()
            .find(|(_, material)| **material == SolidMaterialRole::Snow)
            .expect("snowy outer crown")
            .0;
        assert!(!authority.permits_bare_cap(white, Some(SolidMaterialRole::Grass)));
        assert_eq!(world.volume.mask, before.volume.mask);
        assert_eq!(world.volume.surfaces, before.volume.surfaces);
        assert_eq!(world.layout, before.layout);
        assert_eq!(world.blockers, before.blockers);
        assert_eq!(world.structures, before.structures);
        assert_eq!(world.interiors, before.interiors);
        assert_eq!(world.biome_regions, before.biome_regions);
        assert_eq!(world.liquids, before.liquids);
        assert_eq!(world.lights, before.lights);
        assert_eq!(world.anchors, before.anchors);
        assert_eq!(world.observation_anchors, before.observation_anchors);
        assert_eq!(
            world.features.protected_routes,
            before.features.protected_routes
        );
        assert_eq!(world.features.clearings, before.features.clearings);
        let mut changed_caps = 0;
        for (coord, original) in &before.volume.columns {
            let after = &world.volume.columns[coord];
            assert_eq!(after.elements.len(), original.elements.len());
            for (old, new) in original.elements.iter().zip(&after.elements) {
                if old == new {
                    continue;
                }
                let (VolumeElement::Solid(old), VolumeElement::Solid(new)) = (old, new) else {
                    panic!("changed occupancy");
                };
                assert_eq!(old.levels, new.levels);
                assert_eq!(old.cutaway_for, new.cutaway_for);
                let cap = TilePos::new(*coord, old.levels.bottom);
                assert_eq!(old.levels.top, cap.level + 1);
                assert_eq!(authority.caps.get(&cap), Some(&new.material));
                changed_caps += 1;
            }
        }
        assert!(changed_caps > 500);
        assert_eq!(world.features.by_id.len(), before.features.by_id.len());
        let temperate = super::super::super::vegetation::TemperateTreeSet::resolve(catalog, "test")
            .expect("trees");
        let snowy = SnowyVegetationSet::resolve(catalog, "test").expect("snowy trees");
        let mut changed_trees = 0;
        for (id, old) in &before.features.by_id {
            let new = &world.features.by_id[id];
            assert_eq!(new.root, old.root);
            assert_eq!(new.rotation, old.rotation);
            assert_eq!(new.blocker_footprint, old.blocker_footprint);
            assert_eq!(new.kind, old.kind);
            if old.kind == FeatureKind::Tree && authority.mask.contains(&old.root.coord) {
                assert_eq!(new.object_id, snowy.small_broadleaf.id);
                let old_volume = temperate
                    .small_broadleaf
                    .project_visual_volume(old.root, old.rotation)
                    .expect("old foliage");
                let new_volume = snowy
                    .small_broadleaf
                    .project_visual_volume(new.root, new.rotation)
                    .expect("snowy foliage");
                assert_eq!(old_volume.cells, new_volume.cells);
                assert_eq!(old_volume.structural_cells, new_volume.structural_cells);
                changed_trees += 1;
            } else {
                assert_eq!(new, old);
            }
        }
        assert!(changed_trees >= 30);
        let snowed = world.clone();
        author(&plan, 0, catalog, &mut world).expect("idempotent crown snow");
        assert_eq!(world.volume, snowed.volume);
        assert_eq!(world.features, snowed.features);
        authority.validate(&world).expect("final world authority");
        let cap = *authority.caps.keys().next().expect("cap");
        world.volume.surfaces.remove(&cap);
        assert!(authority.validate(&world).is_err());
        world = snowed.clone();
        let column = world.volume.columns.get_mut(&cap.coord).expect("column");
        let VolumeElement::Solid(first) = &mut column.elements[0] else {
            panic!("solid base");
        };
        first.material = SolidMaterialRole::Metal;
        assert!(authority.validate(&world).is_err());
        world = snowed;
        world
            .features
            .by_id
            .values_mut()
            .find(|feature| feature.kind == FeatureKind::Tree)
            .expect("tree")
            .root
            .level += 1;
        assert!(authority.validate(&world).is_err());
    }

    #[test]
    fn crystal_snowy_tree_counterpart_changes_only_existing_foliage_styles() {
        let catalog = super::super::super::vegetation::tests::runtime_art_catalog();
        let temperate = super::super::super::vegetation::TemperateTreeSet::resolve(catalog, "test")
            .expect("trees");
        let frozen = SnowyVegetationSet::resolve(catalog, "test").expect("snowy trees");
        let original = catalog
            .object(&temperate.small_broadleaf.id)
            .expect("original");
        let snowy = catalog.object(&frozen.small_broadleaf.id).expect("snowy");
        validate_tree_counterpart(original, snowy).expect("accepted partial snow counterpart");
        let snow_style = &snowy
            .placements
            .iter()
            .find(|voxel| voxel.style.as_str() == "plant/snow-cover")
            .expect("white canopy tops")
            .style;
        assert!(
            snowy
                .placements
                .iter()
                .any(|voxel| voxel.part == ObjectPart::Plant(PlantPart::Foliage)
                    && &voxel.style != snow_style),
            "partial snow retains green canopy undersides"
        );
        let mut bad = snowy.clone();
        bad.placements.pop();
        assert!(validate_tree_counterpart(original, &bad).is_err());
        bad = snowy.clone();
        bad.blocker_footprint.clear();
        assert!(validate_tree_counterpart(original, &bad).is_err());
        bad = snowy.clone();
        bad.placements
            .iter_mut()
            .find(|voxel| voxel.part != ObjectPart::Plant(PlantPart::Foliage))
            .expect("trunk")
            .style = snow_style.clone();
        assert!(validate_tree_counterpart(original, &bad).is_err());
    }

    #[test]
    fn crystal_crown_radial_snow_is_bare_at_hole_and_full_at_edge() {
        assert_eq!(snow_coverage(11), 0);
        assert_eq!(snow_coverage(19), 5_000);
        assert_eq!(snow_coverage(27), COVERAGE_SCALE);
        for seed in [0, 1, 7, 14, 175, 9_999] {
            let mut bands = [(0_u32, 0_u32); 3];
            for coord in HexCoord::ORIGIN.within_radius(32) {
                let radius = HexCoord::ORIGIN.distance(coord);
                let material = crown_cap_material(seed, coord);
                if radius == 11 {
                    assert_eq!(material, SolidMaterialRole::Grass);
                }
                if radius >= 27 {
                    assert_eq!(material, SolidMaterialRole::Snow);
                }
                let band = match radius {
                    12..=16 => 0,
                    17..=21 => 1,
                    22..=26 => 2,
                    _ => continue,
                };
                bands[band].0 += u32::from(material == SolidMaterialRole::Snow);
                bands[band].1 += 1;
                assert_eq!(material, crown_cap_material(seed, coord));
            }
            for pair in bands.windows(2) {
                assert!(pair[0].0 * pair[1].1 < pair[1].0 * pair[0].1);
            }
            assert!(bands[0].0 * 100 < bands[0].1 * 35);
            assert!(bands[2].0 * 100 > bands[2].1 * 75);
        }
    }
}
