//! Explicit synthetic world-readability fixtures, never natural-combat evidence.
//!
//! Selection reads the admitted world's geometry. All cuts are exact real
//! `TerrainImpact` announcements; ordinary ArenaTick publication and collision
//! refresh must settle before a frame is eligible. No player or enemy is moved,
//! damaged, frozen, or granted discoveries by this module.

use std::collections::{BTreeMap, BTreeSet};

use bevy::{ecs::message::MessageCursor, prelude::*};
use hex_arena::ArenaSession;
use hex_core::arena::{ArenaMaterials, ArenaSolidSpan, ArenaTerrainView, ArenaVoxelGeometry};
use hex_core::{
    HexCoord, SubstanceId, TerrainBatchId, TerrainDamageKind, TerrainImpact,
    TerrainImpactDisposition, TerrainImpactOutcome, TerrainImpactResult, TilePos,
    MAX_TERRAIN_TOUGHNESS,
};

const STAGE_FRAME: u32 = 20;
// Reserved capture-only namespace, far above gameplay's sequential run batches.
const SYNTHETIC_BATCH: u64 = 0x5359_4E54_4800_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fixture {
    Craters,
    CratersRear,
    TreeCut,
    WaterUnder,
    WaterEdge,
}

impl Fixture {
    fn parse(view: &str) -> Option<Self> {
        match view {
            "expedition-craters" => Some(Self::Craters),
            "expedition-craters-rear" => Some(Self::CratersRear),
            "expedition-tree-cut" => Some(Self::TreeCut),
            "expedition-water-under" => Some(Self::WaterUnder),
            "expedition-water-edge" => Some(Self::WaterEdge),
            _ => None,
        }
    }

    fn batch(self) -> TerrainBatchId {
        TerrainBatchId(
            SYNTHETIC_BATCH
                + match self {
                    Self::Craters => 1,
                    Self::CratersRear => 2,
                    Self::TreeCut => 3,
                    Self::WaterUnder => 4,
                    Self::WaterEdge => 5,
                },
        )
    }
}

#[derive(Clone, Copy)]
struct Framing {
    target: Vec3,
    desired: Vec3,
    // Water views use an exact point, so collision retraction cannot silently
    // turn the underwater fixture into an above-water photograph.
    exact: bool,
    underwater: bool,
}

struct Plan {
    removed: BTreeSet<TilePos>,
    retained: BTreeMap<TilePos, SubstanceId>,
    liquids: Vec<ArenaSolidSpan>,
    // (original center surface, retained floor): typed 2/8-level measurements.
    depths: Vec<(TilePos, TilePos)>,
    shade_occluders: Vec<TilePos>,
    framing: Framing,
}

/// Pre-edit fixture identity, expected publication and independent outcome cursor.
/// Installed only by explicit windowless capture staging; never ordinary play.
#[derive(Resource)]
pub(super) struct ReadabilityCapture {
    fixture: Fixture,
    plan: Plan,
    impact: Option<TerrainImpact>,
    outcomes: MessageCursor<TerrainImpactOutcome>,
    applied: bool,
    verified: bool,
    camera: Option<Transform>,
    initial_revision: u64,
    verified_revision: Option<u64>,
}

pub(super) fn fixture_view(view: &str) -> bool {
    Fixture::parse(view).is_some()
}

pub(super) fn description(view: &str) -> Option<&'static str> {
    Fixture::parse(view).map(|fixture| match fixture {
        Fixture::Craters | Fixture::CratersRear => "SYNTHETIC_PRESENTATION: paired seven-column forest craters are cut exactly 2 and 8 voxel levels by a frame-20 TerrainImpact. Published object occupancy blocks the current sun ray at both centers. Ordinary ticks must report every removal, preserve floors/shade/water and refresh geometry. External composition camera; no casting, movement or natural-combat claim.",
        Fixture::TreeCut => "SYNTHETIC_PRESENTATION: frame 20 announces a small exact TerrainImpact notch in a tall forest object's trunk. Ordinary ticks must remove the notch while upper trunk/crown cells and water remain. External composition camera; unsupported geometry is deliberately retained. No natural-combat claim.",
        Fixture::WaterUnder => "SYNTHETIC_PRESENTATION: an external camera is placed inside a published river-water voxel, at least three voxel levels below the surface. No terrain, actor or liquid mutation; the camera-based underwater effect must use this actual camera pose. No swimming or movement claim.",
        Fixture::WaterEdge => "SYNTHETIC_PRESENTATION: frame 20 removes a small riverbank inspection notch through TerrainImpact, exposing the unchanged adjacent water volume. The camera occupies dry air in the notch below the water surface. Ordinary ticks must report every solid removal and preserve every liquid span. No fluid draining, casting or movement claim.",
    })
}

/// Called in the existing capture branch before its ordinary ArenaTick steps.
pub(super) fn stage(world: &mut World, frame: u32, view: &str) -> Result<(), String> {
    let Some(fixture) = Fixture::parse(view) else {
        return Ok(());
    };
    if frame == STAGE_FRAME {
        if world
            .resource::<ArenaSession>()
            .expedition_progress()
            .is_none()
        {
            return Err("Readability fixture requires the admitted expedition package.".into());
        }
        let sun = world
            .query_filtered::<&Transform, With<DirectionalLight>>()
            .iter(world)
            .next()
            .map(|pose| pose.rotation * Vec3::Z)
            .ok_or("Forest shade fixture requires the actual directional light.")?;
        let geometry = *world.resource::<ArenaVoxelGeometry>();
        let terrain = world.resource::<ArenaTerrainView>();
        let plan = match fixture {
            Fixture::Craters | Fixture::CratersRear => {
                crater_plan(terrain, geometry, sun, fixture)?
            }
            Fixture::TreeCut => tree_plan(terrain, geometry)?,
            Fixture::WaterUnder | Fixture::WaterEdge => water_plan(terrain, geometry, fixture)?,
        };
        let impact = (!plan.removed.is_empty()).then(|| TerrainImpact {
            batch: fixture.batch(),
            volume: plan.removed.iter().copied().collect(),
            kind: TerrainDamageKind::Elemental(world.resource::<ArenaMaterials>().fire),
            power: MAX_TERRAIN_TOUGHNESS,
        });
        let state = ReadabilityCapture {
            fixture,
            plan,
            applied: impact.is_none(),
            verified: false,
            camera: None,
            initial_revision: terrain.revision,
            verified_revision: None,
            outcomes: world
                .resource::<Messages<TerrainImpactOutcome>>()
                .get_cursor_current(),
            impact: impact.clone(),
        };
        world.insert_resource(state);
        if let Some(impact) = impact {
            world.write_message(impact);
        }
        info!(
            fixture = description(view),
            "Staged world-readability fixture"
        );
    }
    if frame < STAGE_FRAME {
        return Ok(());
    }
    world.resource_scope(|world, mut capture: Mut<ReadabilityCapture>| {
        // This cursor does not drain messages or hide ordinary gameplay outcomes.
        let outcomes: Vec<_> = capture.outcomes
            .read(world.resource::<Messages<TerrainImpactOutcome>>())
            .filter(|outcome| outcome.batch == fixture.batch()).cloned().collect();
        for outcome in outcomes {
            let request = capture.impact.as_ref().ok_or("Unexpected synthetic water outcome.")?;
            if !fully_destroyed(&outcome, request) {
                return Err(format!("Readability impact failed: {:?}", outcome.result));
            }
            capture.applied = true;
        }
        let terrain = world.resource::<ArenaTerrainView>();
        if capture.verified && capture.verified_revision == Some(terrain.revision) {
            return Ok(());
        }
        capture.verified = false;
        capture.camera = None;
        let geometry = *world.resource::<ArenaVoxelGeometry>();
        if capture.applied && publication_matches(&capture.plan, terrain) {
            let frame = capture.plan.framing;
            let position = if frame.exact { frame.desired } else {
                world.resource::<ArenaSession>().camera_position(frame.target, frame.desired)
            };
            let pose = Transform::from_translation(position).looking_at(frame.target, Vec3::Y);
            // A close wall may retract an orbit, but it must not hide the whole
            // subject or move the exact water fixtures across the surface.
            if camera_matches(terrain, geometry, frame, &pose) {
                capture.camera = Some(pose);
                capture.verified = true;
                capture.verified_revision = Some(terrain.revision);
            }
        }
        if frame > 160 && !capture.verified {
            return Err(format!("Readability fixture did not settle: applied={}, publication_matches={}, camera={:?}",
                capture.applied, publication_matches(&capture.plan, terrain), capture.camera));
        }
        Ok(())
    })
}

fn fully_destroyed(outcome: &TerrainImpactOutcome, request: &TerrainImpact) -> bool {
    !request.volume.is_empty()
        && outcome.is_consistent_with(request)
        && matches!(&outcome.result,
        TerrainImpactResult::Applied(cells) if cells.iter().all(|cell|
            cell.disposition == TerrainImpactDisposition::Destroyed && cell.after.is_none()))
}

fn publication_matches(plan: &Plan, terrain: &ArenaTerrainView) -> bool {
    plan.removed.iter().all(|cell| {
        terrain.solid_at(*cell).is_none()
            && !terrain.static_spans.iter().any(|span| {
                span.bottom.coord == cell.coord
                    && (span.bottom.level..=span.top_level).contains(&cell.level)
                    && (span.blocks_movement || span.blocks_projectiles || span.blocks_sight)
            })
    }) && plan
        .retained
        .iter()
        .all(|(cell, material)| terrain.solid_at(*cell) == Some(*material))
        && terrain.liquids == plan.liquids
}

fn camera_matches(
    terrain: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    frame: Framing,
    pose: &Transform,
) -> bool {
    let Some(cell) = geometry.voxel_at(pose.translation) else {
        return false;
    };
    let clear = terrain.solid_at(cell).is_none()
        && !terrain.static_spans.iter().any(|span| {
            span.bottom.coord == cell.coord
                && (span.bottom.level..=span.top_level).contains(&cell.level)
                && span.blocks_movement
        });
    clear
        && in_water(terrain, cell) == frame.underwater
        && if frame.exact {
            pose.translation.distance(frame.desired) < 0.001
        } else {
            pose.translation.distance(frame.target) >= 4.0
        }
}

/// Cached typed eligibility, refreshed from world publication by `stage`.
pub(super) fn ready(capture: Option<&ReadabilityCapture>, view: &str) -> bool {
    capture.is_some_and(|capture| Fixture::parse(view) == Some(capture.fixture) && capture.verified)
}

/// Exact staged camera. Apply before camera-based water presentation runs.
pub(super) fn camera(capture: Option<&ReadabilityCapture>, view: &str) -> Option<Transform> {
    let capture = capture?;
    (Fixture::parse(view) == Some(capture.fixture) && capture.verified)
        .then_some(capture.camera)
        .flatten()
}

/// Add to the capture's JSON receipt; positions and depths remain auditable.
pub(super) fn receipt(capture: Option<&ReadabilityCapture>) -> serde_json::Value {
    let Some(capture) = capture else {
        return serde_json::Value::Null;
    };
    serde_json::json!({
        "classification": "SYNTHETIC_PRESENTATION",
        "batch": capture.impact.as_ref().map(|impact| impact.batch.0),
        "all_requested_cells_destroyed": capture.applied,
        "verified": capture.verified,
        "initial_revision": capture.initial_revision,
        "verified_revision": capture.verified_revision,
        "removed": capture.plan.removed.iter().map(cell_json).collect::<Vec<_>>(),
        "retained": capture.plan.retained.keys().map(cell_json).collect::<Vec<_>>(),
        "depths": capture.plan.depths.iter().map(|(surface, floor)| serde_json::json!({
            "surface": cell_json(surface), "floor": cell_json(floor),
            "voxel_levels": surface.level - floor.level,
        })).collect::<Vec<_>>(),
        "sun_occluders": capture.plan.shade_occluders.iter().map(cell_json).collect::<Vec<_>>(),
        "water_span_count": capture.plan.liquids.len(),
        "camera": capture.camera.map(|pose| pose.translation.to_array()),
        "camera_in_water": capture.plan.framing.underwater,
        "expected_session_notice": capture.impact.as_ref().map(|_| "Unmatched terrain outcome; reset the arena."),
    })
}

fn cell_json(cell: &TilePos) -> serde_json::Value {
    serde_json::json!({ "q": cell.coord.x(), "r": cell.coord.y(), "level": cell.level })
}

fn terrain_surface(terrain: &ArenaTerrainView, coord: HexCoord) -> Option<TilePos> {
    terrain
        .voxels
        .range(TilePos::new(coord, i32::MIN)..=TilePos::new(coord, i32::MAX))
        .next_back()
        .map(|(cell, _)| *cell)
}

fn object_at(terrain: &ArenaTerrainView, cell: TilePos) -> bool {
    terrain
        .object_columns
        .get(&cell.coord)
        .is_some_and(|spans| {
            spans
                .iter()
                .any(|span| (span.bottom.level..=span.top_level).contains(&cell.level))
        })
}

fn in_water(terrain: &ArenaTerrainView, cell: TilePos) -> bool {
    terrain.liquids.iter().any(|span| {
        span.bottom.coord == cell.coord
            && (span.bottom.level..=span.top_level).contains(&cell.level)
    })
}

fn base_plan(terrain: &ArenaTerrainView, framing: Framing) -> Plan {
    Plan {
        removed: BTreeSet::new(),
        retained: BTreeMap::new(),
        liquids: terrain.liquids.clone(),
        depths: Vec::new(),
        shade_occluders: Vec::new(),
        framing,
    }
}

fn retain(plan: &mut Plan, terrain: &ArenaTerrainView, cell: TilePos) -> Option<()> {
    plan.retained.insert(cell, terrain.solid_at(cell)?);
    Some(())
}

fn sun_occluder(
    terrain: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    surface: TilePos,
    sun: Vec3,
) -> Option<TilePos> {
    let origin = surface.coord.to_world(geometry.top(surface) + 0.1);
    (1..400_u16).find_map(|step| {
        let cell = geometry.voxel_at(origin + sun * (f32::from(step) * 0.25))?;
        object_at(terrain, cell).then_some(cell)
    })
}

fn crater_plan(
    terrain: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    sun: Vec3,
    fixture: Fixture,
) -> Result<Plan, String> {
    let sites = terrain.expedition.as_ref().ok_or("Missing forest sites.")?;
    let focus = sites
        .encounters
        .get("forest_troll")
        .ok_or("Missing forest center.")?
        .deployment
        .preferred
        .coord;
    let mut candidates = focus.within_radius(65);
    candidates.sort_by_key(|coord| (coord.distance(focus), *coord));
    for coord in candidates {
        let other = HexCoord::from_axial(coord.x() + 4, coord.y());
        let Some(a) = terrain_surface(terrain, coord) else {
            continue;
        };
        let Some(b) = terrain_surface(terrain, other) else {
            continue;
        };
        if (a.level - b.level).abs() > 1 {
            continue;
        }
        let Some(shade_a) = sun_occluder(terrain, geometry, a, sun) else {
            continue;
        };
        let Some(shade_b) = sun_occluder(terrain, geometry, b, sun) else {
            continue;
        };
        let target = (geometry.center(a) + geometry.center(b)) * 0.5 + Vec3::Y * 1.2;
        let offset = Vec3::new(
            0.0,
            7.0,
            if fixture == Fixture::CratersRear {
                -10.0
            } else {
                10.0
            },
        );
        let mut plan = base_plan(
            terrain,
            Framing {
                target,
                desired: target + offset,
                exact: false,
                underwater: false,
            },
        );
        if !cut_crater(&mut plan, terrain, a, 2) || !cut_crater(&mut plan, terrain, b, 8) {
            continue;
        }
        if retain(&mut plan, terrain, shade_a).is_none()
            || retain(&mut plan, terrain, shade_b).is_none()
        {
            continue;
        }
        plan.shade_occluders = vec![shade_a, shade_b];
        // Both azimuths use the same pair. Refuse a nearby trunk/hill that would
        // retract one camera until only a single crater remained in frame.
        if ![-1.0, 1.0].into_iter().all(|side| {
            clear_segment(
                terrain,
                geometry,
                target,
                target + Vec3::new(0.0, 7.0, 10.0 * side),
                &plan.removed,
            )
        }) {
            continue;
        }
        return Ok(plan);
    }
    Err(
        "No paired dry, tree-shaded forest footprints with intact 2/8-level floors were published."
            .into(),
    )
}

fn cut_crater(plan: &mut Plan, terrain: &ArenaTerrainView, center: TilePos, depth: i32) -> bool {
    for coord in center.coord.within_radius(1) {
        let Some(surface) = terrain_surface(terrain, coord) else {
            return false;
        };
        if (surface.level - center.level).abs() > 1 {
            return false;
        }
        let floor = TilePos::new(coord, surface.level - depth);
        if retain(plan, terrain, floor).is_none() {
            return false;
        }
        for level in floor.level + 1..=surface.level + 8 {
            let cell = TilePos::new(coord, level);
            if object_at(terrain, cell) || in_water(terrain, cell) {
                return false;
            }
            if level <= surface.level {
                if !terrain.voxels.contains_key(&cell) {
                    return false;
                }
                plan.removed.insert(cell);
            }
        }
    }
    plan.depths
        .push((center, TilePos::new(center.coord, center.level - depth)));
    true
}

fn clear_segment(
    terrain: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    start: Vec3,
    end: Vec3,
    removed: &BTreeSet<TilePos>,
) -> bool {
    (0..=100_u16).all(|step| {
        let point = start.lerp(end, f32::from(step) / 100.0);
        geometry
            .voxel_at(point)
            .is_some_and(|cell| removed.contains(&cell) || terrain.solid_at(cell).is_none())
    })
}

fn tree_plan(terrain: &ArenaTerrainView, geometry: ArenaVoxelGeometry) -> Result<Plan, String> {
    let focus = terrain
        .expedition
        .as_ref()
        .and_then(|sites| sites.encounters.get("forest_troll"))
        .ok_or("Missing forest center.")?
        .deployment
        .preferred
        .coord;
    let mut candidates = focus.within_radius(35);
    candidates.sort_by_key(|coord| (coord.distance(focus), *coord));
    for coord in candidates {
        let Some(surface) = terrain_surface(terrain, coord) else {
            continue;
        };
        let Some(trunk) = terrain.object_columns.get(&coord).and_then(|spans| {
            spans.iter().find(|span| {
                span.bottom.level <= surface.level + 2 && span.top_level >= surface.level + 35
            })
        }) else {
            continue;
        };
        for neighbor in coord
            .within_radius(1)
            .into_iter()
            .filter(|neighbor| *neighbor != coord)
        {
            let outside =
                HexCoord::from_axial(neighbor.x() * 2 - coord.x(), neighbor.y() * 2 - coord.y());
            let target = geometry.center(TilePos::new(coord, surface.level + 6));
            let outward = (outside.to_world(0.0) - coord.to_world(0.0)).normalize_or(Vec3::Z);
            // Choose a visible trunk edge, not a notch buried in the central giant.
            if (surface.level + 3..=surface.level + 10)
                .any(|level| terrain.solid_at(TilePos::new(outside, level)).is_some())
            {
                continue;
            }
            let mut plan = base_plan(
                terrain,
                Framing {
                    target,
                    desired: target + outward * 6.0 + Vec3::Y * 2.5,
                    exact: false,
                    underwater: false,
                },
            );
            for cut_coord in coord.within_radius(1) {
                for level in surface.level + 3..=surface.level + 8 {
                    let cell = TilePos::new(cut_coord, level);
                    if object_at(terrain, cell) && !terrain.voxels.contains_key(&cell) {
                        plan.removed.insert(cell);
                    }
                }
            }
            if !plan
                .removed
                .contains(&TilePos::new(coord, surface.level + 6))
            {
                continue;
            }
            for level in [surface.level + 12, trunk.top_level] {
                let _retained = retain(&mut plan, terrain, TilePos::new(coord, level));
            }
            if plan.retained.len() != 2
                || !clear_segment(
                    terrain,
                    geometry,
                    target,
                    plan.framing.desired,
                    &plan.removed,
                )
            {
                continue;
            }
            return Ok(plan);
        }
    }
    Err("No tall forest trunk with a clear side and surviving upper geometry was published.".into())
}

fn water_plan(
    terrain: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    fixture: Fixture,
) -> Result<Plan, String> {
    let spawn = terrain
        .spawns
        .first()
        .copied()
        .ok_or("Missing bridge spawn.")?;
    let focus = spawn + Vec3::Z * 18.0;
    let fountain_columns: BTreeSet<_> = terrain
        .expedition
        .as_ref()
        .ok_or("Missing expedition water.")?
        .fountains
        .values()
        .flat_map(|pool| pool.cells.iter().map(|cell| cell.coord))
        .collect();
    let mut water: Vec<_> = terrain
        .liquids
        .iter()
        .filter(|span| {
            span.top_level - span.bottom.level >= 5
                && !fountain_columns.contains(&span.bottom.coord)
                && span.bottom.coord.to_world(0.0).distance(spawn.with_y(0.0)) < 70.0
        })
        .collect();
    water.sort_by(|a, b| {
        a.bottom
            .coord
            .to_world(0.0)
            .distance_squared(focus.with_y(0.0))
            .total_cmp(
                &b.bottom
                    .coord
                    .to_world(0.0)
                    .distance_squared(focus.with_y(0.0)),
            )
            .then_with(|| a.bottom.cmp(&b.bottom))
    });
    for span in water {
        let top = TilePos::new(span.bottom.coord, span.top_level);
        let water_top = geometry.top(top);
        if fixture == Fixture::WaterUnder {
            let cell = TilePos::new(top.coord, top.level - 3);
            if terrain.solid_at(cell).is_some() {
                continue;
            }
            let desired = geometry.center(cell);
            let target = desired + Vec3::new(-8.0, 1.5, 1.0);
            return Ok(base_plan(
                terrain,
                Framing {
                    target,
                    desired,
                    exact: true,
                    underwater: true,
                },
            ));
        }
        for bank in top
            .coord
            .within_radius(1)
            .into_iter()
            .filter(|bank| *bank != top.coord)
        {
            if terrain
                .liquids
                .iter()
                .any(|liquid| liquid.bottom.coord == bank)
            {
                continue;
            }
            let Some(bank_top) = terrain_surface(terrain, bank) else {
                continue;
            };
            if !(top.level..=top.level + 6).contains(&bank_top.level) {
                continue;
            }
            let desired = bank.to_world(water_top - 0.45);
            let target = top.coord.to_world(water_top - 0.65);
            let mut plan = base_plan(
                terrain,
                Framing {
                    target,
                    desired,
                    exact: true,
                    underwater: false,
                },
            );
            for coord in bank.within_radius(1) {
                if terrain
                    .liquids
                    .iter()
                    .any(|liquid| liquid.bottom.coord == coord)
                {
                    continue;
                }
                for level in top.level - 7..=bank_top.level + 4 {
                    let cell = TilePos::new(coord, level);
                    if terrain.solid_at(cell).is_some() {
                        plan.removed.insert(cell);
                    }
                }
            }
            if plan.removed.is_empty()
                || retain(&mut plan, terrain, TilePos::new(bank, top.level - 8)).is_none()
            {
                continue;
            }
            return Ok(plan);
        }
    }
    Err(
        "No deep river column / supported bank suitable for the water fixture was published."
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_terrain() -> ArenaTerrainView {
        let mut terrain = ArenaTerrainView::default();
        for coord in HexCoord::ORIGIN.within_radius(8) {
            for level in 0..=12 {
                terrain
                    .voxels
                    .insert(TilePos::new(coord, level), SubstanceId(1));
            }
        }
        terrain
    }

    #[test]
    fn crater_request_has_exact_depths_and_canonical_disjoint_cells() {
        let terrain = flat_terrain();
        let mut plan = base_plan(
            &terrain,
            Framing {
                target: Vec3::ZERO,
                desired: Vec3::Z,
                exact: false,
                underwater: false,
            },
        );
        assert!(cut_crater(
            &mut plan,
            &terrain,
            TilePos::new(HexCoord::ORIGIN, 12),
            2
        ));
        assert!(cut_crater(
            &mut plan,
            &terrain,
            TilePos::new(HexCoord::from_axial(4, 0), 12),
            8
        ));
        assert_eq!(plan.removed.len(), 70);
        assert_eq!(plan.retained.len(), 14);
        assert_eq!(
            plan.depths
                .iter()
                .map(|(top, floor)| top.level - floor.level)
                .collect::<Vec<_>>(),
            vec![2, 8]
        );
        assert!(plan
            .removed
            .iter()
            .all(|cell| !plan.retained.contains_key(cell)));
        assert!(plan
            .removed
            .iter()
            .zip(plan.removed.iter().skip(1))
            .all(|(a, b)| a < b));
    }

    #[test]
    fn crater_refuses_liquid_or_a_trunk_in_its_exact_footprint() {
        let mut terrain = flat_terrain();
        terrain.object_columns.insert(
            HexCoord::ORIGIN,
            vec![ArenaSolidSpan {
                bottom: TilePos::new(HexCoord::ORIGIN, 13),
                top_level: 30,
                substance: SubstanceId(1),
            }],
        );
        let mut plan = base_plan(
            &terrain,
            Framing {
                target: Vec3::ZERO,
                desired: Vec3::Z,
                exact: false,
                underwater: false,
            },
        );
        assert!(!cut_crater(
            &mut plan,
            &terrain,
            TilePos::new(HexCoord::ORIGIN, 12),
            2
        ));
        terrain.object_columns.clear();
        terrain.liquids.push(ArenaSolidSpan {
            bottom: TilePos::new(HexCoord::ORIGIN, 11),
            top_level: 12,
            substance: SubstanceId(1),
        });
        assert!(!cut_crater(
            &mut plan,
            &terrain,
            TilePos::new(HexCoord::ORIGIN, 12),
            2
        ));
    }

    #[test]
    fn readiness_rejects_stale_collision_missing_upper_geometry_and_changed_water() {
        let mut terrain = flat_terrain();
        let mut plan = base_plan(
            &terrain,
            Framing {
                target: Vec3::ZERO,
                desired: Vec3::Z,
                exact: false,
                underwater: false,
            },
        );
        assert!(cut_crater(
            &mut plan,
            &terrain,
            TilePos::new(HexCoord::ORIGIN, 12),
            2
        ));
        assert!(!publication_matches(&plan, &terrain));
        for cell in &plan.removed {
            terrain.voxels.remove(cell);
        }
        assert!(publication_matches(&plan, &terrain));
        terrain.static_spans.push(hex_core::arena::ArenaStaticSpan {
            bottom: TilePos::new(HexCoord::ORIGIN, 11),
            top_level: 12,
            blocks_movement: false,
            blocks_projectiles: true,
            blocks_sight: true,
        });
        assert!(!publication_matches(&plan, &terrain));
        terrain.static_spans.clear();
        terrain.voxels.remove(&TilePos::new(HexCoord::ORIGIN, 10));
        assert!(!publication_matches(&plan, &terrain));
        terrain
            .voxels
            .insert(TilePos::new(HexCoord::ORIGIN, 10), SubstanceId(1));
        terrain.liquids.push(ArenaSolidSpan {
            bottom: TilePos::new(HexCoord::ORIGIN, 11),
            top_level: 12,
            substance: SubstanceId(1),
        });
        assert!(!publication_matches(&plan, &terrain));
    }

    #[test]
    fn underwater_readiness_requires_the_exact_camera_inside_the_published_volume() {
        let mut terrain = ArenaTerrainView::default();
        terrain.liquids.push(ArenaSolidSpan {
            bottom: TilePos::new(HexCoord::ORIGIN, 1),
            top_level: 12,
            substance: SubstanceId(1),
        });
        let geometry = ArenaVoxelGeometry::default();
        let desired = geometry.center(TilePos::new(HexCoord::ORIGIN, 8));
        let frame = Framing {
            target: desired + Vec3::Z,
            desired,
            exact: true,
            underwater: true,
        };
        let pose = Transform::from_translation(desired).looking_at(frame.target, Vec3::Y);
        assert!(camera_matches(&terrain, geometry, frame, &pose));
        let dry = Transform::from_translation(desired + Vec3::Y * 10.0);
        assert!(!camera_matches(&terrain, geometry, frame, &dry));
        terrain.liquids.clear();
        assert!(!camera_matches(&terrain, geometry, frame, &pose));
    }
}
