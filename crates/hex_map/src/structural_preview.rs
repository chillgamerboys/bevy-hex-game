//! Renderer-free structural review data for the Grand V3 world.
//!
//! This module deliberately consumes the generator-neutral world snapshot rather
//! than any private procedural plan. Developer tools therefore inspect the exact
//! public terrain, material, blocker, anchor, and liquid consequences without
//! launching the game or growing a second terrain implementation.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Arguments, Write as _};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use hex_core::{HexCoord, Level, MapAnchorId, MapObservationAnchors, TilePos};
use hex_multiplayer::{WorldLiquidFlowV1, WorldSnapshotV1};
use hex_schematic::{FeatureKind, SchematicCoord, SchematicPlanV1};

use crate::procedural_v3::grand_v3_structural_review_draft_enabled;
use crate::{V3_SCHEMATIC_CELL_PITCH, V3_SCHEMATIC_GRID_RADIUS};

/// Current on-disk structural-preview schema.
pub const GRAND_V3_STRUCTURAL_PREVIEW_VERSION: u16 = 1;
/// Default shipped Grand V3 review seed.
pub const GRAND_V3_STRUCTURAL_PREVIEW_HERO_SEED: u64 = 1_592_598_566;

const PEAK_SECTION_MARGIN: u32 = V3_SCHEMATIC_CELL_PITCH;
const PEAK_SIDE_HALF_LENGTH: i32 = 44;
const PEAK_EXACT_HALF_LENGTH: i32 = 32;
const MASSIF_RADIAL_LENGTH: i32 = 88;
const FROZEN_CRYSTAL_MARGIN: u32 = V3_SCHEMATIC_CELL_PITCH;
const WATERFALL_GORGE_HALF_WIDTH: i32 = 12;

const AXES: [(i32, i32); 3] = [(1, 0), (0, 1), (-1, 1)];
const DIRECTIONS: [(i32, i32); 6] = [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)];

/// One exact horizontal sample used by a structural cross-section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StructuralSample {
    /// Signed distance along the section.
    pub offset: i32,
    /// Exact sampled coordinate.
    pub coord: HexCoord,
    /// Highest exact semantic surface carrying biome ownership.
    pub terrain_level: Option<Level>,
    /// Highest material voxel in the public column snapshot.
    pub material_top_level: Option<Level>,
    /// Highest authored liquid voxel at the coordinate.
    pub liquid_top_level: Option<Level>,
    /// Stable map-local biome region of `terrain_level`.
    pub biome_region: Option<u32>,
    /// Whether the exact semantic terrain surface is contextually blocked.
    pub terrain_blocked: bool,
}

/// One named, ordered structural transect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuralSection {
    /// Stable output identity.
    pub name: String,
    /// Ordered exact samples.
    pub samples: Vec<StructuralSample>,
}

/// One exact directed-waterfall centerline node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaterfallCenterlineSample {
    /// Zero-based position along the exact downstream chain.
    pub index: usize,
    /// Exact liquid voxel selected for the centerline.
    pub position: TilePos,
    /// Published flow class.
    pub flow: WorldLiquidFlowV1,
    /// Exact downstream target, when the base terminates the sampled chain.
    pub downstream: Option<TilePos>,
    /// Canonical cross-axis used for the gorge row.
    pub cross_axis: (i32, i32),
    /// Contiguous wet width through offset zero.
    pub wet_width: u32,
    /// Contiguous semantic-floor width at or below the local water level.
    pub gorge_width_at_or_below_water: u32,
}

/// Exact cross-row paired with one waterfall centerline node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaterfallGorgeRow {
    /// Matching centerline index.
    pub centerline_index: usize,
    /// Ordered offsets `-12..=12` across the local tangent.
    pub samples: Vec<StructuralSample>,
}

/// Complete deterministic Grand V3 structural-review projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrandV3StructuralPreview {
    /// Preview schema version.
    pub version: u16,
    /// Exact generation seed supplied by the tool.
    pub seed: u64,
    /// Exact schematic semantic identity.
    pub schematic_fingerprint: u64,
    /// Inclusive axial bounds of the public materialized world.
    pub bounds: StructuralBounds,
    /// Exact field data in coordinate order.
    pub height_samples: Vec<StructuralSample>,
    /// Peak-chain front/side sections, six Massif radials, and Frozen/Crystal transects.
    pub sections: Vec<StructuralSection>,
    /// Exact directed waterfall chain from crown to base.
    pub waterfall_centerline: Vec<WaterfallCenterlineSample>,
    /// Exact width samples paired with the waterfall chain.
    pub waterfall_gorge_rows: Vec<WaterfallGorgeRow>,
}

/// Inclusive rectangular axial bounds used by CSV and PGM output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StructuralBounds {
    /// Minimum axial q.
    pub minimum_q: i32,
    /// Maximum axial q.
    pub maximum_q: i32,
    /// Minimum axial r.
    pub minimum_r: i32,
    /// Maximum axial r.
    pub maximum_r: i32,
}

/// A malformed or incomplete snapshot cannot produce an authoritative preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuralPreviewError(String);

impl std::fmt::Display for StructuralPreviewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for StructuralPreviewError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerrainSurface {
    position: TilePos,
    biome_region: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DirectedLiquid {
    flow: WorldLiquidFlowV1,
    downstream: Option<TilePos>,
}

#[derive(Debug, Clone, Copy)]
struct ProfilePanelLayout {
    left_margin: u32,
    top_margin: u32,
    panel_width: u32,
    panel_height: u32,
    column_gap: u32,
    row_gap: u32,
}

#[derive(Debug, Default)]
struct StructuralFields {
    terrain: BTreeMap<HexCoord, TerrainSurface>,
    material: BTreeMap<HexCoord, Level>,
    liquid: BTreeMap<HexCoord, Level>,
    blockers: BTreeSet<TilePos>,
    directed_liquids: BTreeMap<TilePos, DirectedLiquid>,
}

impl StructuralFields {
    fn from_snapshot(snapshot: &WorldSnapshotV1) -> Result<Self, StructuralPreviewError> {
        snapshot
            .validate()
            .map_err(|error| StructuralPreviewError(format!("invalid world snapshot: {error}")))?;

        let mut fields = Self::default();
        for column in snapshot.columns.iter() {
            let material_top = column
                .runs
                .iter()
                .map(|run| run.position.level)
                .max()
                .ok_or_else(|| {
                    StructuralPreviewError(format!(
                        "snapshot column {:?} has no material run",
                        column.coord
                    ))
                })?;
            fields.material.insert(column.coord, material_top);
        }
        for entry in snapshot.biome_regions.iter() {
            let candidate = TerrainSurface {
                position: entry.position,
                biome_region: entry.region,
            };
            let replace = fields
                .terrain
                .get(&entry.position.coord)
                .is_none_or(|current| candidate.position > current.position);
            if replace {
                fields.terrain.insert(entry.position.coord, candidate);
            }
        }
        for entry in snapshot.liquids.iter() {
            fields
                .liquid
                .entry(entry.position.coord)
                .and_modify(|level| *level = (*level).max(entry.position.level))
                .or_insert(entry.position.level);
            fields.directed_liquids.insert(
                entry.position,
                DirectedLiquid {
                    flow: entry.flow,
                    downstream: entry.downstream,
                },
            );
        }
        fields.blockers.extend(snapshot.blockers.iter().copied());
        Ok(fields)
    }

    fn sample(&self, coord: HexCoord, offset: i32) -> StructuralSample {
        let terrain = self.terrain.get(&coord).copied();
        StructuralSample {
            offset,
            coord,
            terrain_level: terrain.map(|surface| surface.position.level),
            material_top_level: self.material.get(&coord).copied(),
            liquid_top_level: self.liquid.get(&coord).copied(),
            biome_region: terrain.map(|surface| surface.biome_region),
            terrain_blocked: terrain
                .is_some_and(|surface| self.blockers.contains(&surface.position)),
        }
    }

    fn all_samples(&self) -> Vec<StructuralSample> {
        self.material
            .keys()
            .chain(self.terrain.keys())
            .chain(self.liquid.keys())
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|coord| self.sample(coord, 0))
            .collect()
    }

    fn bounds(&self) -> Result<StructuralBounds, StructuralPreviewError> {
        let minimum_q = self
            .material
            .keys()
            .map(|coord| coord.x())
            .min()
            .ok_or_else(|| StructuralPreviewError("snapshot has no material columns".to_owned()))?;
        let maximum_q = self
            .material
            .keys()
            .map(|coord| coord.x())
            .max()
            .ok_or_else(|| StructuralPreviewError("snapshot has no material columns".to_owned()))?;
        let minimum_r = self
            .material
            .keys()
            .map(|coord| coord.y())
            .min()
            .ok_or_else(|| StructuralPreviewError("snapshot has no material columns".to_owned()))?;
        let maximum_r = self
            .material
            .keys()
            .map(|coord| coord.y())
            .max()
            .ok_or_else(|| StructuralPreviewError("snapshot has no material columns".to_owned()))?;
        Ok(StructuralBounds {
            minimum_q,
            maximum_q,
            minimum_r,
            maximum_r,
        })
    }
}

/// Builds every required structural view from exact public world consequences.
pub fn build_grand_v3_structural_preview(
    plan: &SchematicPlanV1,
    snapshot: &WorldSnapshotV1,
    observation_anchors: &MapObservationAnchors,
    seed: u64,
) -> Result<GrandV3StructuralPreview, StructuralPreviewError> {
    let fields = StructuralFields::from_snapshot(snapshot)?;
    let expected_footprint = HexCoord::ORIGIN
        .within_radius(V3_SCHEMATIC_GRID_RADIUS)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let material_footprint = fields.material.keys().copied().collect::<BTreeSet<_>>();
    let terrain_footprint = fields.terrain.keys().copied().collect::<BTreeSet<_>>();
    if material_footprint != expected_footprint || terrain_footprint != expected_footprint {
        return Err(StructuralPreviewError(format!(
            "Grand V3 preview requires the complete radius-{} footprint: expected {} columns, material={}, terrain={}, missing_material={}, extra_material={}, missing_terrain={}, extra_terrain={}",
            V3_SCHEMATIC_GRID_RADIUS,
            expected_footprint.len(),
            material_footprint.len(),
            terrain_footprint.len(),
            expected_footprint.difference(&material_footprint).count(),
            material_footprint.difference(&expected_footprint).count(),
            expected_footprint.difference(&terrain_footprint).count(),
            terrain_footprint.difference(&expected_footprint).count(),
        )));
    }
    let bounds = fields.bounds()?;
    let anchor = |name: &str| resolve_anchor(snapshot, observation_anchors, name);

    let mut sections = peak_chain_sections(plan, &fields)?;
    let massif_crest = anchor("grand_v3.massif_crest")?;
    sections.extend(massif_radial_sections(massif_crest.coord, &fields));
    sections.extend(frozen_plateau_sections(plan, &fields)?);
    let crystal_lower = anchor("crystal_ascent.lower_entry")?;
    let crystal_summit = anchor("grand_v3.crystal_summit")?;
    sections.push(path_section(
        "crystal-shell-transect",
        extended_line(
            crystal_lower.coord,
            crystal_summit.coord,
            FROZEN_CRYSTAL_MARGIN,
        ),
        &fields,
    ));
    sections.push(path_section(
        "crystal-frozen-exit-transect",
        extended_line(
            anchor("grand_v3.frozen_woods")?.coord,
            crystal_summit.coord,
            FROZEN_CRYSTAL_MARGIN,
        ),
        &fields,
    ));

    let crown = anchor("grand_v3.waterfall_crown")?;
    let base = anchor("grand_v3.waterfall_base")?;
    let waterfall_evidence = trace_waterfall(&fields.directed_liquids, crown, base)
        .map(|positions| waterfall_rows(&positions, &fields));
    let (waterfall_centerline, waterfall_gorge_rows) = match waterfall_evidence {
        Ok(evidence) => evidence,
        Err(error) if grand_v3_structural_review_draft_enabled() => {
            eprintln!(
                "Grand V3 structural-review draft: omitting incomplete waterfall evidence: {error}"
            );
            (Vec::new(), Vec::new())
        }
        Err(error) => return Err(error),
    };

    Ok(GrandV3StructuralPreview {
        version: GRAND_V3_STRUCTURAL_PREVIEW_VERSION,
        seed,
        schematic_fingerprint: plan.semantic_fingerprint,
        bounds,
        height_samples: fields.all_samples(),
        sections,
        waterfall_centerline,
        waterfall_gorge_rows,
    })
}

const GRAND_V3_STRUCTURAL_PREVIEW_OUTPUTS: [&str; 8] = [
    "height-map.csv",
    "terrain-height-map.pgm",
    "material-height-map.pgm",
    "cross-sections.csv",
    "profiles.svg",
    "waterfall-centerline.csv",
    "waterfall-gorge-width.csv",
    "manifest.txt",
];
const GRAND_V3_STRUCTURAL_PREVIEW_INCOMPLETE: &str = "INCOMPLETE.txt";
const GRAND_V3_STRUCTURAL_PREVIEW_INCOMPLETE_NOTICE: &str =
    "INCOMPLETE — the latest Grand V3 structural preview attempt did not finish.\n\
     Do not use any other file in this directory as current review evidence.\n";

/// Invalidates an earlier publication before strict generation begins.
///
/// The strict preview can fail while compiling the schematic, before there is a preview value to
/// pass to [`write_grand_v3_structural_preview`]. Writing the marker first and then removing every
/// owned artifact prevents a previous successful pack from surviving that failure as apparently
/// current evidence. Files not owned by this publisher are deliberately preserved.
pub fn begin_grand_v3_structural_preview_publication(output_directory: &Path) -> io::Result<()> {
    fs::create_dir_all(output_directory)?;
    fs::write(
        output_directory.join(GRAND_V3_STRUCTURAL_PREVIEW_INCOMPLETE),
        GRAND_V3_STRUCTURAL_PREVIEW_INCOMPLETE_NOTICE,
    )?;
    for name in GRAND_V3_STRUCTURAL_PREVIEW_OUTPUTS {
        match fs::remove_file(output_directory.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Writes deterministic CSV/PGM review artifacts and returns their ordered paths.
pub fn write_grand_v3_structural_preview(
    preview: &GrandV3StructuralPreview,
    output_directory: &Path,
) -> io::Result<Vec<PathBuf>> {
    begin_grand_v3_structural_preview_publication(output_directory)?;
    let incomplete = output_directory.join(GRAND_V3_STRUCTURAL_PREVIEW_INCOMPLETE);
    let outputs = [
        ("height-map.csv", render_height_csv(&preview.height_samples)),
        (
            "terrain-height-map.pgm",
            render_pgm(preview, |sample| sample.terrain_level),
        ),
        (
            "material-height-map.pgm",
            render_pgm(preview, |sample| sample.material_top_level),
        ),
        ("cross-sections.csv", render_sections_csv(&preview.sections)),
        ("profiles.svg", render_profiles_svg(preview)),
        (
            "waterfall-centerline.csv",
            render_waterfall_centerline_csv(&preview.waterfall_centerline),
        ),
        (
            "waterfall-gorge-width.csv",
            render_waterfall_gorge_csv(&preview.waterfall_gorge_rows),
        ),
        ("manifest.txt", render_manifest(preview)),
    ];
    let mut paths = Vec::with_capacity(outputs.len());
    for (name, contents) in outputs {
        let path = output_directory.join(name);
        fs::write(&path, contents)?;
        paths.push(path);
    }
    fs::remove_file(incomplete)?;
    Ok(paths)
}

fn resolve_anchor(
    snapshot: &WorldSnapshotV1,
    observations: &MapObservationAnchors,
    name: &str,
) -> Result<TilePos, StructuralPreviewError> {
    if let Some(position) = snapshot
        .anchors
        .iter()
        .find(|entry| entry.name.as_str() == name)
        .map(|entry| entry.position)
    {
        return Ok(position);
    }
    observations.get(&MapAnchorId::from(name)).ok_or_else(|| {
        StructuralPreviewError(format!("Grand V3 preview is missing anchor {name:?}"))
    })
}

fn peak_chain_sections(
    plan: &SchematicPlanV1,
    fields: &StructuralFields,
) -> Result<Vec<StructuralSection>, StructuralPreviewError> {
    let peak_cells = plan
        .cells
        .iter()
        .filter(|cell| cell.facts.overlays.contains(&FeatureKind::PeakRing))
        .map(|cell| (cell.coord, u32::from(cell.id.get())))
        .collect::<BTreeMap<_, _>>();
    let peak_coords = peak_cells.keys().copied().collect::<BTreeSet<_>>();
    let components = schematic_components(&peak_coords)?;
    if components.len() != 2 || components.iter().any(|component| component.len() != 6) {
        return Err(StructuralPreviewError(format!(
            "Grand V3 preview requires two six-cell peak chains; found component sizes {:?}",
            components.iter().map(BTreeSet::len).collect::<Vec<_>>()
        )));
    }

    // Four overview panels remain useful for scanning the complete silhouettes.
    // The exact panels below add 24 summit-pin and ten inter-peak saddle witnesses.
    let mut sections = Vec::with_capacity(38);
    for (index, component) in components.iter().enumerate() {
        let ordered = ordered_component_path(component).ok_or_else(|| {
            StructuralPreviewError(format!(
                "peak chain {} has no path through all six coarse peak cells",
                index + 1
            ))
        })?;
        let centers = ordered
            .iter()
            .copied()
            .map(schematic_world_center)
            .collect::<Result<Vec<_>, _>>()?;
        let region_ids = ordered
            .iter()
            .map(|coord| {
                peak_cells.get(coord).copied().ok_or_else(|| {
                    StructuralPreviewError(format!(
                        "peak chain {} lost schematic cell {coord:?}",
                        index + 1
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let summit_pins = region_ids
            .iter()
            .copied()
            .map(|region| exact_region_summit(region, fields))
            .collect::<Result<Vec<_>, _>>()?;
        let front = extended_path(join_center_path(&centers), PEAK_SECTION_MARGIN);
        let front_midpoint = front.get(front.len() / 2).copied().ok_or_else(|| {
            StructuralPreviewError("peak-chain front profile is empty".to_owned())
        })?;
        let high_point = highest_near_centers(&centers, fields).unwrap_or(front_midpoint);
        let first = centers
            .first()
            .copied()
            .ok_or_else(|| StructuralPreviewError("peak chain has no first center".to_owned()))?;
        let last = centers
            .last()
            .copied()
            .ok_or_else(|| StructuralPreviewError("peak chain has no last center".to_owned()))?;
        let side_axis = most_perpendicular_axis(first, last);
        sections.push(path_section(
            &format!("peak-chain-{}-front", index + 1),
            front,
            fields,
        ));
        sections.push(axis_section(
            &format!("peak-chain-{}-side", index + 1),
            high_point,
            side_axis,
            PEAK_SIDE_HALF_LENGTH,
            PEAK_SIDE_HALF_LENGTH,
            fields,
        ));

        for (summit_index, (region, summit)) in region_ids
            .iter()
            .copied()
            .zip(summit_pins.iter().copied())
            .enumerate()
        {
            let previous = summit_pins
                .get(summit_index.saturating_sub(1))
                .copied()
                .unwrap_or(summit);
            let next = summit_pins
                .get(summit_index.saturating_add(1))
                .copied()
                .unwrap_or(summit);
            let tangent = if previous.coord == next.coord {
                delta(
                    centers[summit_index.saturating_sub(1)],
                    centers[(summit_index + 1).min(centers.len().saturating_sub(1))],
                )
            } else {
                delta(previous.coord, next.coord)
            };
            let front_axis = most_parallel_axis(tangent);
            let side_axis = most_perpendicular_axis_for_delta(tangent);
            sections.push(axis_section(
                &format!(
                    "peak-chain-{}-summit-{}-cell-{}-front",
                    index + 1,
                    summit_index + 1,
                    region
                ),
                summit.coord,
                front_axis,
                PEAK_EXACT_HALF_LENGTH,
                PEAK_EXACT_HALF_LENGTH,
                fields,
            ));
            sections.push(axis_section(
                &format!(
                    "peak-chain-{}-summit-{}-cell-{}-side",
                    index + 1,
                    summit_index + 1,
                    region
                ),
                summit.coord,
                side_axis,
                PEAK_EXACT_HALF_LENGTH,
                PEAK_EXACT_HALF_LENGTH,
                fields,
            ));
        }

        for saddle_index in 0..region_ids.len().saturating_sub(1) {
            let first_region = region_ids[saddle_index];
            let second_region = region_ids[saddle_index + 1];
            let (first, second) = exact_interpeak_saddle(
                first_region,
                second_region,
                summit_pins[saddle_index],
                summit_pins[saddle_index + 1],
                fields,
            )?;
            let transverse_axis = delta(first.coord, second.coord);
            sections.push(axis_section(
                &format!(
                    "peak-chain-{}-saddle-{}-cells-{}-{}-transverse",
                    index + 1,
                    saddle_index + 1,
                    first_region,
                    second_region
                ),
                first.coord,
                transverse_axis,
                PEAK_EXACT_HALF_LENGTH,
                PEAK_EXACT_HALF_LENGTH,
                fields,
            ));
        }
    }
    Ok(sections)
}

fn exact_region_summit(
    region: u32,
    fields: &StructuralFields,
) -> Result<TilePos, StructuralPreviewError> {
    fields
        .terrain
        .values()
        .filter(|surface| surface.biome_region == region)
        .map(|surface| surface.position)
        .max_by(|left, right| {
            left.level
                .cmp(&right.level)
                .then_with(|| right.coord.cmp(&left.coord))
        })
        .ok_or_else(|| {
            StructuralPreviewError(format!(
                "peak cell {region} has no exact patch-owned terrain summit"
            ))
        })
}

fn exact_interpeak_saddle(
    first_region: u32,
    second_region: u32,
    first_summit: TilePos,
    second_summit: TilePos,
    fields: &StructuralFields,
) -> Result<(TilePos, TilePos), StructuralPreviewError> {
    let candidates = fields
        .terrain
        .values()
        .filter(|surface| surface.biome_region == first_region)
        .flat_map(|first| {
            first
                .position
                .coord
                .neighbors()
                .into_iter()
                .filter_map(|coord| {
                    fields
                        .terrain
                        .get(&coord)
                        .filter(|second| second.biome_region == second_region)
                        .map(|second| (first.position, second.position))
                })
        });
    candidates
        .min_by_key(|(first, second)| {
            let maximum = first.level.max(second.level);
            let relief = first
                .level
                .saturating_add(second.level)
                .saturating_sub(maximum);
            let summit_distance = first
                .coord
                .distance(first_summit.coord)
                .saturating_add(second.coord.distance(second_summit.coord));
            (maximum, relief, summit_distance, *first, *second)
        })
        .ok_or_else(|| {
            StructuralPreviewError(format!(
                "peak cells {first_region} and {second_region} have no exact adjacent scenic saddle seam"
            ))
        })
}

fn ordered_component_path(component: &BTreeSet<SchematicCoord>) -> Option<Vec<SchematicCoord>> {
    fn visit(
        component: &BTreeSet<SchematicCoord>,
        path: &mut Vec<SchematicCoord>,
        best: &mut Option<(u32, Vec<SchematicCoord>)>,
    ) {
        if path.len() == component.len() {
            let Some(span) = path
                .first()
                .copied()
                .zip(path.last().copied())
                .and_then(|(first, last)| first.checked_distance(last))
            else {
                return;
            };
            if best.as_ref().is_none_or(|(best_span, best_path)| {
                span > *best_span || (span == *best_span && path.as_slice() < best_path.as_slice())
            }) {
                *best = Some((span, path.clone()));
            }
            return;
        }
        let Some(current) = path.last().copied() else {
            return;
        };
        let Some(neighbors) = current.neighbors() else {
            return;
        };
        let mut candidates = neighbors
            .into_iter()
            .filter(|neighbor| component.contains(neighbor) && !path.contains(neighbor))
            .collect::<Vec<_>>();
        candidates.sort_unstable();
        for candidate in candidates {
            path.push(candidate);
            visit(component, path, best);
            path.pop();
        }
    }

    let mut best = None;
    for start in component.iter().copied() {
        visit(component, &mut vec![start], &mut best);
    }
    best.map(|(_, path)| path)
}

fn frozen_plateau_sections(
    plan: &SchematicPlanV1,
    fields: &StructuralFields,
) -> Result<Vec<StructuralSection>, StructuralPreviewError> {
    let centers = plan
        .cells
        .iter()
        .filter(|cell| cell.facts.overlays.contains(&FeatureKind::FrozenWoods))
        .map(|cell| schematic_world_center(cell.coord))
        .collect::<Result<Vec<_>, _>>()?;
    let (first, last) = farthest_pair(&centers).ok_or_else(|| {
        StructuralPreviewError("Frozen-Woods footprint has fewer than two centers".to_owned())
    })?;
    let longitudinal = extended_line(first, last, FROZEN_CRYSTAL_MARGIN);
    let midpoint = longitudinal
        .get(longitudinal.len() / 2)
        .copied()
        .ok_or_else(|| StructuralPreviewError("Frozen-Woods transect is empty".to_owned()))?;
    let transverse_axis = most_perpendicular_axis(first, last);
    Ok(vec![
        path_section("frozen-plateau-longitudinal", longitudinal, fields),
        axis_section(
            "frozen-plateau-transverse",
            midpoint,
            transverse_axis,
            PEAK_SIDE_HALF_LENGTH,
            PEAK_SIDE_HALF_LENGTH,
            fields,
        ),
    ])
}

fn schematic_components(
    coords: &BTreeSet<SchematicCoord>,
) -> Result<Vec<BTreeSet<SchematicCoord>>, StructuralPreviewError> {
    let mut unseen = coords.clone();
    let mut result = Vec::new();
    while let Some(start) = unseen.iter().next().copied() {
        unseen.remove(&start);
        let mut component = BTreeSet::from([start]);
        let mut frontier = vec![start];
        while let Some(coord) = frontier.pop() {
            let neighbors = coord.neighbors().ok_or_else(|| {
                StructuralPreviewError("peak-chain coordinate overflow".to_owned())
            })?;
            for neighbor in neighbors {
                if unseen.remove(&neighbor) {
                    component.insert(neighbor);
                    frontier.push(neighbor);
                }
            }
        }
        result.push(component);
    }
    result.sort_by_key(|component| component.iter().next().copied());
    Ok(result)
}

fn schematic_world_center(coord: SchematicCoord) -> Result<HexCoord, StructuralPreviewError> {
    let pitch = i32::try_from(V3_SCHEMATIC_CELL_PITCH)
        .map_err(|error| StructuralPreviewError(format!("schematic pitch exceeds i32: {error}")))?;
    let q = coord
        .q()
        .checked_mul(pitch)
        .ok_or_else(|| StructuralPreviewError("schematic q projection overflow".to_owned()))?;
    let r = coord
        .r()
        .checked_mul(pitch)
        .ok_or_else(|| StructuralPreviewError("schematic r projection overflow".to_owned()))?;
    Ok(HexCoord::from_axial(q, r))
}

fn farthest_pair(coords: &[HexCoord]) -> Option<(HexCoord, HexCoord)> {
    let mut best = None::<(u32, HexCoord, HexCoord)>;
    for (index, first) in coords.iter().copied().enumerate() {
        for second in coords.iter().copied().skip(index + 1) {
            let pair = if first <= second {
                (first, second)
            } else {
                (second, first)
            };
            let candidate = (first.distance(second), pair.0, pair.1);
            if best.is_none_or(|current| {
                candidate.0 > current.0
                    || (candidate.0 == current.0 && pair < (current.1, current.2))
            }) {
                best = Some(candidate);
            }
        }
    }
    best.map(|(_, first, second)| (first, second))
}

fn join_center_path(centers: &[HexCoord]) -> Vec<HexCoord> {
    let mut result = Vec::new();
    for pair in centers.windows(2) {
        let [first, second] = pair else {
            continue;
        };
        let mut segment = first.line_between(*second);
        if !result.is_empty() && !segment.is_empty() {
            segment.remove(0);
        }
        result.extend(segment);
    }
    if result.is_empty() {
        result.extend(centers.iter().copied());
    }
    result
}

fn extended_path(mut line: Vec<HexCoord>, margin: u32) -> Vec<HexCoord> {
    if line.len() < 2 || margin == 0 {
        return line;
    }
    let Some(line_first) = line.first().copied() else {
        return line;
    };
    let Some(line_second) = line.get(1).copied() else {
        return line;
    };
    let Some(line_last) = line.last().copied() else {
        return line;
    };
    let Some(line_penultimate) = line.get(line.len().saturating_sub(2)).copied() else {
        return line;
    };
    let first_delta = delta(line_first, line_second);
    let last_delta = delta(line_penultimate, line_last);
    let mut prefix = (1..=i32::try_from(margin).unwrap_or(i32::MAX))
        .rev()
        .filter_map(|distance| step(line_first, (-first_delta.0, -first_delta.1), distance))
        .collect::<Vec<_>>();
    prefix.append(&mut line);
    let end = *prefix.last().unwrap_or(&line_last);
    prefix.extend(
        (1..=i32::try_from(margin).unwrap_or(i32::MAX))
            .filter_map(|distance| step(end, last_delta, distance)),
    );
    prefix
}

fn highest_near_centers(centers: &[HexCoord], fields: &StructuralFields) -> Option<HexCoord> {
    let radius = V3_SCHEMATIC_CELL_PITCH;
    centers
        .iter()
        .flat_map(|center| center.within_radius(radius))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|coord| {
            fields
                .terrain
                .get(&coord)
                .map(|surface| (surface.position.level, coord))
        })
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)))
        .map(|(_, coord)| coord)
}

fn massif_radial_sections(center: HexCoord, fields: &StructuralFields) -> Vec<StructuralSection> {
    DIRECTIONS
        .iter()
        .copied()
        .enumerate()
        .map(|(index, direction)| {
            let coords = (0..=MASSIF_RADIAL_LENGTH)
                .filter_map(|offset| step(center, direction, offset))
                .collect::<Vec<_>>();
            let samples = coords
                .into_iter()
                .enumerate()
                .map(|(offset, coord)| {
                    fields.sample(coord, i32::try_from(offset).unwrap_or(i32::MAX))
                })
                .collect();
            StructuralSection {
                name: format!("massif-radial-{}", index + 1),
                samples,
            }
        })
        .collect()
}

fn extended_line(first: HexCoord, last: HexCoord, margin: u32) -> Vec<HexCoord> {
    extended_path(first.line_between(last), margin)
}

fn path_section(name: &str, coords: Vec<HexCoord>, fields: &StructuralFields) -> StructuralSection {
    let center = i32::try_from(coords.len() / 2).unwrap_or(i32::MAX);
    let samples = coords
        .into_iter()
        .enumerate()
        .map(|(index, coord)| {
            fields.sample(
                coord,
                i32::try_from(index)
                    .unwrap_or(i32::MAX)
                    .saturating_sub(center),
            )
        })
        .collect();
    StructuralSection {
        name: name.to_owned(),
        samples,
    }
}

fn axis_section(
    name: &str,
    center: HexCoord,
    axis: (i32, i32),
    negative: i32,
    positive: i32,
    fields: &StructuralFields,
) -> StructuralSection {
    let samples = (-negative..=positive)
        .filter_map(|offset| step(center, axis, offset).map(|coord| fields.sample(coord, offset)))
        .collect();
    StructuralSection {
        name: name.to_owned(),
        samples,
    }
}

fn most_perpendicular_axis(first: HexCoord, last: HexCoord) -> (i32, i32) {
    most_perpendicular_axis_for_delta(delta(first, last))
}

fn most_perpendicular_axis_for_delta(tangent: (i32, i32)) -> (i32, i32) {
    AXES.iter()
        .copied()
        .enumerate()
        .min_by_key(|(index, axis)| (world_dot(*axis, tangent).unsigned_abs(), *index))
        .map_or_else(|| AXES.first().copied().unwrap_or((1, 0)), |(_, axis)| axis)
}

fn most_parallel_axis(tangent: (i32, i32)) -> (i32, i32) {
    AXES.iter()
        .copied()
        .enumerate()
        .max_by_key(|(index, axis)| (world_dot(*axis, tangent).unsigned_abs(), Reverse(*index)))
        .map_or_else(|| AXES.first().copied().unwrap_or((1, 0)), |(_, axis)| axis)
}

fn world_dot(first: (i32, i32), second: (i32, i32)) -> i64 {
    let (aq, ar) = (i64::from(first.0), i64::from(first.1));
    let (bq, br) = (i64::from(second.0), i64::from(second.1));
    (2 * aq + ar) * (2 * bq + br) + 3 * ar * br
}

fn delta(first: HexCoord, second: HexCoord) -> (i32, i32) {
    (
        second.x().saturating_sub(first.x()),
        second.y().saturating_sub(first.y()),
    )
}

fn step(coord: HexCoord, direction: (i32, i32), distance: i32) -> Option<HexCoord> {
    let q = coord.x().checked_add(direction.0.checked_mul(distance)?)?;
    let r = coord.y().checked_add(direction.1.checked_mul(distance)?)?;
    Some(HexCoord::from_axial(q, r))
}

fn trace_waterfall(
    liquids: &BTreeMap<TilePos, DirectedLiquid>,
    crown: TilePos,
    base: TilePos,
) -> Result<Vec<(TilePos, DirectedLiquid)>, StructuralPreviewError> {
    let mut failures = Vec::new();
    for start in directed_liquid_source_candidates(liquids, crown)? {
        let raw_path = match trace_directed_liquid_path_from(liquids, start) {
            Ok(path) => path,
            Err(error) => {
                failures.push(error.0);
                continue;
            }
        };
        // Snapshot V1 publishes exact directed liquid edges, but not the
        // authored three-lane row identities used before normalization.
        // Direction-change caps and high drops can therefore make a unique
        // reconstructed center lane unknowable. The exact directed chain is
        // the authoritative public consequence and is preferable to inferred
        // row geometry for bounding crown-to-base structural evidence.
        let mut result = raw_path;
        let (base_index, distance) = result
            .iter()
            .enumerate()
            .map(|(index, (position, _))| {
                (
                    index,
                    position.coord.distance(base.coord),
                    position.level.abs_diff(base.level),
                    *position,
                )
            })
            .min_by_key(|(_, distance, level_delta, position)| (*distance, *level_delta, *position))
            .map(|(index, distance, _, _)| (index, distance))
            .ok_or_else(|| StructuralPreviewError("waterfall centerline is empty".to_owned()))?;
        if distance > 3 {
            failures.push(format!(
                "waterfall centerline seeded at {start:?} never approaches base review anchor {base:?}; nearest distance is {distance}"
            ));
            continue;
        }
        result.truncate(base_index.saturating_add(1));
        return Ok(result);
    }
    Err(StructuralPreviewError(format!(
        "no directed waterfall source produced a reviewable chain near crown {crown:?}: {}",
        failures.join("; ")
    )))
}

fn directed_liquid_source_candidates(
    liquids: &BTreeMap<TilePos, DirectedLiquid>,
    crown: TilePos,
) -> Result<Vec<TilePos>, StructuralPreviewError> {
    let downstream_targets = liquids
        .values()
        .filter_map(|liquid| liquid.downstream)
        .collect::<BTreeSet<_>>();
    let mut starts = liquids
        .iter()
        .filter(|(position, liquid)| {
            liquid.downstream.is_some() && !downstream_targets.contains(position)
        })
        .map(|(position, _)| *position)
        .collect::<Vec<_>>();
    starts.sort_by_key(|position| {
        (
            position.coord.distance(crown.coord),
            position.level.abs_diff(crown.level),
            *position,
        )
    });
    if starts.is_empty() {
        return Err(StructuralPreviewError(format!(
            "no directed liquid source can seed the waterfall near crown {crown:?}"
        )));
    }
    Ok(starts)
}

fn trace_directed_liquid_path_from(
    liquids: &BTreeMap<TilePos, DirectedLiquid>,
    start: TilePos,
) -> Result<Vec<(TilePos, DirectedLiquid)>, StructuralPreviewError> {
    if !liquids.contains_key(&start) {
        return Err(StructuralPreviewError(format!(
            "waterfall source {start:?} is absent"
        )));
    }

    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    let mut current = start;
    loop {
        if !seen.insert(current) {
            return Err(StructuralPreviewError(format!(
                "waterfall downstream chain cycles at {current:?}"
            )));
        }
        let liquid = liquids.get(&current).copied().ok_or_else(|| {
            StructuralPreviewError(format!("waterfall downstream target {current:?} is absent"))
        })?;
        result.push((current, liquid));
        let Some(downstream) = liquid.downstream else {
            break;
        };
        current = downstream;
        if result.len() > liquids.len() {
            return Err(StructuralPreviewError(
                "waterfall chain exceeded liquid projection size".to_owned(),
            ));
        }
    }

    Ok(result)
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct DirectedLiquidRow {
    center: TilePos,
    members: BTreeSet<TilePos>,
    transverse_axis: (i32, i32),
}

#[cfg(test)]
fn semantic_three_lane_centerline(
    raw_path: &[(TilePos, DirectedLiquid)],
    liquids: &BTreeMap<TilePos, DirectedLiquid>,
) -> Result<Vec<(TilePos, DirectedLiquid)>, StructuralPreviewError> {
    if raw_path.is_empty() {
        return Err(StructuralPreviewError(
            "waterfall directed path is empty".to_owned(),
        ));
    }
    let positions_by_coord = liquids.keys().copied().fold(
        BTreeMap::<HexCoord, Vec<TilePos>>::new(),
        |mut positions, position| {
            positions.entry(position.coord).or_default().push(position);
            positions
        },
    );
    let mut result = Vec::with_capacity(raw_path.len());
    for (index, (position, _)) in raw_path.iter().copied().enumerate() {
        let previous = raw_path
            .get(index.saturating_sub(2))
            .or_else(|| raw_path.get(index.saturating_sub(1)))
            .map_or(position.coord, |entry| entry.0.coord);
        let next = raw_path
            .get(index.saturating_add(2))
            .or_else(|| raw_path.get(index.saturating_add(1)))
            .map_or(position.coord, |entry| entry.0.coord);
        let tangent = if previous == next {
            raw_path[index]
                .1
                .downstream
                .map_or((0, 1), |downstream| delta(position.coord, downstream.coord))
        } else {
            delta(previous, next)
        };
        let row =
            directed_three_lane_row_containing(position, tangent, liquids, &positions_by_coord);
        let semantic_center = match row {
            Ok(row) => {
                let adjacent_neighbors = row
                    .members
                    .iter()
                    .filter(|member| {
                        member.coord != row.center.coord
                            && member.coord.distance(row.center.coord) == 1
                    })
                    .count();
                if row.members.len() != 3 || adjacent_neighbors != 2 {
                    return Err(StructuralPreviewError(format!(
                        "waterfall row around {position:?} has no unique semantic center lane"
                    )));
                }
                row.center
            }
            Err(_error)
                if index > 0
                    && index.saturating_add(1) < raw_path.len()
                    && raw_path[index.saturating_sub(1)]
                        .0
                        .coord
                        .distance(position.coord)
                        == 1
                    && position
                        .coord
                        .distance(raw_path[index.saturating_add(1)].0.coord)
                        == 1
                    && raw_path[index.saturating_sub(1)].0.level >= position.level
                    && position.level >= raw_path[index.saturating_add(1)].0.level =>
            {
                // A normalized high-drop bend can collapse the three semantic
                // lanes into two adjacent rows joined by one exact directed
                // connector. Retain that authoritative connector instead of
                // inventing a collinear width claim absent from the snapshot.
                position
            }
            Err(error) => return Err(error),
        };
        if result
            .last()
            .is_some_and(|(current, _)| *current == semantic_center)
        {
            continue;
        }
        let liquid = liquids.get(&semantic_center).copied().ok_or_else(|| {
            StructuralPreviewError(format!(
                "waterfall semantic center {semantic_center:?} has no directed liquid"
            ))
        })?;
        result.push((semantic_center, liquid));
    }
    if result.windows(2).any(|pair| {
        pair[0].0.coord.distance(pair[1].0.coord) > 1 || pair[1].0.level > pair[0].0.level
    }) {
        return Err(StructuralPreviewError(
            "waterfall semantic center rows skip a fine-grid row or climb upstream".to_owned(),
        ));
    }
    Ok(result)
}

#[cfg(test)]
fn directed_three_lane_row_containing(
    position: TilePos,
    tangent: (i32, i32),
    liquids: &BTreeMap<TilePos, DirectedLiquid>,
    positions_by_coord: &BTreeMap<HexCoord, Vec<TilePos>>,
) -> Result<DirectedLiquidRow, StructuralPreviewError> {
    let candidate_centers = std::iter::once(position.coord)
        .chain(position.coord.neighbors())
        .collect::<BTreeSet<_>>();
    let mut candidates = Vec::new();
    for center_coord in candidate_centers.iter().copied() {
        for center in positions_by_coord
            .get(&center_coord)
            .into_iter()
            .flatten()
            .copied()
            .filter(|center| center.level.abs_diff(position.level) <= 1)
        {
            for (axis_index, axis) in AXES.iter().copied().enumerate() {
                let Some(negative_coord) = step(center_coord, axis, -1) else {
                    continue;
                };
                let Some(positive_coord) = step(center_coord, axis, 1) else {
                    continue;
                };
                let side_position = |coord: HexCoord| {
                    positions_by_coord
                        .get(&coord)
                        .into_iter()
                        .flatten()
                        .copied()
                        .filter(|side| side.level.abs_diff(center.level) <= 1)
                        .min_by_key(|side| (side.level.abs_diff(center.level), *side))
                };
                let Some((negative, positive)) =
                    side_position(negative_coord).zip(side_position(positive_coord))
                else {
                    continue;
                };
                let members = BTreeSet::from([negative, center, positive]);
                let minimum_level = members.iter().map(|member| member.level).min();
                let maximum_level = members.iter().map(|member| member.level).max();
                if !members.contains(&position)
                    || minimum_level
                        .zip(maximum_level)
                        .is_none_or(|(minimum, maximum)| maximum.saturating_sub(minimum) > 1)
                {
                    continue;
                }
                let internal_edges = members
                    .iter()
                    .filter(|member| {
                        liquids
                            .get(member)
                            .and_then(|liquid| liquid.downstream)
                            .is_some_and(|downstream| members.contains(&downstream))
                    })
                    .count();
                candidates.push((
                    0_u8,
                    internal_edges,
                    world_dot(axis, tangent).unsigned_abs(),
                    center_coord.distance(position.coord),
                    axis_index,
                    center,
                    DirectedLiquidRow {
                        center,
                        members,
                        transverse_axis: axis,
                    },
                ));
            }
        }
    }
    // At a one-row direction change, two consecutive normalized ribbon rows
    // can share a corner. Their physical three-lane footprint is then a bent
    // cap (three mutually connected hexes), not three collinear hexes. Accept
    // that exact bend consequence only after exhausting straight-row claims;
    // the raw directed member must still belong to the cap and all three
    // liquid tops must remain within one level.
    for center_coord in candidate_centers {
        for center in positions_by_coord
            .get(&center_coord)
            .into_iter()
            .flatten()
            .copied()
            .filter(|center| center.level.abs_diff(position.level) <= 1)
        {
            let neighbors = center_coord
                .neighbors()
                .into_iter()
                .filter_map(|coord| {
                    positions_by_coord
                        .get(&coord)
                        .into_iter()
                        .flatten()
                        .copied()
                        .filter(|neighbor| neighbor.level.abs_diff(center.level) <= 1)
                        .min_by_key(|neighbor| (neighbor.level.abs_diff(center.level), *neighbor))
                })
                .collect::<Vec<_>>();
            for first_index in 0..neighbors.len() {
                for second_index in first_index.saturating_add(1)..neighbors.len() {
                    let Some((first, second)) = neighbors
                        .get(first_index)
                        .copied()
                        .zip(neighbors.get(second_index).copied())
                    else {
                        continue;
                    };
                    if first.coord.distance(second.coord) != 1 {
                        continue;
                    }
                    let members = BTreeSet::from([first, center, second]);
                    let minimum_level = members.iter().map(|member| member.level).min();
                    let maximum_level = members.iter().map(|member| member.level).max();
                    if !members.contains(&position)
                        || minimum_level
                            .zip(maximum_level)
                            .is_none_or(|(minimum, maximum)| maximum.saturating_sub(minimum) > 1)
                    {
                        continue;
                    }
                    let internal_edges = members
                        .iter()
                        .filter(|member| {
                            liquids
                                .get(member)
                                .and_then(|liquid| liquid.downstream)
                                .is_some_and(|downstream| members.contains(&downstream))
                        })
                        .count();
                    let first_delta = delta(center_coord, first.coord);
                    let second_delta = delta(center_coord, second.coord);
                    let tangent_imbalance = world_dot(first_delta, tangent)
                        .saturating_add(world_dot(second_delta, tangent))
                        .unsigned_abs();
                    let transverse_axis = most_perpendicular_axis_for_delta(tangent);
                    candidates.push((
                        1_u8,
                        internal_edges,
                        tangent_imbalance,
                        center_coord.distance(position.coord),
                        AXES.len(),
                        center,
                        DirectedLiquidRow {
                            center,
                            members,
                            transverse_axis,
                        },
                    ));
                }
            }
        }
    }
    candidates
        .into_iter()
        .min_by_key(|candidate| {
            (
                candidate.0,
                candidate.1,
                candidate.2,
                candidate.3,
                candidate.4,
                candidate.5,
            )
        })
        .map(|candidate| candidate.6)
        .ok_or_else(|| {
            StructuralPreviewError(format!(
                "directed waterfall voxel {position:?} belongs to no level-tolerant three-lane semantic row"
            ))
        })
}

fn waterfall_rows(
    path: &[(TilePos, DirectedLiquid)],
    fields: &StructuralFields,
) -> (Vec<WaterfallCenterlineSample>, Vec<WaterfallGorgeRow>) {
    let mut centerline = Vec::with_capacity(path.len());
    let mut rows = Vec::with_capacity(path.len());
    for (index, (position, liquid)) in path.iter().copied().enumerate() {
        let previous = path
            .get(index.saturating_sub(1))
            .map_or(position.coord, |entry| entry.0.coord);
        let next = path
            .get(index.saturating_add(1))
            .map_or(position.coord, |entry| entry.0.coord);
        let tangent = if previous == next {
            (0, 1)
        } else {
            delta(previous, next)
        };
        let cross_axis = AXES
            .iter()
            .copied()
            .enumerate()
            .min_by_key(|(axis_index, axis)| {
                (world_dot(*axis, tangent).unsigned_abs(), *axis_index)
            })
            .map_or_else(|| AXES.first().copied().unwrap_or((1, 0)), |(_, axis)| axis);
        let samples = (-WATERFALL_GORGE_HALF_WIDTH..=WATERFALL_GORGE_HALF_WIDTH)
            .filter_map(|offset| {
                step(position.coord, cross_axis, offset).map(|coord| fields.sample(coord, offset))
            })
            .collect::<Vec<_>>();
        let wet_width = contiguous_width(&samples, |sample| sample.liquid_top_level.is_some());
        let gorge_width = contiguous_width(&samples, |sample| {
            sample
                .terrain_level
                .is_some_and(|level| level <= position.level)
        });
        centerline.push(WaterfallCenterlineSample {
            index,
            position,
            flow: liquid.flow,
            downstream: liquid.downstream,
            cross_axis,
            wet_width,
            gorge_width_at_or_below_water: gorge_width,
        });
        rows.push(WaterfallGorgeRow {
            centerline_index: index,
            samples,
        });
    }
    (centerline, rows)
}

fn contiguous_width(
    samples: &[StructuralSample],
    admitted: impl Fn(&StructuralSample) -> bool,
) -> u32 {
    let Some(center) = samples.iter().position(|sample| sample.offset == 0) else {
        return 0;
    };
    if !samples.get(center).is_some_and(&admitted) {
        return 0;
    }
    let left = samples
        .iter()
        .take(center)
        .rev()
        .take_while(|sample| admitted(sample))
        .count();
    let right = samples
        .iter()
        .skip(center.saturating_add(1))
        .take_while(|sample| admitted(sample))
        .count();
    u32::try_from(left.saturating_add(1).saturating_add(right)).unwrap_or(u32::MAX)
}

fn append_format(output: &mut String, arguments: Arguments<'_>) {
    let _formatting_result = output.write_fmt(arguments);
}

fn render_manifest(preview: &GrandV3StructuralPreview) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "grand_v3_structural_preview_version={}\n",
        preview.version
    ));
    output.push_str(&format!("seed={}\n", preview.seed));
    output.push_str(&format!(
        "schematic_fingerprint=0x{:016x}\n",
        preview.schematic_fingerprint
    ));
    output.push_str(&format!(
        "bounds_q={}..{}\n",
        preview.bounds.minimum_q, preview.bounds.maximum_q
    ));
    output.push_str(&format!(
        "bounds_r={}..{}\n",
        preview.bounds.minimum_r, preview.bounds.maximum_r
    ));
    output.push_str(&format!(
        "height_samples={}\n",
        preview.height_samples.len()
    ));
    for section in &preview.sections {
        output.push_str(&format!(
            "section={},{}\n",
            section.name,
            section.samples.len()
        ));
    }
    output.push_str(&format!(
        "waterfall_centerline_samples={}\n",
        preview.waterfall_centerline.len()
    ));
    if let Some(first) = preview.waterfall_centerline.first() {
        output.push_str(&format!(
            "waterfall_source={},{},{}\n",
            first.position.coord.x(),
            first.position.coord.y(),
            first.position.level,
        ));
    }
    if let Some(last) = preview.waterfall_centerline.last() {
        output.push_str(&format!(
            "waterfall_basin_approach={},{},{}\n",
            last.position.coord.x(),
            last.position.coord.y(),
            last.position.level,
        ));
    }
    output
}

fn render_height_csv(samples: &[StructuralSample]) -> String {
    let mut output = String::from(
        "q,r,terrain_level,material_top_level,liquid_top_level,biome_region,terrain_blocked\n",
    );
    for sample in samples {
        write_sample_fields(&mut output, sample, false);
    }
    output
}

fn render_sections_csv(sections: &[StructuralSection]) -> String {
    let mut output = String::from(
        "section,index,offset,q,r,terrain_level,material_top_level,liquid_top_level,biome_region,terrain_blocked\n",
    );
    for section in sections {
        for (index, sample) in section.samples.iter().enumerate() {
            append_format(&mut output, format_args!("{},{index},", section.name));
            write_sample_fields(&mut output, sample, true);
        }
    }
    output
}

fn render_waterfall_centerline_csv(samples: &[WaterfallCenterlineSample]) -> String {
    let mut output = String::from(
        "index,q,r,level,flow,downstream_q,downstream_r,downstream_level,cross_axis_q,cross_axis_r,wet_width,gorge_width_at_or_below_water\n",
    );
    for sample in samples {
        let downstream_q = sample.downstream.map(|position| position.coord.x());
        let downstream_r = sample.downstream.map(|position| position.coord.y());
        let downstream_level = sample.downstream.map(|position| position.level);
        append_format(
            &mut output,
            format_args!(
                "{},{},{},{},{},{},{},{},{},{},{},{}",
                sample.index,
                sample.position.coord.x(),
                sample.position.coord.y(),
                sample.position.level,
                flow_name(sample.flow),
                optional_display(downstream_q),
                optional_display(downstream_r),
                optional_display(downstream_level),
                sample.cross_axis.0,
                sample.cross_axis.1,
                sample.wet_width,
                sample.gorge_width_at_or_below_water,
            ),
        );
        output.push('\n');
    }
    output
}

fn render_waterfall_gorge_csv(rows: &[WaterfallGorgeRow]) -> String {
    let mut output = String::from(
        "centerline_index,offset,q,r,terrain_level,material_top_level,liquid_top_level,biome_region,terrain_blocked\n",
    );
    for row in rows {
        for sample in &row.samples {
            append_format(&mut output, format_args!("{},", row.centerline_index));
            write_sample_fields(&mut output, sample, true);
        }
    }
    output
}

fn write_sample_fields(output: &mut String, sample: &StructuralSample, include_offset: bool) {
    if include_offset {
        append_format(output, format_args!("{},", sample.offset));
    }
    append_format(
        output,
        format_args!(
            "{},{},{},{},{},{},{}",
            sample.coord.x(),
            sample.coord.y(),
            optional_display(sample.terrain_level),
            optional_display(sample.material_top_level),
            optional_display(sample.liquid_top_level),
            optional_display(sample.biome_region),
            u8::from(sample.terrain_blocked),
        ),
    );
    output.push('\n');
}

fn optional_display<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}

fn flow_name(flow: WorldLiquidFlowV1) -> &'static str {
    match flow {
        WorldLiquidFlowV1::Still => "still",
        WorldLiquidFlowV1::Current => "current",
        WorldLiquidFlowV1::Rapid => "rapid",
        WorldLiquidFlowV1::Fall => "fall",
    }
}

fn render_pgm(
    preview: &GrandV3StructuralPreview,
    select: impl Fn(&StructuralSample) -> Option<Level>,
) -> String {
    let values = preview
        .height_samples
        .iter()
        .filter_map(|sample| select(sample).map(|level| (sample.coord, level)))
        .collect::<BTreeMap<_, _>>();
    let maximum_level = values.values().copied().max().unwrap_or_default().max(0);
    let maximum_pixel = maximum_level.saturating_add(1).clamp(1, 65_535);
    let width = preview
        .bounds
        .maximum_q
        .saturating_sub(preview.bounds.minimum_q)
        .saturating_add(1);
    let height = preview
        .bounds
        .maximum_r
        .saturating_sub(preview.bounds.minimum_r)
        .saturating_add(1);
    let mut output = format!(
        "P2\n# axial q={}..{} r={}..{}; 0=missing, pixel=level+1\n{width} {height}\n{maximum_pixel}\n",
        preview.bounds.minimum_q,
        preview.bounds.maximum_q,
        preview.bounds.minimum_r,
        preview.bounds.maximum_r,
    );
    for r in preview.bounds.minimum_r..=preview.bounds.maximum_r {
        for q in preview.bounds.minimum_q..=preview.bounds.maximum_q {
            let pixel = values
                .get(&HexCoord::from_axial(q, r))
                .copied()
                .map_or(0, |level| level.saturating_add(1).clamp(0, maximum_pixel));
            append_format(&mut output, format_args!("{pixel} "));
        }
        output.push('\n');
    }
    output
}

fn render_profiles_svg(preview: &GrandV3StructuralPreview) -> String {
    const CANVAS_WIDTH: u32 = 1_200;
    const PANEL_WIDTH: u32 = 560;
    const PANEL_HEIGHT: u32 = 190;
    const LEFT_MARGIN: u32 = 32;
    const TOP_MARGIN: u32 = 72;
    const COLUMN_GAP: u32 = 24;
    const ROW_GAP: u32 = 18;
    let layout = ProfilePanelLayout {
        left_margin: LEFT_MARGIN,
        top_margin: TOP_MARGIN,
        panel_width: PANEL_WIDTH,
        panel_height: PANEL_HEIGHT,
        column_gap: COLUMN_GAP,
        row_gap: ROW_GAP,
    };

    let panel_count = preview.sections.len().saturating_add(1);
    let row_count = panel_count.saturating_add(1) / 2;
    let canvas_height = TOP_MARGIN
        .saturating_add(
            u32::try_from(row_count)
                .unwrap_or(u32::MAX)
                .saturating_mul(PANEL_HEIGHT.saturating_add(ROW_GAP)),
        )
        .saturating_add(34);
    let mut output = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{CANVAS_WIDTH}\" height=\"{canvas_height}\" viewBox=\"0 0 {CANVAS_WIDTH} {canvas_height}\">\n\
         <rect width=\"100%\" height=\"100%\" fill=\"#f7f8fa\"/>\n\
         <style>text{{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;fill:#18212f}} .axis{{stroke:#9aa4b2;stroke-width:1}} .terrain{{fill:none;stroke:#735a3a;stroke-width:2}} .material{{fill:none;stroke:#28384f;stroke-width:2}} .liquid{{fill:none;stroke:#1686d9;stroke-width:2.5}}</style>\n\
         <text x=\"32\" y=\"28\" font-size=\"18\" font-weight=\"700\">Grand V3 structural profiles — seed {}</text>\n\
         <text x=\"32\" y=\"49\" font-size=\"12\">fingerprint 0x{:016x} · independent vertical scale per panel · terrain / material / liquid</text>\n",
        preview.seed, preview.schematic_fingerprint,
    );

    for (index, section) in preview.sections.iter().enumerate() {
        let samples = section
            .samples
            .iter()
            .map(|sample| {
                (
                    sample.terrain_level,
                    sample.material_top_level,
                    sample.liquid_top_level,
                )
            })
            .collect::<Vec<_>>();
        render_profile_panel(&mut output, index, &section.name, &samples, layout);
    }

    let waterfall = preview
        .waterfall_centerline
        .iter()
        .map(|sample| {
            let terrain = preview
                .waterfall_gorge_rows
                .iter()
                .find(|row| row.centerline_index == sample.index)
                .and_then(|row| row.samples.iter().find(|entry| entry.offset == 0))
                .and_then(|entry| entry.terrain_level);
            (terrain, terrain, Some(sample.position.level))
        })
        .collect::<Vec<_>>();
    render_profile_panel(
        &mut output,
        preview.sections.len(),
        "waterfall-centerline",
        &waterfall,
        layout,
    );
    output.push_str("</svg>\n");
    output
}

fn render_profile_panel(
    output: &mut String,
    index: usize,
    name: &str,
    samples: &[(Option<Level>, Option<Level>, Option<Level>)],
    layout: ProfilePanelLayout,
) {
    const PLOT_LEFT: f64 = 42.0;
    const PLOT_TOP: f64 = 30.0;
    const PLOT_RIGHT: f64 = 12.0;
    const PLOT_BOTTOM: f64 = 24.0;

    let column = u32::try_from(index % 2).unwrap_or_default();
    let row = u32::try_from(index / 2).unwrap_or_default();
    let origin_x = layout
        .left_margin
        .saturating_add(column.saturating_mul(layout.panel_width + layout.column_gap));
    let origin_y = layout
        .top_margin
        .saturating_add(row.saturating_mul(layout.panel_height + layout.row_gap));
    let levels = samples
        .iter()
        .flat_map(|sample| [sample.0, sample.1, sample.2])
        .flatten()
        .collect::<Vec<_>>();
    let minimum = levels
        .iter()
        .copied()
        .min()
        .unwrap_or_default()
        .saturating_sub(4);
    let maximum = levels
        .iter()
        .copied()
        .max()
        .unwrap_or(minimum.saturating_add(8))
        .saturating_add(4)
        .max(minimum.saturating_add(8));
    let plot_width = f64::from(layout.panel_width) - PLOT_LEFT - PLOT_RIGHT;
    let plot_height = f64::from(layout.panel_height) - PLOT_TOP - PLOT_BOTTOM;
    let plot_x = f64::from(origin_x) + PLOT_LEFT;
    let plot_y = f64::from(origin_y) + PLOT_TOP;
    output.push_str(&format!(
        "<g><rect x=\"{origin_x}\" y=\"{origin_y}\" width=\"{}\" height=\"{}\" rx=\"5\" fill=\"#fff\" stroke=\"#c7ced8\"/>\n\
         <text x=\"{}\" y=\"{}\" font-size=\"13\" font-weight=\"700\">{name}</text>\n\
         <line class=\"axis\" x1=\"{plot_x:.2}\" y1=\"{plot_y:.2}\" x2=\"{plot_x:.2}\" y2=\"{:.2}\"/>\n\
         <line class=\"axis\" x1=\"{plot_x:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\"/>\n\
         <text x=\"{}\" y=\"{}\" font-size=\"10\">{maximum}</text>\n\
         <text x=\"{}\" y=\"{}\" font-size=\"10\">{minimum}</text>\n\
         <text x=\"{}\" y=\"{}\" font-size=\"10\">{} samples</text>\n",
        layout.panel_width,
        layout.panel_height,
        origin_x.saturating_add(10),
        origin_y.saturating_add(19),
        plot_y + plot_height,
        plot_y + plot_height,
        plot_x + plot_width,
        plot_y + plot_height,
        origin_x.saturating_add(4),
        origin_y.saturating_add(34),
        origin_x.saturating_add(4),
        origin_y
            .saturating_add(layout.panel_height)
            .saturating_sub(18),
        origin_x
            .saturating_add(layout.panel_width)
            .saturating_sub(86),
        origin_y
            .saturating_add(layout.panel_height)
            .saturating_sub(7),
        samples.len(),
    ));

    for (class, select) in [
        ("terrain", 0_usize),
        ("material", 1_usize),
        ("liquid", 2_usize),
    ] {
        let values = samples
            .iter()
            .map(|sample| match select {
                0 => sample.0,
                1 => sample.1,
                _ => sample.2,
            })
            .collect::<Vec<_>>();
        let path = profile_path(
            &values,
            minimum,
            maximum,
            plot_x,
            plot_y,
            plot_width,
            plot_height,
        );
        if !path.is_empty() {
            output.push_str(&format!("<path class=\"{class}\" d=\"{path}\"/>\n"));
        }
    }
    output.push_str("</g>\n");
}

fn profile_path(
    values: &[Option<Level>],
    minimum: Level,
    maximum: Level,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> String {
    let mut path = String::new();
    let denominator =
        f64::from(u32::try_from(values.len().saturating_sub(1).max(1)).unwrap_or(u32::MAX));
    let level_span = f64::from(maximum.saturating_sub(minimum).max(1));
    let mut drawing = false;
    for (index, level) in values.iter().copied().enumerate() {
        let Some(level) = level else {
            drawing = false;
            continue;
        };
        let point_x = x + width * f64::from(u32::try_from(index).unwrap_or(u32::MAX)) / denominator;
        let point_y = y + height - height * f64::from(level.saturating_sub(minimum)) / level_span;
        path.push_str(&format!(
            "{} {point_x:.2} {point_y:.2} ",
            if drawing { 'L' } else { 'M' }
        ));
        drawing = true;
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields_with_levels(entries: &[(HexCoord, Level)]) -> StructuralFields {
        let mut fields = StructuralFields::default();
        for (index, (coord, level)) in entries.iter().copied().enumerate() {
            let position = TilePos::new(coord, level);
            fields.terrain.insert(
                coord,
                TerrainSurface {
                    position,
                    biome_region: u32::try_from(index).unwrap_or(u32::MAX),
                },
            );
            fields.material.insert(coord, level);
        }
        fields
    }

    #[test]
    fn massif_radials_are_six_canonical_adjacent_profiles() {
        let center = HexCoord::from_axial(3, -7);
        let fields = fields_with_levels(&[(center, 200)]);
        let sections = massif_radial_sections(center, &fields);
        assert_eq!(sections.len(), 6);
        assert_eq!(
            sections
                .iter()
                .map(|section| section.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "massif-radial-1",
                "massif-radial-2",
                "massif-radial-3",
                "massif-radial-4",
                "massif-radial-5",
                "massif-radial-6",
            ]
        );
        for (section, direction) in sections.iter().zip(DIRECTIONS) {
            assert_eq!(
                section.samples.first().map(|sample| sample.coord),
                Some(center)
            );
            assert_eq!(section.samples.first().map(|sample| sample.offset), Some(0));
            assert_eq!(
                section.samples.get(1).map(|sample| sample.coord),
                step(center, direction, 1)
            );
            assert!(section.samples.windows(2).all(|pair| {
                pair.first()
                    .zip(pair.last())
                    .is_some_and(|(first, last)| first.coord.distance(last.coord) == 1)
            }));
        }
    }

    #[test]
    fn reference_plan_emits_two_peak_chains_with_front_and_side_profiles() {
        let template = hex_schematic::grand_v3_reference_template().expect("template parses");
        let generated =
            hex_schematic::reference_plan(&template, GRAND_V3_STRUCTURAL_PREVIEW_HERO_SEED)
                .expect("reference plan remains valid");
        let peaks = generated
            .plan
            .cells
            .iter()
            .filter(|cell| cell.facts.overlays.contains(&FeatureKind::PeakRing))
            .map(|cell| {
                (
                    u32::from(cell.id.get()),
                    cell.coord,
                    schematic_world_center(cell.coord).expect("coarse center projects"),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(peaks.len(), 12, "the authored plan retains twelve peaks");

        // The preview consumes exact owned terrain. Give each authored peak a
        // summit and each neighboring pair a lower, adjacent ownership seam.
        let mut fields = StructuralFields::default();
        for (first_region, first_cell, first_center) in peaks.iter().copied() {
            for (second_region, second_cell, second_center) in peaks.iter().copied() {
                if first_region >= second_region
                    || first_cell.checked_distance(second_cell) != Some(1)
                {
                    continue;
                }
                for coord in first_center.line_between(second_center) {
                    let first_distance = coord.distance(first_center);
                    let second_distance = coord.distance(second_center);
                    let biome_region = if first_distance <= second_distance {
                        first_region
                    } else {
                        second_region
                    };
                    fields.terrain.insert(
                        coord,
                        TerrainSurface {
                            position: TilePos::new(coord, 120),
                            biome_region,
                        },
                    );
                    fields.material.insert(coord, 120);
                }
            }
        }
        for (region, _, center) in peaks.iter().copied() {
            fields.terrain.insert(
                center,
                TerrainSurface {
                    position: TilePos::new(center, 200),
                    biome_region: region,
                },
            );
            fields.material.insert(center, 200);
        }

        let sections = peak_chain_sections(&generated.plan, &fields)
            .expect("reference peak rings retain owned summits and saddle seams");
        assert_eq!(sections.len(), 38);
        assert_eq!(
            sections
                .iter()
                .filter(|section| {
                    !section.name.contains("-summit-") && !section.name.contains("-saddle-")
                })
                .map(|section| section.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "peak-chain-1-front",
                "peak-chain-1-side",
                "peak-chain-2-front",
                "peak-chain-2-side",
            ]
        );
        assert!(sections.iter().all(|section| {
            section
                .samples
                .iter()
                .any(|sample| sample.terrain_level.is_some())
        }));
        for (region, _, center) in &peaks {
            let summit_sections = sections
                .iter()
                .filter(|section| section.name.contains(&format!("-cell-{region}-")))
                .collect::<Vec<_>>();
            assert_eq!(
                summit_sections.len(),
                2,
                "each summit has front and side views"
            );
            for section in summit_sections {
                let pin = section
                    .samples
                    .iter()
                    .find(|sample| sample.offset == 0)
                    .expect("the exact summit pin is sampled");
                assert_eq!(pin.coord, *center);
                assert_eq!(pin.terrain_level, Some(200));
                assert_eq!(pin.material_top_level, Some(200));
                assert_eq!(pin.biome_region, Some(*region));
            }
        }
        let saddle_sections = sections
            .iter()
            .filter(|section| section.name.contains("-saddle-"))
            .collect::<Vec<_>>();
        assert_eq!(
            saddle_sections.len(),
            10,
            "each chain has five saddle views"
        );
        for section in saddle_sections {
            let seam = section
                .samples
                .iter()
                .find(|sample| sample.offset == 0)
                .expect("the exact saddle seam is sampled");
            assert_eq!(seam.terrain_level, Some(120));
            assert!(
                seam.coord.neighbors().iter().any(|coord| {
                    fields.terrain.get(coord).is_some_and(|neighbor| {
                        neighbor.position.level == 120
                            && Some(neighbor.biome_region) != seam.biome_region
                    })
                }),
                "the saddle joins two exact neighboring owners"
            );
        }
        let peak_centers = peaks
            .iter()
            .map(|(_, _, center)| *center)
            .collect::<BTreeSet<_>>();
        let sampled_front = sections
            .iter()
            .filter(|section| section.name.ends_with("-front"))
            .flat_map(|section| section.samples.iter().map(|sample| sample.coord))
            .collect::<BTreeSet<_>>();
        assert!(peak_centers.is_subset(&sampled_front));
    }

    #[test]
    fn frozen_plateau_emits_longitudinal_and_transverse_profiles() {
        let template = hex_schematic::grand_v3_reference_template().expect("template parses");
        let generated =
            hex_schematic::reference_plan(&template, GRAND_V3_STRUCTURAL_PREVIEW_HERO_SEED)
                .expect("reference plan remains valid");
        let sections = frozen_plateau_sections(&generated.plan, &StructuralFields::default())
            .expect("locked Frozen-Woods footprint remains reviewable");
        assert_eq!(
            sections
                .iter()
                .map(|section| section.name.as_str())
                .collect::<Vec<_>>(),
            vec!["frozen-plateau-longitudinal", "frozen-plateau-transverse"]
        );
        assert!(sections.iter().all(|section| !section.samples.is_empty()));
    }

    #[test]
    fn directed_waterfall_trace_is_exact_and_fails_on_a_cycle() {
        let crown = TilePos::new(HexCoord::from_axial(0, 0), 10);
        let middle = TilePos::new(HexCoord::from_axial(0, 1), 8);
        let base = TilePos::new(HexCoord::from_axial(0, 2), 6);
        let chain = BTreeMap::from([
            (
                crown,
                DirectedLiquid {
                    flow: WorldLiquidFlowV1::Fall,
                    downstream: Some(middle),
                },
            ),
            (
                middle,
                DirectedLiquid {
                    flow: WorldLiquidFlowV1::Rapid,
                    downstream: Some(base),
                },
            ),
            (
                base,
                DirectedLiquid {
                    flow: WorldLiquidFlowV1::Current,
                    downstream: None,
                },
            ),
        ]);
        let traced = trace_waterfall(&chain, crown, base).expect("exact chain reaches its base");
        assert_eq!(
            traced.iter().map(|entry| entry.0).collect::<Vec<_>>(),
            vec![crown, middle, base]
        );

        let bank_anchor = TilePos::new(HexCoord::from_axial(1, 2), 6);
        let traced = trace_waterfall(&chain, crown, bank_anchor)
            .expect("a bank-side review anchor resolves to the adjacent liquid chain");
        assert_eq!(traced.last().map(|entry| entry.0), Some(base));

        let mut cycle = chain;
        cycle
            .get_mut(&base)
            .expect("base remains present")
            .downstream = Some(middle);
        let beyond_cycle = TilePos::new(HexCoord::from_axial(0, 3), 5);
        let error = trace_waterfall(&cycle, crown, beyond_cycle)
            .expect_err("a directed cycle cannot masquerade as a centerline");
        assert!(error.to_string().contains("cycles"));
    }

    #[test]
    fn semantic_waterfall_rows_accept_one_level_lane_stagger() {
        let centers = [
            TilePos::new(HexCoord::from_axial(0, 0), 10),
            TilePos::new(HexCoord::from_axial(1, 0), 8),
            TilePos::new(HexCoord::from_axial(2, 0), 6),
        ];
        let mut liquids = BTreeMap::new();
        for (index, center) in centers.iter().copied().enumerate() {
            let downstream = centers.get(index.saturating_add(1)).copied();
            liquids.insert(
                center,
                DirectedLiquid {
                    flow: WorldLiquidFlowV1::Fall,
                    downstream,
                },
            );
            for (coord, level) in [
                (HexCoord::from_axial(center.coord.x(), -1), center.level),
                (
                    HexCoord::from_axial(center.coord.x(), 1),
                    center.level.saturating_sub(1),
                ),
            ] {
                liquids.insert(
                    TilePos::new(coord, level),
                    DirectedLiquid {
                        flow: WorldLiquidFlowV1::Current,
                        downstream: None,
                    },
                );
            }
        }
        let raw = centers
            .iter()
            .copied()
            .map(|position| (position, liquids[&position]))
            .collect::<Vec<_>>();

        let semantic = semantic_three_lane_centerline(&raw, &liquids)
            .expect("a one-level physical lane stagger retains one semantic centerline");
        assert_eq!(
            semantic
                .iter()
                .map(|(position, _)| *position)
                .collect::<Vec<_>>(),
            centers
        );
    }

    #[test]
    fn semantic_waterfall_rows_accept_normalized_bend_caps() {
        let center = TilePos::new(HexCoord::ORIGIN, 10);
        let first = TilePos::new(HexCoord::from_axial(-1, 1), 10);
        let second = TilePos::new(HexCoord::from_axial(0, 1), 9);
        let liquids = [center, first, second]
            .into_iter()
            .map(|position| {
                (
                    position,
                    DirectedLiquid {
                        flow: WorldLiquidFlowV1::Rapid,
                        downstream: None,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let positions_by_coord = liquids.keys().copied().fold(
            BTreeMap::<HexCoord, Vec<TilePos>>::new(),
            |mut positions, position| {
                positions.entry(position.coord).or_default().push(position);
                positions
            },
        );

        let row = directed_three_lane_row_containing(second, (1, 0), &liquids, &positions_by_coord)
            .expect("a normalized one-level bend cap remains a physical three-lane row");
        assert_eq!(row.center, center);
        assert_eq!(row.members, BTreeSet::from([center, first, second]));
    }

    #[test]
    fn gorge_width_counts_only_the_contiguous_center_run() {
        let fields = fields_with_levels(
            &(-3..=3)
                .map(|q| {
                    (
                        HexCoord::from_axial(q, 0),
                        if (-1..=1).contains(&q) { 4 } else { 9 },
                    )
                })
                .collect::<Vec<_>>(),
        );
        let samples = (-3..=3)
            .map(|offset| fields.sample(HexCoord::from_axial(offset, 0), offset))
            .collect::<Vec<_>>();
        assert_eq!(
            contiguous_width(&samples, |sample| sample
                .terrain_level
                .is_some_and(|level| level <= 4)),
            3
        );
    }

    #[test]
    fn csv_and_pgm_rendering_are_canonical() {
        let first = HexCoord::from_axial(-1, 0);
        let second = HexCoord::from_axial(0, 0);
        let fields = fields_with_levels(&[(second, 7), (first, 3)]);
        let samples = fields.all_samples();
        let csv = render_height_csv(&samples);
        assert!(csv
            .lines()
            .nth(1)
            .is_some_and(|line| line.starts_with("-1,0,3,3")));
        assert!(csv
            .lines()
            .nth(2)
            .is_some_and(|line| line.starts_with("0,0,7,7")));

        let preview = GrandV3StructuralPreview {
            version: 1,
            seed: 1,
            schematic_fingerprint: 2,
            bounds: StructuralBounds {
                minimum_q: -1,
                maximum_q: 0,
                minimum_r: 0,
                maximum_r: 0,
            },
            height_samples: samples,
            sections: Vec::new(),
            waterfall_centerline: Vec::new(),
            waterfall_gorge_rows: Vec::new(),
        };
        assert_eq!(
            render_pgm(&preview, |sample| sample.terrain_level),
            "P2\n# axial q=-1..0 r=0..0; 0=missing, pixel=level+1\n2 1\n8\n4 8 \n"
        );
        let svg = render_profiles_svg(&preview);
        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(svg.contains("fingerprint 0x0000000000000002"));
        assert!(svg.contains(">waterfall-centerline</text>"));
        assert_eq!(svg, render_profiles_svg(&preview));
    }

    #[test]
    fn publication_start_invalidates_stale_success_and_preserves_unowned_files() {
        let directory = std::env::temp_dir().join(format!(
            "hex-map-grand-v3-preview-publication-{}",
            std::process::id()
        ));
        let _cleanup = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("preview test directory should be creatable");
        for name in GRAND_V3_STRUCTURAL_PREVIEW_OUTPUTS {
            fs::write(directory.join(name), "stale-success")
                .expect("stale preview artifact should be writable");
        }
        fs::write(directory.join("review-notes.txt"), "preserve me")
            .expect("unowned review notes should be writable");

        begin_grand_v3_structural_preview_publication(&directory)
            .expect("publication preflight should succeed");

        assert_eq!(
            fs::read_to_string(directory.join(GRAND_V3_STRUCTURAL_PREVIEW_INCOMPLETE))
                .expect("incomplete marker should be readable"),
            GRAND_V3_STRUCTURAL_PREVIEW_INCOMPLETE_NOTICE
        );
        assert!(GRAND_V3_STRUCTURAL_PREVIEW_OUTPUTS
            .iter()
            .all(|name| !directory.join(name).exists()));
        assert_eq!(
            fs::read_to_string(directory.join("review-notes.txt"))
                .expect("unowned review notes should survive"),
            "preserve me"
        );

        let preview = GrandV3StructuralPreview {
            version: GRAND_V3_STRUCTURAL_PREVIEW_VERSION,
            seed: 7,
            schematic_fingerprint: 11,
            bounds: StructuralBounds {
                minimum_q: 0,
                maximum_q: 0,
                minimum_r: 0,
                maximum_r: 0,
            },
            height_samples: Vec::new(),
            sections: Vec::new(),
            waterfall_centerline: Vec::new(),
            waterfall_gorge_rows: Vec::new(),
        };
        let outputs = write_grand_v3_structural_preview(&preview, &directory)
            .expect("a completed publication should succeed");
        assert_eq!(outputs.len(), GRAND_V3_STRUCTURAL_PREVIEW_OUTPUTS.len());
        assert!(!directory
            .join(GRAND_V3_STRUCTURAL_PREVIEW_INCOMPLETE)
            .exists());
        assert!(GRAND_V3_STRUCTURAL_PREVIEW_OUTPUTS
            .iter()
            .all(|name| directory.join(name).is_file()));

        fs::remove_dir_all(directory).expect("preview test directory should be removable");
    }
}
