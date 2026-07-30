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
                .filter(|placement| {
                    matches!(
                        placement.part,
                        ObjectPart::Plant(PlantPart::Root | PlantPart::Trunk)
                    ) || blueprint.category == ObjectCategory::Prop
                })
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
mod tests {
    use super::*;

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
