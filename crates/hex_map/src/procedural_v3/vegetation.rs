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

pub(super) const SMALL_BROADLEAF_ID: &str = "plant/small-broadleaf";
pub(super) const TALL_NARROW_ID: &str = "plant/tall-narrow";
pub(super) const OLD_GROWTH_ID: &str = "plant/old-growth";
pub(super) const GRASS_TUFT_ID: &str = "prop/grass-tuft";
#[expect(
    clippy::allow_attributes,
    reason = "staged sibling-lane authority is deliberately unused on this source branch"
)]
#[allow(dead_code, reason = "consumed by frozen landform integration")]
pub(super) const SNOWY_SMALL_BROADLEAF_ID: &str = "plant/snowy-small-broadleaf";
#[expect(
    clippy::allow_attributes,
    reason = "staged sibling-lane authority is deliberately unused on this source branch"
)]
#[allow(dead_code, reason = "consumed by frozen landform integration")]
pub(super) const SNOWY_TALL_NARROW_ID: &str = "plant/snowy-tall-narrow";
#[expect(
    clippy::allow_attributes,
    reason = "staged sibling-lane authority is deliberately unused on this source branch"
)]
#[allow(dead_code, reason = "consumed by frozen landform integration")]
pub(super) const SNOWY_OLD_GROWTH_ID: &str = "plant/snowy-old-growth";
#[expect(
    clippy::allow_attributes,
    reason = "staged sibling-lane authority is deliberately unused on this source branch"
)]
#[allow(dead_code, reason = "consumed by frozen landform integration")]
pub(super) const SNOWY_GRASS_TUFT_ID: &str = "prop/snowy-grass-tuft";
#[expect(
    clippy::allow_attributes,
    reason = "staged sibling-lane authority is deliberately unused on this source branch"
)]
#[allow(dead_code, reason = "consumed by cave integration")]
pub(super) const CAVE_MOSS_ID: &str = "prop/cave-moss";
#[expect(
    clippy::allow_attributes,
    reason = "staged sibling-lane authority is deliberately unused on this source branch"
)]
#[allow(dead_code, reason = "consumed by cave integration")]
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
#[allow(
    dead_code,
    clippy::allow_attributes,
    reason = "staged authority is consumed by frozen landform integration"
)]
pub(super) struct SnowyVegetationSet {
    pub(super) small_broadleaf: VegetationObjectSpec,
    pub(super) tall_narrow: VegetationObjectSpec,
    pub(super) old_growth: VegetationObjectSpec,
    pub(super) grass_tuft: VegetationObjectSpec,
}

impl SnowyVegetationSet {
    #[expect(
        clippy::allow_attributes,
        reason = "staged sibling-lane authority is deliberately unused on this source branch"
    )]
    #[allow(dead_code, reason = "consumed by frozen landform integration")]
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

/// Nonblocking cave vegetation resolved as an exact pair of authored props.
#[derive(Debug, Clone)]
#[allow(
    dead_code,
    clippy::allow_attributes,
    reason = "staged authority is consumed by cave integration"
)]
pub(super) struct CaveVegetationSet {
    pub(super) moss: VegetationObjectSpec,
    pub(super) lichen: VegetationObjectSpec,
}

impl CaveVegetationSet {
    #[expect(
        clippy::allow_attributes,
        reason = "staged sibling-lane authority is deliberately unused on this source branch"
    )]
    #[allow(dead_code, reason = "consumed by cave integration")]
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
}
