//! Fine-scale ecology and the authored mountain-lake garden for Grand V3.
//!
//! The schematic describes broad climate and woodland intent. This module owns
//! the compiler's narrower physical interpretation: an organic snowline, a lower
//! treeline, and the one deliberately unnatural temperate island inside the
//! alpine mountain lake.

use std::collections::{BTreeMap, BTreeSet};

use hex_core::{HexCoord, Level, SpecialMovementRegion, TilePos};
use hex_schematic::{
    CellPlan, ClimateKind, FeatureKind as SchematicFeature, LandformKind, SchematicPlanV1,
    SurfaceKind, VegetationDensity,
};

use super::volume::{
    LevelInterval, SolidMass, SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeElement,
};
use super::world::{GeneratedWorldPlan, PlannedStructure, StructureId, StructureKind};
use super::V3GenerationError;
use crate::settings::V3GrandV3BasicTerrainProfile;

const SCHEMATIC_CELL_PITCH: i32 = 22;
const WORLD_NAMESPACE: u32 = 255 << 24;
const GARDEN_STRUCTURE_ID: StructureId = StructureId(WORLD_NAMESPACE | 0x0005_0000);
const INACCESSIBLE_MOVEMENT_REGION: SpecialMovementRegion =
    SpecialMovementRegion(WORLD_NAMESPACE | 2);

/// The mean Grand treeline. Fine-coordinate variation prevents a contour-ring
/// edge while retaining a strict maximum beyond which ordinary trees never grow.
const TREELINE_BASE: Level = 96;
const TREELINE_VARIATION: Level = 9;
/// The mean permanent-snow threshold. Its independent fine stream keeps the
/// snow edge from following either coarse biome borders or the tree edge.
const SNOWLINE_BASE: Level = 200;
const SNOWLINE_VARIATION: Level = 32;
const SNOWLINE_CORRELATION_HEXES: u16 = 16;
/// Exact authored summit bands from the connected highland field. Exposed rock
/// in these bands still receives a one-voxel snow cap; otherwise a later stone
/// grading pass can leave the highest silhouettes visibly bare even though the
/// surrounding natural terrain crossed the organic snowline.
const PEAK_SUMMIT_MIN: Level = 200;
const MASSIF_SUMMIT_MIN: Level = 224;

/// One authored garden support, relative to the island's coarse centre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GardenColumn {
    offset: HexCoord,
    rise: Level,
}

/// Six irregular old columns echo the standalone Garden's enclosure without
/// importing that unfinished recipe or making Grand depend on its branch.
const GARDEN_COLUMNS: [GardenColumn; 6] = [
    GardenColumn {
        offset: HexCoord::from_axial(-3, -3),
        rise: 12,
    },
    GardenColumn {
        offset: HexCoord::from_axial(3, -6),
        rise: 12,
    },
    GardenColumn {
        offset: HexCoord::from_axial(-6, 3),
        rise: 12,
    },
    GardenColumn {
        offset: HexCoord::from_axial(6, -3),
        rise: 10,
    },
    GardenColumn {
        offset: HexCoord::from_axial(3, 3),
        rise: 6,
    },
    GardenColumn {
        offset: HexCoord::from_axial(-3, 6),
        rise: 8,
    },
];

/// A broken one-voxel canopy joins the three tallest supports. The gaps leave
/// shafts of light and enough open sky for the dense, temperate canopy below.
const GARDEN_CANOPY: [HexCoord; 13] = [
    HexCoord::from_axial(-3, -3),
    HexCoord::from_axial(-2, -3),
    HexCoord::from_axial(-1, -4),
    HexCoord::from_axial(0, -4),
    HexCoord::from_axial(1, -5),
    HexCoord::from_axial(2, -5),
    HexCoord::from_axial(3, -6),
    HexCoord::from_axial(-3, -2),
    HexCoord::from_axial(-4, -1),
    HexCoord::from_axial(-4, 0),
    HexCoord::from_axial(-5, 1),
    HexCoord::from_axial(-5, 2),
    HexCoord::from_axial(-6, 3),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VegetationFamily {
    Temperate,
    Frozen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct VegetationPolicy {
    pub(super) density: VegetationDensity,
    pub(super) family: VegetationFamily,
    pub(super) prefer_old_growth: bool,
}

/// Whether a cell carries one exact overlay.
fn has_overlay(cell: &CellPlan, overlay: SchematicFeature) -> bool {
    cell.facts.overlays.contains(&overlay)
}

/// Resolves Grand's intentional vegetation gradient without discarding the
/// schematic planner's seeded woodland choice.
///
/// The schematic still selects the woodland footprint and its None/Dense
/// variation. Nonempty cells resolve through landform-specific coverage bands:
/// valleys are at least Moderate, hills at most Light, and mountain bases at
/// most Sparse, while massif and sharp summits remain empty.
pub(super) fn vegetation_policy(cell: &CellPlan) -> VegetationPolicy {
    if has_overlay(cell, SchematicFeature::LakeIsland) {
        return VegetationPolicy {
            density: VegetationDensity::Dense,
            family: VegetationFamily::Temperate,
            prefer_old_growth: true,
        };
    }
    if has_overlay(cell, SchematicFeature::FrozenWoods) {
        return VegetationPolicy {
            density: VegetationDensity::Dense,
            family: VegetationFamily::Frozen,
            prefer_old_growth: false,
        };
    }
    let density = match cell.facts.landform {
        // Every seeded nonempty valley band is promoted to at least Moderate.
        // The planner still decides where woodland exists and whether it becomes
        // Dense, while the compiled map keeps a robust visual separation from
        // Light hills and Sparse mountain bases.
        LandformKind::Valley => match cell.facts.vegetation {
            VegetationDensity::None => VegetationDensity::None,
            density => density.max(VegetationDensity::Moderate),
        },
        LandformKind::Plateau => cell.facts.vegetation.min(VegetationDensity::Moderate),
        LandformKind::Hill => cell.facts.vegetation.min(VegetationDensity::Light),
        LandformKind::Mountain => cell.facts.vegetation.min(VegetationDensity::Sparse),
        LandformKind::Massif | LandformKind::SharpPeak => VegetationDensity::None,
        LandformKind::None | LandformKind::Island | LandformKind::Beach | LandformKind::Shore => {
            cell.facts.vegetation
        }
    };
    VegetationPolicy {
        density,
        family: if cell.facts.climate == ClimateKind::Frozen {
            VegetationFamily::Frozen
        } else {
            VegetationFamily::Temperate
        },
        prefer_old_growth: false,
    }
}

/// Exact root-level treeline admission. Frozen Woods, the Crystal crown, and
/// the magical lake island are authored exceptions rather than loopholes in the
/// ordinary alpine policy.
pub(super) fn tree_root_is_admitted(cell: &CellPlan, root: TilePos, seed: u64) -> bool {
    if has_overlay(cell, SchematicFeature::LakeIsland)
        || has_overlay(cell, SchematicFeature::FrozenWoods)
        || has_overlay(cell, SchematicFeature::CrystalAscent)
    {
        return true;
    }
    root.level <= organic_treeline(seed, root.coord)
}

/// Intended horizontal canopy percentage for one resolved density band.
///
/// This stays beside [`vegetation_policy`] so final-world acceptance can compare
/// aggregate valley, hill, and mountain-base bands without duplicating their
/// semantic meaning or depending on fragile exact tree-object counts.
pub(super) const fn vegetation_coverage_percent(density: VegetationDensity) -> u32 {
    match density {
        VegetationDensity::None => 0,
        VegetationDensity::Sparse => 4,
        VegetationDensity::Light => 14,
        VegetationDensity::Moderate => 30,
        VegetationDensity::Dense => 52,
    }
}

/// A coherent terraced mound replaces the old independent random height in
/// every island column. The shore stays low, the interior rises gradually, and
/// the exact centre owns the high point.
pub(super) fn lake_island_surface_level(
    coord: HexCoord,
    center: HexCoord,
    profile: V3GrandV3BasicTerrainProfile,
    seed: u64,
) -> Level {
    let distance = i32::try_from(center.distance(coord)).unwrap_or(i32::MAX);
    let span = profile
        .lake_island_max_level
        .saturating_sub(profile.lake_island_min_level);
    if distance == 0 {
        return profile.lake_island_max_level;
    }
    let inward = 10_i32.saturating_sub(distance).max(0);
    let broad_rise = span.saturating_mul(inward) / 10;
    let terrace = if distance < 9 && named_sample(seed, "garden_island_terraces", coord) % 5 == 0 {
        1
    } else {
        0
    };
    profile
        .lake_island_min_level
        .saturating_add(broad_rise)
        .saturating_add(terrace)
        .clamp(profile.lake_island_min_level, profile.lake_island_max_level)
}

/// Fine cap-material override. The garden is the sole warm exception in the
/// high lake; Frozen Woods is unconditionally snowy; exposed alpine land
/// follows the organic snowline instead of landform names or coarse hex boundaries.
pub(super) fn cap_material_override(
    cell: &CellPlan,
    surface: TilePos,
    seed: u64,
) -> Option<SolidMaterialRole> {
    if has_overlay(cell, SchematicFeature::LakeIsland) {
        return Some(SolidMaterialRole::Grass);
    }
    if has_overlay(cell, SchematicFeature::FrozenWoods) {
        return Some(SolidMaterialRole::Snow);
    }
    if cell.facts.surface == SurfaceKind::Land
        && cell.facts.climate == ClimateKind::Alpine
        && surface.level >= organic_snowline(seed, surface.coord)
    {
        return Some(SolidMaterialRole::Snow);
    }
    None
}

/// Whether this exact exposed surface belongs to a highland silhouette whose
/// final cap must be snow even when a later author replaced the natural cap with
/// a longer Stone interval.
pub(super) fn summit_band_requires_snow_cap(cell: &CellPlan, surface: TilePos) -> bool {
    if has_overlay(cell, SchematicFeature::LakeIsland) {
        return false;
    }
    match cell.facts.landform {
        LandformKind::SharpPeak => surface.level >= PEAK_SUMMIT_MIN,
        LandformKind::Massif => surface.level >= MASSIF_SUMMIT_MIN,
        LandformKind::None
        | LandformKind::Island
        | LandformKind::Beach
        | LandformKind::Shore
        | LandformKind::Valley
        | LandformKind::Plateau
        | LandformKind::Hill
        | LandformKind::Mountain => false,
    }
}

/// Recolour one exposed semantic cap without recolouring the mountain beneath
/// it. Natural caps retain the prior one-voxel rule. Summit-band Stone may be a
/// long post-grading run, so it is split into the original lower rock and one
/// exact Snow voxel at the exposed boundary.
fn reconcile_surface_cap_material(
    column: &mut super::volume::VolumeColumn,
    surface: TilePos,
    material: SolidMaterialRole,
    allow_stone_split: bool,
) -> bool {
    let top = surface.level.saturating_add(1);
    let Some((index, mass)) =
        column
            .elements
            .iter()
            .copied()
            .enumerate()
            .find_map(|(index, element)| {
                let VolumeElement::Solid(mass) = element else {
                    return None;
                };
                (mass.levels.bottom <= surface.level && mass.levels.top == top)
                    .then_some((index, mass))
            })
    else {
        return false;
    };
    if mass.material == material {
        return true;
    }
    let natural_one_voxel = mass.levels.bottom == surface.level
        && matches!(
            mass.material,
            SolidMaterialRole::Dirt
                | SolidMaterialRole::Grass
                | SolidMaterialRole::Gravel
                | SolidMaterialRole::Sand
                | SolidMaterialRole::Snow
        );
    if !natural_one_voxel && !(allow_stone_split && mass.material == SolidMaterialRole::Stone) {
        return false;
    }

    let mut replacement = Vec::with_capacity(2);
    if mass.levels.bottom < surface.level {
        replacement.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(mass.levels.bottom, surface.level),
            material: mass.material,
            cutaway_for: mass.cutaway_for,
        }));
    }
    replacement.push(VolumeElement::Solid(SolidMass {
        levels: LevelInterval::new(surface.level, top),
        material,
        cutaway_for: mass.cutaway_for,
    }));
    column.elements.splice(index..=index, replacement);
    true
}

/// Replays the snowline after routes and landmarks have changed cap columns.
/// Natural caps keep the one-voxel recolour rule. Exact SharpPeak and Massif
/// summit bands additionally split a long Stone run to add one exposed snow
/// voxel; architecture, bridges, tunnel masonry, and other authored materials
/// retain their exact roles.
pub(super) fn reconcile_alpine_caps(
    plan: &SchematicPlanV1,
    seed: u64,
    world: &mut GeneratedWorldPlan,
) -> Result<(), V3GenerationError> {
    let cells = plan
        .cells
        .iter()
        .map(|cell| (u32::from(cell.id.get()), cell))
        .collect::<BTreeMap<_, _>>();
    for (patch_id, patch) in &world.layout.patches {
        let cell = cells.get(&patch_id.0).copied().ok_or_else(|| {
            V3GenerationError::RecipeContract(format!(
                "Grand ecology patch {} has no schematic cell",
                patch_id.0
            ))
        })?;
        for coord in &patch.mask {
            let surfaces = world
                .volume
                .surfaces_at_coord(*coord)
                .map(|(surface, _)| *surface)
                .collect::<Vec<_>>();
            for surface in surfaces {
                let force_summit_snow = summit_band_requires_snow_cap(cell, surface);
                let material = if force_summit_snow {
                    SolidMaterialRole::Snow
                } else {
                    let Some(material) = cap_material_override(cell, surface, seed) else {
                        continue;
                    };
                    material
                };
                let Some(column) = world.volume.columns.get_mut(coord) else {
                    continue;
                };
                let _reconciled =
                    reconcile_surface_cap_material(column, surface, material, force_summit_snow);
            }
        }
    }
    Ok(())
}

/// Adds the lake island's exact six supports and broken Garden canopy. Terrain
/// remains the source of occupancy and rendering; the structure record supplies
/// exact semantic ownership and keeps vegetation clear of every authored voxel.
pub(super) fn author_lake_island_garden(
    plan: &SchematicPlanV1,
    profile: V3GrandV3BasicTerrainProfile,
    world: &mut GeneratedWorldPlan,
) -> Result<(), V3GenerationError> {
    let mut lake_islands = plan
        .cells
        .iter()
        .filter(|cell| has_overlay(cell, SchematicFeature::LakeIsland));
    let cell = lake_islands.next().ok_or_else(|| {
        V3GenerationError::RecipeContract("Grand ecology has no locked lake island".to_owned())
    })?;
    if lake_islands.next().is_some() {
        return Err(V3GenerationError::RecipeContract(
            "Grand ecology requires exactly one locked lake island".to_owned(),
        ));
    }
    let center = HexCoord::from_axial(
        cell.coord.q().saturating_mul(SCHEMATIC_CELL_PITCH),
        cell.coord.r().saturating_mul(SCHEMATIC_CELL_PITCH),
    );
    let patch = world
        .layout
        .patches
        .get(&super::layout::PatchId(u32::from(cell.id.get())))
        .ok_or_else(|| {
            V3GenerationError::RecipeContract(
                "Grand garden island has no resolved ownership patch".to_owned(),
            )
        })?;
    let biome = patch.biome_region;
    let canopy_level = profile.lake_island_min_level.saturating_add(12);
    let mut voxels = BTreeSet::new();
    let support_offsets = GARDEN_COLUMNS
        .iter()
        .map(|column| column.offset)
        .collect::<BTreeSet<_>>();

    for column in GARDEN_COLUMNS {
        let coord = translate(center, column.offset);
        ensure_owned(patch.mask.contains(&coord), coord)?;
        let (ground, metadata) = world.volume.top_surface_at_coord(coord).ok_or_else(|| {
            V3GenerationError::RecipeContract(format!(
                "Grand garden support {coord:?} has no dry island surface"
            ))
        })?;
        let top = profile
            .lake_island_min_level
            .saturating_add(column.rise)
            .max(ground.level.saturating_add(2));
        let volume_column = world.volume.columns.get_mut(&coord).ok_or_else(|| {
            V3GenerationError::RecipeContract(format!(
                "Grand garden support {coord:?} has no volume column"
            ))
        })?;
        volume_column.elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(ground.level.saturating_add(1), top.saturating_add(1)),
            material: SolidMaterialRole::WorkedStone,
            cutaway_for: None,
        }));
        let _old_surface = world.volume.surfaces.remove(&ground);
        let _old_biome = world.biome_regions.remove(&ground);
        let top_surface = TilePos::new(coord, top);
        world.volume.surfaces.insert(top_surface, metadata);
        world.biome_regions.insert(top_surface, biome);
        voxels
            .extend((ground.level.saturating_add(1)..=top).map(|level| TilePos::new(coord, level)));
    }

    for offset in GARDEN_CANOPY {
        if support_offsets.contains(&offset) {
            continue;
        }
        let coord = translate(center, offset);
        ensure_owned(patch.mask.contains(&coord), coord)?;
        let (ground, _) = world.volume.top_surface_at_coord(coord).ok_or_else(|| {
            V3GenerationError::RecipeContract(format!(
                "Grand garden canopy {coord:?} has no island footing"
            ))
        })?;
        if canopy_level <= ground.level.saturating_add(2) {
            return Err(V3GenerationError::RecipeContract(format!(
                "Grand garden canopy {coord:?} has insufficient clearance above {ground:?}"
            )));
        }
        let volume_column = world.volume.columns.get_mut(&coord).ok_or_else(|| {
            V3GenerationError::RecipeContract(format!(
                "Grand garden canopy {coord:?} has no volume column"
            ))
        })?;
        volume_column.elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(canopy_level, canopy_level.saturating_add(1)),
            material: SolidMaterialRole::WorkedStone,
            cutaway_for: None,
        }));
        let canopy = TilePos::new(coord, canopy_level);
        world.volume.surfaces.insert(
            canopy,
            SurfaceMetadata {
                access: SurfaceAccess::SpecialMovement(INACCESSIBLE_MOVEMENT_REGION),
                interior: None,
            },
        );
        world.biome_regions.insert(canopy, biome);
        voxels.insert(canopy);
    }

    if world
        .structures
        .by_id
        .insert(
            GARDEN_STRUCTURE_ID,
            PlannedStructure {
                kind: StructureKind::Tower,
                voxels,
            },
        )
        .is_some()
    {
        return Err(V3GenerationError::RecipeContract(
            "Grand garden structure ID collided with another world-owned feature".to_owned(),
        ));
    }
    Ok(())
}

/// Private vegetation reservation; this leaves the scenic island's access
/// metadata intact instead of turning it into an Ordinary feature clearing.
pub(super) fn garden_courtyard_reservation(plan: &SchematicPlanV1) -> BTreeSet<HexCoord> {
    plan.cells
        .iter()
        .find(|cell| has_overlay(cell, SchematicFeature::LakeIsland))
        .map(|cell| {
            garden_courtyard_coords(HexCoord::from_axial(
                cell.coord.q().saturating_mul(SCHEMATIC_CELL_PITCH),
                cell.coord.r().saturating_mul(SCHEMATIC_CELL_PITCH),
            ))
        })
        .unwrap_or_default()
}

/// The closed polygon enclosed by the six authored supports. Reserving its full
/// projection excludes overhanging crowns as well as tree roots, while leaving
/// the surrounding island available for its temperate woodland.
pub(super) fn garden_courtyard_coords(center: HexCoord) -> BTreeSet<HexCoord> {
    HexCoord::ORIGIN
        .within_radius(6)
        .into_iter()
        .filter(|offset| {
            let q = offset.x();
            let r = offset.y();
            (q - r).abs() <= 9 && (2 * q + r).abs() <= 9 && (q + 2 * r).abs() <= 9
        })
        .map(|offset| translate(center, offset))
        .collect()
}

fn organic_treeline(seed: u64, coord: HexCoord) -> Level {
    TREELINE_BASE.saturating_add(
        i32::try_from(
            named_sample(seed, "grand_treeline", coord)
                % u64::try_from(TREELINE_VARIATION.saturating_add(1)).unwrap_or(1),
        )
        .unwrap_or_default(),
    )
}

fn organic_snowline(seed: u64, coord: HexCoord) -> Level {
    SNOWLINE_BASE.saturating_add(crate::terrain_noise::coherent_level_offset(
        seed,
        b"review-snow-coherent",
        coord,
        SNOWLINE_CORRELATION_HEXES,
        SNOWLINE_VARIATION,
    ))
}

fn translate(origin: HexCoord, offset: HexCoord) -> HexCoord {
    HexCoord::from_axial(
        origin.x().saturating_add(offset.x()),
        origin.y().saturating_add(offset.y()),
    )
}

fn ensure_owned(owned: bool, coord: HexCoord) -> Result<(), V3GenerationError> {
    if owned {
        Ok(())
    } else {
        Err(V3GenerationError::RecipeContract(format!(
            "Grand garden architecture leaves the lake-island mask at {coord:?}"
        )))
    }
}

fn named_sample(seed: u64, stream: &str, coord: HexCoord) -> u64 {
    let mut state = 0xcbf2_9ce4_8422_2325_u64;
    for bytes in [
        seed.to_le_bytes().as_slice(),
        stream.as_bytes(),
        coord.x().to_le_bytes().as_slice(),
        coord.y().to_le_bytes().as_slice(),
    ] {
        for byte in bytes {
            state ^= u64::from(*byte);
            state = state.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_schematic::{
        AccessIntent, CellFacts, CellId, CellProvenance, LayerProvenance, SchematicCoord, StableId,
        SurfaceKind,
    };

    fn cell_with_vegetation(
        landform: LandformKind,
        climate: ClimateKind,
        overlays: Vec<SchematicFeature>,
        vegetation: VegetationDensity,
    ) -> CellPlan {
        let source = || LayerProvenance::Seeded {
            stream: StableId::new("stream/test").expect("valid fixture stream"),
        };
        CellPlan {
            id: CellId::new(1).expect("valid fixture id"),
            coord: SchematicCoord::new(0, 0, 0).expect("valid fixture coordinate"),
            facts: CellFacts {
                surface: SurfaceKind::Land,
                landform,
                climate,
                vegetation,
                access: AccessIntent::Scenic,
                overlays,
            },
            provenance: CellProvenance {
                surface: source(),
                landform: source(),
                climate: source(),
                vegetation: source(),
                access: source(),
                overlays: Vec::new(),
            },
        }
    }

    fn cell(
        landform: LandformKind,
        climate: ClimateKind,
        overlays: Vec<SchematicFeature>,
    ) -> CellPlan {
        cell_with_vegetation(landform, climate, overlays, VegetationDensity::Sparse)
    }

    #[test]
    fn gradient_caps_seeded_density_without_erasing_woodland_variation() {
        assert_eq!(
            vegetation_policy(&cell_with_vegetation(
                LandformKind::Valley,
                ClimateKind::Temperate,
                vec![],
                VegetationDensity::Dense,
            ))
            .density,
            VegetationDensity::Dense
        );
        assert_eq!(
            vegetation_policy(&cell_with_vegetation(
                LandformKind::Valley,
                ClimateKind::Temperate,
                vec![],
                VegetationDensity::Light,
            ))
            .density,
            VegetationDensity::Moderate
        );
        assert_eq!(
            vegetation_policy(&cell_with_vegetation(
                LandformKind::Valley,
                ClimateKind::Temperate,
                vec![],
                VegetationDensity::None,
            ))
            .density,
            VegetationDensity::None,
            "an unselected valley must remain available for seeded woodland variation"
        );
        assert_eq!(
            vegetation_policy(&cell_with_vegetation(
                LandformKind::Hill,
                ClimateKind::Temperate,
                vec![],
                VegetationDensity::Dense,
            ))
            .density,
            VegetationDensity::Light
        );
        assert_eq!(
            vegetation_policy(&cell_with_vegetation(
                LandformKind::Hill,
                ClimateKind::Temperate,
                vec![],
                VegetationDensity::None,
            ))
            .density,
            VegetationDensity::None,
            "an unselected seeded woodland cell must stay unselected"
        );
        assert_eq!(
            vegetation_policy(&cell_with_vegetation(
                LandformKind::Mountain,
                ClimateKind::Alpine,
                vec![],
                VegetationDensity::Dense,
            ))
            .density,
            VegetationDensity::Sparse
        );
        assert_eq!(
            vegetation_policy(&cell_with_vegetation(
                LandformKind::SharpPeak,
                ClimateKind::Alpine,
                vec![],
                VegetationDensity::Dense,
            ))
            .density,
            VegetationDensity::None
        );
        let frozen = vegetation_policy(&cell(
            LandformKind::Shore,
            ClimateKind::Frozen,
            vec![SchematicFeature::FrozenWoods],
        ));
        assert_eq!(frozen.density, VegetationDensity::Dense);
        assert_eq!(frozen.family, VegetationFamily::Frozen);
        let garden = vegetation_policy(&cell(
            LandformKind::Island,
            ClimateKind::Alpine,
            vec![SchematicFeature::LakeIsland],
        ));
        assert_eq!(garden.density, VegetationDensity::Dense);
        assert_eq!(garden.family, VegetationFamily::Temperate);
        assert!(garden.prefer_old_growth);
    }

    #[test]
    fn treeline_rejects_every_ordinary_high_root_but_preserves_authored_exceptions() {
        let mountain = cell(LandformKind::Mountain, ClimateKind::Alpine, vec![]);
        let high = TilePos::new(HexCoord::ORIGIN, TREELINE_BASE + TREELINE_VARIATION + 1);
        assert!(!tree_root_is_admitted(&mountain, high, 17));
        let frozen = cell(
            LandformKind::Shore,
            ClimateKind::Frozen,
            vec![SchematicFeature::FrozenWoods],
        );
        assert!(tree_root_is_admitted(&frozen, high, 17));
        let garden = cell(
            LandformKind::Island,
            ClimateKind::Alpine,
            vec![SchematicFeature::LakeIsland],
        );
        assert!(tree_root_is_admitted(&garden, high, 17));
    }

    #[test]
    fn snowline_covers_every_high_alpine_landform_and_crystal_terrain() {
        let high = TilePos::new(HexCoord::ORIGIN, SNOWLINE_BASE + SNOWLINE_VARIATION + 1);
        for landform in [
            LandformKind::Shore,
            LandformKind::Valley,
            LandformKind::Plateau,
            LandformKind::Hill,
            LandformKind::Mountain,
            LandformKind::Massif,
            LandformKind::SharpPeak,
        ] {
            assert_eq!(
                cap_material_override(&cell(landform, ClimateKind::Alpine, vec![]), high, 99),
                Some(SolidMaterialRole::Snow),
                "high Alpine {landform:?} terrain escaped the snowline"
            );
        }
        let crystal = cell(
            LandformKind::Mountain,
            ClimateKind::Alpine,
            vec![SchematicFeature::CrystalAscent],
        );
        assert_eq!(
            cap_material_override(&crystal, high, 99),
            Some(SolidMaterialRole::Snow)
        );
        let low_alpine = TilePos::new(HexCoord::ORIGIN, SNOWLINE_BASE - SNOWLINE_VARIATION - 1);
        assert_eq!(
            cap_material_override(
                &cell(LandformKind::Plateau, ClimateKind::Alpine, vec![]),
                low_alpine,
                99,
            ),
            None
        );
    }

    #[test]
    fn snowline_preserves_frozen_woods_and_the_warm_garden_exception() {
        let high = TilePos::new(HexCoord::ORIGIN, SNOWLINE_BASE + SNOWLINE_VARIATION + 1);
        let low = TilePos::new(HexCoord::ORIGIN, SNOWLINE_BASE - 1);
        let frozen = cell(
            LandformKind::Shore,
            ClimateKind::Frozen,
            vec![SchematicFeature::FrozenWoods],
        );
        assert_eq!(
            cap_material_override(&frozen, low, 99),
            Some(SolidMaterialRole::Snow)
        );
        let garden = cell(
            LandformKind::Island,
            ClimateKind::Alpine,
            vec![SchematicFeature::LakeIsland],
        );
        assert_eq!(
            cap_material_override(&garden, high, 99),
            Some(SolidMaterialRole::Grass)
        );
        let temperate = cell(LandformKind::Hill, ClimateKind::Temperate, vec![]);
        assert_eq!(cap_material_override(&temperate, high, 99), None);
        let mut alpine_lake = cell(LandformKind::None, ClimateKind::Alpine, vec![]);
        alpine_lake.facts.surface = SurfaceKind::OpenWater;
        assert_eq!(cap_material_override(&alpine_lake, high, 99), None);
    }

    #[test]
    fn summit_snow_splits_only_the_exposed_voxel_from_a_long_stone_run() {
        let surface = TilePos::new(HexCoord::ORIGIN, PEAK_SUMMIT_MIN);
        let mut column = super::super::volume::VolumeColumn {
            elements: vec![VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(1, surface.level.saturating_add(1)),
                material: SolidMaterialRole::Stone,
                cutaway_for: None,
            })],
        };
        assert!(reconcile_surface_cap_material(
            &mut column,
            surface,
            SolidMaterialRole::Snow,
            true,
        ));
        assert_eq!(
            column.elements,
            vec![
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(1, surface.level),
                    material: SolidMaterialRole::Stone,
                    cutaway_for: None,
                }),
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(surface.level, surface.level.saturating_add(1)),
                    material: SolidMaterialRole::Snow,
                    cutaway_for: None,
                }),
            ]
        );
    }

    #[test]
    fn summit_snow_is_exact_and_never_overrides_the_magical_garden() {
        let peak = cell(LandformKind::SharpPeak, ClimateKind::Alpine, vec![]);
        let massif = cell(LandformKind::Massif, ClimateKind::Alpine, vec![]);
        assert!(!summit_band_requires_snow_cap(
            &peak,
            TilePos::new(HexCoord::ORIGIN, PEAK_SUMMIT_MIN - 1),
        ));
        assert!(summit_band_requires_snow_cap(
            &peak,
            TilePos::new(HexCoord::ORIGIN, PEAK_SUMMIT_MIN),
        ));
        assert!(summit_band_requires_snow_cap(
            &massif,
            TilePos::new(HexCoord::ORIGIN, MASSIF_SUMMIT_MIN),
        ));
        let garden = cell(
            LandformKind::SharpPeak,
            ClimateKind::Alpine,
            vec![SchematicFeature::LakeIsland],
        );
        assert!(!summit_band_requires_snow_cap(
            &garden,
            TilePos::new(HexCoord::ORIGIN, MASSIF_SUMMIT_MIN),
        ));
    }

    #[test]
    fn garden_terrain_is_a_bounded_coherent_mound() {
        let profile = V3GrandV3BasicTerrainProfile::canonical();
        let center = HexCoord::ORIGIN;
        assert_eq!(
            lake_island_surface_level(center, center, profile, 1),
            profile.lake_island_max_level
        );
        for direction in 0..6_u8 {
            let edge = step(center, direction, 11);
            assert_eq!(
                lake_island_surface_level(edge, center, profile, 1),
                profile.lake_island_min_level
            );
        }
    }

    #[test]
    fn garden_courtyard_reserves_the_enclosed_polygon_without_the_outer_woodland() {
        let center = HexCoord::from_axial(44, -66);
        let courtyard = garden_courtyard_coords(center);
        // Area 81, boundary 18: Pick's theorem gives 91 occupied hex centres.
        assert_eq!(courtyard.len(), 91);
        assert!(courtyard.contains(&center));
        assert!(GARDEN_COLUMNS
            .iter()
            .all(|support| { courtyard.contains(&translate(center, support.offset)) }));
        assert!(!courtyard.contains(&translate(center, HexCoord::from_axial(6, 0))));
        assert!(!courtyard.contains(&translate(center, HexCoord::from_axial(0, -6))));
    }

    #[test]
    fn garden_architecture_has_six_unique_supports_and_one_broken_canopy() {
        let supports = GARDEN_COLUMNS
            .iter()
            .map(|column| column.offset)
            .collect::<BTreeSet<_>>();
        let canopy = GARDEN_CANOPY.into_iter().collect::<BTreeSet<_>>();
        assert_eq!(supports.len(), 6);
        assert_eq!(canopy.len(), 13);
        assert_eq!(supports.intersection(&canopy).count(), 3);
        assert_eq!(
            GARDEN_COLUMNS
                .iter()
                .map(|column| column.rise)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([6, 8, 10, 12])
        );
        assert!(supports
            .union(&canopy)
            .all(|offset| offset.distance(HexCoord::ORIGIN) <= 6));
    }

    fn step(origin: HexCoord, direction: u8, distance: i32) -> HexCoord {
        let offset = match direction {
            0 => HexCoord::from_axial(distance, 0),
            1 => HexCoord::from_axial(distance, -distance),
            2 => HexCoord::from_axial(0, -distance),
            3 => HexCoord::from_axial(-distance, 0),
            4 => HexCoord::from_axial(-distance, distance),
            _ => HexCoord::from_axial(0, distance),
        };
        translate(origin, offset)
    }
}
