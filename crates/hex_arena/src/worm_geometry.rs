//! Bounded proposed-step geometry, independent of the burrow request lifecycle.

use std::collections::{BTreeMap, BTreeSet};

use bevy_math::Vec3;
use hex_core::arena::{
    ArenaBurrowMaterials, ArenaTerrainView, ArenaVoxelGeometry, MAX_ARENA_BURROW_CELLS,
};
use hex_core::{HexCoord, SubstanceId, TilePos};

use crate::collision::SKIN;
use crate::hex_prisms::HexPrism;
use crate::{Actor, BodyPrismSnapshot, Species};

/// Fixed physical identity/order across one proposed step; no body mutation.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PrismPose {
    pub feet: Vec3,
    pub parts: BodyPrismSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepRejection {
    InvalidGeometry,
    OutsideWorld,
    CandidateBudget,
    TooManyCells,
    Protected,
    Liquid,
    Static,
    Ineligible,
    Actor,
    Unrefreshed,
}

#[derive(Debug)]
pub(crate) enum Admission {
    Clear,
    NeedsConversion(Vec<TilePos>),
    Blocked(StepRejection),
}

/// One current publication and current living-body view; never an owned world clone.
#[derive(Clone, Copy)]
pub(crate) struct BurrowContext<'a> {
    pub world: &'a ArenaTerrainView,
    pub policy: &'a ArenaBurrowMaterials,
    pub dirt: SubstanceId,
    pub bodies: &'a [Actor],
    pub owner: u8,
    pub geometry: ArenaVoxelGeometry,
}

const MAX_CANDIDATE_TESTS: u64 = 4096;

#[cfg(test)]
#[path = "worm_geometry_tests.rs"]
mod tests;

/// Index only public nonterrain intervals. Dirt eligibility is NEVER cached:
/// every admission reads the latest published per-cell material directly.
#[derive(Debug, Default)]
pub(crate) struct BurrowQuery {
    revision: Option<u64>,
    static_cells: BTreeMap<HexCoord, Vec<(i32, i32)>>,
    liquid_cells: BTreeMap<HexCoord, Vec<(i32, i32)>>,
}

impl BurrowQuery {
    pub(crate) fn refresh(&mut self, view: &ArenaTerrainView) {
        if self.revision == Some(view.revision) {
            return;
        }
        let rebuild = view.full_rebuild
            || self.revision.and_then(|old| old.checked_add(1)) != Some(view.revision);
        if rebuild {
            self.static_cells.clear();
            self.liquid_cells.clear();
            for span in &view.static_spans {
                self.static_cells
                    .entry(span.bottom.coord)
                    .or_default()
                    .push((span.bottom.level, span.top_level));
            }
            for span in &view.liquids {
                self.liquid_cells
                    .entry(span.bottom.coord)
                    .or_default()
                    .push((span.bottom.level, span.top_level));
            }
        }
        self.revision = Some(view.revision);
    }

    /// Initial above-ground bodies obey every Worm occupancy mask before any
    /// material policy or conversion request exists. Air is the only free volume.
    pub(crate) fn above_ground_clear(
        &self,
        pose: PrismPose,
        view: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) -> bool {
        if self.revision != Some(view.revision) {
            return false;
        }
        swept_prism_cells(pose, pose, geometry).is_ok_and(|cells| {
            cells.into_iter().all(|pos| {
                self.blocked_cell(pos, view).is_none()
                    && view
                        .voxels
                        .get(&pos)
                        .is_none_or(|material| *material == SubstanceId::AIR)
            })
        })
    }

    fn blocked_cell(&self, pos: TilePos, view: &ArenaTerrainView) -> Option<StepRejection> {
        if in_intervals(&view.edit_protected, pos) {
            Some(StepRejection::Protected)
        } else if in_intervals(&self.liquid_cells, pos) {
            Some(StepRejection::Liquid)
        } else if in_intervals(&self.static_cells, pos) {
            Some(StepRejection::Static)
        } else {
            None
        }
    }

    pub(crate) fn admit(
        &self,
        before: PrismPose,
        after: PrismPose,
        context: &BurrowContext<'_>,
    ) -> Admission {
        let BurrowContext {
            world,
            policy,
            dirt,
            bodies,
            owner,
            geometry,
        } = *context;
        if self.revision != Some(world.revision) {
            return Admission::Blocked(StepRejection::Unrefreshed);
        }
        if !policy.eligible.contains(&dirt) {
            return Admission::Blocked(StepRejection::Ineligible);
        }
        let cells = match swept_prism_cells(before, after, geometry) {
            Ok(cells) => cells,
            Err(reason) => return Admission::Blocked(reason),
        };
        let mut conversion = false;
        for pos in &cells {
            if let Some(reason) = self.blocked_cell(*pos, world) {
                return Admission::Blocked(reason);
            }
            if let Some(material) = world.voxels.get(pos) {
                if *material == dirt || *material == SubstanceId::AIR {
                    continue;
                }
                if !policy.eligible.contains(material) {
                    return Admission::Blocked(StepRejection::Ineligible);
                }
                conversion = true;
            }
        }
        for actor in bodies.iter().filter(|a| a.id != owner && a.hp > 0.0) {
            if sweep_hits_body(before, after, actor) {
                return Admission::Blocked(StepRejection::Actor);
            }
        }
        if conversion {
            Admission::NeedsConversion(cells)
        } else {
            Admission::Clear
        }
    }
}

fn in_intervals(intervals: &BTreeMap<HexCoord, Vec<(i32, i32)>>, pos: TilePos) -> bool {
    intervals.get(&pos.coord).is_some_and(|runs| {
        runs.iter()
            .any(|(low, high)| (*low..=*high).contains(&pos.level))
    })
}

fn matched_parts(before: PrismPose, after: PrismPose) -> bool {
    before.parts.iter().len() == after.parts.iter().len()
        && before.parts.iter().zip(after.parts.iter()).all(|(a, b)| {
            a.height.to_bits() == b.height.to_bits()
                && HexPrism::new(before.feet + a.offset, a.height).is_some()
                && HexPrism::new(after.feet + b.offset, b.height).is_some()
        })
}

/// Exact interiors of all continuously translated components, including air.
/// Broad candidates are bounded before looping; the final volume is never cut.
pub(crate) fn swept_prism_cells(
    before: PrismPose,
    after: PrismPose,
    geometry: ArenaVoxelGeometry,
) -> Result<Vec<TilePos>, StepRejection> {
    if !valid_geometry(geometry) || !matched_parts(before, after) {
        return Err(StepRejection::InvalidGeometry);
    }
    if !contained(before, geometry) || !contained(after, geometry) {
        return Err(StepRejection::OutsideWorld);
    }
    let mut cells = BTreeSet::new();
    let mut candidates = 0_u64;
    for (a, b) in before.parts.iter().zip(after.parts.iter()) {
        let start = before.feet + a.offset;
        let end = after.feet + b.offset;
        let prism = HexPrism::new(start, a.height).ok_or(StepRejection::InvalidGeometry)?;
        let start_coord = HexCoord::from_world(start);
        let end_coord = HexCoord::from_world(end);
        // Native fractional axial coordinates are linear over this translation.
        // Rounding adds <1 cell and the sum of the two native footprints adds
        // <2 axial units. A two-cell margin therefore bounds every overlap.
        let min_q = start_coord.x().min(end_coord.x()) - 2;
        let max_q = start_coord.x().max(end_coord.x()) + 2;
        let min_r = start_coord.y().min(end_coord.y()) - 2;
        let max_r = start_coord.y().max(end_coord.y()) + 2;
        let low = (start.y.min(end.y) - geometry.vertical_offset) / geometry.level_height;
        let high =
            (start.y.max(end.y) + a.height - geometry.vertical_offset) / geometry.level_height;
        // Admission above limits all world coordinates and levels before casts.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "finite bounded arena levels"
        )]
        let min_level = low.floor() as i32 - 1;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "finite bounded arena levels"
        )]
        let max_level = high.ceil() as i32 + 1;
        let size = u64::try_from(max_q - min_q + 1)
            .ok()
            .and_then(|q| u64::try_from(max_r - min_r + 1).ok().map(|r| q * r))
            .and_then(|qr| {
                u64::try_from(max_level - min_level + 1)
                    .ok()
                    .map(|l| qr * l)
            })
            .ok_or(StepRejection::CandidateBudget)?;
        candidates = candidates.saturating_add(size);
        if candidates > MAX_CANDIDATE_TESTS {
            return Err(StepRejection::CandidateBudget);
        }
        for q in min_q..=max_q {
            for r in min_r..=max_r {
                for level in min_level..=max_level {
                    let pos = TilePos::new(HexCoord::from_axial(q, r), level);
                    let voxel = HexPrism::new(
                        pos.coord
                            .to_world(geometry.top(pos) - geometry.level_height),
                        geometry.level_height,
                    )
                    .ok_or(StepRejection::InvalidGeometry)?;
                    if !prism.swept_overlaps_prism(end - start, voxel, SKIN) {
                        continue;
                    }
                    if !geometry.contains_column(pos.coord)
                        || !(geometry.min_level..=geometry.max_level).contains(&pos.level)
                    {
                        return Err(StepRejection::OutsideWorld);
                    }
                    cells.insert(pos);
                    if cells.len() > MAX_ARENA_BURROW_CELLS {
                        return Err(StepRejection::TooManyCells);
                    }
                }
            }
        }
    }
    if cells.is_empty() {
        return Err(StepRejection::InvalidGeometry);
    }
    Ok(cells.into_iter().collect())
}

fn valid_geometry(g: ArenaVoxelGeometry) -> bool {
    g.level_height.is_finite()
        && g.level_height >= SKIN * 4.0
        && g.vertical_offset.is_finite()
        && g.vertical_offset.abs() <= 10_000.0
        && g.radius <= 10_000
        && g.min_level >= -10_000
        && g.max_level <= 10_000
        && g.min_level <= g.max_level
        && g.top(TilePos::new(HexCoord::ORIGIN, g.min_level))
            .is_finite()
        && g.top(TilePos::new(HexCoord::ORIGIN, g.max_level))
            .is_finite()
}

pub(crate) fn contained(pose: PrismPose, g: ArenaVoxelGeometry) -> bool {
    let hull_axes = [
        Vec3::Z,
        Vec3::new(0.866_025_4, 0.0, 0.5),
        Vec3::new(0.866_025_4, 0.0, -0.5),
    ];
    let limit = f64::from(g.radius) * 1.5;
    let low = g.top(TilePos::new(HexCoord::ORIGIN, g.min_level)) - g.level_height;
    let high = g.top(TilePos::new(HexCoord::ORIGIN, g.max_level));
    pose.parts.iter().all(|part| {
        let feet = pose.feet + part.offset;
        feet.is_finite()
            && feet.abs().max_element() <= 10_000.0
            && (feet.y + part.height).is_finite()
            && feet.y >= low - SKIN
            && feet.y + part.height <= high + SKIN
            && HexPrism::new(feet, part.height).is_some_and(|prism| {
                prism.horizontal_vertices().all(|point| {
                    hull_axes
                        .into_iter()
                        .all(|axis| f64::from(point.dot(axis).abs()) <= limit)
                })
            })
    })
}

fn sweep_hits_body(before: PrismPose, after: PrismPose, other: &Actor) -> bool {
    before.parts.iter().zip(after.parts.iter()).any(|(a, b)| {
        let start = before.feet + a.offset;
        let delta = after.feet + b.offset - start;
        let Some(prism) = HexPrism::new(start, a.height) else {
            return true;
        };
        match other.species {
            Species::Golem | Species::Wisp | Species::Worm => {
                let mut count = 0;
                let contact = other.body_hex_prisms().any(|part| {
                    count += 1;
                    HexPrism::new(other.feet + part.offset, part.height)
                        .is_none_or(|body| prism.swept_overlaps_prism(delta, body, SKIN))
                });
                contact || count == 0
            }
            Species::Dragon => {
                prism.swept_overlaps_box(delta, other.feet, other.dimensions, other.body_yaw, SKIN)
            }
            Species::Human | Species::Shadow | Species::Goblin | Species::Shaman => prism
                .swept_overlaps_capsule(
                    delta,
                    other.feet,
                    other.dimensions.y,
                    other.dimensions.x * 0.5,
                    SKIN,
                ),
        }
    })
}

/// Exact positively overlapped horizontal columns for every current component.
/// Deployment must admit all of these columns, not merely rounded segment centers.
pub(crate) fn footprint_columns(pose: PrismPose) -> Option<Vec<HexCoord>> {
    let mut result = BTreeSet::new();
    for part in pose.parts.iter() {
        result.extend(part_columns(pose.feet + part.offset)?);
    }
    Some(result.into_iter().collect())
}

/// Current head footprint, in canonical order, independent of a support choice.
pub(crate) fn head_columns(pose: PrismPose) -> Option<Vec<HexCoord>> {
    let head = pose.parts.iter().next()?;
    part_columns(pose.feet + head.offset)
}

fn part_columns(feet: Vec3) -> Option<Vec<HexCoord>> {
    if !feet.is_finite() || feet.abs().max_element() > 10_000.0 {
        return None;
    }
    // Use identical artificial vertical intervals only to isolate exact native
    // horizontal overlap. No Y or support identity is inferred by this helper.
    let prism = HexPrism::new(feet.with_y(0.0), 1.0)?;
    let mut columns: Vec<_> = HexCoord::from_world(feet)
        .within_radius(2)
        .into_iter()
        .filter(|coord| {
            HexPrism::new(coord.to_world(0.0), 1.0)
                .is_some_and(|cell| prism.overlap_prism(cell, SKIN).is_some())
        })
        .collect();
    columns.sort_unstable();
    (!columns.is_empty()).then_some(columns)
}

/// Verify the caller's explicit current support band under the complete head.
/// A missing, buried, duplicate, or unrelated surface refuses exposure. Higher
/// roofs are never selected implicitly. The caller admits an attack only when
/// this minimum clearance reaches one complete level.
pub(crate) fn head_clearance(
    pose: PrismPose,
    supports: &[TilePos],
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) -> Option<f32> {
    if !valid_geometry(geometry) {
        return None;
    }
    let head = pose.parts.iter().next()?;
    let head_pose = PrismPose {
        feet: pose.feet,
        parts: BodyPrismSnapshot::try_from_parts(&[head])?,
    };
    if !contained(head_pose, geometry) {
        return None;
    }
    let feet = pose.feet + head.offset;
    let prism = HexPrism::new(feet, head.height)?;
    let columns = head_columns(pose)?;
    if supports.len() != columns.len() {
        return None;
    }
    let mut clearance = f32::INFINITY;
    for coord in columns {
        let mut matching = supports.iter().filter(|pos| pos.coord == coord);
        let support = *matching.next()?;
        if matching.next().is_some()
            || !(geometry.min_level..=geometry.max_level).contains(&support.level)
            || !view
                .voxels
                .get(&support)
                .is_some_and(|substance| *substance != SubstanceId::AIR)
        {
            return None;
        }
        let top = geometry.top(support);
        if top > feet.y + SKIN {
            return None;
        }
        // Reject a stale lower support when any higher solid now lies below or
        // inside the head. A distinct roof wholly above the head remains legal.
        if support.level < geometry.max_level
            && view
                .voxels
                .range(
                    TilePos::new(coord, support.level + 1)
                        ..=TilePos::new(coord, geometry.max_level),
                )
                .any(|(pos, substance)| {
                    *substance != SubstanceId::AIR
                        && geometry.top(*pos) - geometry.level_height < feet.y + head.height - SKIN
                })
        {
            return None;
        }
        clearance = clearance.min(feet.y - top);
    }
    // Head clearance is an ordinary physical-world rule: no dirt exception.
    let cells = swept_prism_cells(head_pose, head_pose, geometry).ok()?;
    if cells.iter().any(|pos| {
        view.voxels
            .get(pos)
            .is_some_and(|substance| *substance != SubstanceId::AIR)
    }) {
        return None;
    }
    if view.static_spans.iter().any(|span| {
        HexPrism::new(
            span.bottom
                .coord
                .to_world(geometry.top(span.bottom) - geometry.level_height),
            geometry.top(TilePos::new(span.bottom.coord, span.top_level))
                - geometry.top(span.bottom)
                + geometry.level_height,
        )
        .is_none_or(|solid| prism.overlap_prism(solid, SKIN).is_some())
    }) || view.liquids.iter().any(|span| {
        HexPrism::new(
            span.bottom
                .coord
                .to_world(geometry.top(span.bottom) - geometry.level_height),
            geometry.top(TilePos::new(span.bottom.coord, span.top_level))
                - geometry.top(span.bottom)
                + geometry.level_height,
        )
        .is_none_or(|liquid| prism.overlap_prism(liquid, SKIN).is_some())
    }) {
        return None;
    }
    clearance.is_finite().then_some(clearance)
}
