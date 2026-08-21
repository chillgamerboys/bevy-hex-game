//! Exact authored vegetation used by Arid recipes.
//!
//! Desert recipes keep object art authoritative while publishing only the
//! separately authored single-root traversal blocker of each accepted palm.

use std::collections::{BTreeMap, BTreeSet};

use hex_assets::{HexObjectRotation, ObjectCategory, RuntimeArtCatalog};
use hex_core::{HexCoord, TilePos};

use super::seed::SeedStream;
use super::vegetation::VegetationObjectSpec;
use super::world::{FeatureId, FeatureKind, FeaturePlan, PlannedFeature};

pub(super) const DATE_PALM_ID: &str = "plant/date-palm";

/// Exact accepted object data required by Oasis.
#[derive(Debug, Clone)]
pub(super) struct DesertVegetationSet {
    pub(super) date_palm: VegetationObjectSpec,
}

impl DesertVegetationSet {
    pub(super) fn resolve(catalog: &RuntimeArtCatalog, recipe: &str) -> Result<Self, String> {
        Ok(Self {
            date_palm: VegetationObjectSpec::resolve(
                catalog,
                DATE_PALM_ID,
                ObjectCategory::Plant,
                1,
                recipe,
            )?,
        })
    }
}

/// Places the exact requested palm count without blocked-route, terrain, or
/// authored-volume overlap.
///
/// Only the palm's root is a traversal blocker. Its high canopy may overhang
/// water, anchors, or a protected walker approach just as a real palm shades a
/// path beneath it.
pub(super) fn place_date_palms(
    recipe: &str,
    palm: &VegetationObjectSpec,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    candidates: &BTreeSet<HexCoord>,
    reserved: &BTreeSet<HexCoord>,
    target: usize,
    priority_stream: Option<SeedStream<'_>>,
    rotation_stream: Option<SeedStream<'_>>,
) -> Result<(FeaturePlan, BTreeSet<TilePos>), String> {
    let mut roots = candidates
        .iter()
        .filter_map(|coord| (!reserved.contains(coord)).then_some(*coord))
        .collect::<Vec<_>>();
    roots.sort_unstable_by_key(|coord| {
        (
            sample(priority_stream, *coord, 0),
            coord.distance(HexCoord::ORIGIN),
            *coord,
        )
    });

    let mut occupied_visual = BTreeSet::new();
    let mut occupied_blockers = BTreeSet::new();
    let mut planned = BTreeMap::new();
    for coord in roots {
        if planned.len() >= target {
            break;
        }
        let Some(root) = surfaces.get(&coord).copied() else {
            continue;
        };
        let first = u8::try_from(sample(rotation_stream, coord, 17) % 6).unwrap_or_default();
        let mut accepted = None;
        for offset in 0..6 {
            let steps = first.saturating_add(offset) % 6;
            let rotation = HexObjectRotation::new(steps)
                .map_err(|error| format!("{recipe} date-palm rotation failed: {error}"))?;
            let Some(visual) = palm.project_visual_volume(root, rotation) else {
                continue;
            };
            if visual.cells.iter().any(|cell| {
                surfaces
                    .get(&cell.coord)
                    .is_none_or(|support| cell.level <= support.level)
            }) || !visual.cells.is_disjoint(&occupied_visual)
            {
                continue;
            }
            let Some(blockers) = palm.project_blockers(root, rotation, surfaces) else {
                continue;
            };
            if !blockers.is_disjoint(&occupied_blockers)
                || blockers
                    .iter()
                    .any(|blocker| reserved.contains(&blocker.coord))
            {
                continue;
            }
            accepted = Some((rotation, visual.cells, blockers));
            break;
        }
        let Some((rotation, visual, blockers)) = accepted else {
            continue;
        };
        occupied_visual.extend(visual);
        occupied_blockers.extend(blockers.iter().copied());
        let id = FeatureId(u32::try_from(planned.len()).unwrap_or(u32::MAX));
        planned.insert(
            id,
            PlannedFeature {
                root,
                kind: FeatureKind::Tree,
                object_id: palm.id.clone(),
                rotation,
                blocker_footprint: blockers,
            },
        );
    }
    if planned.len() != target {
        return Err(format!(
            "{recipe} could place only {} of {target} exact date palms",
            planned.len()
        ));
    }
    Ok((
        FeaturePlan {
            by_id: planned,
            protected_routes: BTreeMap::new(),
            clearings: BTreeMap::new(),
        },
        occupied_blockers,
    ))
}

fn sample(stream: Option<SeedStream<'_>>, coord: HexCoord, salt: u64) -> u64 {
    stream.map_or_else(
        || {
            let x = u64::from_le_bytes(i64::from(coord.x()).to_le_bytes());
            let y = u64::from_le_bytes(i64::from(coord.y()).to_le_bytes());
            let z = u64::from_le_bytes(i64::from(coord.z()).to_le_bytes());
            x.rotate_left(7)
                ^ y.rotate_left(23)
                ^ z.rotate_left(41)
                ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        },
        |stream| stream.sample_coord(coord, salt),
    )
}
