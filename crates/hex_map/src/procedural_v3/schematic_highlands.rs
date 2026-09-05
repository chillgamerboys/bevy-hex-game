//! Connected highland shaping for the Grand V3 schematic compiler.
//!
//! Schematic cells retain semantic ownership, but they are not individual
//! terrain stamps. The authored peak cells become distinct crowns joined by
//! lower ridge saddles, the massif becomes one broad centrally crested body,
//! and Crystal Ascent receives an offset natural mountain screen outside its
//! exact radius-32 authored site. When
//! that exact site separates the fine Massif ownership mask, the height field
//! may cross the shortest eligible Mountain corridor without transferring any
//! column to another biome owner.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};

use hex_core::{HexCoord, Level};
use hex_schematic::{
    CellPlan, FeatureKind as SchematicFeature, LandformKind, NetworkKind, SchematicCoord,
    SchematicPlanV1, SurfaceKind,
};

use super::layout::{PatchId, ResolvedLayoutPlan};
use super::{grand_v3_structural_review_draft_enabled, V3GenerationError};
use crate::settings::{V3CrystalAscentSettings, V3GrandV3BasicTerrainProfile, MAX_V3_LEVEL};

const CELL_PITCH: i32 = 22;
pub(super) const CRYSTAL_SITE_RADIUS: u32 = 32;
const CRYSTAL_MANTLE_EXIT_CLEARANCE_DEPTH: u32 = 20;
const CRYSTAL_MANTLE_EXIT_CLEARANCE_BUFFER: u32 = 1;
const CRYSTAL_ENCLOSURE_OUTER_RADIUS: u32 = 82;
const CRYSTAL_ENCLOSURE_SOURCE_RADIUS: u32 = 50;
const CRYSTAL_ENCLOSURE_OPENING_HALF_WIDTH: u32 = 3;
const CRYSTAL_ENCLOSURE_HIGH_INNER_RADIUS: u32 = 42;
const CRYSTAL_ENCLOSURE_HIGH_OUTER_RADIUS: u32 = 66;
const CRYSTAL_ENCLOSURE_EDGE_RISE_PER_HEX: Level = 7;
pub(super) const CRYSTAL_ENCLOSURE_HIGH_MIN: Level = 192;
const CRYSTAL_ENCLOSURE_HIGH_MAX: Level = 208;
const CRYSTAL_ENCLOSURE_SUPPORT_SLOPE: Level = 1;
const CRYSTAL_SHELL_FROZEN_TRANSITION_DEPTH: u32 = 2;
pub(super) const CRYSTAL_SHELL_MAXIMUM_APRON_RISE: Level = 12;
const CRYSTAL_ENCLOSURE_FORCED_LOW_MAXIMUM_DEPTH: u32 = 2;
pub(super) const CRYSTAL_ENCLOSURE_FORCED_LOW_MAXIMUM_FRACTION_DIVISOR: usize = 4;
pub(super) const PEAK_SUMMIT_MIN: Level = 260;
pub(super) const PEAK_SUMMIT_MAX: Level = 300;
/// Above this level every upper crown must remain a separate component.
pub(super) const PEAK_VISUAL_WALL_THRESHOLD: Level = 240;
const PEAK_BODY_SLOPE: Level = 7;
// A sharp seven-level crown is desirable near each summit, but carrying that
// same pitch down to the coarse-cell bench produces a steep-sided drum. A
// second, lower source takes over after the upper four-to-six rows and keeps
// the lower/middle mountain broad enough for neighboring peak bodies to read
// as one field.
const PEAK_LOWER_BODY_SOURCE_DROP: Level = 18;
const PEAK_LOWER_BODY_SLOPE: Level = 4;
const PEAK_SADDLE_DEPTH: Level = 30;
// PeakRing ownership ends near the middle of a high slope.  Continuing that
// scalar slope through a neighboring overlay-free Mountain band prevents the
// stable coarse-cell boundary from becoming the visible cylindrical base of a
// peak.  Sixteen rows are enough to absorb the tallest canonical boundary
// contribution while still staying inside the immediately neighboring coarse
// mountain terrain.
const PEAK_OUTER_FEATHER_DEPTH: u32 = 16;
const PEAK_OUTER_FEATHER_MAXIMUM_STEP: Level = 9;
// The Frozen-to-saddle lower-body reservation is four columns wide. Keeping
// its complete spine and support outside this radius leaves enough horizontal
// run for the nine-level shoulder envelope to meet Patch 88's immutable crown
// without lowering the authored summit pin.
const INNER_PEAK_INGRESS_SUMMIT_CLEARANCE: u32 = 5;
const INNER_PEAK_INGRESS_FROZEN_PATCH: PatchId = PatchId(123);
const INNER_PEAK_INGRESS_PEAK_PATCH: PatchId = PatchId(88);
const INNER_PEAK_ROUTE_SADDLE_CEILING: Level = 218;
const INNER_PEAK_TRANSIT_OWNER: PatchId = PatchId(59);
const INNER_PEAK_TRANSIT_INGRESS: (PatchId, PatchId) = (PatchId(58), PatchId(59));
const INNER_PEAK_TRANSIT_EGRESS: (PatchId, PatchId) = (PatchId(36), PatchId(59));
// Patch 59's two existing sub-240 saddle components are separated by this
// single shoulder column. It is the smallest deterministic route-domain
// addition that joins the retained west approach; widening the generic
// route-ready mask would grant the route authority over the rest of the upper
// crown. The scalar projection still owns the bounded surrounding blend.
const INNER_PEAK_TRANSIT_WEST_NOTCH: HexCoord = HexCoord::from_axial(34, -78);
const INNER_PEAK_TRANSIT_WEST_NOTCH_RAW_MAX: Level = PEAK_VISUAL_WALL_THRESHOLD.saturating_add(8);
// The lowest 59/36 saddle tip at (51, -81) is visually healthy but cannot
// reach the remaining 36/19/38 route at any exact level. The complete reverse
// proof identifies the middle of the same authored boundary as the first
// broad, suffix-compatible handoff: it retains thirteen viable levels while
// avoiding the upper 234-level shoulder that already failed exact grading.
const INNER_PEAK_TRANSIT_EGRESS_ANCHOR: HexCoord = HexCoord::from_axial(43, -77);
// Both owner-side runway ends are authored at this shared datum. Level 217 is
// inside the complete reverse proof's suffix-compatible 213..=225 interval at
// the retained Patch-59/Patch-36 handoff. It also leaves the final interior
// step below the exact nine-level ceiling propagated from the nearby lower
// saddle; the exterior portal cells remain independently bounded alternatives.
const INNER_PEAK_TRANSIT_EGRESS_LEVEL_MIN: Level = 213;
const INNER_PEAK_TRANSIT_EGRESS_LEVEL_MAX: Level = 225;
const INNER_PEAK_TRANSIT_RUNWAY_ENDPOINT_LEVEL: Level =
    INNER_PEAK_ROUTE_SADDLE_CEILING.saturating_sub(1);
pub(super) const MASSIF_SUMMIT_MIN: Level = 330;
pub(super) const MASSIF_SUMMIT_MAX: Level = 350;
// The blended coarse field is intentionally low at the Massif boundary.  A
// second absolute profile prevents that low datum from keeping most of the
// interior below Crystal: it starts below ordinary boundary terrain and rises
// at the same nine-level maximum enforced by the final whole-body seam check.
// Taking the maximum of the relative and absolute profiles preserves relief
// while making the body broadly, not selectively, high.
const MASSIF_ABSOLUTE_BODY_BASE: Level = 48;
const MASSIF_ABSOLUTE_BODY_RISE_PER_HEX: Level = 9;
// The single tallest crest falls away at the same rate as the safe inward
// body. Offset summits then use the gentler support slope below, allowing them
// to form separate high shoulders instead of disappearing under one broad
// central cone.
const MASSIF_PRIMARY_SUMMIT_SUPPORT_SLOPE: Level = 9;
// Distance one also pays the regular four-level falloff, so five is the
// largest initial drop that keeps the crest-to-neighbour transition at the
// global nine-level terrain-step limit.
const MASSIF_BODY_NEAR_CREST_INITIAL_DROP: Level = 5;
const MASSIF_BODY_CREST_FALLOFF_PER_HEX: Level = 4;
// The summit sheds height slowly enough to read as one broad mountain body,
// rather than a narrow cone standing on an otherwise low Massif. Three levels
// per column also keeps off-axis lobes inside the radial reversal budget while
// leaving room for the shared fine-field relief.
const MASSIF_SUMMIT_SUPPORT_SLOPE: Level = 5;
const MASSIF_SUMMIT_EDGE_LIFT: Level = 24;
// The summit envelope controls the crown, while three lower and more distant
// sources pull broad shoulders out of the otherwise boundary-depth-concentric
// body. Their influence is capped by the true outer-boundary depth below, so
// irregularity cannot recreate a cliff at the visual-mask edge.
const MASSIF_SHOULDER_SUPPORT_SLOPE: Level = 3;
const MASSIF_SHOULDER_EDGE_RISE_PER_HEX: Level = 9;
// Shoulder sources may stand above the final local continuity cap at their
// authored pin so their three-level influence can continue inward. The final
// resolve still applies the universal nine-level outer envelope, and source
// selection below proves the capped source itself remains a real witness.
const MASSIF_SHOULDER_EDGE_LIFT: Level = 36;
const MASSIF_SHOULDER_MIN_DISTANCE: u32 = 12;
const MASSIF_SHOULDER_MAX_DISTANCE: u32 = 32;
pub(super) const MASSIF_SUMMIT_BODY_RADIUS: u32 = 14;
const MASSIF_DISTRIBUTED_SUMMIT_RADIUS: u32 = 32;
// Route construction preserves the complete tested summit transect. The
// scalar envelope continues beyond it, but generic routes cannot flatten the
// core into a high shelf.
const MASSIF_PROTECTED_BODY_RADIUS: u32 = MASSIF_SUMMIT_BODY_RADIUS;
// The Massif's physical foothills extend into neighboring Mountain ownership.
// A two-column feather kept semantic ownership correct but left too little
// horizontal room for a broadly high body to descend without either a wall or
// a low majority. Eight fine columns preserve the coarse biome identities
// while giving the scalar field a natural shoulder before it becomes inert.
const MASSIF_OUTER_FEATHER_DEPTH: u32 = 16;
const MASSIF_CONNECTOR_MINIMUM_TAPER_DEPTH: u32 = 3;
const MASSIF_CONNECTOR_BRIDGE_RISE_PER_HEX: Level = 5;
const FROZEN_PLATEAU_LEVEL: Level = 152;
const FROZEN_PLATEAU_MIN: Level = 151;
const FROZEN_PLATEAU_MAX: Level = 153;
const FROZEN_PLATEAU_HALO_DEPTH: u32 = 6;
const FROZEN_PLATEAU_MAXIMUM_STEP: Level = 8;
const CRYSTAL_EXIT_BLEND_DEPTH: u32 = 16;
const CRYSTAL_EXIT_MAXIMUM_STEP: Level = 9;

#[derive(Debug, Clone, PartialEq, Eq)]
struct MassifField {
    mask: BTreeSet<HexCoord>,
    semantic_owner_mask: BTreeSet<HexCoord>,
    connector_mask: BTreeSet<HexCoord>,
    connector_distance: BTreeMap<HexCoord, u32>,
    boundary_depth: BTreeMap<HexCoord, u32>,
    crest: HexCoord,
    summit: Level,
    summit_sources: BTreeMap<HexCoord, Level>,
    summit_support: BTreeMap<HexCoord, Level>,
    shoulder_sources: BTreeMap<HexCoord, Level>,
    shoulder_support: BTreeMap<HexCoord, Level>,
    summit_core: BTreeSet<HexCoord>,
    floor: Level,
}

fn massif_body_crest_cap(summit: Level, distance: u32) -> Level {
    if distance == 0 {
        summit
    } else {
        summit.saturating_sub(
            MASSIF_BODY_NEAR_CREST_INITIAL_DROP.saturating_add(
                i32::try_from(distance)
                    .unwrap_or(i32::MAX)
                    .saturating_mul(MASSIF_BODY_CREST_FALLOFF_PER_HEX),
            ),
        )
    }
}

fn massif_contour_variation(coord: HexCoord) -> Level {
    let phase = (i64::from(coord.x())
        .saturating_mul(31)
        .saturating_add(i64::from(coord.y()).saturating_mul(17)))
    .rem_euclid(5);
    match phase {
        0 => -1,
        3 => 1,
        _ => 0,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FrozenPlateauField {
    levels: BTreeMap<HexCoord, Level>,
    halo_distance: BTreeMap<HexCoord, u32>,
}

fn crystal_frozen_shell_transition_grades(
    shell_floors: &BTreeMap<HexCoord, Level>,
    frozen_levels: &BTreeMap<HexCoord, Level>,
) -> BTreeMap<HexCoord, Level> {
    frozen_levels
        .iter()
        .filter_map(|(coord, frozen_level)| {
            let required = shell_floors
                .iter()
                .map(|(source, floor)| {
                    floor.saturating_sub(
                        i32::try_from(source.distance(*coord))
                            .unwrap_or(i32::MAX)
                            .saturating_mul(FROZEN_PLATEAU_MAXIMUM_STEP),
                    )
                })
                .max()
                .unwrap_or(*frozen_level);
            (required > *frozen_level).then_some((*coord, required))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PeakFeatherContribution {
    target: Level,
    outer_depth: u32,
}

impl PeakFeatherContribution {
    fn resolve(self, baseline: Level) -> Level {
        let edge_cap = baseline.saturating_add(
            i32::try_from(self.outer_depth)
                .unwrap_or(i32::MAX)
                .saturating_mul(PEAK_OUTER_FEATHER_MAXIMUM_STEP),
        );
        baseline.max(self.target.min(edge_cap))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PeakFeatherField {
    contributions: BTreeMap<HexCoord, PeakFeatherContribution>,
    owners: BTreeMap<HexCoord, PatchId>,
    boundary_edges: BTreeSet<(HexCoord, HexCoord)>,
}

/// Final-world evidence for the two locked Grand V3 peak chains.
///
/// Route construction is allowed to cut narrow saddles and foothill ledges
/// through PeakRing ownership after the scalar field is built.  Keeping the
/// exact six patch masks and seeded summit pins separate from those routes lets
/// final validation prove that both high ridges survived without requiring
/// every coordinate in a PeakRing patch to remain high.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PeakRidgeAuthority {
    pub(super) components: Vec<PeakRidgeComponentAuthority>,
}

/// Ordered foundation evidence for one peak patch that must carry an Ordinary
/// route between two independently authored scenic saddles.
///
/// The unordered saddle swaths remain the visual contract. This authority
/// retains the exact low connector that highland composition used, its typed
/// boundary portals, and the four-row terrain reservation that competing route
/// phases must leave alone. It grants no mutation permission by itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OrderedPeakSaddleSpineAuthority {
    pub(super) owner: PatchId,
    pub(super) ingress_from: PatchId,
    pub(super) egress_to: PatchId,
    pub(super) ingress_portals: BTreeSet<(HexCoord, HexCoord)>,
    pub(super) centerline: Vec<HexCoord>,
    pub(super) egress_portals: BTreeSet<(HexCoord, HexCoord)>,
    pub(super) support_domain: BTreeSet<HexCoord>,
    /// Exact one-level route datum authored by the highland phase across the
    /// complete owner-side centerline. The portal coordinates remain reserved,
    /// while the route phase selects an exterior portal level within its own
    /// independently derived bounds.
    pub(super) authored_grades: BTreeMap<HexCoord, Level>,
}

impl OrderedPeakSaddleSpineAuthority {
    pub(super) fn reservation_coords(&self) -> BTreeSet<HexCoord> {
        let mut reservation = self.support_domain.clone();
        reservation.extend(
            self.ingress_portals
                .iter()
                .flat_map(|(from, to)| [*from, *to]),
        );
        reservation.extend(
            self.egress_portals
                .iter()
                .flat_map(|(from, to)| [*from, *to]),
        );
        reservation
    }

    pub(super) fn required_grade_coords(&self) -> BTreeSet<HexCoord> {
        self.centerline.iter().copied().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PeakRidgeComponentAuthority {
    pub(super) patch_masks: BTreeMap<PatchId, BTreeSet<HexCoord>>,
    /// Stable physical portions of one connected lower mountain body. These
    /// normally match semantic patch ownership; `borrowed_crown_cells` records
    /// the bounded exception used beside an immutable authored opening. Only
    /// the upper crown band is separated; neighboring lower slopes meet
    /// through the exact scenic saddle swaths below.
    pub(super) expected_peak_bodies: BTreeMap<PatchId, BTreeMap<HexCoord, Level>>,
    /// Bounded physical crown cells borrowed from the existing Mountain
    /// feather when an immutable authored opening makes a semantic peak body
    /// unable to retain its summit. Semantic patch masks and ownership do not
    /// change.
    pub(super) borrowed_crown_cells: BTreeMap<PatchId, BTreeSet<HexCoord>>,
    /// Flattened exact connected-chain profile retained for route-grade sealing.
    pub(super) expected_ridge_profile: BTreeMap<HexCoord, Level>,
    pub(super) expected_high_band: BTreeMap<HexCoord, Level>,
    pub(super) summit_pins: BTreeMap<HexCoord, Level>,
    /// Multi-column low terrain between every pair of touching authored peak
    /// patches. These swaths, rather than a single pin-to-pin sample, prove the
    /// mountains remain separate silhouettes below the visual-wall threshold.
    pub(super) expected_saddle_swaths: BTreeMap<(PatchId, PatchId), BTreeSet<HexCoord>>,
    /// Exact ordered low connectors retained only where a later Ordinary route
    /// must cross one intermediate scenic-saddle owner.
    pub(super) ordered_saddle_spines: BTreeMap<PatchId, OrderedPeakSaddleSpineAuthority>,
    /// Exact Patch-88 columns on the external Frozen-123/Peak-88 seam. The
    /// foundation joins this low ingress to the 88/58 scenic saddle before any
    /// Ordinary route is carved, so route construction never has to invent a
    /// shoulder beside an immutable upper crown.
    pub(super) expected_external_ingress_swaths: BTreeMap<(PatchId, PatchId), BTreeSet<HexCoord>>,
    /// Neighboring overlay-free Mountain columns that continue the physical
    /// lower slope beyond the stable PeakRing ownership boundary.
    pub(super) feather_owners: BTreeMap<HexCoord, PatchId>,
    /// Canonically oriented inner and outer feather edges.  Final validation
    /// measures every one, so a later terrain pass cannot recreate a cliff at
    /// either coarse ownership boundary.
    pub(super) feather_boundary_edges: BTreeSet<(HexCoord, HexCoord)>,
    /// Exact levels intentionally graded by the four authored peak routes.
    ///
    /// Foundation construction leaves this unsealed.  The schematic compiler
    /// seals it immediately after publishing those routes and before any
    /// generic connector is allowed to mutate terrain.  Final validation can
    /// consequently admit only the production-observed footprint, rather than
    /// every coordinate that happens to belong to a broad route corridor.
    pub(super) authorized_route_grades: Option<BTreeMap<HexCoord, Level>>,
    /// Exact ridge-profile coordinates superseded by the deliberately open
    /// waterfall core, cascade, or fall-air floor. These may contain the
    /// authored vertical drops which are invalid for an ordinary dry slope;
    /// the independent waterfall authority validates their final geometry.
    pub(super) authorized_waterfall_openings: Option<BTreeMap<HexCoord, Level>>,
}

/// Immutable geometry for Crystal's broad neighboring-biome enclosure.
///
/// This deliberately has no connected-ridge contract. Six broad scalar lobes
/// raise the surrounding biome bodies while the exact lower tunnel and upper
/// Frozen-Woods routes remain open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CrystalMantleAuthority {
    pub(super) crystal_center: HexCoord,
    /// Highest final natural surface inside the composite Crystal site.
    ///
    /// This includes the exact worked rib and its composite-only natural
    /// overburden. Coverage admission must use this datum rather than the
    /// lower standalone architecture maximum.
    pub(super) composite_crystal_top: Level,
    pub(super) uplift_core: BTreeSet<HexCoord>,
    /// Complete scalar-support footprint, including the low inner and outer
    /// feather where the uplift contribution becomes exactly inert.
    pub(super) support_footprint: BTreeSet<HexCoord>,
    pub(super) enclosure_band: BTreeSet<HexCoord>,
    pub(super) route_exclusion: BTreeSet<HexCoord>,
    pub(super) sector_pins: BTreeMap<u8, (HexCoord, Level)>,
    pub(super) opening_clearance: BTreeSet<HexCoord>,
    /// Exact shell columns that receive composite-only Stone/Snow overburden
    /// after the authored fragment is merged.
    pub(super) natural_shell_skin: BTreeSet<HexCoord>,
    /// Exact lower-aperture and summit-trail shell columns which retain their
    /// walkable surface and clearance. Grand ecology may recolour a natural cap.
    pub(super) exposed_shell_openings: BTreeSet<HexCoord>,
    /// Minimum natural surface beside each buried exterior shell column.
    ///
    /// Values follow the exact adjacent worked-stone profile: ordinary wall
    /// segments do not inherit the height of a distant pointed rib. The shared
    /// terrain projection applies these floors before any route compiler is
    /// allowed to grade the apron.
    pub(super) shell_concealment_floors: BTreeMap<HexCoord, Level>,
    /// Maximum natural surface beside each covered shell column.
    ///
    /// This is the lowest adjacent Stone/Snow shell top plus the bounded
    /// twelve-level natural rise. It prevents an overlapping Massif field from
    /// turning the concealment apron into a retaining wall facing Crystal.
    pub(super) shell_concealment_ceilings: BTreeMap<HexCoord, Level>,
    /// Enclosure-band columns whose typed Frozen-halo ceiling cannot rise
    /// above `composite_crystal_top`.
    pub(super) forced_low_frozen_halo: BTreeMap<HexCoord, Level>,
    /// Enclosure-band columns whose typed upper-exit ceiling cannot rise above
    /// `composite_crystal_top`.
    pub(super) forced_low_exit_blend: BTreeMap<HexCoord, Level>,
    pub(super) expected_uplift_caps: Option<BTreeMap<HexCoord, Level>>,
}

impl CrystalMantleAuthority {
    /// Natural columns immediately outside Crystal's buried shell.
    ///
    /// Start from the typed composite shell skin rather than the whole radius-32
    /// disk: authored lower and upper apertures remain governed exclusively by
    /// `opening_clearance`, while every other exterior contact receives enough
    /// natural terrain to hide the worked-stone side wall.
    pub(super) fn shell_concealment_apron(&self) -> BTreeSet<HexCoord> {
        self.shell_concealment_floors.keys().copied().collect()
    }

    /// Terrain which later route graders must leave to the shell/Frozen
    /// transition authority. Two rows beyond the apron absorb the largest
    /// current 19-level overlap at Frozen's stricter eight-level step budget.
    pub(super) fn shell_concealment_route_reservation(&self) -> BTreeSet<HexCoord> {
        self.shell_concealment_floors
            .keys()
            .flat_map(|coord| coord.within_radius(CRYSTAL_SHELL_FROZEN_TRANSITION_DEPTH))
            .filter(|coord| !self.opening_clearance.contains(coord))
            .collect()
    }

    /// Geometric enclosure columns which can legally stand above the final
    /// composite Crystal top. Only immutable, pre-route scalar ceilings may
    /// remove a coordinate from this admission domain.
    pub(super) fn attainable_enclosure_band(&self) -> BTreeSet<HexCoord> {
        self.enclosure_band
            .iter()
            .copied()
            .filter(|coord| {
                !self.forced_low_frozen_halo.contains_key(coord)
                    && !self.forced_low_exit_blend.contains_key(coord)
            })
            .collect()
    }

    /// Captures the live Crystal enclosure columns which a later shared
    /// terrain compiler must keep above the composite site.
    ///
    /// The returned values are deliberately only the least legal floor, not
    /// the current terrain caps. A route may therefore grade and publish these
    /// columns while preserving the already-admitted global, per-sector, and
    /// radial coverage witnesses. Taking this snapshot from the live volume
    /// also avoids assuming that the raw `uplift_core` is still the complete
    /// witness set after earlier authorized construction phases.
    pub(super) fn transit_minimums(
        &self,
        stage: &str,
        level_at: impl Fn(HexCoord) -> Option<Level>,
    ) -> Result<BTreeMap<HexCoord, Level>, String> {
        let minimum = self.composite_crystal_top.saturating_add(1);
        let minimums = self
            .attainable_enclosure_band()
            .into_iter()
            .filter(|coord| level_at(*coord).is_some_and(|level| level >= minimum))
            .map(|coord| (coord, minimum))
            .collect::<BTreeMap<_, _>>();
        self.validate_attainable_coverage(stage, |coord| {
            Some(
                minimums
                    .get(&coord)
                    .copied()
                    .unwrap_or(self.composite_crystal_top),
            )
        })?;
        Ok(minimums)
    }

    /// Directed inside-to-outside edges at the mantle's true radial exterior.
    ///
    /// The live transit floors above protect the enclosure's high witnesses,
    /// but a route can carry that grade through the inert outer feather. If the
    /// shoulder compiler sees only Peak terrain, the route can then terminate
    /// at this boundary as a retaining cliff. The exact enclosed Crystal is a
    /// separately validated hole and never grants mutation authority here.
    pub(super) fn transit_transition_edges(
        &self,
        enclosed_crystal: &BTreeSet<HexCoord>,
    ) -> BTreeSet<(HexCoord, HexCoord)> {
        self.support_footprint
            .iter()
            .copied()
            .filter(|inside| {
                self.crystal_center.distance(*inside) == CRYSTAL_ENCLOSURE_OUTER_RADIUS
            })
            .flat_map(|inside| {
                inside
                    .neighbors()
                    .into_iter()
                    .filter(|outside| {
                        !self.support_footprint.contains(outside)
                            && !enclosed_crystal.contains(outside)
                            && self.crystal_center.distance(*outside)
                                > CRYSTAL_ENCLOSURE_OUTER_RADIUS
                    })
                    .map(move |outside| (inside, outside))
            })
            .collect()
    }

    /// Proves that an actual shared route crossing stays within its admitted
    /// transition budget. A route-adjacent scenic column is not itself a
    /// crossing and may form a deliberate cliff.
    pub(super) fn validate_transit_transition(
        &self,
        stage: &str,
        enclosed_crystal: &BTreeSet<HexCoord>,
        touched: &BTreeSet<HexCoord>,
        maximum_step: Level,
        level_at: impl Fn(HexCoord) -> Option<Level>,
    ) -> Result<(), String> {
        let maximum_step = maximum_step.unsigned_abs();
        for (inside, outside) in self.transit_transition_edges(enclosed_crystal) {
            if !touched.contains(&inside) || !touched.contains(&outside) {
                continue;
            }
            let Some(inside_level) = level_at(inside) else {
                return Err(format!(
                    "{stage} shared Crystal transit lost its mantle-side surface at {inside:?}"
                ));
            };
            let Some(outside_level) = level_at(outside) else {
                continue;
            };
            if inside_level.abs_diff(outside_level) > maximum_step {
                return Err(format!(
                    "{stage} shared Crystal transit ends at an ungraded mantle edge {inside:?}@{inside_level} -> {outside:?}@{outside_level}; maximum step {maximum_step}"
                ));
            }
        }
        Ok(())
    }

    pub(super) fn validate_attainable_coverage(
        &self,
        stage: &str,
        level_at: impl Fn(HexCoord) -> Option<Level>,
    ) -> Result<(), String> {
        let attainable = self.attainable_enclosure_band();
        if attainable.is_empty() {
            return Err(format!(
                "{stage} Crystal enclosure has no attainable neighboring-biome band"
            ));
        }
        let high = attainable
            .iter()
            .copied()
            .filter(|coord| {
                level_at(*coord).is_some_and(|level| level > self.composite_crystal_top)
            })
            .collect::<BTreeSet<_>>();
        if high.len().saturating_mul(5) < attainable.len().saturating_mul(3) {
            return Err(format!(
                "{stage} Crystal enclosure retained only {} high columns across {} attainable neighboring-biome columns above composite top {}",
                high.len(),
                attainable.len(),
                self.composite_crystal_top,
            ));
        }
        let mut failures = Vec::new();
        for sector in 0..6 {
            let sector_coords = attainable
                .iter()
                .copied()
                .filter(|coord| enclosure_sector(self.crystal_center, *coord) == sector)
                .collect::<BTreeSet<_>>();
            let sector_band = sector_coords.len();
            let available_radii = sector_coords
                .iter()
                .map(|coord| self.crystal_center.distance(*coord))
                .collect::<BTreeSet<_>>();
            let sector_high = high
                .iter()
                .copied()
                .filter(|coord| enclosure_sector(self.crystal_center, *coord) == sector)
                .collect::<BTreeSet<_>>();
            let represented_radii = sector_high
                .iter()
                .map(|coord| self.crystal_center.distance(*coord))
                .collect::<BTreeSet<_>>();
            let required = sector_band.saturating_add(2) / 3;
            let required_radial_depth =
                crystal_enclosure_required_radial_depth(available_radii.len());
            if sector_high.len() < required || represented_radii.len() < required_radial_depth {
                failures.push(format!(
                    "sector {sector}: high={}/{sector_band} required={required}, radial-depth={}/{} required={required_radial_depth}",
                    sector_high.len(),
                    represented_radii.len(),
                    available_radii.len(),
                ));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "{stage} Crystal enclosure lacks broad attainable coverage above composite top {}: {}",
                self.composite_crystal_top,
                failures.join("; "),
            ))
        }
    }
}

/// Immutable ownership evidence for the connected visual Massif field.
///
/// Crystal's exact site may split the semantic Massif masks for some generated
/// schematics. The scalar terrain field is allowed to bridge those pieces only
/// through overlay-free Mountain columns, without transferring their stable
/// biome ownership. Capturing both masks and every connector owner here lets the
/// final-world validator distinguish that visual projection from semantic
/// ownership instead of silently weakening either contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MassifVisualAuthority {
    pub(super) visual_mask: BTreeSet<HexCoord>,
    pub(super) semantic_owner_mask: BTreeSet<HexCoord>,
    pub(super) connector_owners: BTreeMap<HexCoord, PatchId>,
    /// Eight overlay-free Mountain rows that let the physical summit influence
    /// decay before the true visual-mask edge. These never change stable biome
    /// ownership and are distinct from the narrow component connector.
    pub(super) feather_owners: BTreeMap<HexCoord, PatchId>,
}

impl MassifVisualAuthority {
    /// Fine-grid body that owns the Massif silhouette before the outer
    /// Mountain feather is applied.
    ///
    /// Ordinary access may be authored inside this body, but it must not turn
    /// the feather into a sequence of independent notches.  Keeping this
    /// projection beside the visual mask gives the route compiler a stable
    /// place to build one internal contour after it has crossed the true outer
    /// seam once.
    pub(super) fn connected_core(&self) -> BTreeSet<HexCoord> {
        self.semantic_owner_mask
            .iter()
            .copied()
            .chain(self.connector_owners.keys().copied())
            .collect()
    }

    /// Exact inner edge of the final visual Massif mask.
    ///
    /// This is deliberately narrower than `route_taper_avoidance`: the latter
    /// also protects Crystal's internal support transition.  An authored
    /// Massif portal is allowed to cross this outer seam exactly once and is
    /// never allowed to use the Crystal seam as an alternative entrance.
    pub(super) fn outer_seam(&self) -> BTreeSet<HexCoord> {
        self.visual_mask
            .iter()
            .copied()
            .filter(|coord| {
                coord
                    .neighbors()
                    .into_iter()
                    .any(|neighbor| !self.visual_mask.contains(&neighbor))
            })
            .collect()
    }

    /// Compact two-sided band around the true outer seam.
    pub(super) fn outer_taper_band(&self) -> BTreeSet<HexCoord> {
        self.outer_seam()
            .into_iter()
            .flat_map(|coord| coord.within_radius(1))
            .collect()
    }

    /// Keep route graders clear of the Massif's true outer feather.
    ///
    /// Crystal's scalar support boundary is deliberately *not* another seam
    /// inside semantic Massif terrain. In the composite world the continuous
    /// Massif field owns that overlap; preserving the derived radius-82 edge
    /// as a second exact boundary split the body into lobes and recreated the
    /// retaining wall this field exists to avoid. The six inner Crystal screen
    /// lobes remain independently sealed by `CrystalMantleAuthority`.
    pub(super) fn route_taper_exclusion(
        &self,
        highland_screen: &BTreeSet<HexCoord>,
        opening_clearance: &BTreeSet<HexCoord>,
    ) -> BTreeSet<HexCoord> {
        let massif_outer_seam = self
            .visual_mask
            .iter()
            .copied()
            .filter(|coord| {
                coord
                    .neighbors()
                    .into_iter()
                    .any(|neighbor| !self.visual_mask.contains(&neighbor))
            })
            .filter(|coord| highland_screen.contains(coord))
            .collect::<BTreeSet<_>>();
        massif_outer_seam
            .into_iter()
            .filter(|coord| !opening_clearance.contains(coord))
            .collect()
    }

    /// Natural columns that generic route grading must avoid completely.
    ///
    /// Required authored routes are resolved before this authority is applied.
    /// A later generic connector has no permission to use the Massif edge as a
    /// shortcut: doing so turned isolated edge cells into conspicuous towers.
    /// If a future schematic genuinely requires a crossing, it needs a
    /// separately authored, one-level portal rather than implicit regrading.
    pub(super) fn route_taper_avoidance(
        &self,
        _highland_screen: &BTreeSet<HexCoord>,
        opening_clearance: &BTreeSet<HexCoord>,
    ) -> BTreeSet<HexCoord> {
        let massif_outer_seam = self
            .visual_mask
            .iter()
            .copied()
            .filter(|coord| {
                coord
                    .neighbors()
                    .into_iter()
                    .any(|neighbor| !self.visual_mask.contains(&neighbor))
            })
            .collect::<BTreeSet<_>>();
        massif_outer_seam
            .into_iter()
            .flat_map(|coord| coord.within_radius(1))
            .filter(|coord| !opening_clearance.contains(coord))
            .collect()
    }

    /// Search-only exclusion used while selecting the sole exterior Massif
    /// portal.
    ///
    /// The composite Crystal/Massif overlap is mutable continuous terrain, but
    /// it is still the wrong place to *locate* the exterior portal. Keeping its
    /// derived boundary out of that candidate search preserves stable portal
    /// selection without restoring exact cap or taper authority to the ring.
    pub(super) fn portal_candidate_avoidance(
        &self,
        highland_support: &BTreeSet<HexCoord>,
        opening_clearance: &BTreeSet<HexCoord>,
    ) -> BTreeSet<HexCoord> {
        let crystal_outer_seam_inside_massif = highland_support
            .iter()
            .copied()
            .filter(|coord| self.visual_mask.contains(coord))
            .filter(|coord| {
                coord
                    .neighbors()
                    .into_iter()
                    .any(|neighbor| !highland_support.contains(&neighbor))
            });
        self.outer_seam()
            .into_iter()
            .chain(crystal_outer_seam_inside_massif)
            .flat_map(|coord| coord.within_radius(1))
            .filter(|coord| !opening_clearance.contains(coord))
            .collect()
    }
}

impl MassifField {
    fn resolve(&self, coord: HexCoord, baseline: Level) -> Level {
        if !self.mask.contains(&coord) {
            return baseline;
        }
        let boundary_depth = self.boundary_depth.get(&coord).copied().unwrap_or_default();
        let crest_depth = self
            .boundary_depth
            .get(&self.crest)
            .copied()
            .unwrap_or_default();
        // One absolute inward profile prevents fine-relief differences in the
        // blended coarse field from being added to the Massif rise.  Adding
        // `baseline + rise` produced fourteen-level risers when a relief edge
        // and a boundary-depth edge aligned.  The absolute profile still
        // preserves the successful terraced flanks, while `max(baseline)`
        // leaves the outermost row exactly on the continuous world surface.
        let absolute_body = MASSIF_ABSOLUTE_BODY_BASE.saturating_add(
            i32::try_from(boundary_depth)
                .unwrap_or(i32::MAX)
                .saturating_mul(MASSIF_ABSOLUTE_BODY_RISE_PER_HEX),
        );
        let inward_envelope = self.summit.saturating_sub(
            i32::try_from(crest_depth.saturating_sub(boundary_depth))
                .unwrap_or(i32::MAX)
                .saturating_mul(MASSIF_ABSOLUTE_BODY_RISE_PER_HEX),
        );
        let body = absolute_body.min(inward_envelope).max(baseline);
        // Boundary-depth plateaus can contain many equally deepest columns.
        // Reserve the final levels for the irregular summit envelopes and
        // keep the underlying body descending gently away from the crest, so
        // the terraced formation cannot terminate in a clipped mesa.
        let body = body.min(massif_body_crest_cap(
            self.summit,
            self.crest.distance(coord),
        ));
        let mut propagated_support = self
            .summit_support
            .get(&coord)
            .copied()
            .unwrap_or(self.floor)
            .max(baseline);
        let primary_support = self.summit.saturating_sub(
            i32::try_from(self.crest.distance(coord))
                .unwrap_or(i32::MAX)
                .saturating_mul(MASSIF_PRIMARY_SUMMIT_SUPPORT_SLOPE),
        );
        // A pure hex-distance cone can hold the same level for a long run when
        // an off-axis source is viewed along a canonical ray. Break only those
        // offset contours by one level; the central nine-level envelope and
        // exact source pins remain untouched, preserving the local-step bound.
        if propagated_support > primary_support
            && !self.summit_sources.contains_key(&coord)
            && !self.connector_distance.contains_key(&coord)
            && propagated_support > baseline
        {
            propagated_support = propagated_support.saturating_add(massif_contour_variation(coord));
        }
        // The inward-rise field is the safe outer-edge envelope, not the final
        // crown silhouette. Taking `min(body, support)` everywhere made its
        // nine-level central cone cap every three-level offset summit and
        // silently erased the authored Massif group. Let the propagated
        // multi-summit envelope override only that inward cone while retaining
        // the absolute boundary-depth cap; the outer body still cannot rise
        // faster than nine levels per column.
        let propagated_crown = propagated_support
            .min(absolute_body.saturating_add(MASSIF_SUMMIT_EDGE_LIFT))
            .max(baseline);
        let crown_resolved = body.max(propagated_crown);
        let shoulder_edge_cap = MASSIF_ABSOLUTE_BODY_BASE
            .saturating_add(
                i32::try_from(boundary_depth)
                    .unwrap_or(i32::MAX)
                    .saturating_mul(MASSIF_SHOULDER_EDGE_RISE_PER_HEX),
            )
            .saturating_add(MASSIF_SHOULDER_EDGE_LIFT);
        let mut irregular_shoulder = self
            .shoulder_support
            .get(&coord)
            .copied()
            .unwrap_or(self.floor)
            .min(shoulder_edge_cap)
            .max(baseline);
        if !self.shoulder_sources.contains_key(&coord)
            && !self.connector_distance.contains_key(&coord)
            && irregular_shoulder > baseline
        {
            irregular_shoulder = irregular_shoulder
                .saturating_add(massif_contour_variation(coord))
                .min(shoulder_edge_cap)
                .max(baseline);
        }
        // Distant sources stay below the crown. Their maximum breaks the
        // regular mid-body contour without widening the near-maximum crest or
        // overriding the outer taper.
        let resolved = crown_resolved.max(irregular_shoulder);
        let connector_bridge = self
            .connector_distance
            .get(&coord)
            .copied()
            .map(|distance| {
                let lateral_depth = MASSIF_OUTER_FEATHER_DEPTH
                    .saturating_add(1)
                    .saturating_sub(distance);
                let bridge_depth = boundary_depth.min(lateral_depth);
                baseline.saturating_add(
                    i32::try_from(bridge_depth)
                        .unwrap_or(i32::MAX)
                        .saturating_mul(MASSIF_CONNECTOR_BRIDGE_RISE_PER_HEX),
                )
            });
        let combined = resolved.max(connector_bridge.unwrap_or(baseline));
        let combined = if coord == self.crest {
            combined
        } else {
            combined.min(self.summit.saturating_sub(1))
        };
        // The authored visual mask supplies the Massif's initial physical
        // shoulder regardless of semantic biome ownership. The final graph
        // projection may continue its grade across connected dry terrain, but
        // this mask's exact boundary depth remains a safe local ceiling
        // against each column's real blended datum.  The low absolute origin
        // is intentionally below every canonical mountain datum: at the true
        // edge this cap is exactly `baseline`, while deeper rows retain enough
        // room for the distributed summit and shoulder sources.
        let outer_cap = MASSIF_ABSOLUTE_BODY_BASE.saturating_add(
            i32::try_from(boundary_depth)
                .unwrap_or(i32::MAX)
                .saturating_mul(MASSIF_ABSOLUTE_BODY_RISE_PER_HEX),
        );
        combined.min(outer_cap.max(baseline))
    }
}

/// Exact deterministic highland corrections applied over the shared scalar base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GrandHighlandField {
    peak_levels: BTreeMap<HexCoord, Level>,
    peak_feather: BTreeMap<HexCoord, PeakFeatherContribution>,
    peak_authority: PeakRidgeAuthority,
    massif: MassifField,
    massif_visual_authority: MassifVisualAuthority,
    frozen_plateau: FrozenPlateauField,
    crystal_exit_ceiling: BTreeMap<HexCoord, Level>,
    crystal_mantle: BTreeMap<HexCoord, Level>,
    crystal_mantle_edge_depth: BTreeMap<HexCoord, u32>,
    crystal_mantle_authority: CrystalMantleAuthority,
    combined_surface_projection: BTreeMap<HexCoord, Level>,
    #[cfg(test)]
    crystal_center: HexCoord,
    crystal_mask: BTreeSet<HexCoord>,
    #[cfg(test)]
    crystal_mantle_exit_clearance: BTreeSet<HexCoord>,
}

struct OrderedPeakSaddleProjectionContract {
    reservation_ceilings: BTreeMap<HexCoord, Level>,
    authored_grades: BTreeMap<HexCoord, Level>,
}

fn ordered_peak_saddle_projection_contract(
    authority: &PeakRidgeAuthority,
) -> Result<OrderedPeakSaddleProjectionContract, V3GenerationError> {
    let ordered = authority
        .components
        .iter()
        .flat_map(|component| {
            component
                .ordered_saddle_spines
                .values()
                .map(move |spine| (component, spine))
        })
        .collect::<Vec<_>>();
    if ordered.len() != 1 {
        return Err(contract(format!(
            "Grand V3 highland projection requires exactly one ordered peak-transit spine, found {}",
            ordered.len()
        )));
    }
    let (component, spine) = ordered[0];
    if spine.owner != INNER_PEAK_TRANSIT_OWNER
        || spine.ingress_from != INNER_PEAK_TRANSIT_INGRESS.0
        || spine.egress_to != INNER_PEAK_TRANSIT_EGRESS.0
    {
        return Err(contract(format!(
            "Grand V3 highland projection received mistyped ordered transit {} -> {} -> {}",
            spine.ingress_from.0, spine.owner.0, spine.egress_to.0
        )));
    }
    let reservation = spine.reservation_coords();
    if reservation.is_empty() {
        return Err(contract(
            "Grand V3 ordered peak-transit reservation is empty",
        ));
    }
    let required_grades = spine.required_grade_coords();
    let authored_grade_coords = spine
        .authored_grades
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    let ingress_level = spine
        .centerline
        .first()
        .and_then(|coord| spine.authored_grades.get(coord))
        .copied();
    let egress_level = spine
        .centerline
        .last()
        .and_then(|coord| spine.authored_grades.get(coord))
        .copied();
    if authored_grade_coords != required_grades
        || authored_grade_coords
            .iter()
            .any(|coord| !reservation.contains(coord))
        || spine.centerline.windows(2).any(|pair| {
            spine.authored_grades[&pair[0]].abs_diff(spine.authored_grades[&pair[1]]) > 1
        })
        || spine.authored_grades.values().any(|level| {
            *level < INNER_PEAK_TRANSIT_RUNWAY_ENDPOINT_LEVEL.saturating_sub(PEAK_SADDLE_DEPTH)
                || *level > INNER_PEAK_ROUTE_SADDLE_CEILING
        })
        || ingress_level != Some(INNER_PEAK_TRANSIT_RUNWAY_ENDPOINT_LEVEL)
        || egress_level != Some(INNER_PEAK_TRANSIT_RUNWAY_ENDPOINT_LEVEL)
        || egress_level.is_none_or(|level| {
            !(INNER_PEAK_TRANSIT_EGRESS_LEVEL_MIN..=INNER_PEAK_TRANSIT_EGRESS_LEVEL_MAX)
                .contains(&level)
        })
    {
        return Err(contract(format!(
            "Grand V3 ordered peak-transit grade contract is incomplete or over-steep: required={}, authored={}",
            required_grades.len(),
            authored_grade_coords.len()
        )));
    }
    let mut reservation_ceilings = BTreeMap::new();
    for coord in reservation {
        let raw_ceiling = component
            .expected_ridge_profile
            .get(&coord)
            .copied()
            .ok_or_else(|| {
                contract(format!(
                    "Grand V3 ordered peak-transit reservation {coord:?} has no authored peak-profile ceiling"
                ))
            })?;
        let is_authored_west_notch = spine.authored_grades.contains_key(&coord)
            && ordered_peak_saddle_is_bounded_west_notch(
                coord,
                raw_ceiling,
                &component.expected_ridge_profile,
            );
        if raw_ceiling >= PEAK_VISUAL_WALL_THRESHOLD && !is_authored_west_notch {
            return Err(contract(format!(
                "Grand V3 ordered peak-transit reservation {coord:?} entered the upper crown at {raw_ceiling}"
            )));
        }
        // The centerline may need to rise above a naturally deep scalar-field
        // pocket to remain a one-level Ordinary route. Permit exactly the
        // nine-level lower envelope required by the authored grade, while
        // retaining the existing low profile everywhere it is already higher.
        let required_support = spine
            .authored_grades
            .iter()
            .map(|(source, level)| {
                level.saturating_sub(
                    i32::try_from(source.distance(coord))
                        .unwrap_or(i32::MAX)
                        .saturating_mul(PEAK_OUTER_FEATHER_MAXIMUM_STEP),
                )
            })
            .max()
            .unwrap_or(Level::MIN);
        // The exact west notch is deliberately lowered out of the raw upper
        // shoulder. Every other reservation cell retains the old requirement
        // that it was already part of the low scenic saddle.
        let ceiling = raw_ceiling
            .max(required_support)
            .min(PEAK_VISUAL_WALL_THRESHOLD.saturating_sub(1));
        if let Some(previous) = reservation_ceilings.insert(coord, ceiling) {
            if previous != ceiling {
                return Err(contract(format!(
                    "Grand V3 ordered peak-transit reservation {coord:?} has conflicting ceilings {previous} and {ceiling}"
                )));
            }
        }
    }
    Ok(OrderedPeakSaddleProjectionContract {
        reservation_ceilings,
        authored_grades: spine.authored_grades.clone(),
    })
}

impl GrandHighlandField {
    pub(super) fn build(
        plan: &SchematicPlanV1,
        layout: &ResolvedLayoutPlan,
        profile: V3GrandV3BasicTerrainProfile,
    ) -> Result<Self, V3GenerationError> {
        let crystal = crystal_context(plan, layout, profile)?;

        let peak_cells = landmark_cells(plan, layout, |cell| {
            cell.facts.overlays.contains(&SchematicFeature::PeakRing)
        })?;
        let peak_components = schematic_components(&peak_cells)?;
        if peak_cells.len() != 12
            || peak_components.len() != 2
            || peak_components.iter().any(|component| component.len() != 6)
        {
            return Err(contract(format!(
                "Grand V3 highlands require the locked two six-cell peak chains; found {} cells in component sizes {:?}",
                peak_cells.len(),
                peak_components
                    .iter()
                    .map(BTreeSet::len)
                    .collect::<Vec<_>>()
            )));
        }
        let crystal_exit_ceiling =
            build_crystal_exit_ceiling(&crystal.exit_clearance, &layout.footprint);
        let massif_cells = landmark_cells(plan, layout, |cell| {
            cell.facts.surface == SurfaceKind::Land && cell.facts.landform == LandformKind::Massif
        })?;
        let massif_owner_mask = union_masks(layout, massif_cells.values().map(|cell| cell.patch))?;
        let massif_visual_authority =
            build_massif_visual_authority(plan, layout, &massif_owner_mask, &crystal.mask)?;
        let massif_crest_owner_mask = union_masks(
            layout,
            massif_cells.iter().filter_map(|(schematic, cell)| {
                schematic
                    .checked_distance(crystal.schematic)
                    .is_some_and(|distance| distance >= 2)
                    .then_some(cell.patch)
            }),
        )?;
        let massif = build_massif_field(
            &massif_visual_authority.visual_mask,
            &massif_visual_authority.semantic_owner_mask,
            &massif_visual_authority
                .connector_owners
                .keys()
                .copied()
                .collect(),
            &massif_crest_owner_mask,
            &crystal.mask,
            profile,
            plan.provenance.world_seed,
        )?;
        // The Massif owns its separately validated broad Mountain feather.
        // Subtract that exact visual authority before deriving each peak-chain
        // feather, so overlapping scalar systems cannot fight over one stable
        // Mountain column. Peak slopes still meet the Massif directly where
        // the two highland bodies touch; everywhere else they taper through
        // their own overlay-free Mountain band.
        let mut mountain_feather_owners = overlay_free_mountain_owners(plan, layout)?;
        mountain_feather_owners.retain(|coord, _| !massif.mask.contains(coord));
        let frozen_plateau =
            build_frozen_plateau(plan, layout, profile, plan.provenance.world_seed)?;
        let (peak_levels, peak_bodies, peak_feathers, peak_external_ingresses, peak_summit_pins) =
            build_peak_field(
                &peak_cells,
                &peak_components,
                layout,
                profile,
                plan.provenance.world_seed,
                &frozen_plateau,
                &crystal.exit_clearance,
                &crystal_exit_ceiling,
                &mountain_feather_owners,
                &massif.mask,
            )?;
        let peak_authority = build_peak_ridge_authority(
            layout,
            &peak_cells,
            &peak_components,
            &peak_levels,
            &peak_bodies,
            &peak_feathers,
            &peak_external_ingresses,
            &peak_summit_pins,
            &frozen_plateau,
            &crystal_exit_ceiling,
        )?;
        let peak_feather = peak_feathers
            .iter()
            .flat_map(|field| {
                field
                    .contributions
                    .iter()
                    .map(|(coord, contribution)| (*coord, *contribution))
            })
            .collect::<BTreeMap<_, _>>();
        let (crystal_mantle, crystal_mantle_edge_depth, crystal_mantle_authority) =
            build_crystal_mantle(
                plan,
                layout,
                profile,
                &crystal,
                &frozen_plateau,
                &crystal_exit_ceiling,
            )?;
        if crystal_mantle
            .keys()
            .any(|coord| crystal.mask.contains(coord))
        {
            return Err(contract(
                "Grand V3 Crystal mantle entered the exact radius-32 authored site",
            ));
        }

        Ok(Self {
            peak_levels,
            peak_feather,
            peak_authority,
            massif,
            massif_visual_authority,
            frozen_plateau,
            crystal_exit_ceiling,
            crystal_mantle,
            crystal_mantle_edge_depth,
            crystal_mantle_authority,
            combined_surface_projection: BTreeMap::new(),
            #[cfg(test)]
            crystal_center: crystal.center,
            crystal_mask: crystal.mask,
            #[cfg(test)]
            crystal_mantle_exit_clearance: crystal.exit_clearance,
        })
    }

    pub(super) fn resolve_surface_level(
        &self,
        cell: &CellPlan,
        coord: HexCoord,
        baseline: Level,
    ) -> Level {
        let shaped = if self.massif.mask.contains(&coord) {
            // A visual-only connector can traverse an ordinary Mountain patch.
            // Field membership, rather than semantic ownership, is the exact
            // authority for applying its continuous scalar surface.
            self.massif.resolve(coord, baseline)
        } else if let Some(level) = self.peak_levels.get(&coord).copied() {
            level
        } else if let Some(contribution) = self.peak_feather.get(&coord).copied() {
            contribution.resolve(baseline)
        } else {
            match cell.facts.landform {
                LandformKind::SharpPeak => baseline,
                LandformKind::None
                | LandformKind::Island
                | LandformKind::Beach
                | LandformKind::Shore
                | LandformKind::Valley
                | LandformKind::Plateau
                | LandformKind::Hill
                | LandformKind::Mountain
                | LandformKind::Massif => baseline,
            }
        };
        let resolved = self
            .crystal_mantle
            .get(&coord)
            .copied()
            .map_or(shaped, |uplift| {
                // The absolute target supplies the broad shoulder, while the
                // true distance from every exterior or opening edge limits how
                // quickly it can depart from the already blended base. The
                // enclosed Crystal disk is deliberately not such an edge; its
                // typed shell/aperture authorities govern that inner contact.
                // At a true edge depth is zero, making the mantle exactly inert
                // instead of ending as a retaining wall.
                let edge_depth = self
                    .crystal_mantle_edge_depth
                    .get(&coord)
                    .copied()
                    .unwrap_or_default();
                // Resolve the mantle from the shared fine-terrain datum, then
                // union it with the independently graded highland field.
                // Applying the mantle rise to `shaped` compounded its
                // eight-level taper with the Massif's nine-level taper and
                // produced repeatable seventeen-level terrace seams.
                shaped.max(edge_blended_uplift(baseline, uplift, edge_depth))
            });
        let resolved = if let Some(level) = self.frozen_plateau.levels.get(&coord) {
            *level
        } else {
            self.frozen_plateau
                .halo_distance
                .get(&coord)
                .copied()
                .map_or(resolved, |distance| {
                    let ceiling = FROZEN_PLATEAU_MAX.saturating_add(
                        i32::try_from(distance)
                            .unwrap_or(i32::MAX)
                            .saturating_mul(FROZEN_PLATEAU_MAXIMUM_STEP),
                    );
                    resolved.min(ceiling)
                })
        };
        let resolved = self
            .crystal_exit_ceiling
            .get(&coord)
            .copied()
            .map_or(resolved, |ceiling| resolved.min(ceiling));
        self.combined_surface_projection
            .get(&coord)
            .copied()
            .unwrap_or(resolved)
    }

    /// Projects the fully composed Massif foundation, after fine relief,
    /// Crystal mantle, Frozen plateau, exit ceilings, and peak adjacency have
    /// all resolved, onto one deterministic nine-Lipschitz surface.
    pub(super) fn project_combined_surface(
        &mut self,
        raw_surface: &BTreeMap<HexCoord, Level>,
        baseline_surface: &BTreeMap<HexCoord, Level>,
        footprint: &BTreeSet<HexCoord>,
        projectable_dry_land: &BTreeSet<HexCoord>,
    ) -> Result<(), V3GenerationError> {
        const MAXIMUM_STEP: Level = 9;
        if !self.combined_surface_projection.is_empty() {
            return Err(contract(
                "Grand V3 combined highland projection may run exactly once",
            ));
        }
        let ordered_transit = ordered_peak_saddle_projection_contract(&self.peak_authority)?;
        let mut variable = self.massif_visual_authority.visual_mask.clone();
        variable.extend(
            self.crystal_mantle_authority
                .support_footprint
                .iter()
                .copied(),
        );
        variable.extend(self.peak_levels.keys().copied());
        variable.extend(self.peak_feather.keys().copied());
        variable.extend(self.frozen_plateau.levels.keys().copied());
        variable.extend(self.frozen_plateau.halo_distance.keys().copied());
        variable.extend(self.crystal_exit_ceiling.keys().copied());
        variable.retain(|coord| {
            footprint.contains(coord)
                && raw_surface.contains_key(coord)
                && projectable_dry_land.contains(coord)
                && !self.crystal_mask.contains(coord)
        });
        // The authored highland masks can end while their globally blended
        // baseline is still high. A one-row fixed exterior then asks the
        // projection to reconcile that high shelf with nearby immutable
        // water in too little horizontal distance. Give the final surface a
        // connected dry-land feather; ownership and all semantic facts remain
        // unchanged. Following the component to its actual water/boundary edge
        // is necessary: stopping at an arbitrary radius can leave the last
        // fixed dry row itself more than nine levels above adjacent water.
        let mut frontier = variable.iter().copied().collect::<VecDeque<_>>();
        while let Some(coord) = frontier.pop_front() {
            for neighbor in coord.neighbors() {
                if footprint.contains(&neighbor)
                    && raw_surface.contains_key(&neighbor)
                    && projectable_dry_land.contains(&neighbor)
                    && !self.crystal_mask.contains(&neighbor)
                    && variable.insert(neighbor)
                {
                    frontier.push_back(neighbor);
                }
            }
        }
        let crystal_shell_apron = self.crystal_mantle_authority.shell_concealment_apron();
        let crystal_shell_route_reservation = self
            .crystal_mantle_authority
            .shell_concealment_route_reservation();
        if crystal_shell_apron.is_empty()
            || !crystal_shell_apron.is_subset(&variable)
            || !crystal_shell_apron.is_disjoint(&self.crystal_mantle_authority.opening_clearance)
            || self
                .crystal_mantle_authority
                .shell_concealment_floors
                .keys()
                .ne(self
                    .crystal_mantle_authority
                    .shell_concealment_ceilings
                    .keys())
        {
            let outside_projection = crystal_shell_apron
                .difference(&variable)
                .copied()
                .take(12)
                .collect::<Vec<_>>();
            return Err(contract(format!(
                "Grand V3 Crystal shell-concealment apron is empty, enters an opening, or escapes projectable dry land; apron={}, outside={outside_projection:?}",
                crystal_shell_apron.len()
            )));
        }
        let frozen_shell_transition = crystal_frozen_shell_transition_grades(
            &self.crystal_mantle_authority.shell_concealment_floors,
            &self.frozen_plateau.levels,
        );
        if frozen_shell_transition.len().saturating_mul(10)
            > self.frozen_plateau.levels.len().saturating_mul(3)
            || frozen_shell_transition.keys().any(|coord| {
                crystal_shell_apron
                    .iter()
                    .map(|apron| apron.distance(*coord))
                    .min()
                    .is_none_or(|distance| distance > CRYSTAL_SHELL_FROZEN_TRANSITION_DEPTH)
            })
        {
            return Err(contract(format!(
                "Grand V3 Crystal/Frozen shell transition exceeds its two-row or thirty-percent budget: transition={}, Frozen core={}",
                frozen_shell_transition.len(),
                self.frozen_plateau.levels.len()
            )));
        }
        let outer = variable
            .iter()
            .flat_map(|coord| coord.neighbors())
            .filter(|coord| {
                footprint.contains(coord)
                    && !variable.contains(coord)
                    // Crystal is an authored interior opening. Treating its
                    // level-150 surface as the Massif's outside datum would
                    // flatten the non-opening concealment shoulder to 159.
                    && !self.crystal_mask.contains(coord)
            })
            .collect::<BTreeSet<_>>();
        let domain = variable
            .iter()
            .copied()
            .chain(outer.iter().copied())
            .collect::<BTreeSet<_>>();
        if variable.is_empty()
            || outer.is_empty()
            || domain.iter().any(|coord| !raw_surface.contains_key(coord))
            || variable
                .iter()
                .any(|coord| !baseline_surface.contains_key(coord))
        {
            return Err(contract(
                "Grand V3 combined highland projection lacks a complete raw, baseline, or true-outer surface",
            ));
        }

        let mut fixed = outer
            .iter()
            .map(|coord| (*coord, raw_surface[coord]))
            .collect::<BTreeMap<_, _>>();
        let mut fix = |coord: HexCoord, level: Level, reason: &str| {
            if !domain.contains(&coord) {
                return Ok(());
            }
            if let Some(previous) = fixed.insert(coord, level) {
                if previous != level {
                    return Err(contract(format!(
                        "Grand V3 combined highland projection has conflicting {reason} pin at {coord:?}: {previous} versus {level}"
                    )));
                }
            }
            Ok(())
        };
        let crest_raw = raw_surface.get(&self.massif.crest).copied();
        if crest_raw != Some(self.massif.summit) {
            return Err(contract(format!(
                "Grand V3 combined highland projection received moved crest: expected {}, got {crest_raw:?}",
                self.massif.summit
            )));
        }
        fix(self.massif.crest, self.massif.summit, "crest")?;
        for (coord, level) in &self.frozen_plateau.levels {
            if !frozen_shell_transition.contains_key(coord) {
                fix(*coord, *level, "Frozen core")?;
            }
        }
        for (coord, level) in &frozen_shell_transition {
            fix(*coord, *level, "Crystal/Frozen shell transition")?;
        }
        drop(fix);

        if let Some((coord, level, floor)) = crystal_shell_apron.iter().find_map(|coord| {
            let floor = self.crystal_mantle_authority.shell_concealment_floors[coord];
            fixed
                .get(coord)
                .copied()
                .filter(|level| *level < floor)
                .map(|level| (*coord, level, floor))
        }) {
            return Err(contract(format!(
                "Grand V3 fixed terrain {coord:?}@{level} cuts below Crystal shell-concealment floor {floor} outside a declared opening"
            )));
        }
        if let Some((coord, level, ceiling)) = crystal_shell_apron.iter().find_map(|coord| {
            let ceiling = self.crystal_mantle_authority.shell_concealment_ceilings[coord];
            fixed
                .get(coord)
                .copied()
                .filter(|level| *level > ceiling)
                .map(|level| (*coord, level, ceiling))
        }) {
            return Err(contract(format!(
                "Grand V3 fixed terrain {coord:?}@{level} rises above Crystal shell-concealment ceiling {ceiling}"
            )));
        }

        for (coord, ceiling) in &ordered_transit.reservation_ceilings {
            if !domain.contains(coord) || !variable.contains(coord) {
                return Err(contract(format!(
                    "Grand V3 ordered peak-transit reservation {coord:?} escaped the variable projection domain"
                )));
            }
            if fixed
                .get(coord)
                .is_some_and(|fixed_level| fixed_level > ceiling)
            {
                return Err(contract(format!(
                    "Grand V3 fixed highland authority conflicts with ordered peak transit at {coord:?}: fixed={}, ceiling={ceiling}",
                    fixed[coord]
                )));
            }
        }
        for (coord, level) in &ordered_transit.authored_grades {
            if !domain.contains(coord) || !variable.contains(coord) {
                return Err(contract(format!(
                    "Grand V3 ordered peak-transit grade {coord:?}@{level} escaped the variable projection domain"
                )));
            }
            if fixed.contains_key(coord) {
                return Err(contract(format!(
                    "Grand V3 ordered peak-transit grade {coord:?}@{level} overlaps a fixed highland authority"
                )));
            }
        }

        let mut authored_upper = domain
            .iter()
            .map(|coord| (*coord, MAX_V3_LEVEL.saturating_sub(1)))
            .collect::<BTreeMap<_, _>>();
        for (coord, level) in &fixed {
            authored_upper.insert(*coord, *level);
        }
        for (coord, level) in &self.frozen_plateau.levels {
            if frozen_shell_transition.contains_key(coord) {
                continue;
            }
            authored_upper
                .entry(*coord)
                .and_modify(|current| *current = (*current).min(*level))
                .or_insert(*level);
        }
        for (coord, distance) in &self.frozen_plateau.halo_distance {
            if crystal_shell_route_reservation.contains(coord) {
                continue;
            }
            let level = FROZEN_PLATEAU_MAX.saturating_add(
                i32::try_from(*distance)
                    .unwrap_or(i32::MAX)
                    .saturating_mul(FROZEN_PLATEAU_MAXIMUM_STEP),
            );
            authored_upper
                .entry(*coord)
                .and_modify(|current| *current = (*current).min(level))
                .or_insert(level);
        }
        for (coord, level) in &self.crystal_exit_ceiling {
            if crystal_shell_route_reservation.contains(coord) {
                continue;
            }
            authored_upper
                .entry(*coord)
                .and_modify(|current| *current = (*current).min(*level))
                .or_insert(*level);
        }
        for (coord, ceiling) in &self.crystal_mantle_authority.shell_concealment_ceilings {
            authored_upper
                .entry(*coord)
                .and_modify(|current| *current = (*current).min(*ceiling))
                .or_insert(*ceiling);
        }
        // The complete four-row reservation stays below the upper crowns and
        // permits only the minimum lift needed to feather the one-level route
        // grade into its natural nine-level shoulder.
        for (coord, level) in &ordered_transit.reservation_ceilings {
            authored_upper
                .entry(*coord)
                .and_modify(|current| *current = (*current).min(*level))
                .or_insert(*level);
        }
        // The route itself is highland-authored terrain, not a later request
        // for the route solver to rediscover a compatible scalar profile.
        for (coord, level) in &ordered_transit.authored_grades {
            authored_upper
                .entry(*coord)
                .and_modify(|current| *current = (*current).min(*level))
                .or_insert(*level);
        }
        if let Some((coord, ceiling, floor)) = crystal_shell_apron.iter().find_map(|coord| {
            let floor = self.crystal_mantle_authority.shell_concealment_floors[coord];
            authored_upper
                .get(coord)
                .copied()
                .filter(|ceiling| *ceiling < floor)
                .map(|ceiling| (*coord, ceiling, floor))
        }) {
            return Err(contract(format!(
                "Grand V3 authored ceiling {coord:?}@{ceiling} cuts below Crystal shell-concealment floor {floor} outside a declared opening"
            )));
        }
        let authored_upper_closed =
            minimum_lipschitz_envelope(&domain, &variable, &authored_upper, MAXIMUM_STEP);
        let admissible_summit_ceiling = |coord: HexCoord| authored_upper_closed[&coord];
        // A seeded summit can land in, or too near to, the separately authored
        // Crystal exit/Frozen transition. Those ceilings are immutable and
        // cannot support a 260+ crown under the nine-level slope contract, so
        // relocate only such a pin within its own stable peak body. The exact
        // graph envelope respects excluded authored holes and supplies the
        // final admissible height at every candidate.
        let mut retired_peak_crown_ceilings = BTreeMap::new();
        let mut relocated_old_peak_pins = BTreeSet::new();
        for component in &mut self.peak_authority.components {
            let touches_crystal_frozen_transition = component
                .patch_masks
                .values()
                .flat_map(|mask| mask.iter())
                .any(|coord| {
                    self.crystal_exit_ceiling.contains_key(coord)
                        || self.frozen_plateau.halo_distance.contains_key(coord)
                        || coord.neighbors().into_iter().any(|neighbor| {
                            self.crystal_exit_ceiling.contains_key(&neighbor)
                                || self.frozen_plateau.halo_distance.contains_key(&neighbor)
                        })
                });
            let bodies = component.expected_peak_bodies.clone();
            for (patch, body) in bodies {
                let pins = component
                    .summit_pins
                    .iter()
                    .filter(|(coord, _)| body.contains_key(coord))
                    .map(|(coord, level)| (*coord, *level))
                    .collect::<Vec<_>>();
                let [(pin, authored_summit)] = pins.as_slice() else {
                    return Err(contract(format!(
                        "Grand V3 peak patch {} has {} summit pins before projection relocation",
                        patch.0,
                        pins.len()
                    )));
                };
                let pin_ceiling = admissible_summit_ceiling(*pin);
                let forbidden_pin = self.crystal_exit_ceiling.contains_key(pin)
                    || self
                        .crystal_mantle_authority
                        .opening_clearance
                        .contains(pin)
                    || self.frozen_plateau.levels.contains_key(pin)
                    || self.frozen_plateau.halo_distance.contains_key(pin);
                if !forbidden_pin && pin_ceiling >= *authored_summit {
                    continue;
                }
                let raw_upper_crown = body
                    .iter()
                    .filter_map(|(coord, level)| (*level >= PEAK_SUMMIT_MIN).then_some(*coord))
                    .collect::<BTreeSet<_>>();
                let retired_component = fine_components(&raw_upper_crown)
                    .into_iter()
                    .find(|crown| crown.contains(pin))
                    .ok_or_else(|| {
                        contract(format!(
                            "Grand V3 ceiling-conflicted peak patch {} lost the old raw crown at {pin:?}",
                            patch.0
                        ))
                    })?;
                let prospective_ceiling = |candidate: HexCoord| {
                    let existing = admissible_summit_ceiling(candidate);
                    if candidate == *pin {
                        existing
                    } else {
                        let retired = retired_component
                            .iter()
                            .map(|coord| {
                                PEAK_SUMMIT_MIN.saturating_sub(1).saturating_add(
                                    i32::try_from(coord.distance(candidate))
                                        .unwrap_or(i32::MAX)
                                        .saturating_mul(MAXIMUM_STEP),
                                )
                            })
                            .min()
                            .unwrap_or(existing);
                        existing.min(retired)
                    }
                };
                let other_summit_pins = component
                    .summit_pins
                    .keys()
                    .filter(|coord| **coord != *pin)
                    .copied()
                    .collect::<BTreeSet<_>>();
                let saddle_exclusions = component
                    .expected_saddle_swaths
                    .values()
                    .chain(component.expected_external_ingress_swaths.values())
                    .flat_map(|swath| swath.iter().copied())
                    .chain(
                        component
                            .ordered_saddle_spines
                            .values()
                            .flat_map(OrderedPeakSaddleSpineAuthority::reservation_coords),
                    )
                    .collect::<BTreeSet<_>>();
                let ordinary_replacement = body
                    .keys()
                    .copied()
                    .filter(|coord| variable.contains(coord))
                    .filter(|coord| !self.crystal_exit_ceiling.contains_key(coord))
                    .filter(|coord| {
                        !self
                            .crystal_mantle_authority
                            .opening_clearance
                            .contains(coord)
                    })
                    .filter(|coord| !self.frozen_plateau.levels.contains_key(coord))
                    .filter(|coord| !self.frozen_plateau.halo_distance.contains_key(coord))
                    .filter(|coord| !other_summit_pins.contains(coord))
                    .filter(|coord| !saddle_exclusions.contains(coord))
                    .filter(|coord| !retired_peak_crown_ceilings.contains_key(coord))
                    .filter(|coord| !relocated_old_peak_pins.contains(coord))
                    .filter(|coord| prospective_ceiling(*coord) >= PEAK_SUMMIT_MIN)
                    .min_by_key(|coord| (pin.distance(*coord), *coord));
                let mut borrowed = BTreeSet::new();
                let replacement = if let Some(replacement) = ordinary_replacement {
                    replacement
                } else if touches_crystal_frozen_transition {
                    let body_coords = body.keys().copied().collect::<BTreeSet<_>>();
                    let other_bodies = component
                        .expected_peak_bodies
                        .iter()
                        .filter(|(owner, _)| **owner != patch)
                        .flat_map(|(_, other)| other.keys().copied())
                        .collect::<BTreeSet<_>>();
                    let borrow_region = body_coords
                        .iter()
                        .flat_map(|coord| coord.within_radius(3))
                        .filter(|coord| {
                            self.peak_feather.contains_key(coord)
                                && variable.contains(coord)
                                && !other_bodies.contains(coord)
                                && !saddle_exclusions.contains(coord)
                                && !retired_peak_crown_ceilings.contains_key(coord)
                                && !relocated_old_peak_pins.contains(coord)
                                && !self.crystal_exit_ceiling.contains_key(coord)
                                && !self
                                    .crystal_mantle_authority
                                    .opening_clearance
                                    .contains(coord)
                                && !self.frozen_plateau.levels.contains_key(coord)
                                && !self.frozen_plateau.halo_distance.contains_key(coord)
                        })
                        .collect::<BTreeSet<_>>();
                    let mut allowed = body_coords.clone();
                    allowed.extend(borrow_region.iter().copied());
                    let goals = borrow_region
                        .iter()
                        .filter(|coord| prospective_ceiling(**coord) >= PEAK_SUMMIT_MIN)
                        .copied()
                        .collect::<BTreeSet<_>>();
                    let path = shortest_path_between_sets(&allowed, &body_coords, &goals)
                        .filter(|path| path.len().saturating_sub(1) <= 3)
                        .ok_or_else(|| {
                            contract(format!(
                                "Grand V3 peak patch {} has no admissible replacement for ceiling-conflicted summit {pin:?}, including its three-row physical feather borrow",
                                patch.0
                            ))
                        })?;
                    borrowed.extend(
                        path.iter()
                            .copied()
                            .filter(|coord| !body.contains_key(coord)),
                    );
                    *path.last().ok_or_else(|| {
                        contract("Grand V3 peak crown borrow resolved an empty path")
                    })?
                } else {
                    return Err(contract(format!(
                        "Grand V3 peak patch {} has no admissible replacement for ceiling-conflicted summit {pin:?}",
                        patch.0
                    )));
                };
                let recorded = (*authored_summit).min(prospective_ceiling(replacement));
                if recorded < PEAK_SUMMIT_MIN {
                    return Err(contract(format!(
                        "Grand V3 peak patch {} replacement {replacement:?} resolves below minimum summit: {recorded}",
                        patch.0
                    )));
                }
                if replacement != *pin {
                    if component
                        .summit_pins
                        .keys()
                        .any(|other| *other != *pin && retired_component.contains(other))
                    {
                        return Err(contract(format!(
                            "Grand V3 retired peak patch {} crown contains another active summit",
                            patch.0
                        )));
                    }
                    for coord in retired_component {
                        if coord != replacement {
                            retired_peak_crown_ceilings
                                .entry(coord)
                                .and_modify(|ceiling: &mut Level| {
                                    *ceiling = (*ceiling).min(PEAK_SUMMIT_MIN.saturating_sub(1));
                                })
                                .or_insert(PEAK_SUMMIT_MIN.saturating_sub(1));
                        }
                    }
                    relocated_old_peak_pins.insert(*pin);
                }
                component.summit_pins.remove(pin);
                component
                    .expected_peak_bodies
                    .get_mut(&patch)
                    .ok_or_else(|| contract("Grand V3 relocated crown lost its body"))?
                    .insert(replacement, recorded);
                if !borrowed.is_empty() {
                    if borrowed.len() > 3 {
                        return Err(contract(format!(
                            "Grand V3 peak patch {} borrowed {} physical crown cells, maximum 3",
                            patch.0,
                            borrowed.len()
                        )));
                    }
                    let physical_body = component
                        .expected_peak_bodies
                        .get_mut(&patch)
                        .ok_or_else(|| contract("Grand V3 peak crown borrow lost its body"))?;
                    for coord in &borrowed {
                        physical_body.insert(*coord, raw_surface[coord]);
                    }
                    component
                        .borrowed_crown_cells
                        .insert(patch, borrowed.clone());
                }
                if component
                    .summit_pins
                    .insert(replacement, recorded)
                    .is_some()
                {
                    return Err(contract(format!(
                        "Grand V3 peak patch {} relocated onto another summit at {replacement:?}",
                        patch.0
                    )));
                }
            }
        }
        let borrowed_peak_patches = self
            .peak_authority
            .components
            .iter()
            .flat_map(|component| component.borrowed_crown_cells.keys())
            .count();
        let active_peak_pins = self
            .peak_authority
            .components
            .iter()
            .flat_map(|component| component.summit_pins.keys().copied())
            .collect::<BTreeSet<_>>();
        if retired_peak_crown_ceilings
            .keys()
            .any(|coord| active_peak_pins.contains(coord))
            || !relocated_old_peak_pins.is_disjoint(&active_peak_pins)
        {
            return Err(contract(
                "Grand V3 retired crown authority overlaps an active summit pin",
            ));
        }
        if borrowed_peak_patches > 3 {
            return Err(contract(format!(
                "Grand V3 peak projection borrowed physical crown authority for {borrowed_peak_patches} patches, maximum 3"
            )));
        }
        let peak_summit_ceilings = self
            .peak_authority
            .components
            .iter()
            .flat_map(|component| {
                component
                    .summit_pins
                    .iter()
                    .map(|(coord, level)| (*coord, *level))
            })
            .collect::<BTreeMap<_, _>>();
        let mut peak_body_ceilings = BTreeMap::new();
        for component in &self.peak_authority.components {
            for (patch, body) in &component.expected_peak_bodies {
                let pins = component
                    .summit_pins
                    .iter()
                    .filter(|(coord, _)| body.contains_key(coord))
                    .map(|(_, level)| *level)
                    .collect::<Vec<_>>();
                let [summit] = pins.as_slice() else {
                    return Err(contract(format!(
                        "Grand V3 peak patch {} has {} summit pins in its stable body",
                        patch.0,
                        pins.len()
                    )));
                };
                for coord in body.keys() {
                    peak_body_ceilings
                        .entry(*coord)
                        .and_modify(|ceiling: &mut Level| *ceiling = (*ceiling).min(*summit))
                        .or_insert(*summit);
                }
            }
        }
        let mut no_shoulders = self.massif.clone();
        no_shoulders.shoulder_sources.clear();
        no_shoulders.shoulder_support.clear();
        let mut shoulder_witness_clusters = BTreeMap::new();
        let mut shoulder_witness_floors = BTreeMap::new();
        for source in self.massif.shoulder_sources.keys() {
            let mut candidates = source
                .within_radius(3)
                .into_iter()
                .filter(|coord| *coord != *source)
                .filter(|coord| variable.contains(coord) && self.massif.mask.contains(coord))
                .filter_map(|coord| {
                    let baseline = baseline_surface.get(&coord).copied()?;
                    let full = self.massif.resolve(coord, baseline);
                    let without = no_shoulders.resolve(coord, baseline);
                    (full >= without.saturating_add(2)
                        && raw_surface[&coord] >= without.saturating_add(2))
                    .then_some((coord, without.saturating_add(2)))
                })
                .collect::<Vec<_>>();
            candidates.sort_by_key(|(coord, _)| (source.distance(*coord), *coord));
            candidates.truncate(7);
            if candidates.len() < 3 {
                return Err(contract(format!(
                    "Grand V3 shoulder source {source:?} has only {} non-source projection witnesses",
                    candidates.len()
                )));
            }
            let cluster = candidates
                .iter()
                .map(|(coord, _)| *coord)
                .collect::<BTreeSet<_>>();
            for (coord, floor) in candidates {
                shoulder_witness_floors
                    .entry(coord)
                    .and_modify(|current: &mut Level| *current = (*current).max(floor))
                    .or_insert(floor);
            }
            shoulder_witness_clusters.insert(*source, cluster);
        }
        let mut lower = BTreeMap::new();
        let mut upper = BTreeMap::new();
        for coord in &domain {
            if let Some(level) = fixed.get(coord).copied() {
                lower.insert(*coord, level);
                upper.insert(*coord, level);
                continue;
            }
            // The raw surface is the projection objective, not a hard lower
            // bound. A constant high baseline can cross many rows before a
            // low immutable neighbor; any finite per-column allowance merely
            // moves the infeasible edge inward instead of producing the
            // required continuous grade. Authored crests, summit regions,
            // shoulders, Frozen terrain, and mantle sectors receive explicit
            // constraints below.
            let mut minimum = 0;
            let mut maximum = MAX_V3_LEVEL.saturating_sub(1);
            if let Some(summit_ceiling) = peak_summit_ceilings.get(coord).copied() {
                minimum = minimum.max(summit_ceiling);
                maximum = maximum.min(summit_ceiling);
            }
            if let Some(body_ceiling) = peak_body_ceilings.get(coord).copied() {
                maximum = maximum.min(body_ceiling);
            }
            if let Some(retired_ceiling) = retired_peak_crown_ceilings.get(coord).copied() {
                maximum = maximum.min(retired_ceiling);
            }
            if self.massif.summit_sources.contains_key(coord) && *coord != self.massif.crest {
                let distance = self.massif.crest.distance(*coord);
                let single_body = self.massif.summit.saturating_sub(
                    MASSIF_BODY_NEAR_CREST_INITIAL_DROP.saturating_add(
                        i32::try_from(distance)
                            .unwrap_or(i32::MAX)
                            .saturating_mul(MASSIF_BODY_CREST_FALLOFF_PER_HEX),
                    ),
                );
                minimum = minimum.max(single_body.saturating_add(2));
            }
            if let Some(witness_floor) = shoulder_witness_floors.get(coord).copied() {
                minimum = minimum.max(witness_floor);
            }
            if let Some(sector_floor) = self
                .crystal_mantle_authority
                .sector_pins
                .values()
                .find_map(|(pin, level)| (*pin == *coord).then_some(*level))
            {
                minimum = minimum.max(sector_floor);
            }
            if let Some(shell_floor) = self
                .crystal_mantle_authority
                .shell_concealment_floors
                .get(coord)
                .copied()
            {
                minimum = minimum.max(shell_floor);
            }
            if let Some(shell_ceiling) = self
                .crystal_mantle_authority
                .shell_concealment_ceilings
                .get(coord)
                .copied()
            {
                maximum = maximum.min(shell_ceiling);
            }
            if let Some(distance) = self
                .frozen_plateau
                .halo_distance
                .get(coord)
                .copied()
                .filter(|_| !crystal_shell_route_reservation.contains(coord))
            {
                maximum = maximum.min(
                    FROZEN_PLATEAU_MAX.saturating_add(
                        i32::try_from(distance)
                            .unwrap_or(i32::MAX)
                            .saturating_mul(FROZEN_PLATEAU_MAXIMUM_STEP),
                    ),
                );
            }
            if let Some(ceiling) = self
                .crystal_exit_ceiling
                .get(coord)
                .copied()
                .filter(|_| !crystal_shell_route_reservation.contains(coord))
            {
                maximum = maximum.min(ceiling);
            }
            if let Some(ceiling) = ordered_transit.reservation_ceilings.get(coord).copied() {
                maximum = maximum.min(ceiling);
            }
            if let Some(level) = ordered_transit.authored_grades.get(coord).copied() {
                minimum = minimum.max(level);
                maximum = maximum.min(level);
            }
            lower.insert(*coord, minimum);
            upper.insert(*coord, maximum);
        }
        let lower_closed = maximum_lipschitz_envelope(&domain, &variable, &lower, MAXIMUM_STEP);
        let upper_closed = minimum_lipschitz_envelope(&domain, &variable, &upper, MAXIMUM_STEP);
        if let Some((coord, minimum, maximum)) = domain.iter().find_map(|coord| {
            let minimum = lower_closed.get(coord).copied()?;
            let maximum = upper_closed.get(coord).copied()?;
            (minimum > maximum).then_some((*coord, minimum, maximum))
        }) {
            let fixed_neighbors = coord
                .within_radius(4)
                .into_iter()
                .filter_map(|nearby| fixed.get(&nearby).map(|level| (nearby, *level)))
                .collect::<Vec<_>>();
            let lower_contributors = lower
                .iter()
                .filter_map(|(source, level)| {
                    let contribution = level.saturating_sub(
                        i32::try_from(source.distance(coord))
                            .unwrap_or(i32::MAX)
                            .saturating_mul(MAXIMUM_STEP),
                    );
                    (contribution == minimum).then_some((*source, *level))
                })
                .take(12)
                .collect::<Vec<_>>();
            let upper_contributors = upper
                .iter()
                .filter_map(|(source, level)| {
                    let contribution = level.saturating_add(
                        i32::try_from(source.distance(coord))
                            .unwrap_or(i32::MAX)
                            .saturating_mul(MAXIMUM_STEP),
                    );
                    (contribution == maximum).then_some((*source, *level))
                })
                .take(12)
                .collect::<Vec<_>>();
            return Err(contract(format!(
                "Grand V3 combined highland projection is infeasible at {coord:?}: closed lower {minimum} exceeds upper {maximum}; lower contributors={lower_contributors:?}; upper contributors={upper_contributors:?}; nearby fixed pins={fixed_neighbors:?}"
            )));
        }
        let desired = domain
            .iter()
            .map(|coord| {
                let desired = raw_surface[coord]
                    .max(lower_closed[coord])
                    .min(upper_closed[coord]);
                (*coord, desired)
            })
            .collect::<BTreeMap<_, _>>();
        let projected = maximum_lipschitz_envelope(&domain, &variable, &desired, MAXIMUM_STEP);
        validate_massif_connector_profile(&self.massif, Some(&projected))?;
        validate_combined_surface_projection(
            self,
            &domain,
            &variable,
            &fixed,
            &upper,
            &projected,
            &shoulder_witness_clusters,
            &shoulder_witness_floors,
            &relocated_old_peak_pins,
        )?;
        self.combined_surface_projection = variable
            .iter()
            .map(|coord| (*coord, projected[coord]))
            .collect();
        Ok(())
    }

    pub(super) const fn massif_crest(&self) -> (HexCoord, Level) {
        (self.massif.crest, self.massif.summit)
    }

    pub(super) const fn massif_summit_core(&self) -> &BTreeSet<HexCoord> {
        &self.massif.summit_core
    }

    pub(super) const fn peak_ridge_authority(&self) -> &PeakRidgeAuthority {
        &self.peak_authority
    }

    pub(super) const fn massif_visual_authority(&self) -> &MassifVisualAuthority {
        &self.massif_visual_authority
    }

    pub(super) const fn crystal_mantle_authority(&self) -> &CrystalMantleAuthority {
        &self.crystal_mantle_authority
    }
}

fn minimum_lipschitz_envelope(
    domain: &BTreeSet<HexCoord>,
    variable: &BTreeSet<HexCoord>,
    sources: &BTreeMap<HexCoord, Level>,
    maximum_step: Level,
) -> BTreeMap<HexCoord, Level> {
    let mut levels = BTreeMap::new();
    let mut queue = BinaryHeap::new();
    for (coord, level) in sources {
        levels.insert(*coord, *level);
        queue.push(Reverse((*level, *coord)));
    }
    while let Some(Reverse((level, coord))) = queue.pop() {
        if levels.get(&coord).copied() != Some(level) {
            continue;
        }
        for neighbor in coord.neighbors() {
            if !domain.contains(&neighbor)
                || (!variable.contains(&coord) && !variable.contains(&neighbor))
            {
                continue;
            }
            let candidate = level.saturating_add(maximum_step);
            if levels
                .get(&neighbor)
                .is_none_or(|current| candidate < *current)
            {
                levels.insert(neighbor, candidate);
                queue.push(Reverse((candidate, neighbor)));
            }
        }
    }
    levels
}

fn maximum_lipschitz_envelope(
    domain: &BTreeSet<HexCoord>,
    variable: &BTreeSet<HexCoord>,
    sources: &BTreeMap<HexCoord, Level>,
    maximum_step: Level,
) -> BTreeMap<HexCoord, Level> {
    let mut levels = BTreeMap::new();
    let mut queue = BinaryHeap::new();
    for (coord, level) in sources {
        levels.insert(*coord, *level);
        queue.push((*level, Reverse(*coord)));
    }
    while let Some((level, Reverse(coord))) = queue.pop() {
        if levels.get(&coord).copied() != Some(level) {
            continue;
        }
        for neighbor in coord.neighbors() {
            if !domain.contains(&neighbor)
                || (!variable.contains(&coord) && !variable.contains(&neighbor))
            {
                continue;
            }
            let candidate = level.saturating_sub(maximum_step);
            if levels
                .get(&neighbor)
                .is_none_or(|current| candidate > *current)
            {
                levels.insert(neighbor, candidate);
                queue.push((candidate, Reverse(neighbor)));
            }
        }
    }
    levels
}

fn validate_combined_surface_projection(
    field: &GrandHighlandField,
    domain: &BTreeSet<HexCoord>,
    variable: &BTreeSet<HexCoord>,
    fixed: &BTreeMap<HexCoord, Level>,
    local_upper: &BTreeMap<HexCoord, Level>,
    projected: &BTreeMap<HexCoord, Level>,
    shoulder_witness_clusters: &BTreeMap<HexCoord, BTreeSet<HexCoord>>,
    shoulder_witness_floors: &BTreeMap<HexCoord, Level>,
    relocated_old_peak_pins: &BTreeSet<HexCoord>,
) -> Result<(), V3GenerationError> {
    if projected.len() != domain.len() {
        return Err(contract(format!(
            "Grand V3 combined highland projection resolved {} of {} columns",
            projected.len(),
            domain.len()
        )));
    }
    if let Some((coord, expected, actual)) = fixed.iter().find_map(|(coord, expected)| {
        let actual = projected.get(coord).copied();
        (actual != Some(*expected)).then_some((*coord, *expected, actual))
    }) {
        return Err(contract(format!(
            "Grand V3 combined highland projection moved fixed pin {coord:?}: expected {expected}, got {actual:?}"
        )));
    }
    if let Some((coord, level, ceiling)) = variable.iter().find_map(|coord| {
        let level = projected[coord];
        let ceiling = local_upper[coord];
        (level > ceiling).then_some((*coord, level, ceiling))
    }) {
        return Err(contract(format!(
            "Grand V3 combined highland projection raised {coord:?} to {level} above ceiling {ceiling}"
        )));
    }
    let crystal_shell_apron = field.crystal_mantle_authority.shell_concealment_apron();
    if let Some((coord, actual, floor)) = crystal_shell_apron.iter().find_map(|coord| {
        let actual = projected.get(coord).copied();
        let floor = field.crystal_mantle_authority.shell_concealment_floors[coord];
        (actual.is_none_or(|level| level < floor)).then_some((*coord, actual, floor))
    }) {
        return Err(contract(format!(
            "Grand V3 combined highland projection left Crystal shell apron {coord:?} at {actual:?}, below concealment floor {floor}"
        )));
    }
    if let Some((coord, actual, ceiling)) = crystal_shell_apron.iter().find_map(|coord| {
        let actual = projected.get(coord).copied();
        let ceiling = field.crystal_mantle_authority.shell_concealment_ceilings[coord];
        (actual.is_none_or(|level| level > ceiling)).then_some((*coord, actual, ceiling))
    }) {
        return Err(contract(format!(
            "Grand V3 combined highland projection left Crystal shell apron {coord:?} at {actual:?}, above concealment ceiling {ceiling}"
        )));
    }
    if let Some((sector, coord, floor, actual)) = field
        .crystal_mantle_authority
        .sector_pins
        .iter()
        .find_map(|(sector, (coord, floor))| {
            let actual = projected.get(coord).copied();
            (actual.is_none_or(|level| level < *floor)).then_some((*sector, *coord, *floor, actual))
        })
    {
        return Err(contract(format!(
            "Grand V3 combined highland projection lowered Crystal enclosure sector {sector} pin {coord:?} below {floor}: {actual:?}"
        )));
    }
    field
        .crystal_mantle_authority
        .validate_attainable_coverage("combined highland projection", |coord| {
            projected.get(&coord).copied()
        })
        .map_err(contract)?;
    if let Some((first, first_level, second, second_level)) = variable.iter().find_map(|coord| {
        let first_level = projected[coord];
        coord.neighbors().into_iter().find_map(|neighbor| {
            if field.crystal_mask.contains(&neighbor) {
                return None;
            }
            projected.get(&neighbor).copied().and_then(|second_level| {
                (first_level.abs_diff(second_level) > 9).then_some((
                    *coord,
                    first_level,
                    neighbor,
                    second_level,
                ))
            })
        })
    }) {
        return Err(contract(format!(
            "Grand V3 combined highland projection left cliff {first:?}={first_level} -> {second:?}={second_level}"
        )));
    }
    if projected.values().any(|level| *level >= MAX_V3_LEVEL) {
        return Err(contract(
            "Grand V3 combined highland projection exceeded the V3 ceiling",
        ));
    }
    let mut all_peak_pins = BTreeSet::new();
    let mut borrowed_peak_patches = 0_usize;
    let mut physical_peak_owners = BTreeMap::<HexCoord, PatchId>::new();
    for component in &field.peak_authority.components {
        if let Some((coord, expected, actual)) = component
            .ordered_saddle_spines
            .values()
            .flat_map(|spine| spine.authored_grades.iter())
            .find_map(|(coord, expected)| {
                let actual = projected.get(coord).copied();
                (actual != Some(*expected)).then_some((*coord, *expected, actual))
            })
        {
            return Err(contract(format!(
                "Grand V3 combined highland projection moved ordered transit grade {coord:?}: expected {expected}, got {actual:?}"
            )));
        }
        let touches_crystal_frozen_transition = component
            .patch_masks
            .values()
            .flat_map(|mask| mask.iter())
            .any(|coord| {
                field.crystal_exit_ceiling.contains_key(coord)
                    || field.frozen_plateau.halo_distance.contains_key(coord)
                    || coord.neighbors().into_iter().any(|neighbor| {
                        field.crystal_exit_ceiling.contains_key(&neighbor)
                            || field.frozen_plateau.halo_distance.contains_key(&neighbor)
                    })
            });
        let saddle_exclusions = component
            .expected_saddle_swaths
            .values()
            .chain(component.expected_external_ingress_swaths.values())
            .flat_map(|swath| swath.iter().copied())
            .chain(
                component
                    .ordered_saddle_spines
                    .values()
                    .flat_map(OrderedPeakSaddleSpineAuthority::reservation_coords),
            )
            .collect::<BTreeSet<_>>();
        let mut upper_crowns = BTreeSet::new();
        for (patch, body) in &component.expected_peak_bodies {
            for coord in body.keys() {
                if let Some(previous) = physical_peak_owners.insert(*coord, *patch) {
                    if previous != *patch {
                        return Err(contract(format!(
                            "Grand V3 physical peak bodies overlap at {coord:?}: patches {} and {}",
                            previous.0, patch.0
                        )));
                    }
                }
            }
            let semantic_mask = component.patch_masks.get(patch).ok_or_else(|| {
                contract(format!(
                    "Grand V3 projected peak patch {} lost its semantic mask",
                    patch.0
                ))
            })?;
            let escaped = body
                .keys()
                .filter(|coord| !semantic_mask.contains(coord))
                .copied()
                .collect::<BTreeSet<_>>();
            let recorded_borrow = component
                .borrowed_crown_cells
                .get(patch)
                .cloned()
                .unwrap_or_default();
            borrowed_peak_patches += usize::from(!recorded_borrow.is_empty());
            if escaped != recorded_borrow
                || recorded_borrow.len() > 3
                || (!recorded_borrow.is_empty() && !touches_crystal_frozen_transition)
                || recorded_borrow
                    .iter()
                    .any(|coord| !field.peak_feather.contains_key(coord))
                || !connected(&body.keys().copied().collect::<BTreeSet<_>>())
            {
                return Err(contract(format!(
                    "Grand V3 projected peak patch {} has invalid physical crown borrow: escaped={}, recorded={}",
                    patch.0,
                    escaped.len(),
                    recorded_borrow.len()
                )));
            }
            let pins = component
                .summit_pins
                .iter()
                .filter(|(coord, _)| body.contains_key(coord))
                .map(|(coord, level)| (*coord, *level))
                .collect::<Vec<_>>();
            let [(pin, summit)] = pins.as_slice() else {
                return Err(contract(format!(
                    "Grand V3 projected peak patch {} has {} authority pins",
                    patch.0,
                    pins.len()
                )));
            };
            if !all_peak_pins.insert(*pin)
                || field.crystal_exit_ceiling.contains_key(pin)
                || field
                    .crystal_mantle_authority
                    .opening_clearance
                    .contains(pin)
                || field.frozen_plateau.levels.contains_key(pin)
                || field.frozen_plateau.halo_distance.contains_key(pin)
                || saddle_exclusions.contains(pin)
                || projected.get(pin).copied() != Some(*summit)
                || *summit < PEAK_SUMMIT_MIN
            {
                return Err(contract(format!(
                    "Grand V3 projected peak patch {} has invalid summit authority at {pin:?}={summit}",
                    patch.0
                )));
            }
            let body_max = body
                .keys()
                .filter_map(|coord| projected.get(coord).copied())
                .max()
                .ok_or_else(|| {
                    contract(format!(
                        "Grand V3 projected peak patch {} lost its stable body",
                        patch.0
                    ))
                })?;
            if body_max > *summit {
                return Err(contract(format!(
                    "Grand V3 projected peak patch {} body exceeds summit: {body_max} > {summit}",
                    patch.0
                )));
            }
            upper_crowns.extend(
                body.keys()
                    .filter(|coord| {
                        projected
                            .get(coord)
                            .is_some_and(|level| *level >= PEAK_SUMMIT_MIN)
                    })
                    .copied(),
            );
        }
        if fine_components(&upper_crowns).len() != 6 {
            return Err(contract(format!(
                "Grand V3 projected peak chain has {} crowns at level {PEAK_SUMMIT_MIN}, expected 6",
                fine_components(&upper_crowns).len()
            )));
        }
    }
    if all_peak_pins.len() != 12 {
        return Err(contract(format!(
            "Grand V3 projected peaks retain {} unique summit pins, expected 12",
            all_peak_pins.len()
        )));
    }
    if borrowed_peak_patches > 3 {
        return Err(contract(format!(
            "Grand V3 projected peaks borrowed {borrowed_peak_patches} physical crown patches, maximum 3"
        )));
    }
    if let Some((coord, level)) = relocated_old_peak_pins.iter().find_map(|coord| {
        projected
            .get(coord)
            .copied()
            .filter(|level| *level >= PEAK_SUMMIT_MIN)
            .map(|level| (*coord, level))
    }) {
        return Err(contract(format!(
            "Grand V3 retired peak summit {coord:?} remained an upper crown at {level}"
        )));
    }
    let summit_witnesses = field
        .massif
        .summit_sources
        .keys()
        .filter(|coord| **coord != field.massif.crest && variable.contains(coord))
        .filter(|coord| {
            let distance = field.massif.crest.distance(**coord);
            let single_body = field.massif.summit.saturating_sub(
                MASSIF_BODY_NEAR_CREST_INITIAL_DROP.saturating_add(
                    i32::try_from(distance)
                        .unwrap_or(i32::MAX)
                        .saturating_mul(MASSIF_BODY_CREST_FALLOFF_PER_HEX),
                ),
            );
            projected
                .get(coord)
                .copied()
                .is_some_and(|level| level >= single_body.saturating_add(2))
        })
        .copied()
        .collect::<BTreeSet<_>>();
    let summit_sectors = summit_witnesses
        .iter()
        .map(|coord| enclosure_sector(field.massif.crest, *coord))
        .collect::<BTreeSet<_>>();
    if summit_witnesses.len() < 4 || summit_sectors.len() < 4 {
        return Err(contract(format!(
            "Grand V3 combined highland projection erased distributed summit character: witnesses={summit_witnesses:?}, sectors={summit_sectors:?}"
        )));
    }
    let shoulder_witnesses = shoulder_witness_clusters
        .iter()
        .filter(|(_, cluster)| {
            cluster
                .iter()
                .filter(|coord| {
                    projected
                        .get(coord)
                        .is_some_and(|level| *level >= shoulder_witness_floors[*coord])
                })
                .count()
                >= 3
        })
        .map(|(source, _)| *source)
        .collect::<BTreeSet<_>>();
    let shoulder_sectors = shoulder_witnesses
        .iter()
        .map(|coord| enclosure_sector(field.massif.crest, *coord))
        .collect::<BTreeSet<_>>();
    if shoulder_witnesses.len() < 3 || shoulder_sectors.len() < 3 {
        return Err(contract(format!(
            "Grand V3 combined highland projection erased distributed shoulder character: witnesses={shoulder_witnesses:?}, sectors={shoulder_sectors:?}"
        )));
    }
    Ok(())
}

fn edge_blended_uplift(baseline: Level, target: Level, edge_depth: u32) -> Level {
    let maximum_added_height = i32::try_from(edge_depth)
        .unwrap_or(i32::MAX)
        .saturating_mul(CRYSTAL_ENCLOSURE_EDGE_RISE_PER_HEX);
    baseline.max(target.min(baseline.saturating_add(maximum_added_height)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LandmarkCell {
    patch: PatchId,
    representative: HexCoord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CrystalContext {
    schematic: SchematicCoord,
    center: HexCoord,
    mask: BTreeSet<HexCoord>,
    rotation_turns: u8,
    tunnel_neighbor: HexCoord,
    exit_clearance: BTreeSet<HexCoord>,
}

fn landmark_cells(
    plan: &SchematicPlanV1,
    layout: &ResolvedLayoutPlan,
    predicate: impl Fn(&CellPlan) -> bool,
) -> Result<BTreeMap<SchematicCoord, LandmarkCell>, V3GenerationError> {
    plan.cells
        .iter()
        .filter(|cell| predicate(cell))
        .map(|cell| {
            let patch_id = PatchId(u32::from(cell.id.get()));
            let patch = layout.patches.get(&patch_id).ok_or_else(|| {
                contract(format!(
                    "Grand V3 highland cell {} has no resolved patch",
                    cell.id.get()
                ))
            })?;
            let nominal = schematic_to_world(cell.coord);
            let representative = patch
                .mask
                .iter()
                .copied()
                .min_by_key(|coord| (coord.distance(nominal), *coord))
                .ok_or_else(|| {
                    contract(format!(
                        "Grand V3 highland cell {} has an empty resolved mask",
                        cell.id.get()
                    ))
                })?;
            Ok((
                cell.coord,
                LandmarkCell {
                    patch: patch_id,
                    representative,
                },
            ))
        })
        .collect()
}

fn union_masks(
    layout: &ResolvedLayoutPlan,
    patches: impl IntoIterator<Item = PatchId>,
) -> Result<BTreeSet<HexCoord>, V3GenerationError> {
    let mut mask = BTreeSet::new();
    for patch_id in patches {
        let patch = layout.patches.get(&patch_id).ok_or_else(|| {
            contract(format!(
                "Grand V3 highland patch {} disappeared during field construction",
                patch_id.0
            ))
        })?;
        mask.extend(patch.mask.iter().copied());
    }
    if mask.is_empty() {
        return Err(contract("Grand V3 highland mask is empty"));
    }
    Ok(mask)
}

fn build_frozen_plateau(
    plan: &SchematicPlanV1,
    layout: &ResolvedLayoutPlan,
    profile: V3GrandV3BasicTerrainProfile,
    seed: u64,
) -> Result<FrozenPlateauField, V3GenerationError> {
    if profile.frozen_woods_level != FROZEN_PLATEAU_LEVEL {
        return Err(contract(format!(
            "Grand V3 Frozen-Woods plateau requires level {FROZEN_PLATEAU_LEVEL}, got {}",
            profile.frozen_woods_level
        )));
    }
    let frozen_cells = landmark_cells(plan, layout, |cell| {
        cell.facts.overlays.contains(&SchematicFeature::FrozenWoods)
    })?;
    let frozen_mask = union_masks(layout, frozen_cells.values().map(|cell| cell.patch))?;
    let levels = frozen_mask
        .iter()
        .copied()
        .map(|coord| {
            let variation = named_sample(seed, "grand_v3.highlands.frozen_plateau", coord) % 12;
            let level = match variation {
                0 => FROZEN_PLATEAU_MIN,
                1 => FROZEN_PLATEAU_MAX,
                _ => FROZEN_PLATEAU_LEVEL,
            };
            (coord, level)
        })
        .collect::<BTreeMap<_, _>>();

    let cells_by_patch = plan
        .cells
        .iter()
        .map(|cell| (PatchId(u32::from(cell.id.get())), cell))
        .collect::<BTreeMap<_, _>>();
    let owner_by_coord = layout
        .patches
        .iter()
        .flat_map(|(patch, resolved)| resolved.mask.iter().map(|coord| (*coord, *patch)))
        .collect::<BTreeMap<_, _>>();
    let eligible_blend = owner_by_coord
        .iter()
        .filter_map(|(coord, patch)| {
            let cell = cells_by_patch.get(patch).copied()?;
            (cell.facts.surface == SurfaceKind::Land
                && matches!(
                    cell.facts.landform,
                    LandformKind::Mountain | LandformKind::Massif | LandformKind::SharpPeak
                )
                && cell.facts.overlays.iter().all(|overlay| {
                    !matches!(
                        overlay,
                        SchematicFeature::MountainLake
                            | SchematicFeature::LakeIsland
                            | SchematicFeature::Waterfall
                            | SchematicFeature::CrystalAscent
                    )
                }))
            .then_some(*coord)
        })
        .collect::<BTreeSet<_>>();
    let halo_distance = frozen_halo_distances(&frozen_mask, &eligible_blend);
    if levels.is_empty() || halo_distance.is_empty() {
        return Err(contract(
            "Grand V3 Frozen-Woods plateau or its six-row mountain blend is empty",
        ));
    }
    Ok(FrozenPlateauField {
        levels,
        halo_distance,
    })
}

fn frozen_halo_distances(
    frozen_mask: &BTreeSet<HexCoord>,
    eligible_blend: &BTreeSet<HexCoord>,
) -> BTreeMap<HexCoord, u32> {
    let mut seen = frozen_mask.clone();
    let mut frontier = frozen_mask.clone();
    let mut halo_distance = BTreeMap::new();
    for distance in 1..=FROZEN_PLATEAU_HALO_DEPTH {
        let next = frontier
            .iter()
            .flat_map(|coord| coord.neighbors())
            // Excluded water, Crystal, and waterfall ownership is a real
            // barrier.  Traversing it before filtering allowed the plateau
            // cap to reappear on a remote mountain behind that barrier.
            .filter(|coord| eligible_blend.contains(coord) && seen.insert(*coord))
            .collect::<BTreeSet<_>>();
        for coord in &next {
            halo_distance.insert(*coord, distance);
        }
        frontier = next;
    }
    halo_distance
}

/// Smooth ceiling for the exact upper Crystal opening and its mountain blend.
///
/// The exit itself meets the level-151..153 Frozen plateau. A sixteen-row
/// nine-level envelope lets adjacent peak and Massif terrain resume its full
/// height without leaving a vertical slot or allowing an upper crown to seal
/// the route. This is a scalar cap only; the later route compiler still owns
/// the exact three-to-five-wide walker surface.
fn build_crystal_exit_ceiling(
    exit_clearance: &BTreeSet<HexCoord>,
    footprint: &BTreeSet<HexCoord>,
) -> BTreeMap<HexCoord, Level> {
    exit_clearance
        .iter()
        .flat_map(|source| source.within_radius(CRYSTAL_EXIT_BLEND_DEPTH))
        .filter(|coord| footprint.contains(coord))
        .map(|coord| {
            let distance = exit_clearance
                .iter()
                .map(|source| source.distance(coord))
                .min()
                .unwrap_or(CRYSTAL_EXIT_BLEND_DEPTH);
            let ceiling = FROZEN_PLATEAU_MAX.saturating_add(
                i32::try_from(distance)
                    .unwrap_or(i32::MAX)
                    .saturating_mul(CRYSTAL_EXIT_MAXIMUM_STEP),
            );
            (coord, ceiling.min(MAX_V3_LEVEL.saturating_sub(1)))
        })
        .collect()
}

/// Connects separated fine Massif ownership components through the shortest
/// available overlay-free Mountain corridor without mutating layout ownership.
///
/// Crystal's exact radius-32 claim is allowed to interrupt the nearest-centre
/// Massif union. The global height field still needs one connected propagation
/// domain, but reassigning Mountain columns would change their stable biome IDs
/// and every downstream semantic lookup. This visual mask is deliberately an
/// independent derived projection.
fn build_massif_visual_authority(
    plan: &SchematicPlanV1,
    layout: &ResolvedLayoutPlan,
    massif_owner_mask: &BTreeSet<HexCoord>,
    crystal_mask: &BTreeSet<HexCoord>,
) -> Result<MassifVisualAuthority, V3GenerationError> {
    if massif_owner_mask.is_empty() || !massif_owner_mask.is_disjoint(crystal_mask) {
        return Err(contract(
            "Grand V3 Massif ownership is empty or enters the exact Crystal site",
        ));
    }
    let mountain_connector_patches = plan
        .cells
        .iter()
        .filter(|cell| {
            cell.facts.surface == SurfaceKind::Land
                && cell.facts.landform == LandformKind::Mountain
                && cell.facts.overlays.is_empty()
        })
        .map(|cell| PatchId(u32::from(cell.id.get())))
        .collect::<BTreeSet<_>>();
    let mut allowed = massif_owner_mask.clone();
    for patch_id in mountain_connector_patches {
        let patch = layout.patches.get(&patch_id).ok_or_else(|| {
            contract(format!(
                "Grand V3 Massif connector patch {} disappeared from the layout",
                patch_id.0
            ))
        })?;
        allowed.extend(
            patch
                .mask
                .iter()
                .copied()
                .filter(|coord| !crystal_mask.contains(coord)),
        );
    }

    let mut components = fine_components(massif_owner_mask);
    let primary_index = components
        .iter()
        .enumerate()
        .max_by_key(|(_, component)| {
            (
                component.len(),
                Reverse(component.first().copied().unwrap_or(HexCoord::ORIGIN)),
            )
        })
        .map(|(index, _)| index)
        .ok_or_else(|| contract("Grand V3 Massif has no fine ownership component"))?;
    let mut connected_body = components.remove(primary_index);
    while !components.is_empty() {
        let goals = components
            .iter()
            .flat_map(|component| component.iter().copied())
            .collect::<BTreeSet<_>>();
        let path = shortest_path_between_sets(&allowed, &connected_body, &goals).ok_or_else(|| {
            contract(format!(
                "Crystal radius-32 claim split the seeded Massif into {} visual components with no overlay-free Mountain connector",
                components.len().saturating_add(1)
            ))
        })?;
        let destination = path
            .last()
            .copied()
            .ok_or_else(|| contract("Grand V3 Massif resolved an empty visual connector"))?;
        let destination_index = components
            .iter()
            .position(|component| component.contains(&destination))
            .ok_or_else(|| {
                contract("Grand V3 Massif connector missed every remaining component")
            })?;
        connected_body.extend(path);
        connected_body.extend(components.remove(destination_index));
    }
    if !massif_owner_mask.is_subset(&connected_body)
        || !connected(&connected_body)
        || !connected_body.is_disjoint(crystal_mask)
    {
        return Err(contract(
            "Grand V3 Massif visual connector did not preserve one Crystal-disjoint body",
        ));
    }
    let cells = plan
        .cells
        .iter()
        .map(|cell| (PatchId(u32::from(cell.id.get())), cell))
        .collect::<BTreeMap<_, _>>();
    let mut connector_owners = BTreeMap::new();
    for coord in connected_body.difference(massif_owner_mask).copied() {
        let owners = layout
            .patches
            .iter()
            .filter_map(|(owner, patch)| patch.mask.contains(&coord).then_some(*owner))
            .collect::<Vec<_>>();
        let [owner] = owners.as_slice() else {
            return Err(contract(format!(
                "Grand V3 Massif visual connector {coord:?} has {} layout owners",
                owners.len()
            )));
        };
        let cell = cells.get(owner).copied().ok_or_else(|| {
            contract(format!(
                "Grand V3 Massif visual connector owner {} has no schematic cell",
                owner.0
            ))
        })?;
        if cell.facts.surface != SurfaceKind::Land
            || cell.facts.landform != LandformKind::Mountain
            || !cell.facts.overlays.is_empty()
            || crystal_mask.contains(&coord)
        {
            return Err(contract(format!(
                "Grand V3 Massif visual connector {coord:?} is not overlay-free Mountain terrain outside Crystal"
            )));
        }
        connector_owners.insert(coord, *owner);
    }
    if connector_owners.keys().any(|coord| {
        coord
            .neighbors()
            .into_iter()
            .all(|neighbor| connected_body.contains(&neighbor))
    }) {
        return Err(contract(
            "Grand V3 Massif visual connector widened into an interior strip instead of remaining a tapered seam",
        ));
    }
    let feather = connected_body
        .iter()
        .flat_map(|coord| coord.within_radius(MASSIF_OUTER_FEATHER_DEPTH))
        .filter(|coord| allowed.contains(coord))
        .filter(|coord| !connected_body.contains(coord))
        .collect::<BTreeSet<_>>();
    let mut feather_owners = BTreeMap::new();
    for coord in &feather {
        let owners = layout
            .patches
            .iter()
            .filter_map(|(owner, patch)| patch.mask.contains(coord).then_some(*owner))
            .collect::<Vec<_>>();
        let [owner] = owners.as_slice() else {
            return Err(contract(format!(
                "Grand V3 Massif outer feather {coord:?} has {} layout owners",
                owners.len()
            )));
        };
        let cell = cells.get(owner).copied().ok_or_else(|| {
            contract(format!(
                "Grand V3 Massif outer-feather owner {} has no schematic cell",
                owner.0
            ))
        })?;
        if cell.facts.surface != SurfaceKind::Land
            || cell.facts.landform != LandformKind::Mountain
            || !cell.facts.overlays.is_empty()
            || crystal_mask.contains(coord)
        {
            return Err(contract(format!(
                "Grand V3 Massif outer feather {coord:?} is not overlay-free Mountain terrain outside Crystal"
            )));
        }
        feather_owners.insert(*coord, *owner);
    }
    let mut visual_mask = connected_body;
    visual_mask.extend(feather);
    if feather_owners.is_empty() || !connected(&visual_mask) {
        return Err(contract(
            "Grand V3 Massif outer feather is empty or disconnected",
        ));
    }
    Ok(MassifVisualAuthority {
        visual_mask,
        semantic_owner_mask: massif_owner_mask.clone(),
        connector_owners,
        feather_owners,
    })
}

fn fine_components(mask: &BTreeSet<HexCoord>) -> Vec<BTreeSet<HexCoord>> {
    let mut remaining = mask.clone();
    let mut components = Vec::new();
    while let Some(start) = remaining.pop_first() {
        let mut component = BTreeSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(current) = queue.pop_front() {
            let mut neighbors = current.neighbors();
            neighbors.sort_unstable();
            for neighbor in neighbors {
                if remaining.remove(&neighbor) {
                    component.insert(neighbor);
                    queue.push_back(neighbor);
                }
            }
        }
        components.push(component);
    }
    components
}

fn shortest_path_between_sets(
    mask: &BTreeSet<HexCoord>,
    sources: &BTreeSet<HexCoord>,
    goals: &BTreeSet<HexCoord>,
) -> Option<Vec<HexCoord>> {
    if sources.is_empty() || goals.is_empty() || !sources.is_subset(mask) || !goals.is_subset(mask)
    {
        return None;
    }
    let mut parent = BTreeMap::<HexCoord, HexCoord>::new();
    let mut visited = sources.clone();
    let mut queue = sources.iter().copied().collect::<VecDeque<_>>();
    let destination = loop {
        let current = queue.pop_front()?;
        if goals.contains(&current) {
            break current;
        }
        let mut neighbors = current.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if mask.contains(&neighbor) && visited.insert(neighbor) {
                parent.insert(neighbor, current);
                queue.push_back(neighbor);
            }
        }
    };
    let mut reversed = vec![destination];
    let mut current = destination;
    while !sources.contains(&current) {
        current = parent.get(&current).copied()?;
        reversed.push(current);
    }
    reversed.reverse();
    Some(reversed)
}

fn schematic_components(
    cells: &BTreeMap<SchematicCoord, LandmarkCell>,
) -> Result<Vec<BTreeSet<SchematicCoord>>, V3GenerationError> {
    let mut remaining = cells.keys().copied().collect::<BTreeSet<_>>();
    let mut components = Vec::new();
    while let Some(start) = remaining.pop_first() {
        let mut component = BTreeSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(current) = queue.pop_front() {
            let neighbors = current
                .neighbors()
                .ok_or_else(|| contract("Grand V3 highland schematic adjacency overflowed"))?;
            for neighbor in neighbors {
                if remaining.remove(&neighbor) {
                    component.insert(neighbor);
                    queue.push_back(neighbor);
                }
            }
        }
        components.push(component);
    }
    components.sort_by_key(|component| component.iter().next().copied());
    Ok(components)
}

fn overlay_free_mountain_owners(
    plan: &SchematicPlanV1,
    layout: &ResolvedLayoutPlan,
) -> Result<BTreeMap<HexCoord, PatchId>, V3GenerationError> {
    let mut owners = BTreeMap::new();
    for cell in &plan.cells {
        if cell.facts.surface != SurfaceKind::Land
            || cell.facts.landform != LandformKind::Mountain
            || !cell.facts.overlays.is_empty()
        {
            continue;
        }
        let patch_id = PatchId(u32::from(cell.id.get()));
        let patch = layout.patches.get(&patch_id).ok_or_else(|| {
            contract(format!(
                "Grand V3 peak feather Mountain patch {} disappeared from the layout",
                patch_id.0
            ))
        })?;
        for coord in &patch.mask {
            if let Some(previous) = owners.insert(*coord, patch_id) {
                return Err(contract(format!(
                    "Grand V3 peak feather coordinate {coord:?} has duplicate Mountain owners {} and {}",
                    previous.0, patch_id.0
                )));
            }
        }
    }
    Ok(owners)
}

fn inner_peak_external_ingress_portals(
    layout: &ResolvedLayoutPlan,
    peak_patch_masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
) -> Result<Option<((PatchId, PatchId), BTreeSet<HexCoord>)>, V3GenerationError> {
    let Some(peak_mask) = peak_patch_masks.get(&INNER_PEAK_INGRESS_PEAK_PATCH) else {
        return Ok(None);
    };
    let frozen_mask = layout
        .patches
        .get(&INNER_PEAK_INGRESS_FROZEN_PATCH)
        .map(|patch| &patch.mask)
        .ok_or_else(|| contract("Grand V3 inner peak ingress lost exact Frozen patch 123"))?;
    let ingress = peak_mask
        .iter()
        .copied()
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .any(|neighbor| frozen_mask.contains(&neighbor))
        })
        .collect::<BTreeSet<_>>();
    if ingress.len() < 4 {
        return Err(contract(format!(
            "Grand V3 external Frozen-123/Peak-88 ingress has fewer than four exact portal columns: {}",
            ingress.len()
        )));
    }
    Ok(Some((
        (
            INNER_PEAK_INGRESS_PEAK_PATCH,
            INNER_PEAK_INGRESS_FROZEN_PATCH,
        ),
        ingress,
    )))
}

fn canonical_inner_peak_external_ingress(
    raw: &BTreeSet<HexCoord>,
) -> Result<BTreeSet<HexCoord>, V3GenerationError> {
    let mut candidates = fine_components(raw)
        .into_iter()
        .filter(|component| component.len() >= 4)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|component| {
        (
            Reverse(component.len()),
            component.iter().next().copied().unwrap_or(HexCoord::ORIGIN),
        )
    });
    candidates.into_iter().next().ok_or_else(|| {
        contract(format!(
            "Grand V3 external Frozen-123/Peak-88 ingress has no canonical connected four-column portal run: raw={}",
            raw.len()
        ))
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InnerPeakIngressProfile {
    /// Exact one-step-reachable target ceiling for the selected seam columns.
    targets: BTreeMap<HexCoord, Level>,
    /// Frozen-halo cap followed by the peak body's nine-level outer shoulder.
    ceilings: BTreeMap<HexCoord, Level>,
}

fn build_inner_peak_ingress_profile(
    patch_mask: &BTreeSet<HexCoord>,
    ingress: &BTreeSet<HexCoord>,
    frozen: &FrozenPlateauField,
    crystal_exit_clearance: &BTreeSet<HexCoord>,
) -> Result<InnerPeakIngressProfile, V3GenerationError> {
    let frozen_mask = frozen.levels.keys().copied().collect::<BTreeSet<_>>();
    let runway_sources = frozen_mask
        .iter()
        .copied()
        .filter(|coord| {
            crystal_exit_clearance.contains(coord)
                || coord
                    .neighbors()
                    .into_iter()
                    .any(|neighbor| crystal_exit_clearance.contains(&neighbor))
        })
        .map(|coord| (coord, frozen.levels[&coord]))
        .collect::<BTreeMap<_, _>>();
    if runway_sources.is_empty() {
        return Err(contract(
            "Grand V3 inner peak ingress has no Frozen plateau runway from the published Crystal exit",
        ));
    }
    let runway_ceiling = propagate_rising_ceiling(&frozen_mask, &runway_sources, 1);
    if runway_ceiling.len() != frozen_mask.len() {
        return Err(contract(
            "Grand V3 inner peak ingress Frozen plateau runway is disconnected",
        ));
    }

    let mut targets = BTreeMap::new();
    for coord in ingress {
        let halo_depth = frozen.halo_distance.get(coord).copied().ok_or_else(|| {
            contract(format!(
                "Grand V3 inner peak ingress {coord:?} is outside the exact Frozen six-row halo"
            ))
        })?;
        let halo_ceiling = FROZEN_PLATEAU_MAX.saturating_add(
            i32::try_from(halo_depth)
                .unwrap_or(i32::MAX)
                .saturating_mul(FROZEN_PLATEAU_MAXIMUM_STEP),
        );
        let walker_ceiling = coord
            .neighbors()
            .into_iter()
            .filter(|neighbor| frozen_mask.contains(neighbor))
            .filter_map(|neighbor| runway_ceiling.get(&neighbor).copied())
            .map(|level| level.saturating_add(1))
            .max()
            .ok_or_else(|| {
                contract(format!(
                    "Grand V3 inner peak ingress {coord:?} has no one-step Frozen runway handoff"
                ))
            })?;
        targets.insert(*coord, halo_ceiling.min(walker_ceiling));
    }
    if targets.len() != ingress.len() {
        return Err(contract(
            "Grand V3 inner peak ingress did not resolve every exact portal target",
        ));
    }

    let mut halo_sources = patch_mask
        .iter()
        .filter_map(|coord| {
            frozen.halo_distance.get(coord).copied().map(|depth| {
                (
                    *coord,
                    FROZEN_PLATEAU_MAX.saturating_add(
                        i32::try_from(depth)
                            .unwrap_or(i32::MAX)
                            .saturating_mul(FROZEN_PLATEAU_MAXIMUM_STEP),
                    ),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    for (coord, target) in &targets {
        halo_sources
            .entry(*coord)
            .and_modify(|ceiling| *ceiling = (*ceiling).min(*target))
            .or_insert(*target);
    }
    let ceilings =
        propagate_rising_ceiling(patch_mask, &halo_sources, PEAK_OUTER_FEATHER_MAXIMUM_STEP);
    if ceilings.len() != patch_mask.len() {
        return Err(contract(
            "Grand V3 inner peak ingress Frozen-halo shoulder did not cover Patch 88",
        ));
    }
    Ok(InnerPeakIngressProfile { targets, ceilings })
}

fn select_inner_peak_summit(
    patch_mask: &BTreeSet<HexCoord>,
    nominal: HexCoord,
    summit: Level,
    ingress_profile: &InnerPeakIngressProfile,
    internal_saddle: &BTreeSet<HexCoord>,
    internal_saddle_ceiling: Level,
    crystal_exit_ceiling: &BTreeMap<HexCoord, Level>,
    seed: u64,
) -> Result<HexCoord, V3GenerationError> {
    let depths = boundary_depth(patch_mask);
    patch_mask
        .iter()
        .copied()
        .filter(|coord| {
            ingress_profile
                .ceilings
                .get(coord)
                .is_some_and(|ceiling| *ceiling >= summit)
        })
        .filter(|coord| {
            crystal_exit_ceiling
                .get(coord)
                .is_none_or(|ceiling| *ceiling >= summit)
        })
        .filter(|coord| {
            internal_saddle
                .iter()
                .map(|saddle| saddle.distance(*coord))
                .min()
                .is_some_and(|distance| {
                    internal_saddle_ceiling.saturating_add(
                        i32::try_from(distance)
                            .unwrap_or(i32::MAX)
                            .saturating_mul(PEAK_OUTER_FEATHER_MAXIMUM_STEP),
                    ) >= summit
                })
        })
        .max_by_key(|coord| {
            (
                depths.get(coord).copied().unwrap_or_default(),
                Reverse(coord.distance(nominal)),
                Reverse(named_sample(
                    seed,
                    "grand_v3.highlands.inner_peak_summit",
                    *coord,
                )),
                Reverse(*coord),
            )
        })
        .ok_or_else(|| {
            contract(format!(
                "Grand V3 Patch-88 has no summit location supporting level {summit} between its Frozen ingress and 88/58 saddle"
            ))
        })
}

fn inner_peak_ingress_lower_body_floors(
    patch_masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    saddle_swaths: &BTreeMap<(PatchId, PatchId), BTreeSet<HexCoord>>,
    saddle_ceilings: &BTreeMap<(PatchId, PatchId), Level>,
    ingress: &BTreeSet<HexCoord>,
    summit: HexCoord,
    levels: &BTreeMap<HexCoord, Level>,
    crown_floor: &BTreeMap<HexCoord, Level>,
    crystal_exit_ceiling: &BTreeMap<HexCoord, Level>,
    ingress_profile: &InnerPeakIngressProfile,
) -> Result<BTreeMap<HexCoord, Level>, V3GenerationError> {
    let patch_mask = patch_masks
        .get(&INNER_PEAK_INGRESS_PEAK_PATCH)
        .ok_or_else(|| contract("Grand V3 inner peak ingress lost Patch 88"))?;
    let internal = saddle_swaths
        .get(&(PatchId(58), INNER_PEAK_INGRESS_PEAK_PATCH))
        .map(|swath| {
            swath
                .intersection(patch_mask)
                .copied()
                .collect::<BTreeSet<_>>()
        })
        .filter(|swath| !swath.is_empty())
        .ok_or_else(|| contract("Grand V3 inner peak ingress lost its 88/58 saddle target"))?;
    let mut internal_components = fine_components(&internal);
    internal_components.sort_by_key(|component| {
        (
            Reverse(component.len()),
            component.iter().next().copied().unwrap_or(HexCoord::ORIGIN),
        )
    });
    let internal_target = internal_components.into_iter().next().ok_or_else(|| {
        contract("Grand V3 inner peak ingress has no connected 88/58 saddle target")
    })?;
    let corridor_mask = patch_mask
        .iter()
        .copied()
        .filter(|coord| summit.distance(*coord) > INNER_PEAK_INGRESS_SUMMIT_CLEARANCE)
        .collect::<BTreeSet<_>>();
    let corridor = lowest_relief_saddle_path(
        &corridor_mask,
        ingress,
        &internal_target,
        summit,
        INNER_PEAK_INGRESS_SUMMIT_CLEARANCE,
        levels,
    )
    .ok_or_else(|| {
        contract("Grand V3 inner peak ingress cannot reserve a lower-body join to the 88/58 saddle")
    })?;
    let reserved_spine = ingress
        .iter()
        .copied()
        .chain(internal_target.iter().copied())
        .chain(corridor)
        .collect::<BTreeSet<_>>();
    if !connected(&reserved_spine) {
        return Err(contract(
            "Grand V3 inner peak ingress portal, join, and saddle target do not form one lower-body spine",
        ));
    }
    let reservation = reserved_spine
        .iter()
        .flat_map(|coord| coord.within_radius(4))
        .filter(|coord| {
            patch_mask.contains(coord)
                && summit.distance(*coord) > INNER_PEAK_INGRESS_SUMMIT_CLEARANCE
        })
        .collect::<BTreeSet<_>>();
    if !reserved_spine.is_subset(&reservation) || !connected(&reservation) {
        return Err(contract(
            "Grand V3 inner peak ingress lower-body reservation is disconnected or omitted a portal column",
        ));
    }

    // The old fixed-radius override assigned the bench floor to every support
    // column and then stopped abruptly.  Where that low support met the
    // retained crown floor, a single edge could jump by more than twenty
    // levels.  Resolve the exact scalar level each reserved column would
    // receive from all authored saddle ceilings, then propagate the minimum
    // nine-level rising envelope through the complete Patch-88 body.  The
    // envelope automatically becomes inert once the retained crown floor is
    // low enough; it therefore expands only as far as the physical transition
    // requires rather than relying on another guessed support radius.
    let mut reservation_targets = BTreeMap::new();
    for coord in &reservation {
        let mut target = levels.get(coord).copied().ok_or_else(|| {
            contract(format!(
                "Grand V3 inner peak ingress support {coord:?} has no authored level"
            ))
        })?;
        target = target.min(
            ingress_profile
                .ceilings
                .get(coord)
                .copied()
                .ok_or_else(|| {
                    contract(format!(
                        "Grand V3 inner peak ingress support {coord:?} lost its Frozen-halo ceiling"
                    ))
                })?,
        );
        for (edge, swath) in saddle_swaths {
            if *edge
                == (
                    INNER_PEAK_INGRESS_PEAK_PATCH,
                    INNER_PEAK_INGRESS_FROZEN_PATCH,
                )
            {
                continue;
            }
            let ceiling = saddle_ceilings.get(edge).copied().ok_or_else(|| {
                contract(format!(
                    "Grand V3 inner peak ingress support lost saddle ceiling {}->{}",
                    edge.0 .0, edge.1 .0
                ))
            })?;
            let distance = swath
                .iter()
                .map(|source| source.distance(*coord))
                .min()
                .ok_or_else(|| contract("Grand V3 inner peak ingress found an empty saddle"))?;
            target = target.min(
                ceiling.saturating_add(
                    i32::try_from(distance)
                        .unwrap_or(i32::MAX)
                        .saturating_mul(PEAK_BODY_SLOPE),
                ),
            );
        }
        if let Some(exit_ceiling) = crystal_exit_ceiling.get(coord).copied() {
            target = target.min(exit_ceiling);
        }
        reservation_targets.insert(*coord, target);
    }
    let shoulder_ceiling = propagate_rising_ceiling(
        patch_mask,
        &reservation_targets,
        PEAK_OUTER_FEATHER_MAXIMUM_STEP,
    );
    if shoulder_ceiling.len() != patch_mask.len() {
        return Err(contract(
            "Grand V3 inner peak ingress shoulder envelope did not cover all of Patch 88",
        ));
    }
    let summit_floor = crown_floor
        .get(&summit)
        .copied()
        .ok_or_else(|| contract("Grand V3 inner peak ingress summit lost its crown floor"))?;
    let structural_review_draft = grand_v3_structural_review_draft_enabled();
    if shoulder_ceiling
        .get(&summit)
        .is_none_or(|ceiling| *ceiling < summit_floor)
    {
        let limiting_source = reservation_targets
            .iter()
            .min_by_key(|(coord, target)| {
                target.saturating_add(
                    i32::try_from(coord.distance(summit))
                        .unwrap_or(i32::MAX)
                        .saturating_mul(PEAK_OUTER_FEATHER_MAXIMUM_STEP),
                )
            })
            .map(|(coord, target)| (*coord, *target, coord.distance(summit)));
        if structural_review_draft {
            eprintln!(
                "Grand V3 structural-review draft: retaining the inner-peak summit before its shoulder taper is complete"
            );
        } else {
            return Err(contract(format!(
                "Grand V3 inner peak ingress shoulder cannot taper into the immutable summit pin: summit={summit:?}, floor={summit_floor}, ceiling={:?}, limiting_source={limiting_source:?}",
                shoulder_ceiling.get(&summit)
            )));
        }
    }
    let mut resolved = patch_mask
        .iter()
        .map(|coord| {
            let retained = crown_floor.get(coord).copied().ok_or_else(|| {
                contract(format!(
                    "Grand V3 inner peak ingress shoulder {coord:?} lost its crown floor"
                ))
            })?;
            let ceiling = shoulder_ceiling.get(coord).copied().ok_or_else(|| {
                contract(format!(
                    "Grand V3 inner peak ingress shoulder {coord:?} lost its taper ceiling"
                ))
            })?;
            Ok((*coord, retained.min(ceiling)))
        })
        .collect::<Result<BTreeMap<_, _>, V3GenerationError>>()?;
    if structural_review_draft {
        resolved.insert(summit, summit_floor);
    }
    if resolved.get(&summit) != Some(&summit_floor) {
        return Err(contract(
            "Grand V3 inner peak ingress shoulder lowered the immutable summit pin",
        ));
    }
    let excessive_taper = (!structural_review_draft).then(|| {
        resolved.iter().find_map(|(coord, floor)| {
            coord.neighbors().into_iter().find_map(|neighbor| {
                resolved.get(&neighbor).and_then(|neighbor_floor| {
                    (floor.abs_diff(*neighbor_floor)
                        > PEAK_OUTER_FEATHER_MAXIMUM_STEP.unsigned_abs())
                    .then_some((*coord, *floor, neighbor, *neighbor_floor))
                })
            })
        })
    });
    if let Some(Some((coord, floor, neighbor, neighbor_floor))) = excessive_taper {
        return Err(contract(format!(
            "Grand V3 inner peak ingress shoulder floor exceeds the nine-level taper at {coord:?} {floor} -> {neighbor:?} {neighbor_floor}"
        )));
    }
    Ok(resolved)
}

fn peak_source_influence(source_level: Level, distance: u32, chisel: Level) -> Level {
    let distance = i32::try_from(distance).unwrap_or(i32::MAX);
    let crown = source_level
        .saturating_sub(distance.saturating_mul(PEAK_BODY_SLOPE))
        .saturating_sub(chisel);
    let lower_body = source_level
        .saturating_sub(PEAK_LOWER_BODY_SOURCE_DROP)
        .saturating_sub(distance.saturating_mul(PEAK_LOWER_BODY_SLOPE))
        .saturating_sub(chisel);
    crown.max(lower_body)
}

/// Rejects a peak chain whose nominal offset lobes disappear beneath the
/// primary summit cones or the later saddle/clearance caps.
///
/// `without_upper_lobes` is deliberately the uncapped scalar field produced by
/// the same primary summits and lower shoulder sources. Comparing the final
/// accepted ridge against that stronger counterfactual is conservative: every
/// reported witness must be a surviving, material contribution from an upper
/// lobe rather than an artifact of a lowered saddle.
fn validate_peak_upper_lobe_contribution(
    patch_masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    final_levels: &BTreeMap<HexCoord, Level>,
    without_upper_lobes: &BTreeMap<HexCoord, Level>,
) -> Result<(), V3GenerationError> {
    let witness_floor = PEAK_VISUAL_WALL_THRESHOLD.saturating_sub(12);
    let witnesses = final_levels
        .iter()
        .filter_map(|(coord, level)| {
            without_upper_lobes
                .get(coord)
                .is_some_and(|counterfactual| {
                    *level >= witness_floor && *level >= counterfactual.saturating_add(2)
                })
                .then_some(*coord)
        })
        .collect::<BTreeSet<_>>();
    let witnessed_patches = patch_masks
        .values()
        .filter(|mask| mask.iter().any(|coord| witnesses.contains(coord)))
        .count();
    let minimum_witnesses = patch_masks.len().max(6);
    let minimum_patches = patch_masks.len().saturating_add(1) / 2;
    if witnesses.len() < minimum_witnesses || witnessed_patches < minimum_patches {
        return Err(contract(format!(
            "Grand V3 peak chain upper lobes do not materially break the primary summit cones: witnesses={}, required={minimum_witnesses}, patches={witnessed_patches}, required_patches={minimum_patches}",
            witnesses.len()
        )));
    }
    Ok(())
}

fn build_peak_field(
    cells: &BTreeMap<SchematicCoord, LandmarkCell>,
    components: &[BTreeSet<SchematicCoord>],
    layout: &ResolvedLayoutPlan,
    profile: V3GrandV3BasicTerrainProfile,
    seed: u64,
    frozen: &FrozenPlateauField,
    crystal_exit_clearance: &BTreeSet<HexCoord>,
    crystal_exit_ceiling: &BTreeMap<HexCoord, Level>,
    mountain_feather_owners: &BTreeMap<HexCoord, PatchId>,
    shared_highland_mask: &BTreeSet<HexCoord>,
) -> Result<
    (
        BTreeMap<HexCoord, Level>,
        BTreeMap<SchematicCoord, BTreeMap<HexCoord, Level>>,
        Vec<PeakFeatherField>,
        Vec<BTreeMap<(PatchId, PatchId), BTreeSet<HexCoord>>>,
        BTreeMap<PatchId, (HexCoord, Level)>,
    ),
    V3GenerationError,
> {
    if PEAK_SUMMIT_MAX >= MASSIF_SUMMIT_MIN
        || profile.sharp_peak_bench_max >= PEAK_VISUAL_WALL_THRESHOLD
    {
        return Err(contract(
            "Grand V3 connected peak chains have an invalid height hierarchy",
        ));
    }
    let summit_span = PEAK_SUMMIT_MAX
        .saturating_sub(PEAK_SUMMIT_MIN)
        .saturating_add(1);
    let summit_by_cell = cells
        .iter()
        .map(|(schematic, cell)| {
            let sampled_summit = PEAK_SUMMIT_MIN.saturating_add(
                i32::try_from(
                    named_sample(seed, "grand_v3.highlands.peak_summits", cell.representative)
                        % u64::try_from(summit_span).unwrap_or(1),
                )
                .unwrap_or_default(),
            );
            // Patch 88 owns the only ordinary Frozen-to-peak ledge. Keep its
            // crown at the valid lower end of the authored peak range so the
            // level-218 scenic saddle can taper into it without a cliff or a
            // synthetic spiral. The other eleven crowns retain full seeded
            // 260..=300 variation and continue to enclose the mountain lake.
            let summit = if cell.patch == INNER_PEAK_INGRESS_PEAK_PATCH {
                PEAK_SUMMIT_MIN
            } else {
                sampled_summit
            };
            (*schematic, summit)
        })
        .collect::<BTreeMap<_, _>>();

    let summit_by_patch = cells
        .iter()
        .map(|(schematic, cell)| {
            summit_by_cell
                .get(schematic)
                .copied()
                .map(|summit| (cell.patch, summit))
                .ok_or_else(|| contract("Grand V3 peak cell lost its seeded summit"))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let mut levels = BTreeMap::<HexCoord, Level>::new();
    let mut peak_bodies = BTreeMap::new();
    let mut peak_feathers = Vec::with_capacity(components.len());
    let mut peak_external_ingresses = Vec::with_capacity(components.len());
    let mut peak_summit_pins = BTreeMap::new();
    for component in components {
        let mut patch_masks = BTreeMap::new();
        for schematic in component {
            let cell = cells.get(schematic).ok_or_else(|| {
                contract("Grand V3 peak component references an absent authored cell")
            })?;
            let patch = layout.patches.get(&cell.patch).ok_or_else(|| {
                contract(format!(
                    "Grand V3 connected peak chain lost resolved patch {}",
                    cell.patch.0
                ))
            })?;
            if patch_masks.insert(cell.patch, patch.mask.clone()).is_some() {
                return Err(contract("Grand V3 peak chain assigned one patch twice"));
            }
        }
        let component_mask = patch_masks
            .values()
            .flat_map(|mask| mask.iter().copied())
            .collect::<BTreeSet<_>>();
        if !connected(&component_mask) {
            return Err(contract(
                "Grand V3 peak-chain ownership is not one connected lower body",
            ));
        }
        let mut saddle_swaths = build_peak_saddle_swaths(&patch_masks)?;
        let external_ingress = inner_peak_external_ingress_portals(layout, &patch_masks)?
            .map(|(edge, raw)| {
                canonical_inner_peak_external_ingress(&raw).map(|ingress| (edge, ingress))
            })
            .transpose()?;
        let external_ingress_profile = external_ingress
            .as_ref()
            .map(|(_, ingress)| {
                let patch_mask = patch_masks
                    .get(&INNER_PEAK_INGRESS_PEAK_PATCH)
                    .ok_or_else(|| contract("Grand V3 inner peak ingress lost Patch 88"))?;
                build_inner_peak_ingress_profile(
                    patch_mask,
                    ingress,
                    frozen,
                    crystal_exit_clearance,
                )
            })
            .transpose()?;

        let mut summit_coord_by_patch = component
            .iter()
            .map(|schematic| {
                let cell = cells
                    .get(schematic)
                    .ok_or_else(|| contract("Grand V3 peak component lost one summit owner"))?;
                Ok((cell.patch, cell.representative))
            })
            .collect::<Result<BTreeMap<_, _>, V3GenerationError>>()?;
        if let (Some((_, _)), Some(ingress_profile)) =
            (&external_ingress, &external_ingress_profile)
        {
            let patch_mask = patch_masks
                .get(&INNER_PEAK_INGRESS_PEAK_PATCH)
                .ok_or_else(|| contract("Grand V3 inner peak summit lost Patch 88"))?;
            let internal_saddle = saddle_swaths
                .get(&(PatchId(58), INNER_PEAK_INGRESS_PEAK_PATCH))
                .map(|swath| {
                    swath
                        .intersection(patch_mask)
                        .copied()
                        .collect::<BTreeSet<_>>()
                })
                .filter(|swath| !swath.is_empty())
                .ok_or_else(|| contract("Grand V3 inner peak summit lost its 88/58 saddle"))?;
            let summit = summit_by_patch[&INNER_PEAK_INGRESS_PEAK_PATCH];
            let nominal = summit_coord_by_patch[&INNER_PEAK_INGRESS_PEAK_PATCH];
            let internal_ceiling = peak_saddle_ceiling(
                (PatchId(58), INNER_PEAK_INGRESS_PEAK_PATCH),
                &summit_by_patch,
            )?;
            let selected = select_inner_peak_summit(
                patch_mask,
                nominal,
                summit,
                ingress_profile,
                &internal_saddle,
                internal_ceiling,
                crystal_exit_ceiling,
                seed,
            )?;
            summit_coord_by_patch.insert(INNER_PEAK_INGRESS_PEAK_PATCH, selected);
        }
        let mut sources = Vec::new();
        let mut sources_without_upper_lobes = Vec::new();
        for schematic in component {
            let cell = cells.get(schematic).ok_or_else(|| {
                contract("Grand V3 peak component references an absent authored cell")
            })?;
            let summit = summit_by_cell
                .get(schematic)
                .copied()
                .ok_or_else(|| contract("Grand V3 peak component omitted one summit level"))?;
            let summit_coord = summit_coord_by_patch[&cell.patch];
            if peak_summit_pins
                .insert(cell.patch, (summit_coord, summit))
                .is_some()
            {
                return Err(contract("Grand V3 peak summit pin was assigned twice"));
            }
            let primary_direction = usize::try_from(
                named_sample(seed, "grand_v3.highlands.peak_body_axis", summit_coord) % 6,
            )
            .unwrap_or_default();
            let shoulder_a_direction = (primary_direction + 1) % 6;
            let shoulder_b_direction = (primary_direction + 5) % 6;
            let shoulder_a_radius = 7_u32.saturating_add(
                u32::try_from(
                    named_sample(
                        seed,
                        "grand_v3.highlands.peak_body_shoulder_a_radius",
                        summit_coord,
                    ) % 4,
                )
                .unwrap_or_default(),
            );
            let shoulder_b_radius = 8_u32.saturating_add(
                u32::try_from(
                    named_sample(
                        seed,
                        "grand_v3.highlands.peak_body_shoulder_b_radius",
                        summit_coord,
                    ) % 5,
                )
                .unwrap_or_default(),
            );
            let patch_mask = patch_masks
                .get(&cell.patch)
                .ok_or_else(|| contract("Grand V3 peak lobe lost its patch mask"))?;
            let mut upper_sources = vec![(summit_coord, summit)];
            sources_without_upper_lobes.push((summit_coord, summit));
            // Give the apex a small asymmetric set of attached facets before
            // the longer shoulder lobes are applied. A lone seven-level cone
            // left only one or two columns within eight levels of the summit
            // and produced the visually perfect pyramids called out in
            // review. These three nearby, sub-summit sources broaden that top
            // without adding another summit pin or touching the lower body and
            // saddle fields. Their uneven directions and drops also prevent a
            // rotationally regular upper silhouette.
            for (facet, (direction_offset, drop)) in
                [(0_usize, 3_i32), (2, 5), (5, 7)].into_iter().enumerate()
            {
                let direction = (primary_direction + direction_offset) % 6;
                let target = step_in_direction(summit_coord, direction, 1);
                let source = patch_mask
                    .iter()
                    .copied()
                    .filter(|candidate| {
                        (1..=2).contains(&summit_coord.distance(*candidate))
                            && upper_sources
                                .iter()
                                .all(|(existing, _)| existing != candidate)
                    })
                    .min_by_key(|candidate| {
                        (
                            candidate.distance(target),
                            Reverse(named_sample(
                                seed,
                                "grand_v3.highlands.peak_apex_facet_source",
                                *candidate,
                            )),
                            *candidate,
                        )
                    })
                    .ok_or_else(|| {
                        contract(format!(
                            "Grand V3 peak patch {} cannot place apex facet {facet}",
                            cell.patch.0
                        ))
                    })?;
                upper_sources.push((source, summit.saturating_sub(drop)));
            }
            for lobe in 0..4_usize {
                let direction = (primary_direction + lobe.saturating_mul(2) + (lobe / 3)) % 6;
                let sample_coord = step_in_direction(
                    summit_coord,
                    direction,
                    3_u32.saturating_add(u32::try_from(lobe).unwrap_or_default()),
                );
                let radius = 3_u32.saturating_add(
                    u32::try_from(
                        named_sample(
                            seed,
                            "grand_v3.highlands.peak_upper_lobe_radius",
                            sample_coord,
                        ) % 6,
                    )
                    .unwrap_or_default(),
                );
                let target = step_in_direction(summit_coord, direction, radius);
                let source = patch_mask
                    .iter()
                    .copied()
                    .filter(|candidate| {
                        (3..=9).contains(&summit_coord.distance(*candidate))
                            && upper_sources
                                .iter()
                                .all(|(existing, _)| existing != candidate)
                    })
                    .min_by_key(|candidate| {
                        (
                            candidate.distance(target),
                            Reverse(named_sample(
                                seed,
                                "grand_v3.highlands.peak_upper_lobe_source",
                                *candidate,
                            )),
                            *candidate,
                        )
                    })
                    .ok_or_else(|| {
                        contract(format!(
                            "Grand V3 peak patch {} cannot place upper lobe {lobe}",
                            cell.patch.0
                        ))
                    })?;
                let drop = 4_i32.saturating_add(
                    i32::try_from(
                        named_sample(seed, "grand_v3.highlands.peak_upper_lobe_drop", source) % 18,
                    )
                    .unwrap_or_default(),
                );
                // Keep every offset lobe below the distinct high-crown band.
                // The lobe still deforms the broad visible shoulder up to
                // level 239, but it cannot grow a detached >=240 island or a
                // propagated floor that closes the low V-saddle route around
                // the summit pin. One authored peak patch therefore retains
                // one upper crown while its sub-crown silhouette is irregular.
                upper_sources.push((
                    source,
                    summit
                        .saturating_sub(drop)
                        .min(PEAK_VISUAL_WALL_THRESHOLD.saturating_sub(1)),
                ));
            }
            sources.extend(upper_sources);
            let lower_shoulders = [
                (
                    step_in_direction(summit_coord, shoulder_a_direction, shoulder_a_radius),
                    summit
                        .saturating_sub(38)
                        .min(PEAK_VISUAL_WALL_THRESHOLD.saturating_sub(1)),
                ),
                (
                    step_in_direction(summit_coord, shoulder_b_direction, shoulder_b_radius),
                    summit
                        .saturating_sub(47)
                        .min(PEAK_VISUAL_WALL_THRESHOLD.saturating_sub(1)),
                ),
            ];
            sources.extend(lower_shoulders);
            sources_without_upper_lobes.extend(lower_shoulders);
        }
        let depths = boundary_depth(&component_mask);
        let bench_span = profile
            .sharp_peak_bench_max
            .saturating_sub(profile.sharp_peak_bench_min);
        let base_levels = component_mask
            .iter()
            .copied()
            .map(|coord| {
                let depth_raise = i32::try_from(depths.get(&coord).copied().unwrap_or_default())
                    .unwrap_or(i32::MAX)
                    .saturating_mul(2)
                    .min(bench_span);
                let base = profile
                    .sharp_peak_bench_min
                    .saturating_add(depth_raise)
                    .saturating_add(
                        i32::try_from(
                            named_sample(seed, "grand_v3.highlands.peak_chain_base", coord) % 2,
                        )
                        .unwrap_or_default(),
                    )
                    .min(profile.sharp_peak_bench_max);
                (coord, base)
            })
            .collect::<BTreeMap<_, _>>();
        let resolve_sources = |candidate_sources: &[(HexCoord, Level)]| {
            component_mask
                .iter()
                .copied()
                .map(|coord| {
                    let base = base_levels[&coord];
                    let crown = candidate_sources
                        .iter()
                        .map(|(source, source_level)| {
                            let chisel = if *source == coord {
                                0
                            } else {
                                i32::try_from(
                                    named_sample(
                                        seed,
                                        "grand_v3.highlands.peak_chain_faces",
                                        coord,
                                    ) % 2,
                                )
                                .unwrap_or_default()
                            };
                            peak_source_influence(*source_level, source.distance(coord), chisel)
                        })
                        .max()
                        .unwrap_or(base);
                    (coord, base.max(crown).min(PEAK_SUMMIT_MAX))
                })
                .collect::<BTreeMap<_, _>>()
        };
        let mut component_levels = resolve_sources(&sources);
        let component_without_upper_lobes = resolve_sources(&sources_without_upper_lobes);
        // A saddle may cut the lower body but may not leave a retained upper
        // crown standing on a newly lowered column. Propagate a nine-level
        // floor outward from the complete >=260 crown core. Combining that
        // floor with the saddle ceiling produces a true V-shaped grade; when
        // the floor is above 239 the route solver correctly treats the crown
        // shoulder as unavailable.
        let crown_core = component_levels
            .iter()
            .filter_map(|(coord, level)| (*level >= PEAK_SUMMIT_MIN).then_some((*coord, *level)))
            .collect::<BTreeMap<_, _>>();
        let crown_floor = component_levels
            .iter()
            .map(|(coord, original)| {
                let floor = crown_core
                    .iter()
                    .map(|(source, source_level)| {
                        source_level.saturating_sub(
                            i32::try_from(source.distance(*coord))
                                .unwrap_or(i32::MAX)
                                .saturating_mul(PEAK_OUTER_FEATHER_MAXIMUM_STEP),
                        )
                    })
                    .max()
                    .unwrap_or(Level::MIN)
                    .min(*original);
                (*coord, floor)
            })
            .collect::<BTreeMap<_, _>>();
        if let Some((edge, swath)) = &external_ingress {
            if saddle_swaths.insert(*edge, swath.clone()).is_some() {
                return Err(contract(
                    "Grand V3 external Frozen-123/Peak-88 ingress duplicated a peak saddle",
                ));
            }
        }
        let mut saddle_ceilings = BTreeMap::new();
        for (first_patch, second_patch) in saddle_swaths.keys() {
            let saddle_ceiling = if external_ingress
                .as_ref()
                .is_some_and(|(edge, _)| edge == &(*first_patch, *second_patch))
            {
                external_ingress_profile
                    .as_ref()
                    .and_then(|profile| profile.targets.values().copied().max())
                    .ok_or_else(|| {
                        contract("Grand V3 external inner-peak ingress lost its derived ceiling")
                    })?
            } else if external_ingress.is_some()
                && (*first_patch, *second_patch) == (PatchId(58), INNER_PEAK_INGRESS_PEAK_PATCH)
            {
                INNER_PEAK_ROUTE_SADDLE_CEILING
            } else {
                summit_by_patch[first_patch]
                    .min(summit_by_patch[second_patch])
                    .saturating_sub(PEAK_SADDLE_DEPTH)
                    .min(PEAK_VISUAL_WALL_THRESHOLD.saturating_sub(1))
            };
            saddle_ceilings.insert((*first_patch, *second_patch), saddle_ceiling);
        }
        let mut saddle_crown_floor = crown_floor.clone();
        if let Some((_, ingress)) = &external_ingress {
            let summit = summit_coord_by_patch
                .get(&INNER_PEAK_INGRESS_PEAK_PATCH)
                .copied()
                .ok_or_else(|| contract("Grand V3 inner peak ingress lost its summit pin"))?;
            let shoulder_floors = inner_peak_ingress_lower_body_floors(
                &patch_masks,
                &saddle_swaths,
                &saddle_ceilings,
                ingress,
                summit,
                &component_levels,
                &crown_floor,
                crystal_exit_ceiling,
                external_ingress_profile.as_ref().ok_or_else(|| {
                    contract("Grand V3 inner peak ingress lost its Frozen-halo profile")
                })?,
            )?;
            saddle_crown_floor.extend(shoulder_floors);
        }
        for ((first_patch, second_patch), swath) in &saddle_swaths {
            let is_external = external_ingress
                .as_ref()
                .is_some_and(|(edge, _)| edge == &(*first_patch, *second_patch));
            let saddle_ceiling = saddle_ceilings
                .get(&(*first_patch, *second_patch))
                .copied()
                .ok_or_else(|| {
                    contract(format!(
                        "Grand V3 peak saddle {}->{} lost its exact ceiling",
                        first_patch.0, second_patch.0
                    ))
                })?;
            for (coord, level) in &mut component_levels {
                let cap = if is_external {
                    external_ingress_profile
                        .as_ref()
                        .and_then(|profile| profile.ceilings.get(coord).copied())
                        .unwrap_or(*level)
                } else {
                    let distance = swath
                        .iter()
                        .map(|saddle| coord.distance(*saddle))
                        .min()
                        .unwrap_or(u32::MAX);
                    saddle_ceiling.saturating_add(
                        i32::try_from(distance)
                            .unwrap_or(i32::MAX)
                            .saturating_mul(PEAK_BODY_SLOPE),
                    )
                };
                let floor = saddle_crown_floor.get(coord).copied().unwrap_or(*level);
                *level = (*level).min(cap).max(floor);
            }
        }
        connect_peak_patch_saddle_groups(
            &patch_masks,
            &saddle_swaths,
            &saddle_ceilings,
            &summit_coord_by_patch,
            &saddle_crown_floor,
            &mut component_levels,
        )?;
        peak_external_ingresses.push(external_ingress.clone().into_iter().collect());
        for (coord, level) in &mut component_levels {
            if let Some(ceiling) = crystal_exit_ceiling.get(coord).copied() {
                let floor = saddle_crown_floor.get(coord).copied().unwrap_or(*level);
                *level = (*level).min(ceiling).max(floor);
            }
        }
        for schematic in component {
            let cell = cells
                .get(schematic)
                .ok_or_else(|| contract("Grand V3 peak chain lost one summit owner"))?;
            let summit = summit_by_cell[schematic];
            let summit_coord = summit_coord_by_patch[&cell.patch];
            if crystal_exit_ceiling
                .get(&summit_coord)
                .is_some_and(|ceiling| *ceiling < summit)
            {
                return Err(contract(format!(
                    "Grand V3 Crystal-to-Frozen opening intersects peak summit {:?}",
                    summit_coord
                )));
            }
            component_levels.insert(summit_coord, summit);
        }
        // Crown pins are restored after the first scalar saddle pass. Re-run
        // the deterministic connector against that final immutable geometry
        // so its low component cannot depend on temporarily lowered crown
        // neighbors.
        connect_peak_patch_saddle_groups(
            &patch_masks,
            &saddle_swaths,
            &saddle_ceilings,
            &summit_coord_by_patch,
            &saddle_crown_floor,
            &mut component_levels,
        )?;
        validate_peak_upper_lobe_contribution(
            &patch_masks,
            &component_levels,
            &component_without_upper_lobes,
        )?;
        for schematic in component {
            let cell = cells
                .get(schematic)
                .ok_or_else(|| contract("Grand V3 peak chain lost one summit owner"))?;
            let summit = summit_by_cell[schematic];
            let summit_coord = summit_coord_by_patch[&cell.patch];
            let patch_mask = patch_masks
                .get(&cell.patch)
                .ok_or_else(|| contract("Grand V3 peak chain lost one patch mask"))?;
            let body = patch_mask
                .iter()
                .filter_map(|coord| {
                    component_levels
                        .get(coord)
                        .copied()
                        .map(|level| (*coord, level))
                })
                .collect::<BTreeMap<_, _>>();
            if body.len() != patch_mask.len()
                || body.get(&summit_coord) != Some(&summit)
                || peak_bodies.insert(*schematic, body).is_some()
            {
                return Err(contract(format!(
                    "Grand V3 peak {schematic:?} lost its stable portion of the connected chain"
                )));
            }
        }
        let excessive_local_slope = (!grand_v3_structural_review_draft_enabled()).then(|| {
            component_levels.iter().find_map(|(coord, level)| {
                coord
                    .neighbors()
                    .into_iter()
                    .any(|neighbor| {
                        component_levels
                            .get(&neighbor)
                            .is_some_and(|neighbor_level| level.abs_diff(*neighbor_level) > 9)
                    })
                    .then(|| {
                        coord.neighbors().into_iter().find_map(|neighbor| {
                            component_levels.get(&neighbor).and_then(|neighbor_level| {
                                (level.abs_diff(*neighbor_level) > 9).then_some((
                                    *coord,
                                    *level,
                                    neighbor,
                                    *neighbor_level,
                                ))
                            })
                        })
                    })
                    .flatten()
            })
        });
        if let Some(Some((coord, level, neighbor, neighbor_level))) = excessive_local_slope {
            return Err(contract(format!(
                "Grand V3 connected peak chain exceeds the nine-level local-slope budget at {coord:?} {level} -> {neighbor:?} {neighbor_level}"
            )));
        }
        let feather = build_peak_feather(
            &component_mask,
            &component_levels,
            mountain_feather_owners,
            shared_highland_mask,
        )?;
        if peak_feathers.iter().any(|existing: &PeakFeatherField| {
            existing
                .contributions
                .keys()
                .any(|coord| feather.contributions.contains_key(coord))
        }) {
            return Err(contract("Grand V3 peak-chain Mountain feathers overlap"));
        }
        peak_feathers.push(feather);
        levels.extend(component_levels);
    }
    Ok((
        levels,
        peak_bodies,
        peak_feathers,
        peak_external_ingresses,
        peak_summit_pins,
    ))
}

fn build_peak_feather(
    component_mask: &BTreeSet<HexCoord>,
    component_levels: &BTreeMap<HexCoord, Level>,
    mountain_owners: &BTreeMap<HexCoord, PatchId>,
    shared_highland_mask: &BTreeSet<HexCoord>,
) -> Result<PeakFeatherField, V3GenerationError> {
    let candidate_mask = component_mask
        .iter()
        .flat_map(|coord| coord.within_radius(PEAK_OUTER_FEATHER_DEPTH))
        .filter(|coord| mountain_owners.contains_key(coord))
        .collect::<BTreeSet<_>>();
    let feather_mask = fine_components(&candidate_mask)
        .into_iter()
        .filter(|candidate| {
            candidate.iter().any(|coord| {
                coord
                    .neighbors()
                    .into_iter()
                    .any(|neighbor| component_mask.contains(&neighbor))
            })
        })
        .flatten()
        .collect::<BTreeSet<_>>();
    if feather_mask.is_empty() {
        return Err(contract(
            "Grand V3 peak chain has no overlay-free Mountain feather",
        ));
    }

    let inner_sources = feather_mask
        .iter()
        .copied()
        .filter_map(|coord| {
            let adjacent_levels = coord
                .neighbors()
                .into_iter()
                .filter_map(|neighbor| component_levels.get(&neighbor).copied())
                .collect::<Vec<_>>();
            let maximum = adjacent_levels.iter().copied().max()?;
            let minimum = adjacent_levels.iter().copied().min().unwrap_or(maximum);
            (maximum.abs_diff(minimum)
                <= PEAK_OUTER_FEATHER_MAXIMUM_STEP
                    .unsigned_abs()
                    .saturating_mul(2))
            .then_some((
                coord,
                maximum.saturating_sub(PEAK_OUTER_FEATHER_MAXIMUM_STEP),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    if inner_sources.is_empty() {
        return Err(contract(
            "Grand V3 peak-chain Mountain feather has no exact inner seam",
        ));
    }
    let propagated = propagate_influence(
        &feather_mask,
        &inner_sources,
        PEAK_OUTER_FEATHER_MAXIMUM_STEP,
        Level::MIN,
    );
    let shared_boundary = feather_mask
        .iter()
        .copied()
        .filter(|coord| {
            coord.neighbors().into_iter().any(|neighbor| {
                mountain_owners.contains_key(&neighbor)
                    && !feather_mask.contains(&neighbor)
                    && !component_mask.contains(&neighbor)
            })
        })
        .collect::<BTreeSet<_>>();
    let outer_depth = distances_from(&feather_mask, shared_boundary.iter().copied());
    if shared_boundary.is_empty() {
        return Err(contract(format!(
            "Grand V3 peak-chain Mountain feather has no outer connection to shared Mountain terrain: feather={}, shared={}",
            feather_mask.len(),
            shared_boundary.len(),
        )));
    }
    let contributions = feather_mask
        .iter()
        .copied()
        .map(|coord| {
            let target = propagated.get(&coord).copied().unwrap_or(Level::MIN);
            // A Mountain pocket between PeakRing and the separately resolved
            // Massif has no lower shared-Mountain edge. Keep the propagated
            // target there; the final Peak↔Massif edge validator proves the
            // two highland fields meet without a cliff.
            let depth = outer_depth
                .get(&coord)
                .copied()
                .unwrap_or(PEAK_OUTER_FEATHER_DEPTH);
            (
                coord,
                PeakFeatherContribution {
                    target,
                    outer_depth: depth,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let owners = feather_mask
        .iter()
        .filter_map(|coord| {
            mountain_owners
                .get(coord)
                .copied()
                .map(|owner| (*coord, owner))
        })
        .collect::<BTreeMap<_, _>>();
    if owners.len() != feather_mask.len() {
        return Err(contract(
            "Grand V3 peak-chain Mountain feather lost stable ownership",
        ));
    }

    let mut boundary_edges = BTreeSet::new();
    for coord in &feather_mask {
        for neighbor in coord.neighbors() {
            let crosses_inner = component_mask.contains(&neighbor);
            let crosses_outer = !feather_mask.contains(&neighbor)
                && (mountain_owners.contains_key(&neighbor)
                    || shared_highland_mask.contains(&neighbor));
            if crosses_inner || crosses_outer {
                boundary_edges.insert(canonical_edge(*coord, neighbor));
            }
        }
    }
    let has_inner = boundary_edges
        .iter()
        .any(|(first, second)| component_mask.contains(first) || component_mask.contains(second));
    let has_outer = boundary_edges.iter().any(|(first, second)| {
        !component_mask.contains(first)
            && !component_mask.contains(second)
            && (feather_mask.contains(first) ^ feather_mask.contains(second))
    });
    if !has_inner || !has_outer {
        return Err(contract(format!(
            "Grand V3 peak-chain Mountain feather lacks a complete inner/outer grade: inner={has_inner}, outer={has_outer}"
        )));
    }
    Ok(PeakFeatherField {
        contributions,
        owners,
        boundary_edges,
    })
}

fn canonical_edge(first: HexCoord, second: HexCoord) -> (HexCoord, HexCoord) {
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
}

fn build_peak_saddle_swaths(
    patch_masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
) -> Result<BTreeMap<(PatchId, PatchId), BTreeSet<HexCoord>>, V3GenerationError> {
    let patches = patch_masks.keys().copied().collect::<Vec<_>>();
    let mut swaths = BTreeMap::new();
    for (index, first_patch) in patches.iter().copied().enumerate() {
        for second_patch in patches.iter().copied().skip(index.saturating_add(1)) {
            let first_mask = &patch_masks[&first_patch];
            let second_mask = &patch_masks[&second_patch];
            let touches = first_mask.iter().any(|coord| {
                coord
                    .neighbors()
                    .into_iter()
                    .any(|neighbor| second_mask.contains(&neighbor))
            });
            if !touches {
                continue;
            }
            let first_edge = first_mask
                .iter()
                .copied()
                .filter(|coord| {
                    coord
                        .neighbors()
                        .into_iter()
                        .any(|neighbor| second_mask.contains(&neighbor))
                })
                .collect::<BTreeSet<_>>();
            let second_edge = second_mask
                .iter()
                .copied()
                .filter(|coord| {
                    coord
                        .neighbors()
                        .into_iter()
                        .any(|neighbor| first_mask.contains(&neighbor))
                })
                .collect::<BTreeSet<_>>();
            let swath = first_edge
                .union(&second_edge)
                .copied()
                .collect::<BTreeSet<_>>();
            if swath.len() < 4 || !connected(&swath) {
                return Err(contract(format!(
                    "Grand V3 adjacent peaks {} and {} have only {} connected seam columns",
                    first_patch.0,
                    second_patch.0,
                    swath.len()
                )));
            }
            swaths.insert((first_patch, second_patch), swath);
        }
    }
    Ok(swaths)
}

/// Joins every pair of low seam swaths that enters the same intermediate peak
/// patch before ordinary route construction begins.
///
/// Capping each inter-patch seam independently can leave two healthy saddles
/// in different sub-240 components when an upper-crown spur reaches the edge
/// between them. The route compiler must not be responsible for cutting that
/// authored mountain defect. Instead, choose the deterministic minimum-relief
/// path inside the shared patch and extend the same seven-level natural slope
/// used by the peak body around a shallow V-shaped centreline.
fn connect_peak_patch_saddle_groups(
    patch_masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    saddle_swaths: &BTreeMap<(PatchId, PatchId), BTreeSet<HexCoord>>,
    saddle_ceilings: &BTreeMap<(PatchId, PatchId), Level>,
    summit_coords: &BTreeMap<PatchId, HexCoord>,
    crown_floor: &BTreeMap<HexCoord, Level>,
    levels: &mut BTreeMap<HexCoord, Level>,
) -> Result<(), V3GenerationError> {
    let chain_mask = patch_masks
        .values()
        .flat_map(|mask| mask.iter().copied())
        .collect::<BTreeSet<_>>();
    for (patch, patch_mask) in patch_masks {
        // An apparent low route around the outside edge of a peak patch is not
        // a saddle: it leaves a 90-level column above the neighboring Mountain
        // cell. Keep internal connectors one full row inside the complete
        // six-peak body. Inter-peak seams remain eligible because both sides
        // belong to `chain_mask`.
        let corridor_mask = patch_mask
            .iter()
            .copied()
            .filter(|coord| {
                coord
                    .neighbors()
                    .into_iter()
                    .all(|neighbor| chain_mask.contains(&neighbor))
                    && crown_floor
                        .get(coord)
                        .is_some_and(|floor| *floor < PEAK_VISUAL_WALL_THRESHOLD)
            })
            .collect::<BTreeSet<_>>();
        let mut groups = saddle_swaths
            .iter()
            .filter_map(|(edge, swath)| {
                (edge.0 == *patch || edge.1 == *patch).then(|| {
                    let local = swath
                        .intersection(patch_mask)
                        .filter(|coord| corridor_mask.contains(coord))
                        .copied()
                        .collect::<BTreeSet<_>>();
                    (*edge, local, saddle_ceilings[edge])
                })
            })
            .filter(|(_, local, _)| !local.is_empty())
            .collect::<Vec<_>>();
        groups.sort_by_key(|(edge, _, _)| *edge);
        if groups.len() < 2 {
            continue;
        }
        let summit = summit_coords.get(patch).copied().ok_or_else(|| {
            contract(format!(
                "Grand V3 peak patch {} has saddle groups but no summit pin",
                patch.0
            ))
        })?;
        let mut joined = groups
            .first()
            .map(|(_, group, _)| group.clone())
            .unwrap_or_default();
        let mut joined_ceiling = groups
            .first()
            .map(|(_, _, ceiling)| *ceiling)
            .unwrap_or(PEAK_VISUAL_WALL_THRESHOLD.saturating_sub(1));
        for (_, target, target_ceiling) in groups.iter().skip(1) {
            let low = corridor_mask
                .iter()
                .copied()
                .filter(|coord| {
                    levels
                        .get(coord)
                        .is_some_and(|level| *level < PEAK_VISUAL_WALL_THRESHOLD)
                })
                .collect::<BTreeSet<_>>();
            if let Some(component) = fine_components(&low)
                .into_iter()
                .find(|component| component.iter().any(|coord| joined.contains(coord)))
            {
                joined = component;
            }
            if target.iter().all(|coord| joined.contains(coord)) {
                joined_ceiling = joined_ceiling.min(*target_ceiling);
                continue;
            }
            let corridor = lowest_relief_saddle_path(
                &corridor_mask,
                &joined,
                target,
                summit,
                2,
                levels,
            )
            .ok_or_else(|| {
                contract(format!(
                    "Grand V3 peak patch {} cannot connect its low saddle seam groups without crossing the summit pin",
                    patch.0
                ))
            })?;
            let ceiling = joined_ceiling
                .min(*target_ceiling)
                .min(PEAK_VISUAL_WALL_THRESHOLD.saturating_sub(1));
            let last = corridor.len().saturating_sub(1);
            let corridor_sources = corridor
                .iter()
                .enumerate()
                .map(|(index, coord)| {
                    let v_depth = index.min(last.saturating_sub(index));
                    let v_depth = i32::try_from(v_depth).unwrap_or(i32::MAX);
                    (*coord, ceiling.saturating_sub(v_depth))
                })
                .collect::<Vec<_>>();
            for (coord, level) in levels.iter_mut() {
                if !patch_mask.contains(coord) {
                    continue;
                }
                let cap = corridor_sources
                    .iter()
                    .map(|(source, source_level)| {
                        // Give the scenic centreline four complete shoulder rows
                        // before resuming the seven-level natural slope. Route
                        // admission treats any adjacent immutable >=249 crown
                        // column as a wall beside a <=239 ledge; the former
                        // point-width profile left three such cells in Patch59.
                        // Subtracting one distance row widens only the authored
                        // saddle terrain and preserves the same <=7 grade.
                        let graded_distance = source.distance(*coord).saturating_sub(4);
                        source_level.saturating_add(
                            i32::try_from(graded_distance)
                                .unwrap_or(i32::MAX)
                                .saturating_mul(PEAK_BODY_SLOPE),
                        )
                    })
                    .min()
                    .unwrap_or(*level);
                let floor = crown_floor.get(coord).copied().unwrap_or(*level);
                *level = (*level).min(cap).max(floor);
            }
            joined.extend(target.iter().copied());
            joined.extend(corridor);
            joined_ceiling = ceiling;
        }
        validate_peak_patch_saddle_connectivity(*patch, patch_mask, &groups, levels)?;
    }
    Ok(())
}

/// Deterministic minimax path: prefer the lowest maximum terrain crossed,
/// then the lowest total relief, then the shortest path and canonical cell.
/// The summit pin itself is immutable and cannot become part of a saddle.
fn lowest_relief_saddle_path(
    mask: &BTreeSet<HexCoord>,
    sources: &BTreeSet<HexCoord>,
    goals: &BTreeSet<HexCoord>,
    summit: HexCoord,
    summit_clearance: u32,
    levels: &BTreeMap<HexCoord, Level>,
) -> Option<Vec<HexCoord>> {
    type Cost = (Level, i64, u32);
    let mut best = BTreeMap::<HexCoord, Cost>::new();
    let mut parent = BTreeMap::<HexCoord, HexCoord>::new();
    let mut queue = BinaryHeap::<Reverse<(Cost, HexCoord)>>::new();
    for source in sources
        .iter()
        .copied()
        .filter(|coord| mask.contains(coord) && summit.distance(*coord) > summit_clearance)
    {
        let level = levels.get(&source).copied()?;
        let cost = (level, i64::from(level), 0);
        best.insert(source, cost);
        queue.push(Reverse((cost, source)));
    }
    let destination = loop {
        let Reverse((cost, current)) = queue.pop()?;
        if best.get(&current).copied() != Some(cost) {
            continue;
        }
        if goals.contains(&current) {
            break current;
        }
        let mut neighbors = current.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if summit.distance(neighbor) <= summit_clearance || !mask.contains(&neighbor) {
                continue;
            }
            let level = levels.get(&neighbor).copied()?;
            let next = (
                cost.0.max(level),
                cost.1.saturating_add(i64::from(level)),
                cost.2.saturating_add(1),
            );
            if best
                .get(&neighbor)
                .is_some_and(|previous| *previous <= next)
            {
                continue;
            }
            best.insert(neighbor, next);
            parent.insert(neighbor, current);
            queue.push(Reverse((next, neighbor)));
        }
    };
    let mut reversed = vec![destination];
    let mut current = destination;
    while !sources.contains(&current) {
        current = parent.get(&current).copied()?;
        reversed.push(current);
    }
    reversed.reverse();
    Some(reversed)
}

fn validate_peak_patch_saddle_connectivity(
    patch: PatchId,
    patch_mask: &BTreeSet<HexCoord>,
    groups: &[((PatchId, PatchId), BTreeSet<HexCoord>, Level)],
    levels: &BTreeMap<HexCoord, Level>,
) -> Result<(), V3GenerationError> {
    let low = patch_mask
        .iter()
        .copied()
        .filter(|coord| {
            levels
                .get(coord)
                .is_some_and(|level| *level < PEAK_VISUAL_WALL_THRESHOLD)
        })
        .collect::<BTreeSet<_>>();
    let components = fine_components(&low);
    let shared = components.iter().find(|component| {
        groups
            .first()
            .is_some_and(|(_, first, _)| first.iter().all(|coord| component.contains(coord)))
    });
    if shared.is_none_or(|component| {
        groups.iter().any(|(_, group, _)| {
            group.is_empty() || group.iter().any(|coord| !component.contains(coord))
        })
    }) {
        return Err(contract(format!(
            "Grand V3 peak patch {} retains disconnected sub-240 saddle seam groups",
            patch.0
        )));
    }
    Ok(())
}

fn validate_inner_peak_external_ingress_connectivity(
    patch_masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    saddle_swaths: &BTreeMap<(PatchId, PatchId), BTreeSet<HexCoord>>,
    external_ingress_swaths: &BTreeMap<(PatchId, PatchId), BTreeSet<HexCoord>>,
    levels: &BTreeMap<HexCoord, Level>,
    frozen: &FrozenPlateauField,
) -> Result<(), V3GenerationError> {
    let Some(patch_mask) = patch_masks.get(&INNER_PEAK_INGRESS_PEAK_PATCH) else {
        if !external_ingress_swaths.is_empty() {
            return Err(contract(
                "Grand V3 external inner-peak ingress was assigned to the wrong peak chain",
            ));
        }
        return Ok(());
    };
    let ingress_key = (
        INNER_PEAK_INGRESS_PEAK_PATCH,
        INNER_PEAK_INGRESS_FROZEN_PATCH,
    );
    let ingress = external_ingress_swaths.get(&ingress_key).ok_or_else(|| {
        contract("Grand V3 inner peak chain lost its typed Frozen-123/Peak-88 ingress swath")
    })?;
    if external_ingress_swaths.len() != 1
        || ingress.len() < 4
        || !ingress.is_subset(patch_mask)
        || !connected(ingress)
        || ingress.iter().any(|coord| {
            let ceiling = frozen.halo_distance.get(coord).copied().map(|depth| {
                FROZEN_PLATEAU_MAX.saturating_add(
                    i32::try_from(depth)
                        .unwrap_or(i32::MAX)
                        .saturating_mul(FROZEN_PLATEAU_MAXIMUM_STEP),
                )
            });
            levels
                .get(coord)
                .zip(ceiling)
                .is_none_or(|(level, ceiling)| *level > ceiling)
        })
    {
        return Err(contract(format!(
            "Grand V3 inner peak ingress is not one connected four-column Frozen-halo swath: count={}",
            ingress.len()
        )));
    }

    let internal_key = (PatchId(58), INNER_PEAK_INGRESS_PEAK_PATCH);
    let internal = saddle_swaths
        .get(&internal_key)
        .map(|swath| {
            swath
                .intersection(patch_mask)
                .copied()
                .collect::<BTreeSet<_>>()
        })
        .filter(|swath| !swath.is_empty())
        .ok_or_else(|| {
            contract("Grand V3 inner peak ingress has no exact Peak-88/Peak-58 saddle target")
        })?;
    let low = patch_mask
        .iter()
        .copied()
        .filter(|coord| {
            levels
                .get(coord)
                .is_some_and(|level| *level < PEAK_VISUAL_WALL_THRESHOLD)
        })
        .collect::<BTreeSet<_>>();
    let connected_low = fine_components(&low)
        .into_iter()
        .find(|component| ingress.iter().all(|coord| component.contains(coord)));
    if connected_low.is_none_or(|component| internal.iter().any(|coord| !component.contains(coord)))
    {
        return Err(contract(
            "Grand V3 Patch-88 external ingress and 88/58 saddle are not one connected sub-240 foundation corridor",
        ));
    }
    Ok(())
}

/// Extends the immutable scenic saddle authority across the exact internal
/// low corridor authored above. The corridor remains nonordinary terrain; it
/// merely becomes part of the same explicit saddle contract that already
/// authorizes the two boundary swaths.
fn extend_peak_saddle_swaths_through_patches(
    patch_masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    swaths: &mut BTreeMap<(PatchId, PatchId), BTreeSet<HexCoord>>,
    summit_coords: &BTreeMap<PatchId, HexCoord>,
    summit_levels: &BTreeMap<PatchId, Level>,
    levels: &BTreeMap<HexCoord, Level>,
) -> Result<(), V3GenerationError> {
    let chain_mask = patch_masks
        .values()
        .flat_map(|mask| mask.iter().copied())
        .collect::<BTreeSet<_>>();
    for (patch, patch_mask) in patch_masks {
        let edges = swaths
            .keys()
            .copied()
            .filter(|edge| edge.0 == *patch || edge.1 == *patch)
            .collect::<Vec<_>>();
        if edges.len() < 2 {
            continue;
        }
        let summit = summit_coords.get(patch).copied().ok_or_else(|| {
            contract(format!(
                "Grand V3 peak patch {} has saddle authority but no summit pin",
                patch.0
            ))
        })?;
        let corridor_mask = patch_mask
            .iter()
            .copied()
            .filter(|coord| {
                coord
                    .neighbors()
                    .into_iter()
                    .all(|neighbor| chain_mask.contains(&neighbor))
            })
            .collect::<BTreeSet<_>>();
        let mut joined_edges = vec![edges[0]];
        let mut joined = swaths[&edges[0]]
            .intersection(patch_mask)
            .copied()
            .collect::<BTreeSet<_>>();
        for edge in edges.iter().copied().skip(1) {
            let ceiling_ready = corridor_mask
                .iter()
                .copied()
                .filter(|coord| {
                    levels
                        .get(coord)
                        .is_some_and(|level| *level < PEAK_VISUAL_WALL_THRESHOLD)
                })
                .collect::<BTreeSet<_>>();
            let target = swaths[&edge]
                .intersection(patch_mask)
                .copied()
                .collect::<BTreeSet<_>>();
            let joined_ready = joined
                .intersection(&ceiling_ready)
                .copied()
                .collect::<BTreeSet<_>>();
            let corridor = lowest_relief_saddle_path(
                &ceiling_ready,
                &joined_ready,
                &target,
                summit,
                0,
                levels,
            )
            .ok_or_else(|| {
                contract(format!(
                    "Grand V3 peak patch {} has shaped low saddles but no internal scenic authority corridor (joined={}, target={}, eligible={})",
                    patch.0,
                    joined_ready.len(),
                    target.len(),
                    ceiling_ready.len()
                ))
            })?;
            for joined_edge in &joined_edges {
                let ceiling = peak_saddle_ceiling(*joined_edge, summit_levels)?;
                extend_saddle_authority_connected(
                    swaths.get_mut(joined_edge).ok_or_else(|| {
                        contract("Grand V3 peak saddle authority lost a joined edge")
                    })?,
                    &corridor,
                    ceiling,
                    levels,
                );
            }
            let edge_ceiling = peak_saddle_ceiling(edge, summit_levels)?;
            extend_saddle_authority_connected(
                swaths
                    .get_mut(&edge)
                    .ok_or_else(|| contract("Grand V3 peak saddle authority lost a target edge"))?,
                &corridor,
                edge_ceiling,
                levels,
            );
            joined = joined_ready;
            joined.extend(target);
            joined.extend(corridor);
            joined_edges.push(edge);
        }
    }
    Ok(())
}

fn extend_saddle_authority_connected(
    swath: &mut BTreeSet<HexCoord>,
    corridor: &[HexCoord],
    ceiling: Level,
    levels: &BTreeMap<HexCoord, Level>,
) {
    let original = swath.clone();
    swath.extend(
        corridor
            .iter()
            .copied()
            .filter(|coord| levels.get(coord).is_some_and(|level| *level <= ceiling)),
    );
    if let Some(component) = fine_components(swath)
        .into_iter()
        .find(|component| original.iter().all(|coord| component.contains(coord)))
    {
        *swath = component;
    } else {
        *swath = original;
    }
}

fn peak_saddle_ceiling(
    edge: (PatchId, PatchId),
    summit_levels: &BTreeMap<PatchId, Level>,
) -> Result<Level, V3GenerationError> {
    let first = summit_levels.get(&edge.0).copied().ok_or_else(|| {
        contract(format!(
            "Grand V3 peak saddle owner {} lost its summit level",
            edge.0 .0
        ))
    })?;
    let second = summit_levels.get(&edge.1).copied().ok_or_else(|| {
        contract(format!(
            "Grand V3 peak saddle owner {} lost its summit level",
            edge.1 .0
        ))
    })?;
    Ok(first
        .min(second)
        .saturating_sub(PEAK_SADDLE_DEPTH)
        .min(PEAK_VISUAL_WALL_THRESHOLD.saturating_sub(1)))
}

/// Conservative highland-side mirror of the route compiler's immutable-crown
/// bound. A sub-240 saddle coordinate beside a >=249 upper-crown column cannot
/// be graded without exceeding either the 239 ledge ceiling or the nine-level
/// neighbor budget, so it is not part of the actual route-admitted component.
fn peak_saddle_route_ready_mask(
    patch_mask: &BTreeSet<HexCoord>,
    levels: &BTreeMap<HexCoord, Level>,
) -> BTreeSet<HexCoord> {
    patch_mask
        .iter()
        .copied()
        .filter(|coord| {
            levels
                .get(coord)
                .is_some_and(|level| *level < PEAK_VISUAL_WALL_THRESHOLD)
        })
        .filter(|coord| {
            coord.neighbors().into_iter().all(|neighbor| {
                levels.get(&neighbor).is_none_or(|level| {
                    *level < PEAK_VISUAL_WALL_THRESHOLD
                        || *level
                            <= PEAK_VISUAL_WALL_THRESHOLD
                                .saturating_sub(1)
                                .saturating_add(9)
                })
            })
        })
        .collect()
}

/// Exact Patch-59 exception that turns the two already-low west saddle
/// components into one authored transit corridor.
///
/// The raw field must retain the reviewed bounded shoulder shape: the exact
/// owner column and each profiled neighbor stay within the nine-level route
/// blend above the visual-wall threshold. Any wider or taller drift fails
/// closed instead of silently widening the exception.
fn ordered_peak_saddle_route_ready_mask(
    owner_mask: &BTreeSet<HexCoord>,
    levels: &BTreeMap<HexCoord, Level>,
) -> Result<BTreeSet<HexCoord>, V3GenerationError> {
    let raw_level = levels.get(&INNER_PEAK_TRANSIT_WEST_NOTCH).copied();
    if !owner_mask.contains(&INNER_PEAK_TRANSIT_WEST_NOTCH) || raw_level.is_none() {
        return Err(contract(format!(
            "Grand V3 ordered peak-transit west notch drifted: expected {:?} in Patch 59, got owner={} level={raw_level:?}",
            INNER_PEAK_TRANSIT_WEST_NOTCH,
            owner_mask.contains(&INNER_PEAK_TRANSIT_WEST_NOTCH)
        )));
    }
    let mut route_ready = peak_saddle_route_ready_mask(owner_mask, levels);
    if ordered_peak_saddle_is_bounded_west_notch(
        INNER_PEAK_TRANSIT_WEST_NOTCH,
        raw_level.unwrap_or(Level::MAX),
        levels,
    ) {
        route_ready.insert(INNER_PEAK_TRANSIT_WEST_NOTCH);
    }
    Ok(route_ready)
}

fn ordered_peak_saddle_is_bounded_west_notch(
    coord: HexCoord,
    raw_level: Level,
    levels: &BTreeMap<HexCoord, Level>,
) -> bool {
    coord == INNER_PEAK_TRANSIT_WEST_NOTCH
        && raw_level <= INNER_PEAK_TRANSIT_WEST_NOTCH_RAW_MAX
        && coord.neighbors().into_iter().all(|neighbor| {
            levels
                .get(&neighbor)
                .is_none_or(|level| *level <= INNER_PEAK_TRANSIT_WEST_NOTCH_RAW_MAX)
        })
}

/// Early highland-side ceiling implied by the independently authored Crystal
/// opening and Frozen plateau. This is a bounded prefilter for known upper
/// constraints; the combined projection remains authoritative because it also
/// sees later outer pins, mantle pins, summit floors, and shoulder floors.
fn ordered_peak_saddle_hard_ceiling(
    coord: HexCoord,
    frozen: &FrozenPlateauField,
    crystal_exit_ceiling: &BTreeMap<HexCoord, Level>,
) -> Level {
    let contribution = |source: HexCoord, level: Level| {
        level.saturating_add(
            i32::try_from(source.distance(coord))
                .unwrap_or(i32::MAX)
                .saturating_mul(PEAK_OUTER_FEATHER_MAXIMUM_STEP),
        )
    };
    crystal_exit_ceiling
        .iter()
        .map(|(source, level)| contribution(*source, *level))
        .chain(
            frozen
                .levels
                .iter()
                .map(|(source, level)| contribution(*source, *level)),
        )
        .chain(frozen.halo_distance.iter().map(|(source, distance)| {
            let level = FROZEN_PLATEAU_MAX.saturating_add(
                i32::try_from(*distance)
                    .unwrap_or(i32::MAX)
                    .saturating_mul(FROZEN_PLATEAU_MAXIMUM_STEP),
            );
            contribution(*source, level)
        }))
        .min()
        .unwrap_or(MAX_V3_LEVEL.saturating_sub(1))
}

/// Jointly selects and grades the ordered Patch-59 runway. Searching exact
/// `(coordinate, level)` states prevents a geometrically attractive low path
/// from approaching the suffix portal through an independently capped terrain
/// cone. Cost prefers the shortest physical runway, then the least total
/// displacement from the authored scalar field; canonical state ordering
/// resolves the remaining ties deterministically.
fn grade_ordered_peak_saddle_path(
    mask: &BTreeSet<HexCoord>,
    sources: &BTreeSet<HexCoord>,
    goals: &BTreeSet<HexCoord>,
    summit: HexCoord,
    levels: &BTreeMap<HexCoord, Level>,
    frozen: &FrozenPlateauField,
    crystal_exit_ceiling: &BTreeMap<HexCoord, Level>,
) -> Option<(Vec<HexCoord>, BTreeMap<HexCoord, Level>)> {
    type State = (HexCoord, Level);
    type Cost = (u32, u64);

    let minimum_level = INNER_PEAK_TRANSIT_RUNWAY_ENDPOINT_LEVEL.saturating_sub(PEAK_SADDLE_DEPTH);
    let maximum_level = INNER_PEAK_ROUTE_SADDLE_CEILING;
    let hard_ceilings = mask
        .iter()
        .copied()
        .map(|coord| {
            (
                coord,
                ordered_peak_saddle_hard_ceiling(coord, frozen, crystal_exit_ceiling),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut best = BTreeMap::<State, Cost>::new();
    let mut parent = BTreeMap::<State, State>::new();
    let mut starts = BTreeSet::<State>::new();
    let mut queue = BinaryHeap::<Reverse<(Cost, HexCoord, Level)>>::new();
    for source in sources
        .iter()
        .copied()
        .filter(|coord| mask.contains(coord) && *coord != summit)
    {
        let natural = levels.get(&source).copied()?;
        if hard_ceilings.get(&source).copied()? < INNER_PEAK_TRANSIT_RUNWAY_ENDPOINT_LEVEL {
            continue;
        }
        let state = (source, INNER_PEAK_TRANSIT_RUNWAY_ENDPOINT_LEVEL);
        let cost = (0, u64::from(natural.abs_diff(state.1)));
        if best.get(&state).is_none_or(|previous| cost < *previous) {
            best.insert(state, cost);
            starts.insert(state);
            queue.push(Reverse((cost, state.0, state.1)));
        }
    }

    let destination = loop {
        let Reverse((cost, coord, level)) = queue.pop()?;
        let state = (coord, level);
        if best.get(&state).copied() != Some(cost) {
            continue;
        }
        if goals.contains(&coord) && level == INNER_PEAK_TRANSIT_RUNWAY_ENDPOINT_LEVEL {
            break state;
        }
        let mut neighbors = coord.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if neighbor == summit || !mask.contains(&neighbor) {
                continue;
            }
            let natural = levels.get(&neighbor).copied()?;
            let ceiling = hard_ceilings.get(&neighbor).copied()?;
            let lowest = level.saturating_sub(1).max(minimum_level);
            let highest = level.saturating_add(1).min(maximum_level).min(ceiling);
            if lowest > highest {
                continue;
            }
            for next_level in lowest..=highest {
                if goals.contains(&neighbor)
                    && next_level != INNER_PEAK_TRANSIT_RUNWAY_ENDPOINT_LEVEL
                {
                    continue;
                }
                let next_state = (neighbor, next_level);
                let next_cost = (
                    cost.0.saturating_add(1),
                    cost.1
                        .saturating_add(u64::from(natural.abs_diff(next_level))),
                );
                if best
                    .get(&next_state)
                    .is_some_and(|previous| *previous <= next_cost)
                {
                    continue;
                }
                best.insert(next_state, next_cost);
                parent.insert(next_state, state);
                queue.push(Reverse((next_cost, next_state.0, next_state.1)));
            }
        }
    };

    let mut reversed = vec![destination];
    let mut current = destination;
    while !starts.contains(&current) {
        current = parent.get(&current).copied()?;
        reversed.push(current);
    }
    reversed.reverse();
    let centerline = reversed.iter().map(|(coord, _)| *coord).collect::<Vec<_>>();
    let unique = centerline.iter().copied().collect::<BTreeSet<_>>();
    let authored_grades = reversed.into_iter().collect::<BTreeMap<_, _>>();
    let has_chord = centerline.iter().enumerate().any(|(index, coord)| {
        centerline
            .iter()
            .skip(index.saturating_add(2))
            .any(|other| coord.distance(*other) == 1)
    });
    (unique.len() == centerline.len()
        && !has_chord
        && authored_grades.len() == centerline.len()
        && centerline.windows(2).all(|pair| {
            pair[0].distance(pair[1]) == 1
                && authored_grades[&pair[0]].abs_diff(authored_grades[&pair[1]]) <= 1
        }))
    .then_some((centerline, authored_grades))
}

fn build_ordered_peak_saddle_spines(
    patch_masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    boundary_saddle_swaths: &BTreeMap<(PatchId, PatchId), BTreeSet<HexCoord>>,
    extended_saddle_swaths: &BTreeMap<(PatchId, PatchId), BTreeSet<HexCoord>>,
    summit_coords: &BTreeMap<PatchId, HexCoord>,
    levels: &BTreeMap<HexCoord, Level>,
    frozen: &FrozenPlateauField,
    crystal_exit_ceiling: &BTreeMap<HexCoord, Level>,
) -> Result<BTreeMap<PatchId, OrderedPeakSaddleSpineAuthority>, V3GenerationError> {
    if !patch_masks.contains_key(&INNER_PEAK_TRANSIT_OWNER) {
        return Ok(BTreeMap::new());
    }
    let owner_mask = patch_masks
        .get(&INNER_PEAK_TRANSIT_OWNER)
        .ok_or_else(|| contract("Grand V3 ordered peak transit lost its owner mask"))?;
    let ingress_mask = patch_masks
        .get(&INNER_PEAK_TRANSIT_INGRESS.0)
        .ok_or_else(|| contract("Grand V3 ordered peak transit lost Patch 58"))?;
    let egress_mask = patch_masks
        .get(&INNER_PEAK_TRANSIT_EGRESS.0)
        .ok_or_else(|| contract("Grand V3 ordered peak transit lost Patch 36"))?;
    let ingress_boundary = boundary_saddle_swaths
        .get(&INNER_PEAK_TRANSIT_INGRESS)
        .ok_or_else(|| contract("Grand V3 ordered peak transit lost its 58/59 boundary swath"))?;
    let egress_boundary = boundary_saddle_swaths
        .get(&INNER_PEAK_TRANSIT_EGRESS)
        .ok_or_else(|| contract("Grand V3 ordered peak transit lost its 59/36 boundary swath"))?;
    let ingress_extended = extended_saddle_swaths
        .get(&INNER_PEAK_TRANSIT_INGRESS)
        .ok_or_else(|| contract("Grand V3 ordered peak transit lost its extended 58/59 swath"))?;
    let egress_extended = extended_saddle_swaths
        .get(&INNER_PEAK_TRANSIT_EGRESS)
        .ok_or_else(|| contract("Grand V3 ordered peak transit lost its extended 59/36 swath"))?;
    let natural_route_ready = peak_saddle_route_ready_mask(owner_mask, levels);
    let route_ready = ordered_peak_saddle_route_ready_mask(owner_mask, levels)?;
    // The old lowest-relief spine looped east around the low Patch-59 pocket
    // and then tried to climb back into this handoff beside a level-144 outer
    // projection source. At x > 43 that source caps successive approach cells
    // below the suffix-compatible grade. Retain the same proven portal but
    // author its runway from the west, where every cell is at least nine hexes
    // from that low source and a sub-219 grade remains physically admissible.
    let west_approach_route_ready = route_ready
        .iter()
        .copied()
        .filter(|coord| coord.x() <= INNER_PEAK_TRANSIT_EGRESS_ANCHOR.x())
        .collect::<BTreeSet<_>>();
    let ingress_targets = ingress_boundary
        .intersection(owner_mask)
        .filter(|coord| west_approach_route_ready.contains(coord))
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .any(|neighbor| ingress_mask.contains(&neighbor))
        })
        .copied()
        .collect::<BTreeSet<_>>();
    let egress_sources = egress_boundary
        .intersection(owner_mask)
        .filter(|coord| **coord == INNER_PEAK_TRANSIT_EGRESS_ANCHOR)
        .filter(|coord| west_approach_route_ready.contains(coord))
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .any(|neighbor| egress_mask.contains(&neighbor))
        })
        .copied()
        .collect::<BTreeSet<_>>();
    let scenic_domain = ingress_extended
        .union(egress_extended)
        .filter(|coord| owner_mask.contains(coord) && west_approach_route_ready.contains(coord))
        .copied()
        .collect::<BTreeSet<_>>();
    let summit = summit_coords
        .get(&INNER_PEAK_TRANSIT_OWNER)
        .copied()
        .ok_or_else(|| contract("Grand V3 ordered peak transit lost its summit pin"))?;
    let mut graded_path = None;
    for (index, domain) in [&scenic_domain, &west_approach_route_ready]
        .into_iter()
        .enumerate()
    {
        if graded_path.is_some() || (index == 1 && scenic_domain == west_approach_route_ready) {
            continue;
        }
        graded_path = grade_ordered_peak_saddle_path(
            domain,
            &ingress_targets,
            &egress_sources,
            summit,
            levels,
            frozen,
            crystal_exit_ceiling,
        );
    }
    let (centerline, authored_grades) = graded_path.ok_or_else(|| {
        contract(format!(
            "Grand V3 ordered peak transit has no projection-feasible graded Patch-59 spine: ingress={}, egress={}, scenic={}, route-ready={}",
            ingress_targets.len(),
            egress_sources.len(),
            scenic_domain.len(),
            west_approach_route_ready.len()
        ))
    })?;
    let unique = centerline.iter().copied().collect::<BTreeSet<_>>();
    let has_chord = centerline.iter().enumerate().any(|(index, coord)| {
        centerline
            .iter()
            .skip(index.saturating_add(2))
            .any(|other| coord.distance(*other) == 1)
    });
    if centerline.len() < 2
        || unique.len() != centerline.len()
        || centerline
            .windows(2)
            .any(|pair| pair[0].distance(pair[1]) != 1)
        || has_chord
        || centerline
            .first()
            .is_none_or(|coord| !ingress_targets.contains(coord))
        || centerline
            .last()
            .is_none_or(|coord| !egress_sources.contains(coord))
        || unique.iter().any(|coord| {
            !owner_mask.contains(coord)
                || !west_approach_route_ready.contains(coord)
                || levels.get(coord).is_none_or(|level| {
                    *level >= PEAK_VISUAL_WALL_THRESHOLD
                        && !ordered_peak_saddle_is_bounded_west_notch(*coord, *level, levels)
                })
        })
    {
        return Err(contract(
            "Grand V3 ordered peak transit did not retain one induced, oriented, west-approach route-ready sub-240 Patch-59 spine",
        ));
    }
    let first = centerline[0];
    let last = *centerline
        .last()
        .ok_or_else(|| contract("Grand V3 ordered peak transit lost its last coordinate"))?;
    let ingress_portals = first
        .neighbors()
        .into_iter()
        .filter(|coord| ingress_mask.contains(coord) && ingress_boundary.contains(coord))
        .map(|from| (from, first))
        .collect::<BTreeSet<_>>();
    let egress_portals = last
        .neighbors()
        .into_iter()
        .filter(|coord| egress_mask.contains(coord) && egress_boundary.contains(coord))
        .map(|to| (last, to))
        .collect::<BTreeSet<_>>();
    let mut support_ready = natural_route_ready;
    support_ready.extend(unique.iter().copied());
    let support_domain = centerline
        .iter()
        .flat_map(|coord| coord.within_radius(4))
        .filter(|coord| owner_mask.contains(coord) && support_ready.contains(coord))
        .collect::<BTreeSet<_>>();
    if ingress_portals.is_empty()
        || egress_portals.is_empty()
        || !unique.is_subset(&support_domain)
        || !connected(&support_domain)
    {
        return Err(contract(format!(
            "Grand V3 ordered peak transit lost typed portals or its connected support reservation: ingress={}, egress={}, spine={}, support={}",
            ingress_portals.len(),
            egress_portals.len(),
            centerline.len(),
            support_domain.len()
        )));
    }
    Ok(BTreeMap::from([(
        INNER_PEAK_TRANSIT_OWNER,
        OrderedPeakSaddleSpineAuthority {
            owner: INNER_PEAK_TRANSIT_OWNER,
            ingress_from: INNER_PEAK_TRANSIT_INGRESS.0,
            egress_to: INNER_PEAK_TRANSIT_EGRESS.0,
            ingress_portals,
            centerline,
            egress_portals,
            support_domain,
            authored_grades,
        },
    )]))
}

fn build_peak_ridge_authority(
    layout: &ResolvedLayoutPlan,
    cells: &BTreeMap<SchematicCoord, LandmarkCell>,
    components: &[BTreeSet<SchematicCoord>],
    peak_levels: &BTreeMap<HexCoord, Level>,
    peak_bodies: &BTreeMap<SchematicCoord, BTreeMap<HexCoord, Level>>,
    peak_feathers: &[PeakFeatherField],
    peak_external_ingresses: &[BTreeMap<(PatchId, PatchId), BTreeSet<HexCoord>>],
    peak_summit_pins: &BTreeMap<PatchId, (HexCoord, Level)>,
    frozen: &FrozenPlateauField,
    crystal_exit_ceiling: &BTreeMap<HexCoord, Level>,
) -> Result<PeakRidgeAuthority, V3GenerationError> {
    if components.len() != 2
        || peak_bodies.len() != cells.len()
        || peak_feathers.len() != components.len()
        || peak_external_ingresses.len() != components.len()
        || peak_summit_pins.len() != cells.len()
    {
        return Err(contract(format!(
            "Grand V3 peak authority requires two chains, twelve independent bodies, two Mountain feathers, and aligned external-ingress evidence; found {}/{}/{}/{}",
            components.len(),
            peak_bodies.len(),
            peak_feathers.len(),
            peak_external_ingresses.len()
        )));
    }

    let mut authority_components = Vec::with_capacity(components.len());
    for (component_index, (component, feather)) in components.iter().zip(peak_feathers).enumerate()
    {
        let mut patch_masks = BTreeMap::new();
        let mut expected_peak_bodies = BTreeMap::new();
        let mut summit_pins = BTreeMap::new();
        let mut summit_coords_by_patch = BTreeMap::new();
        let mut summit_levels_by_patch = BTreeMap::new();
        for schematic in component {
            let cell = cells.get(schematic).ok_or_else(|| {
                contract("Grand V3 peak authority references an absent locked cell")
            })?;
            let patch = layout.patches.get(&cell.patch).ok_or_else(|| {
                contract(format!(
                    "Grand V3 peak authority lost resolved patch {}",
                    cell.patch.0
                ))
            })?;
            if patch_masks.insert(cell.patch, patch.mask.clone()).is_some() {
                return Err(contract("Grand V3 peak authority assigned one patch twice"));
            }
            let body = peak_bodies
                .get(schematic)
                .cloned()
                .ok_or_else(|| contract("Grand V3 peak authority lost one independent body"))?;
            if body.keys().any(|coord| !patch.mask.contains(coord))
                || expected_peak_bodies.insert(cell.patch, body).is_some()
            {
                return Err(contract(
                    "Grand V3 independent peak body escaped or duplicated its stable patch",
                ));
            }
            let (summit_coord, summit) = peak_summit_pins
                .get(&cell.patch)
                .copied()
                .ok_or_else(|| contract("Grand V3 peak authority lost one seeded summit pin"))?;
            if peak_levels.get(&summit_coord) != Some(&summit)
                || !patch.mask.contains(&summit_coord)
            {
                return Err(contract(
                    "Grand V3 peak authority summit pin escaped its exact peak body",
                ));
            }
            summit_pins.insert(summit_coord, summit);
            summit_coords_by_patch.insert(cell.patch, summit_coord);
            summit_levels_by_patch.insert(cell.patch, summit);
        }
        let expected_ridge_profile = expected_peak_bodies
            .values()
            .flat_map(|body| body.iter().map(|(coord, level)| (*coord, *level)))
            .collect::<BTreeMap<_, _>>();
        let expected_high_band = expected_ridge_profile
            .iter()
            .filter_map(|(coord, level)| {
                (*level >= PEAK_VISUAL_WALL_THRESHOLD).then_some((*coord, *level))
            })
            .collect::<BTreeMap<_, _>>();
        let expected_high_coords = expected_high_band.keys().copied().collect::<BTreeSet<_>>();
        let mut expected_saddle_swaths = build_peak_saddle_swaths(&patch_masks)?;
        let boundary_saddle_swaths = expected_saddle_swaths.clone();
        extend_peak_saddle_swaths_through_patches(
            &patch_masks,
            &mut expected_saddle_swaths,
            &summit_coords_by_patch,
            &summit_levels_by_patch,
            &expected_ridge_profile,
        )?;
        let ordered_saddle_spines = build_ordered_peak_saddle_spines(
            &patch_masks,
            &boundary_saddle_swaths,
            &expected_saddle_swaths,
            &summit_coords_by_patch,
            &expected_ridge_profile,
            frozen,
            crystal_exit_ceiling,
        )?;
        let expected_external_ingress_swaths = peak_external_ingresses
            .get(component_index)
            .cloned()
            .ok_or_else(|| contract("Grand V3 peak authority lost external ingress evidence"))?;
        validate_inner_peak_external_ingress_connectivity(
            &patch_masks,
            &expected_saddle_swaths,
            &expected_external_ingress_swaths,
            &expected_ridge_profile,
            frozen,
        )?;
        let high_components = fine_components(&expected_high_coords);
        let summit_memberships = high_components
            .iter()
            .map(|high_component| {
                summit_pins
                    .keys()
                    .filter(|pin| high_component.contains(pin))
                    .count()
            })
            .collect::<Vec<_>>();
        if patch_masks.len() != 6
            || expected_peak_bodies.len() != 6
            || summit_pins.len() != 6
            || !summit_pins
                .keys()
                .all(|coord| expected_high_band.contains_key(coord))
            || high_components.len() != 6
            || expected_saddle_swaths.len() < 5
            || feather.owners.is_empty()
            || feather.boundary_edges.is_empty()
            || !connected(
                &expected_ridge_profile
                    .keys()
                    .copied()
                    .collect::<BTreeSet<_>>(),
            )
        {
            return Err(contract(format!(
                "Grand V3 peak authority did not resolve one connected lower body with six upper crowns: patches={}, bodies={}, pins={}, saddle_swaths={}, high_components={}, minimum_body={}, summit_memberships={summit_memberships:?}",
                patch_masks.len(),
                expected_peak_bodies.len(),
                summit_pins.len(),
                expected_saddle_swaths.len(),
                high_components.len(),
                expected_peak_bodies
                    .values()
                    .map(BTreeMap::len)
                    .min()
                    .unwrap_or_default(),
            )));
        }
        authority_components.push(PeakRidgeComponentAuthority {
            patch_masks,
            expected_peak_bodies,
            borrowed_crown_cells: BTreeMap::new(),
            expected_ridge_profile,
            expected_high_band,
            summit_pins,
            expected_saddle_swaths,
            ordered_saddle_spines,
            expected_external_ingress_swaths,
            feather_owners: feather.owners.clone(),
            feather_boundary_edges: feather.boundary_edges.clone(),
            authorized_route_grades: None,
            authorized_waterfall_openings: None,
        });
    }
    Ok(PeakRidgeAuthority {
        components: authority_components,
    })
}

fn massif_offset_summit_candidate_is_viable(
    crest: HexCoord,
    candidate: HexCoord,
    summit: Level,
    depths: &BTreeMap<HexCoord, u32>,
    profile: V3GrandV3BasicTerrainProfile,
    seed: u64,
) -> bool {
    let crest_depth = depths.get(&crest).copied().unwrap_or_default();
    depths.get(&candidate).is_some_and(|depth| {
        let radial = crest.distance(candidate);
        let absolute_body = MASSIF_ABSOLUTE_BODY_BASE.saturating_add(
            i32::try_from(*depth)
                .unwrap_or(i32::MAX)
                .saturating_mul(MASSIF_ABSOLUTE_BODY_RISE_PER_HEX),
        );
        let inward_envelope = summit.saturating_sub(
            i32::try_from(crest_depth.saturating_sub(*depth))
                .unwrap_or(i32::MAX)
                .saturating_mul(MASSIF_ABSOLUTE_BODY_RISE_PER_HEX),
        );
        let body = absolute_body
            .min(inward_envelope)
            .max(profile.high_core_level)
            .min(massif_body_crest_cap(summit, radial));
        let primary_support = summit.saturating_sub(
            i32::try_from(radial)
                .unwrap_or(i32::MAX)
                .saturating_mul(MASSIF_PRIMARY_SUMMIT_SUPPORT_SLOPE),
        );
        let edge_cap = absolute_body.saturating_add(MASSIF_SUMMIT_EDGE_LIFT);
        let primary_raw = body.max(primary_support.min(edge_cap).max(profile.high_core_level));
        let final_cap = absolute_body.max(profile.high_core_level);
        let final_primary = primary_raw.min(final_cap);
        let drop = 8_i32.saturating_add(
            i32::try_from(
                named_sample(seed, "grand_v3.highlands.massif_crest_lobe_drop", candidate) % 18,
            )
            .unwrap_or_default(),
        );
        let candidate_source = summit
            .saturating_sub(drop)
            .max(primary_support.saturating_add(2))
            .max(primary_raw.saturating_add(2))
            .min(edge_cap)
            .min(summit.saturating_sub(10))
            .min(final_cap);
        final_primary <= summit.saturating_sub(6)
            && candidate_source >= final_primary.saturating_add(3)
    })
}

fn build_massif_field(
    mask: &BTreeSet<HexCoord>,
    semantic_owner_mask: &BTreeSet<HexCoord>,
    connector_mask: &BTreeSet<HexCoord>,
    crest_owner_mask: &BTreeSet<HexCoord>,
    crystal_mask: &BTreeSet<HexCoord>,
    profile: V3GrandV3BasicTerrainProfile,
    seed: u64,
) -> Result<MassifField, V3GenerationError> {
    if !connected(mask)
        || !semantic_owner_mask.is_subset(mask)
        || !connector_mask.is_subset(mask)
        || !connector_mask.is_disjoint(semantic_owner_mask)
    {
        return Err(contract(
            "Grand V3 massif fine mask must remain one connected body containing its semantic owners",
        ));
    }
    let crystal_center = super::schematic::exact_hex_disk_center(crystal_mask, CRYSTAL_SITE_RADIUS)
        .ok_or_else(|| contract("Grand V3 massif requires the exact radius-32 Crystal site"))?;
    // Crystal's exact authored site is an enclosed hole in the composite
    // Massif, not another outside edge.  Counting that hole as a boundary made
    // the visual-only connector a depth-zero trench even though its
    // overlay-free Mountain feather was eight columns wide.  Use the true
    // outside edge for the scalar field, while retaining the unmodified depth
    // map below solely for deterministic crest selection.
    let crest_selection_mask = semantic_owner_mask
        .union(connector_mask)
        .copied()
        .collect::<BTreeSet<_>>();
    let crest_selection_depths = boundary_depth(&crest_selection_mask);
    let depths = outer_boundary_depth(mask, crystal_mask);
    let distance_from_connector = distances_from(mask, connector_mask.iter().copied());
    let centroid = integer_centroid(mask)?;
    let eligible_crests = mask
        .iter()
        .copied()
        .filter(|coord| {
            crest_owner_mask.contains(coord)
                && coord
                    .within_radius(MASSIF_PROTECTED_BODY_RADIUS)
                    .into_iter()
                    .all(|protected| semantic_owner_mask.contains(&protected))
                && coord
                    .distance(crystal_center)
                    .saturating_sub(CRYSTAL_SITE_RADIUS)
                    >= CELL_PITCH.unsigned_abs() / 2
        })
        .collect::<BTreeSet<_>>();
    let summit_span = MASSIF_SUMMIT_MAX
        .saturating_sub(MASSIF_SUMMIT_MIN)
        .saturating_add(1);
    let summit_for_crest = |candidate_crest: HexCoord| {
        MASSIF_SUMMIT_MIN.saturating_add(
            i32::try_from(
                named_sample(seed, "grand_v3.highlands.massif_summit", candidate_crest)
                    % u64::try_from(summit_span).unwrap_or(1),
            )
            .unwrap_or_default(),
        )
    };
    let usable_sectors = |candidate_crest: HexCoord, radius: u32| {
        let candidate_summit = summit_for_crest(candidate_crest);
        mask.iter()
            .copied()
            .filter(|candidate| {
                (5..=radius).contains(&candidate_crest.distance(*candidate))
                    && massif_offset_summit_candidate_is_viable(
                        candidate_crest,
                        *candidate,
                        candidate_summit,
                        &depths,
                        profile,
                        seed,
                    )
            })
            .map(|candidate| enclosure_sector(candidate_crest, candidate))
            .collect::<BTreeSet<_>>()
            .len()
    };
    let usable_sector_counts = eligible_crests
        .iter()
        .map(|coord| {
            (
                *coord,
                usable_sectors(*coord, MASSIF_DISTRIBUTED_SUMMIT_RADIUS),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let protected_sector_counts = eligible_crests
        .iter()
        .map(|coord| (*coord, usable_sectors(*coord, MASSIF_SUMMIT_BODY_RADIUS)))
        .collect::<BTreeMap<_, _>>();
    let crest = eligible_crests
        .iter()
        .copied()
        .filter(|coord| usable_sector_counts[coord] >= 4 && protected_sector_counts[coord] >= 3)
        .max_by_key(|coord| {
            (
                usable_sector_counts[coord],
                protected_sector_counts[coord],
                distance_from_connector
                    .get(coord)
                    .copied()
                    .unwrap_or(u32::MAX),
                crest_selection_depths
                    .get(coord)
                    .copied()
                    .unwrap_or_default(),
                Reverse(coord.distance(centroid)),
                Reverse(named_sample(
                    seed,
                    "grand_v3.highlands.massif_crest",
                    *coord,
                )),
                Reverse(*coord),
            )
        })
        .ok_or_else(|| {
            contract(
                "Grand V3 massif cannot select a central crest with three protected and four distributed usable lobe sectors",
            )
        })?;
    let summit = summit_for_crest(crest);
    // Keep one canonical high point, but give the upper mountain several
    // shorter offset summits. A single radial source produced the pencil/cone
    // silhouette called out in review even when its scalar slope was valid.
    // These sources share the same three-level falloff, so their union remains
    // gradual while reading as a massif group with one tallest crest.
    let primary_direction =
        usize::try_from(named_sample(seed, "grand_v3.highlands.massif_crest_axis", crest) % 6)
            .unwrap_or_default();
    let crest_depth = depths.get(&crest).copied().unwrap_or_default();
    let mut summit_sources = BTreeMap::from([(crest, summit)]);
    let mut summit_source_sectors = BTreeSet::new();
    let mut protected_summit_source_sectors = BTreeSet::new();
    for lobe in 0..6_usize {
        let direction = (primary_direction + lobe) % 6;
        let radius = 5_u32.saturating_add(
            u32::try_from(
                named_sample(
                    seed,
                    "grand_v3.highlands.massif_crest_lobe_radius",
                    step_in_direction(crest, direction, 5),
                ) % 8,
            )
            .unwrap_or_default(),
        );
        let target = step_in_direction(crest, direction, radius);
        // Only the tallest crest is restricted to the non-Crystal coarse
        // owner group. Offset summits belong to the complete physical Massif
        // body, including its owned Mountain feather: seed 175's split
        // semantic mask exposes only two local sectors around the selected
        // crest, while the continuous visual body has safe capped witnesses
        // in all directions.
        let source = mask
            .iter()
            .copied()
            .filter(|candidate| {
                !summit_sources.contains_key(candidate)
                    && (5..=MASSIF_DISTRIBUTED_SUMMIT_RADIUS).contains(&crest.distance(*candidate))
                    && summit_sources
                        .keys()
                        .filter(|source| **source != crest)
                        .all(|source| source.distance(*candidate) >= 3)
                    && massif_offset_summit_candidate_is_viable(
                        crest, *candidate, summit, &depths, profile, seed,
                    )
            })
            .min_by_key(|candidate| {
                let radial = crest.distance(*candidate);
                let sector = enclosure_sector(crest, *candidate);
                (
                    usize::from(
                        protected_summit_source_sectors.len() < 3
                            && !(radial <= MASSIF_SUMMIT_BODY_RADIUS
                                && !protected_summit_source_sectors.contains(&sector)),
                    ),
                    usize::from(
                        summit_source_sectors.len() < 4 && summit_source_sectors.contains(&sector),
                    ),
                    Reverse(
                        distance_from_connector
                            .get(candidate)
                            .copied()
                            .unwrap_or_default(),
                    ),
                    radial.saturating_sub(14),
                    candidate.distance(target),
                    Reverse(
                        crest_selection_depths
                            .get(candidate)
                            .copied()
                            .unwrap_or_default(),
                    ),
                    Reverse(named_sample(
                        seed,
                        "grand_v3.highlands.massif_crest_lobe",
                        *candidate,
                    )),
                    *candidate,
                )
            })
            .ok_or_else(|| {
                contract(format!(
                    "Grand V3 massif cannot place irregular summit lobe {lobe}"
                ))
            })?;
        let drop = 8_i32.saturating_add(
            i32::try_from(
                named_sample(seed, "grand_v3.highlands.massif_crest_lobe_drop", source) % 18,
            )
            .unwrap_or_default(),
        );
        let primary_influence = summit.saturating_sub(
            i32::try_from(crest.distance(source))
                .unwrap_or(i32::MAX)
                .saturating_mul(MASSIF_PRIMARY_SUMMIT_SUPPORT_SLOPE),
        );
        let source_depth = depths.get(&source).copied().unwrap_or_default();
        let source_absolute_body = MASSIF_ABSOLUTE_BODY_BASE.saturating_add(
            i32::try_from(source_depth)
                .unwrap_or(i32::MAX)
                .saturating_mul(MASSIF_ABSOLUTE_BODY_RISE_PER_HEX),
        );
        let source_body = source_absolute_body
            .min(
                summit.saturating_sub(
                    i32::try_from(crest_depth.saturating_sub(source_depth))
                        .unwrap_or(i32::MAX)
                        .saturating_mul(MASSIF_ABSOLUTE_BODY_RISE_PER_HEX),
                ),
            )
            .max(profile.high_core_level)
            .min(massif_body_crest_cap(summit, crest.distance(source)));
        let source_primary = source_body.max(
            primary_influence
                .min(source_absolute_body.saturating_add(MASSIF_SUMMIT_EDGE_LIFT))
                .max(profile.high_core_level),
        );
        let source_level = summit
            .saturating_sub(drop)
            .max(primary_influence.saturating_add(2))
            .max(source_primary.saturating_add(2))
            .min(source_absolute_body.saturating_add(MASSIF_SUMMIT_EDGE_LIFT))
            // Secondary crests should read as neighboring peaks, not merge
            // into one broad near-maximum cap around the tallest summit.
            .min(summit.saturating_sub(10))
            // Source metadata must describe terrain that survives the final
            // universal continuity cap. A higher nominal pin can influence
            // neighboring columns while being absent at its own coordinate,
            // which recreates the single-cone failure behind honest-looking
            // source metadata.
            .min(source_absolute_body.max(profile.high_core_level));
        summit_sources.insert(source, source_level);
        summit_source_sectors.insert(enclosure_sector(crest, source));
        if crest.distance(source) <= MASSIF_SUMMIT_BODY_RADIUS {
            protected_summit_source_sectors.insert(enclosure_sector(crest, source));
        }
    }
    let summit_support = massif_summit_support(mask, &summit_sources, crest, profile.massif_floor);
    let level_without_shoulders = |coord: HexCoord| {
        let depth = depths.get(&coord).copied().unwrap_or_default();
        let absolute_body = MASSIF_ABSOLUTE_BODY_BASE.saturating_add(
            i32::try_from(depth)
                .unwrap_or(i32::MAX)
                .saturating_mul(MASSIF_ABSOLUTE_BODY_RISE_PER_HEX),
        );
        let inward_envelope = summit.saturating_sub(
            i32::try_from(crest_depth.saturating_sub(depth))
                .unwrap_or(i32::MAX)
                .saturating_mul(MASSIF_ABSOLUTE_BODY_RISE_PER_HEX),
        );
        let body = absolute_body
            .min(inward_envelope)
            .max(profile.high_core_level);
        let body = body.min(massif_body_crest_cap(summit, crest.distance(coord)));
        let support = summit_support
            .get(&coord)
            .copied()
            .unwrap_or(profile.massif_floor)
            .max(profile.high_core_level);
        body.max(
            support
                .min(absolute_body.saturating_add(MASSIF_SUMMIT_EDGE_LIFT))
                .max(profile.high_core_level),
        )
    };
    let mut shoulder_sources = BTreeMap::new();
    for lobe in 0..6_usize {
        let direction = (primary_direction + lobe) % 6;
        let target_radius = 14_u32.saturating_add(
            u32::try_from(
                named_sample(
                    seed,
                    "grand_v3.highlands.massif_shoulder_radius",
                    step_in_direction(crest, direction, 14),
                ) % 17,
            )
            .unwrap_or_default(),
        );
        let target = step_in_direction(crest, direction, target_radius);
        let source = semantic_owner_mask
            .iter()
            .copied()
            .filter(|candidate| {
                !shoulder_sources.contains_key(candidate)
                    && (MASSIF_SHOULDER_MIN_DISTANCE..=MASSIF_SHOULDER_MAX_DISTANCE)
                        .contains(&crest.distance(*candidate))
                    && depths
                        .get(candidate)
                        .is_some_and(|depth| *depth > MASSIF_OUTER_FEATHER_DEPTH)
                    && depths.get(candidate).is_some_and(|depth| {
                        let current = level_without_shoulders(*candidate);
                        let edge_cap = MASSIF_ABSOLUTE_BODY_BASE
                            .saturating_add(
                                i32::try_from(*depth)
                                    .unwrap_or(i32::MAX)
                                    .saturating_mul(MASSIF_SHOULDER_EDGE_RISE_PER_HEX),
                            )
                            .saturating_add(MASSIF_SHOULDER_EDGE_LIFT);
                        let final_cap = MASSIF_ABSOLUTE_BODY_BASE
                            .saturating_add(
                                i32::try_from(*depth)
                                    .unwrap_or(i32::MAX)
                                    .saturating_mul(MASSIF_ABSOLUTE_BODY_RISE_PER_HEX),
                            )
                            .max(profile.high_core_level);
                        let drop = 38_i32.saturating_add(
                            i32::try_from(
                                named_sample(
                                    seed,
                                    "grand_v3.highlands.massif_shoulder_drop",
                                    *candidate,
                                ) % 39,
                            )
                            .unwrap_or_default(),
                        );
                        let primary_influence = summit.saturating_sub(
                            i32::try_from(crest.distance(*candidate))
                                .unwrap_or(i32::MAX)
                                .saturating_mul(MASSIF_PRIMARY_SUMMIT_SUPPORT_SLOPE),
                        );
                        let candidate_source = summit
                            .saturating_sub(drop)
                            .max(primary_influence.saturating_add(2))
                            .max(current.saturating_add(2))
                            .min(edge_cap)
                            .min(summit.saturating_sub(28));
                        let final_current = current.min(final_cap);
                        let final_source = candidate_source.min(final_cap);
                        current <= summit.saturating_sub(30)
                            && edge_cap >= current.saturating_add(2)
                            && edge_cap >= summit.saturating_sub(87)
                            && final_source >= final_current.saturating_add(3)
                    })
            })
            .min_by_key(|candidate| {
                (
                    candidate.distance(target),
                    Reverse(depths.get(candidate).copied().unwrap_or_default()),
                    named_sample(
                        seed,
                        "grand_v3.highlands.massif_shoulder_source",
                        *candidate,
                    ),
                    *candidate,
                )
            })
            .ok_or_else(|| {
                contract(format!(
                    "Grand V3 massif cannot place broad shoulder source {lobe}"
                ))
            })?;
        let drop = 38_i32.saturating_add(
            i32::try_from(
                named_sample(seed, "grand_v3.highlands.massif_shoulder_drop", source) % 39,
            )
            .unwrap_or_default(),
        );
        let primary_influence = summit.saturating_sub(
            i32::try_from(crest.distance(source))
                .unwrap_or(i32::MAX)
                .saturating_mul(MASSIF_PRIMARY_SUMMIT_SUPPORT_SLOPE),
        );
        let source_depth = depths.get(&source).copied().unwrap_or_default();
        let shoulder_edge_cap = MASSIF_ABSOLUTE_BODY_BASE
            .saturating_add(
                i32::try_from(source_depth)
                    .unwrap_or(i32::MAX)
                    .saturating_mul(MASSIF_SHOULDER_EDGE_RISE_PER_HEX),
            )
            .saturating_add(MASSIF_SHOULDER_EDGE_LIFT);
        let source_level = summit
            .saturating_sub(drop)
            .max(primary_influence.saturating_add(2))
            .max(level_without_shoulders(source).saturating_add(2))
            .min(shoulder_edge_cap)
            .min(summit.saturating_sub(28));
        shoulder_sources.insert(source, source_level);
    }
    let shoulder_support = propagate_influence(
        mask,
        &shoulder_sources,
        MASSIF_SHOULDER_SUPPORT_SLOPE,
        profile.massif_floor,
    );
    let summit_core = crest
        .within_radius(MASSIF_PROTECTED_BODY_RADIUS)
        .into_iter()
        .filter(|coord| summit_support.contains_key(coord))
        .collect::<BTreeSet<_>>();
    let expected_protected_size = usize::try_from(
        1_u32.saturating_add(
            3_u32
                .saturating_mul(MASSIF_PROTECTED_BODY_RADIUS)
                .saturating_mul(MASSIF_PROTECTED_BODY_RADIUS.saturating_add(1)),
        ),
    )
    .unwrap_or(usize::MAX);
    if summit_core.len() != expected_protected_size {
        return Err(contract(format!(
            "Grand V3 massif cannot preserve its radius-fourteen summit taper: found {} of {expected_protected_size} columns",
            summit_core.len()
        )));
    }
    if summit <= PEAK_SUMMIT_MAX || summit >= MAX_V3_LEVEL {
        return Err(contract(format!(
            "Grand V3 massif summit {summit} must remain above peaks and below the V3 ceiling"
        )));
    }
    let connector_distance = distance_from_connector
        .into_iter()
        .filter(|(_, distance)| *distance <= MASSIF_OUTER_FEATHER_DEPTH)
        .collect::<BTreeMap<_, _>>();
    let field = MassifField {
        mask: mask.clone(),
        semantic_owner_mask: semantic_owner_mask.clone(),
        connector_mask: connector_mask.clone(),
        connector_distance,
        boundary_depth: depths,
        crest,
        summit,
        summit_sources,
        summit_support,
        shoulder_sources,
        shoulder_support,
        summit_core,
        floor: profile.massif_floor,
    };
    validate_massif_connector_scalar_bridge(&field)?;
    validate_massif_irregular_source_contribution(&field, profile.high_core_level)?;
    Ok(field)
}

fn massif_summit_support(
    mask: &BTreeSet<HexCoord>,
    sources: &BTreeMap<HexCoord, Level>,
    crest: HexCoord,
    floor: Level,
) -> BTreeMap<HexCoord, Level> {
    let primary = sources
        .get(&crest)
        .copied()
        .map(|level| BTreeMap::from([(crest, level)]))
        .unwrap_or_default();
    let offset = sources
        .iter()
        .filter(|(coord, _)| **coord != crest)
        .map(|(coord, level)| (*coord, *level))
        .collect::<BTreeMap<_, _>>();
    let mut resolved =
        propagate_influence(mask, &primary, MASSIF_PRIMARY_SUMMIT_SUPPORT_SLOPE, floor);
    for (coord, level) in propagate_influence(mask, &offset, MASSIF_SUMMIT_SUPPORT_SLOPE, floor) {
        resolved
            .entry(coord)
            .and_modify(|current| *current = (*current).max(level))
            .or_insert(level);
    }
    resolved
}

/// Proves that the seeded offset sources survive the Massif's body and edge
/// envelopes. Merely publishing several source coordinates is insufficient:
/// an absolute body or the central crest cone can dominate them completely
/// while retaining all of the nominal source metadata.
fn validate_massif_irregular_source_contribution(
    field: &MassifField,
    baseline: Level,
) -> Result<(), V3GenerationError> {
    let mut full_summits = field.clone();
    full_summits.shoulder_sources.clear();
    full_summits.shoulder_support.clear();
    let mut primary_only = field.clone();
    primary_only
        .summit_sources
        .retain(|coord, _| *coord == field.crest);
    primary_only.summit_support = massif_summit_support(
        &field.mask,
        &primary_only.summit_sources,
        field.crest,
        field.floor,
    );
    primary_only.shoulder_sources.clear();
    primary_only.shoulder_support.clear();
    let summit_witnesses = field
        .mask
        .iter()
        .copied()
        .filter(|coord| field.crest.distance(*coord) <= MASSIF_DISTRIBUTED_SUMMIT_RADIUS)
        .filter(|coord| {
            full_summits.resolve(*coord, baseline)
                >= primary_only.resolve(*coord, baseline).saturating_add(2)
        })
        .collect::<BTreeSet<_>>();
    let summit_sectors = summit_witnesses
        .iter()
        .map(|coord| enclosure_sector(field.crest, *coord))
        .collect::<BTreeSet<_>>();
    let contributing_summit_sources = field
        .summit_sources
        .keys()
        .filter(|coord| **coord != field.crest)
        .filter(|coord| {
            full_summits.resolve(**coord, baseline)
                >= primary_only.resolve(**coord, baseline).saturating_add(2)
        })
        .count();
    if summit_witnesses.len() < 8 || summit_sectors.len() < 4 || contributing_summit_sources < 4 {
        let source_diagnostics = field
            .summit_sources
            .iter()
            .filter(|(coord, _)| **coord != field.crest)
            .map(|(coord, source)| {
                (
                    *coord,
                    *source,
                    full_summits.resolve(*coord, baseline),
                    primary_only.resolve(*coord, baseline),
                    field.boundary_depth.get(coord).copied(),
                    field.connector_distance.get(coord).copied(),
                )
            })
            .collect::<Vec<_>>();
        return Err(contract(format!(
            "Grand V3 Massif offset summits do not materially break the central cone: witnesses={}, sectors={}, contributing_sources={contributing_summit_sources}, sources={source_diagnostics:?}",
            summit_witnesses.len(),
            summit_sectors.len(),
        )));
    }

    let mut without_shoulders = field.clone();
    without_shoulders.shoulder_sources.clear();
    without_shoulders.shoulder_support.clear();
    let shoulder_witnesses = field
        .mask
        .iter()
        .copied()
        .filter(|coord| field.crest.distance(*coord) > 10)
        .filter(|coord| {
            field.resolve(*coord, baseline)
                >= without_shoulders
                    .resolve(*coord, baseline)
                    .saturating_add(2)
        })
        .collect::<BTreeSet<_>>();
    let shoulder_sectors = shoulder_witnesses
        .iter()
        .map(|coord| enclosure_sector(field.crest, *coord))
        .collect::<BTreeSet<_>>();
    let contributing_shoulder_sources = field
        .shoulder_sources
        .keys()
        .filter(|coord| {
            field.resolve(**coord, baseline)
                >= without_shoulders
                    .resolve(**coord, baseline)
                    .saturating_add(2)
        })
        .count();
    if shoulder_witnesses.len() < 16
        || shoulder_sectors.len() < 4
        || contributing_shoulder_sources < 3
    {
        let source_diagnostics = field
            .shoulder_sources
            .iter()
            .map(|(coord, source)| {
                (
                    *coord,
                    *source,
                    field.resolve(*coord, baseline),
                    without_shoulders.resolve(*coord, baseline),
                    field.boundary_depth.get(coord).copied(),
                    field.connector_distance.get(coord).copied(),
                    field.semantic_owner_mask.contains(coord),
                )
            })
            .collect::<Vec<_>>();
        return Err(contract(format!(
            "Grand V3 Massif shoulder sources do not materially broaden the final terrain: witnesses={}, sectors={}, contributing_sources={contributing_shoulder_sources}, sources={source_diagnostics:?}",
            shoulder_witnesses.len(),
            shoulder_sectors.len(),
        )));
    }
    Ok(())
}

/// Proves that a visual-only Massif connector is the centre of a physical
/// Mountain shoulder rather than a one-column seam laid at the shared base.
///
/// The visual mask already includes an eight-row overlay-free Mountain
/// feather.  The important scalar contract is that the exact Crystal hole is
/// ignored while measuring the *outer* boundary: every connector column must
/// consequently be several rows inside that boundary, and each connector
/// component must have a depth-decreasing topological route through
/// non-connector feather columns. Before projection this checks topology only;
/// the same Pareto search runs against the projected levels to prove bounded
/// local reversals and an overall descent to shared terrain.
fn validate_massif_connector_scalar_bridge(field: &MassifField) -> Result<(), V3GenerationError> {
    validate_massif_connector_profile(field, None)
}

fn validate_massif_connector_profile(
    field: &MassifField,
    projected: Option<&BTreeMap<HexCoord, Level>>,
) -> Result<(), V3GenerationError> {
    #[derive(Clone)]
    struct State {
        coord: HexCoord,
        level: Level,
        cumulative_rise: Level,
        feather_steps: u32,
        path: Vec<HexCoord>,
    }
    if field.connector_mask.is_empty() {
        return Ok(());
    }
    if let Some((coord, depth)) = field.connector_mask.iter().find_map(|coord| {
        let depth = field.boundary_depth.get(coord).copied().unwrap_or_default();
        (depth < MASSIF_CONNECTOR_MINIMUM_TAPER_DEPTH).then_some((*coord, depth))
    }) {
        return Err(contract(format!(
            "Grand V3 Massif visual connector collapsed to a narrow scalar seam at {coord:?}: outer depth {depth}"
        )));
    }

    for component in fine_components(&field.connector_mask) {
        let start = component
            .iter()
            .copied()
            .max_by_key(|coord| {
                (
                    field.boundary_depth.get(coord).copied().unwrap_or_default(),
                    Reverse(*coord),
                )
            })
            .ok_or_else(|| contract("Grand V3 Massif resolved an empty connector component"))?;
        let start_depth = field
            .boundary_depth
            .get(&start)
            .copied()
            .unwrap_or_default();
        let level_at = |coord: HexCoord| {
            projected
                .and_then(|levels| levels.get(&coord).copied())
                .unwrap_or_else(|| field.resolve(coord, field.floor))
        };
        let start_level = level_at(start);
        let mut states = vec![State {
            coord: start,
            level: start_level,
            cumulative_rise: 0,
            feather_steps: 0,
            path: vec![start],
        }];
        for next_depth in (0..start_depth).rev() {
            let mut next_states = BTreeMap::<HexCoord, Vec<State>>::new();
            for state in states {
                let mut neighbors = state.coord.neighbors();
                neighbors.sort_unstable();
                for neighbor in neighbors {
                    if !field.mask.contains(&neighbor)
                        || field.boundary_depth.get(&neighbor).copied() != Some(next_depth)
                    {
                        continue;
                    }
                    let level = level_at(neighbor);
                    let outward_rise = level.saturating_sub(state.level).max(0);
                    let cumulative_rise = state.cumulative_rise.saturating_add(outward_rise);
                    if projected.is_some()
                        && (state.level.abs_diff(level) > 9
                            || outward_rise > 3
                            || cumulative_rise > 6)
                    {
                        continue;
                    }
                    let feather_steps = state.feather_steps.saturating_add(u32::from(
                        !field.connector_mask.contains(&neighbor)
                            && !field.semantic_owner_mask.contains(&neighbor),
                    ));
                    let mut path = state.path.clone();
                    path.push(neighbor);
                    let candidate = State {
                        coord: neighbor,
                        level,
                        cumulative_rise,
                        feather_steps,
                        path,
                    };
                    let frontier = next_states.entry(neighbor).or_default();
                    if frontier.iter().any(|existing| {
                        existing.cumulative_rise <= candidate.cumulative_rise
                            && existing.feather_steps >= candidate.feather_steps
                            && (existing.cumulative_rise < candidate.cumulative_rise
                                || existing.feather_steps > candidate.feather_steps
                                || existing.path <= candidate.path)
                    }) {
                        continue;
                    }
                    frontier.retain(|existing| {
                        !(candidate.cumulative_rise <= existing.cumulative_rise
                            && candidate.feather_steps >= existing.feather_steps)
                    });
                    frontier.push(candidate);
                }
            }
            states = next_states.into_values().flatten().collect();
            if states.is_empty() {
                break;
            }
        }
        let accepted = states.iter().find(|state| {
            field.boundary_depth.get(&state.coord).copied() == Some(0)
                && state.feather_steps >= MASSIF_CONNECTOR_MINIMUM_TAPER_DEPTH
                && (projected.is_none()
                    || (state.cumulative_rise <= 6 && state.level < start_level))
        });
        if accepted.is_none() {
            let diagnostic = states
                .iter()
                .min_by_key(|state| {
                    (
                        state.cumulative_rise,
                        Reverse(state.feather_steps),
                        state.level,
                        &state.path,
                    )
                })
                .map(|state| {
                    (
                        state.coord,
                        state.level,
                        state.cumulative_rise,
                        state.feather_steps,
                        state.path.clone(),
                    )
                });
            return Err(contract(format!(
                "Grand V3 Massif connector component has no valid {} radial profile: start={start:?}, start_level={start_level}, best_terminal={diagnostic:?}",
                if projected.is_some() {
                    "projected overall-descending"
                } else {
                    "topological"
                }
            )));
        }
    }
    Ok(())
}

fn crystal_context(
    plan: &SchematicPlanV1,
    layout: &ResolvedLayoutPlan,
    profile: V3GrandV3BasicTerrainProfile,
) -> Result<CrystalContext, V3GenerationError> {
    let crystal_cells = plan
        .cells
        .iter()
        .filter(|cell| {
            cell.facts
                .overlays
                .contains(&SchematicFeature::CrystalAscent)
        })
        .collect::<Vec<_>>();
    let [crystal_cell] = crystal_cells.as_slice() else {
        return Err(contract(format!(
            "Grand V3 highlands require exactly one Crystal cell; found {}",
            crystal_cells.len()
        )));
    };
    let crystal_patch_id = PatchId(u32::from(crystal_cell.id.get()));
    let crystal_patch = layout
        .patches
        .get(&crystal_patch_id)
        .ok_or_else(|| contract("Grand V3 Crystal cell has no resolved radius-32 patch"))?;
    let center = schematic_to_world(crystal_cell.coord);
    let expected_site = 1_u32.saturating_add(
        3_u32
            .saturating_mul(CRYSTAL_SITE_RADIUS)
            .saturating_mul(CRYSTAL_SITE_RADIUS.saturating_add(1)),
    );
    if crystal_patch.mask.len() != usize::try_from(expected_site).unwrap_or(usize::MAX)
        || crystal_patch
            .mask
            .iter()
            .any(|coord| center.distance(*coord) > CRYSTAL_SITE_RADIUS)
    {
        return Err(contract(
            "Grand V3 Crystal claim is not the exact radius-32 authored site",
        ));
    }
    let exit_clearance = crystal_mantle_exit_clearance(
        &crystal_patch.mask,
        crystal_patch.rotation_turns,
        profile,
        &layout.footprint,
    )?;
    let tunnel_neighbors = plan
        .networks
        .iter()
        .filter(|network| network.kind == NetworkKind::Tunnel)
        .flat_map(|network| &network.edges)
        .flat_map(|edge| edge.path.windows(2))
        .filter_map(|pair| {
            (pair[0] == crystal_cell.coord)
                .then_some(pair[1])
                .or_else(|| (pair[1] == crystal_cell.coord).then_some(pair[0]))
        })
        .collect::<BTreeSet<_>>();
    if tunnel_neighbors.len() != 1 {
        return Err(contract(format!(
            "Grand V3 Crystal mantle requires exactly one adjacent locked tunnel cell; found {}",
            tunnel_neighbors.len()
        )));
    }
    let tunnel_neighbor = tunnel_neighbors
        .first()
        .copied()
        .ok_or_else(|| contract("Grand V3 Crystal mantle lost its locked-tunnel neighbor"))?;
    Ok(CrystalContext {
        schematic: crystal_cell.coord,
        center,
        mask: crystal_patch.mask.clone(),
        rotation_turns: crystal_patch.rotation_turns,
        tunnel_neighbor: schematic_to_world(tunnel_neighbor),
        exit_clearance,
    })
}

fn build_crystal_mantle(
    plan: &SchematicPlanV1,
    layout: &ResolvedLayoutPlan,
    profile: V3GrandV3BasicTerrainProfile,
    crystal: &CrystalContext,
    frozen_plateau: &FrozenPlateauField,
    crystal_exit_ceiling: &BTreeMap<HexCoord, Level>,
) -> Result<
    (
        BTreeMap<HexCoord, Level>,
        BTreeMap<HexCoord, u32>,
        CrystalMantleAuthority,
    ),
    V3GenerationError,
> {
    let natural_shell_skin = super::crystal_ascent::macro_composite_natural_shell_skin_coords(
        &crystal.mask,
        crystal.rotation_turns,
    )
    .map_err(contract)?;
    let exposed_shell_openings =
        super::crystal_ascent::macro_composite_exposed_shell_opening_coords(
            &crystal.mask,
            crystal.rotation_turns,
        )
        .map_err(contract)?;
    let mut eligible = BTreeSet::new();
    for cell in &plan.cells {
        if cell.facts.surface != SurfaceKind::Land
            || cell.facts.overlays.iter().any(|overlay| {
                matches!(
                    overlay,
                    SchematicFeature::MountainLake
                        | SchematicFeature::LakeIsland
                        | SchematicFeature::Waterfall
                        | SchematicFeature::PeakRing
                        | SchematicFeature::FrozenWoods
                )
            })
        {
            continue;
        }
        let patch_id = PatchId(u32::from(cell.id.get()));
        let patch = layout.patches.get(&patch_id).ok_or_else(|| {
            contract(format!(
                "Grand V3 mantle cell {} has no resolved patch",
                cell.id.get()
            ))
        })?;
        for coord in &patch.mask {
            if !crystal.mask.contains(coord) && !crystal.exit_clearance.contains(coord) {
                eligible.insert(*coord);
            }
        }
    }

    let tunnel_direction = crystal
        .center
        .neighbors()
        .into_iter()
        .enumerate()
        .min_by_key(|(index, neighbor)| (neighbor.distance(crystal.tunnel_neighbor), *index))
        .map(|(index, _)| index)
        .ok_or_else(|| contract("Grand V3 Crystal mantle has no locked-tunnel direction"))?;
    // Preserve the two authored passages as genuine openings in the terrain
    // field. The tunnel clearance follows its coarse bearing all the way through
    // the neighboring-biome band; the upper clearance is derived from Crystal's
    // exact rotated four-wide terminal.
    let tunnel_clearance = (0..=CRYSTAL_ENCLOSURE_OUTER_RADIUS.saturating_add(4))
        .flat_map(|distance| {
            step_in_direction(crystal.center, tunnel_direction, distance)
                .within_radius(CRYSTAL_ENCLOSURE_OPENING_HALF_WIDTH)
        })
        .filter(|coord| layout.footprint.contains(coord))
        .collect::<BTreeSet<_>>();
    let opening_clearance = tunnel_clearance
        .union(&crystal.exit_clearance)
        .copied()
        .collect::<BTreeSet<_>>();
    // Begin immediately outside the exact authored radius-32 site. The prior
    // radius-36 start left three empty rows around the shell, exposing both a
    // void-like moat and the complete worked-stone cylinder from the valley.
    let inner_radius = CRYSTAL_SITE_RADIUS.saturating_add(1);
    let mut support_footprint = eligible
        .iter()
        .copied()
        .filter(|coord| {
            let radius = crystal.center.distance(*coord);
            (inner_radius..=CRYSTAL_ENCLOSURE_OUTER_RADIUS).contains(&radius)
                && !opening_clearance.contains(coord)
        })
        .collect::<BTreeSet<_>>();
    let mut enclosure_band = support_footprint
        .iter()
        .copied()
        .filter(|coord| {
            (CRYSTAL_ENCLOSURE_HIGH_INNER_RADIUS..=CRYSTAL_ENCLOSURE_HIGH_OUTER_RADIUS)
                .contains(&crystal.center.distance(*coord))
        })
        .collect::<BTreeSet<_>>();
    if enclosure_band.len() < 1_000 {
        return Err(contract(format!(
            "Grand V3 Crystal enclosure has only {} neighboring-biome columns",
            enclosure_band.len()
        )));
    }

    // The authored Crystal disk is an enclosed object, not a scalar-field
    // boundary. Fade only toward the true outer terrain edge and the two exact
    // openings; fading at radius 33 recreates the exposed cylindrical wall.
    let mut edge_depth = outer_boundary_depth(&support_footprint, &crystal.mask);

    // One broad source per hex sector raises actual neighboring biome masks.
    // Sources are deliberately not joined by authored lines; their overlapping
    // scalar shoulders form mountain bodies rather than a retaining ridge.
    let source_span = CRYSTAL_ENCLOSURE_HIGH_MAX
        .saturating_sub(CRYSTAL_ENCLOSURE_HIGH_MIN)
        .saturating_add(1);
    let mut sector_pins = BTreeMap::new();
    let mut sources = BTreeMap::new();
    for direction in 0..6 {
        let radial_jitter = u32::try_from(
            named_sample(
                plan.provenance.world_seed,
                "grand_v3.highlands.crystal_enclosure_radius",
                step_in_direction(crystal.center, direction, CRYSTAL_ENCLOSURE_SOURCE_RADIUS),
            ) % 7,
        )
        .unwrap_or_default();
        let radius = CRYSTAL_ENCLOSURE_SOURCE_RADIUS
            .saturating_sub(3)
            .saturating_add(radial_jitter);
        let target = step_in_direction(crystal.center, direction, radius);
        let source = enclosure_band
            .iter()
            .copied()
            // Search the complete sector so the natural shoulder routes around
            // the Frozen-Woods plateau instead of uplifting its core. Selecting
            // only near the nominal radial target made that authored low opening
            // consume the entire sector.
            .filter(|coord| enclosure_sector(crystal.center, *coord) == direction)
            .max_by_key(|coord| {
                (
                    edge_depth.get(coord).copied().unwrap_or_default(),
                    Reverse(coord.distance(target)),
                    Reverse(named_sample(
                        plan.provenance.world_seed,
                        "grand_v3.highlands.crystal_enclosure_source",
                        *coord,
                    )),
                    Reverse(*coord),
                )
            })
            .ok_or_else(|| {
                contract(format!(
                    "Grand V3 Crystal enclosure cannot resolve broad sector {direction}"
                ))
            })?;
        let level = CRYSTAL_ENCLOSURE_HIGH_MIN.saturating_add(
            i32::try_from(
                named_sample(
                    plan.provenance.world_seed,
                    "grand_v3.highlands.crystal_enclosure_height",
                    source,
                ) % u64::try_from(source_span).unwrap_or(1),
            )
            .unwrap_or_default(),
        );
        sources.insert(source, level);
        // `level` is the scalar source target. The separately applied edge
        // blend may intentionally resolve a few levels below that target while
        // still retaining the authored high shoulder. Record the actual hard
        // contract here: every sector pin must finish at least in the high
        // enclosure band. The compiler seals its exact post-blend cap before
        // any route is allowed to touch the terrain.
        sector_pins.insert(
            u8::try_from(direction).unwrap_or_default(),
            (source, CRYSTAL_ENCLOSURE_HIGH_MIN),
        );
    }
    let crystal_settings = V3CrystalAscentSettings {
        base_level: profile.crystal_base_level,
        rise_levels: profile.crystal_rise_levels,
    };
    let crystal_architecture_top =
        super::crystal_ascent::macro_highest_authored_surface_level(&crystal_settings);
    let exterior_worked_tops = super::crystal_ascent::macro_composite_exterior_worked_tops(
        &crystal.mask,
        crystal.rotation_turns,
        &crystal_settings,
    )
    .map_err(contract)?;
    if exterior_worked_tops.is_empty()
        || exterior_worked_tops
            .keys()
            .any(|coord| !natural_shell_skin.contains(coord))
    {
        return Err(contract(
            "Grand V3 Crystal exterior-worked profile escaped its typed natural shell skin",
        ));
    }
    let shell_overburden = super::crystal_ascent::macro_composite_natural_shell_overburden(
        &crystal.mask,
        crystal.rotation_turns,
    )
    .map_err(contract)?;
    let mut composite_crystal_top = crystal_architecture_top;
    for (shell_coord, level) in &exterior_worked_tops {
        let thickness = shell_overburden.get(shell_coord).copied().ok_or_else(|| {
            contract(format!(
                "Grand V3 Crystal exterior-worked column {shell_coord:?} has no composite overburden"
            ))
        })?;
        composite_crystal_top = composite_crystal_top.max(level.saturating_add(thickness));
    }
    if exterior_worked_tops.values().copied().max() != Some(crystal_architecture_top)
        || composite_crystal_top >= CRYSTAL_ENCLOSURE_HIGH_MIN
    {
        return Err(contract(format!(
            "Grand V3 Crystal enclosure has no typed height band above architecture {crystal_architecture_top} and composite top {composite_crystal_top}"
        )));
    }
    let mut shell_concealment_floors = BTreeMap::new();
    let mut shell_concealment_ceilings = BTreeMap::new();
    for (shell_coord, level) in &exterior_worked_tops {
        let natural_shell_top = level.saturating_add(
            shell_overburden
                .get(shell_coord)
                .copied()
                .ok_or_else(|| {
                    contract(format!(
                        "Grand V3 Crystal exterior-worked column {shell_coord:?} has no composite overburden"
                    ))
                })?,
        );
        for neighbor in shell_coord.neighbors() {
            if crystal.mask.contains(&neighbor) || opening_clearance.contains(&neighbor) {
                continue;
            }
            shell_concealment_floors
                .entry(neighbor)
                .and_modify(|floor: &mut Level| *floor = (*floor).max(*level))
                .or_insert(*level);
            let ceiling = natural_shell_top.saturating_add(CRYSTAL_SHELL_MAXIMUM_APRON_RISE);
            shell_concealment_ceilings
                .entry(neighbor)
                .and_modify(|current: &mut Level| *current = (*current).min(ceiling))
                .or_insert(ceiling);
        }
    }
    if shell_concealment_floors.is_empty()
        || shell_concealment_floors
            .keys()
            .ne(shell_concealment_ceilings.keys())
        || shell_concealment_floors.iter().any(|(coord, floor)| {
            shell_concealment_ceilings
                .get(coord)
                .is_none_or(|ceiling| floor > ceiling)
        })
    {
        return Err(contract(
            "Grand V3 Crystal shell-concealment profile has an empty or incompatible natural apron",
        ));
    }
    let mut mantle = propagate_influence(
        &support_footprint,
        &sources,
        CRYSTAL_ENCLOSURE_SUPPORT_SLOPE,
        profile.beach_level,
    );
    for (coord, level) in &mut mantle {
        if !sources.contains_key(coord) {
            let chisel = i32::try_from(
                named_sample(
                    plan.provenance.world_seed,
                    "grand_v3.highlands.crystal_enclosure_chisel",
                    *coord,
                ) % 4,
            )
            .unwrap_or_default();
            *level = level.saturating_sub(chisel).max(profile.beach_level);
        }
        let radius = crystal.center.distance(*coord);
        if radius <= CRYSTAL_SITE_RADIUS.saturating_add(3) {
            let skin_variation = i32::try_from(
                named_sample(
                    plan.provenance.world_seed,
                    "grand_v3.highlands.crystal_natural_skin",
                    *coord,
                ) % 4,
            )
            .unwrap_or_default();
            *level = (*level).max(
                composite_crystal_top
                    .saturating_add(2)
                    .saturating_add(skin_variation),
            );
        }
    }
    // `propagate_influence` intentionally reaches only source-owned connected
    // components. Publish and blend the exact applied footprint, not unrelated
    // eligible islands that received no mantle sample.
    support_footprint = mantle.keys().copied().collect();
    enclosure_band.retain(|coord| support_footprint.contains(coord));
    edge_depth = outer_boundary_depth(&support_footprint, &crystal.mask);
    let shell_route_reservation = shell_concealment_floors
        .keys()
        .flat_map(|coord| coord.within_radius(CRYSTAL_SHELL_FROZEN_TRANSITION_DEPTH))
        .filter(|coord| !opening_clearance.contains(coord))
        .collect::<BTreeSet<_>>();
    let forced_low_frozen_halo = frozen_plateau
        .halo_distance
        .iter()
        .filter_map(|(coord, distance)| {
            let ceiling = FROZEN_PLATEAU_MAX.saturating_add(
                i32::try_from(*distance)
                    .unwrap_or(i32::MAX)
                    .saturating_mul(FROZEN_PLATEAU_MAXIMUM_STEP),
            );
            (enclosure_band.contains(coord)
                && !shell_route_reservation.contains(coord)
                && ceiling <= composite_crystal_top)
                .then_some((*coord, ceiling))
        })
        .collect::<BTreeMap<_, _>>();
    let forced_low_exit_blend = crystal_exit_ceiling
        .iter()
        .filter(|(coord, ceiling)| {
            enclosure_band.contains(coord)
                && !shell_route_reservation.contains(coord)
                && **ceiling <= composite_crystal_top
        })
        .map(|(coord, ceiling)| (*coord, *ceiling))
        .collect::<BTreeMap<_, _>>();
    let forced_low = forced_low_frozen_halo
        .keys()
        .chain(forced_low_exit_blend.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let forced_low_budget = enclosure_band
        .len()
        .saturating_add(CRYSTAL_ENCLOSURE_FORCED_LOW_MAXIMUM_FRACTION_DIVISOR - 1)
        / CRYSTAL_ENCLOSURE_FORCED_LOW_MAXIMUM_FRACTION_DIVISOR;
    if forced_low.is_empty()
        || forced_low.len() > forced_low_budget
        || forced_low_frozen_halo.keys().any(|coord| {
            frozen_plateau
                .halo_distance
                .get(coord)
                .is_none_or(|distance| *distance > CRYSTAL_ENCLOSURE_FORCED_LOW_MAXIMUM_DEPTH)
        })
        || forced_low_exit_blend.keys().any(|coord| {
            crystal
                .exit_clearance
                .iter()
                .map(|source| source.distance(*coord))
                .min()
                .is_none_or(|distance| distance > CRYSTAL_ENCLOSURE_FORCED_LOW_MAXIMUM_DEPTH)
        })
    {
        return Err(contract(format!(
            "Grand V3 Crystal forced-low coverage authority is empty, detached, or over budget: frozen={}, exit={}, union={}, budget={forced_low_budget}",
            forced_low_frozen_halo.len(),
            forced_low_exit_blend.len(),
            forced_low.len(),
        )));
    }
    let attainable_enclosure_band = enclosure_band
        .difference(&forced_low)
        .copied()
        .collect::<BTreeSet<_>>();
    let uplift_core = mantle
        .iter()
        .filter_map(|(coord, level)| {
            (*level > composite_crystal_top && attainable_enclosure_band.contains(coord))
                .then_some(*coord)
        })
        .collect::<BTreeSet<_>>();
    let required_coverage = attainable_enclosure_band
        .len()
        .saturating_mul(3)
        .saturating_add(4)
        / 5;
    if uplift_core.len() < required_coverage
        || sector_pins.len() != 6
        || sector_pins
            .values()
            .any(|(coord, _)| !uplift_core.contains(coord))
    {
        return Err(contract(format!(
            "Grand V3 Crystal enclosure lacks broad multi-sector height coverage: high={}, attainable-band={}, geometric-band={}, sectors={}",
            uplift_core.len(),
            attainable_enclosure_band.len(),
            enclosure_band.len(),
            sector_pins.len()
        )));
    }
    // Protect only a shallow inner screening band plus small summit-pin
    // islands. The former ridge implementation could reserve its entire narrow
    // shape; doing that to this broad terrain body would make whole neighboring
    // biome patches unavailable to the ordinary-route compiler.
    let route_exclusion = sector_pins
        .values()
        .flat_map(|(pin, _)| pin.within_radius(3))
        .filter(|coord| uplift_core.contains(coord))
        // A target-height sample on the first feather row can still resolve to
        // the shared baseline (or exactly Crystal-top) after edge blending.
        // Protect only the genuinely interior part of each lobe; the broad
        // final enclosure remains validated independently across the complete
        // shoulder band.
        .filter(|coord| edge_depth.get(coord).is_some_and(|depth| *depth >= 2))
        .filter(|coord| !opening_clearance.contains(coord))
        .collect::<BTreeSet<_>>();
    Ok((
        mantle,
        edge_depth,
        CrystalMantleAuthority {
            crystal_center: crystal.center,
            composite_crystal_top,
            uplift_core,
            support_footprint,
            enclosure_band,
            route_exclusion,
            sector_pins,
            opening_clearance,
            natural_shell_skin,
            exposed_shell_openings,
            shell_concealment_floors,
            shell_concealment_ceilings,
            forced_low_frozen_halo,
            forced_low_exit_blend,
            expected_uplift_caps: None,
        },
    ))
}

fn crystal_mantle_exit_clearance(
    crystal_mask: &BTreeSet<HexCoord>,
    rotation_turns: u8,
    profile: V3GrandV3BasicTerrainProfile,
    footprint: &BTreeSet<HexCoord>,
) -> Result<BTreeSet<HexCoord>, V3GenerationError> {
    let upper_rows = super::crystal_ascent::macro_upper_terminal_outward_rows(
        crystal_mask,
        rotation_turns,
        profile
            .crystal_base_level
            .saturating_add(profile.crystal_rise_levels),
        CRYSTAL_MANTLE_EXIT_CLEARANCE_DEPTH,
    )
    .map_err(|error| contract(error))?;
    Ok(upper_rows
        .into_iter()
        .flatten()
        .flat_map(|coord| coord.within_radius(CRYSTAL_MANTLE_EXIT_CLEARANCE_BUFFER))
        .filter(|coord| footprint.contains(coord))
        .collect())
}

pub(super) fn step_in_direction(mut coord: HexCoord, direction: usize, steps: u32) -> HexCoord {
    for _ in 0..steps {
        coord = coord.neighbors()[direction % 6];
    }
    coord
}

fn enclosure_sector(center: HexCoord, coord: HexCoord) -> usize {
    center
        .neighbors()
        .into_iter()
        .enumerate()
        .min_by_key(|(direction, probe)| (probe.distance(coord), *direction))
        .map(|(direction, _)| direction)
        .unwrap_or_default()
}

/// Require the same broad one-third radial representation as the original
/// eight-row check on a complete 25-row enclosure annulus. Some Grand sectors
/// are clipped by the finite world boundary, so their requirement must be
/// derived from the radial rows which actually exist in the typed domain.
fn crystal_enclosure_required_radial_depth(available_radii: usize) -> usize {
    (available_radii / 3).max(1)
}

fn integer_centroid(mask: &BTreeSet<HexCoord>) -> Result<HexCoord, V3GenerationError> {
    let count = i64::try_from(mask.len()).map_err(|_| contract("highland mask is too large"))?;
    if count == 0 {
        return Err(contract(
            "cannot find the centroid of an empty highland mask",
        ));
    }
    let q = mask
        .iter()
        .map(|coord| i64::from(coord.x()))
        .sum::<i64>()
        .checked_div(count)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| contract("highland centroid q coordinate overflowed"))?;
    let r = mask
        .iter()
        .map(|coord| i64::from(coord.y()))
        .sum::<i64>()
        .checked_div(count)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| contract("highland centroid r coordinate overflowed"))?;
    Ok(HexCoord::from_axial(q, r))
}

fn propagate_influence(
    mask: &BTreeSet<HexCoord>,
    sources: &BTreeMap<HexCoord, Level>,
    slope: Level,
    floor: Level,
) -> BTreeMap<HexCoord, Level> {
    let mut result = sources.clone();
    let mut queue = sources
        .iter()
        .map(|(coord, level)| (*level, Reverse(*coord)))
        .collect::<BinaryHeap<_>>();
    while let Some((level, Reverse(coord))) = queue.pop() {
        if result.get(&coord).copied() != Some(level) || level <= floor {
            continue;
        }
        let next_level = level.saturating_sub(slope).max(floor);
        for neighbor in coord.neighbors() {
            if !mask.contains(&neighbor)
                || result
                    .get(&neighbor)
                    .is_some_and(|current| *current >= next_level)
            {
                continue;
            }
            result.insert(neighbor, next_level);
            queue.push((next_level, Reverse(neighbor)));
        }
    }
    result
}

/// Minimum scalar ceiling reachable from exact source levels while rising by
/// at most `slope` per neighboring column.
///
/// This is the lower-envelope dual of [`propagate_influence`].  A min-heap
/// makes its state space exactly one best level per coordinate, so a broad
/// shoulder is resolved in `O((V + E) log V)` without enumerating paths.
fn propagate_rising_ceiling(
    mask: &BTreeSet<HexCoord>,
    sources: &BTreeMap<HexCoord, Level>,
    slope: Level,
) -> BTreeMap<HexCoord, Level> {
    let mut result = BTreeMap::new();
    let mut queue = BinaryHeap::<Reverse<(Level, HexCoord)>>::new();
    for (coord, level) in sources {
        if !mask.contains(coord)
            || result
                .get(coord)
                .is_some_and(|current: &Level| *current <= *level)
        {
            continue;
        }
        result.insert(*coord, *level);
        queue.push(Reverse((*level, *coord)));
    }
    while let Some(Reverse((level, coord))) = queue.pop() {
        if result.get(&coord).copied() != Some(level) {
            continue;
        }
        let next_level = level.saturating_add(slope);
        let mut neighbors = coord.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if !mask.contains(&neighbor)
                || result
                    .get(&neighbor)
                    .is_some_and(|current| *current <= next_level)
            {
                continue;
            }
            result.insert(neighbor, next_level);
            queue.push(Reverse((next_level, neighbor)));
        }
    }
    result
}

fn boundary_depth(mask: &BTreeSet<HexCoord>) -> BTreeMap<HexCoord, u32> {
    let boundary = mask
        .iter()
        .copied()
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .any(|neighbor| !mask.contains(&neighbor))
        })
        .collect::<Vec<_>>();
    distances_from(mask, boundary)
}

fn outer_boundary_depth(
    mask: &BTreeSet<HexCoord>,
    enclosed_hole: &BTreeSet<HexCoord>,
) -> BTreeMap<HexCoord, u32> {
    let boundary = mask
        .iter()
        .copied()
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .any(|neighbor| !mask.contains(&neighbor) && !enclosed_hole.contains(&neighbor))
        })
        .collect::<Vec<_>>();
    distances_from(mask, boundary)
}

fn distances_from(
    mask: &BTreeSet<HexCoord>,
    sources: impl IntoIterator<Item = HexCoord>,
) -> BTreeMap<HexCoord, u32> {
    let mut distances = BTreeMap::new();
    let mut queue = VecDeque::new();
    for source in sources {
        if mask.contains(&source) && distances.insert(source, 0_u32).is_none() {
            queue.push_back(source);
        }
    }
    while let Some(current) = queue.pop_front() {
        let distance = distances.get(&current).copied().unwrap_or_default();
        for neighbor in current.neighbors() {
            if mask.contains(&neighbor) && !distances.contains_key(&neighbor) {
                distances.insert(neighbor, distance.saturating_add(1));
                queue.push_back(neighbor);
            }
        }
    }
    distances
}

fn connected(mask: &BTreeSet<HexCoord>) -> bool {
    let Some(start) = mask.iter().next().copied() else {
        return false;
    };
    distances_from(mask, [start]).len() == mask.len()
}

fn schematic_to_world(coord: SchematicCoord) -> HexCoord {
    HexCoord::from_axial(
        coord.q().saturating_mul(CELL_PITCH),
        coord.r().saturating_mul(CELL_PITCH),
    )
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

fn contract(reason: impl Into<String>) -> V3GenerationError {
    V3GenerationError::RecipeContract(reason.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        ProceduralV3Settings, V3LayoutSettings, V3SchematicLayoutSettings, V3SchematicTemplate,
        V3SchematicTerrainProfile, V3_SCHEMATIC_GRID_RADIUS,
    };

    fn field_for_plan(plan: &SchematicPlanV1) -> GrandHighlandField {
        let settings = ProceduralV3Settings {
            layout: V3LayoutSettings::Schematic(V3SchematicLayoutSettings {
                template: V3SchematicTemplate::GrandV3,
                template_revision: crate::settings::V3_GRAND_V3_TEMPLATE_REVISION,
                cell_pitch: 22,
                terrain_profile: V3SchematicTerrainProfile::GrandV3BasicV1(
                    V3GrandV3BasicTerrainProfile::canonical(),
                ),
            }),
        };
        let mut layout = super::super::layout::resolve_layout(V3_SCHEMATIC_GRID_RADIUS, &settings)
            .expect("reference layout resolves");
        super::super::schematic_crystal::claim_site(plan, &mut layout, 22)
            .expect("Crystal site claim validates");
        GrandHighlandField::build(plan, &layout, V3GrandV3BasicTerrainProfile::canonical())
            .expect("reference highland field builds")
    }

    fn reference_field() -> GrandHighlandField {
        let template = hex_schematic::grand_v3_reference_template().expect("template parses");
        let reference = hex_schematic::reference_plan(&template, 0).expect("reference validates");
        field_for_plan(&reference.plan)
    }

    #[test]
    fn locked_peak_cells_resolve_connected_bases_and_twelve_separate_upper_crowns() {
        let field = reference_field();
        assert_eq!(field.peak_authority.components.len(), 2);
        assert!(field.peak_authority.components.iter().all(|component| {
            component.patch_masks.len() == 6
                && component.expected_peak_bodies.len() == 6
                && component.summit_pins.len() == 6
                && component.expected_saddle_swaths.len() >= 5
                && component.authorized_route_grades.is_none()
                && component.authorized_waterfall_openings.is_none()
                && fine_components(
                    &component
                        .expected_high_band
                        .keys()
                        .copied()
                        .collect::<BTreeSet<_>>(),
                )
                .len()
                    == 6
                && component.expected_peak_bodies.values().all(|body| {
                    body.len() >= 150
                        && connected(&body.keys().copied().collect::<BTreeSet<_>>())
                        && component
                            .summit_pins
                            .keys()
                            .any(|summit| body.contains_key(summit))
                })
        }));
        for component in &field.peak_authority.components {
            let bodies = component
                .expected_peak_bodies
                .values()
                .map(|body| body.keys().copied().collect::<BTreeSet<_>>())
                .collect::<Vec<_>>();
            for (index, body) in bodies.iter().enumerate() {
                assert!(bodies
                    .iter()
                    .skip(index.saturating_add(1))
                    .all(|other| body.is_disjoint(other)));
            }
            assert!(connected(
                &component
                    .expected_ridge_profile
                    .keys()
                    .copied()
                    .collect::<BTreeSet<_>>()
            ));
            for ((first, second), swath) in &component.expected_saddle_swaths {
                let first_pin = component
                    .summit_pins
                    .iter()
                    .find(|(pin, _)| component.expected_peak_bodies[first].contains_key(pin))
                    .expect("first saddle owner retains one summit");
                let second_pin = component
                    .summit_pins
                    .iter()
                    .find(|(pin, _)| component.expected_peak_bodies[second].contains_key(pin))
                    .expect("second saddle owner retains one summit");
                let ceiling = (*first_pin.1)
                    .min(*second_pin.1)
                    .saturating_sub(PEAK_SADDLE_DEPTH)
                    .min(PEAK_VISUAL_WALL_THRESHOLD.saturating_sub(1));
                assert!(swath.len() >= 4 && connected(swath));
                assert!(swath.iter().all(|coord| {
                    component
                        .expected_ridge_profile
                        .get(coord)
                        .is_some_and(|level| *level <= ceiling)
                }));
            }
        }
        let ridge_levels = field
            .peak_authority
            .components
            .iter()
            .flat_map(|component| component.expected_high_band.values().copied())
            .collect::<BTreeSet<_>>();
        assert!(ridge_levels
            .iter()
            .all(|level| *level >= PEAK_VISUAL_WALL_THRESHOLD));
        assert!(ridge_levels.iter().all(|level| *level <= PEAK_SUMMIT_MAX));
        assert!(
            ridge_levels.len() >= 8,
            "separate upper crowns lost their irregular height profile"
        );
    }

    #[test]
    fn sharp_peak_sources_transition_from_a_steep_crown_to_a_broad_lower_body() {
        let summit = 280;
        assert_eq!(peak_source_influence(summit, 0, 0), summit);
        assert_eq!(peak_source_influence(summit, 4, 0), 252);
        assert_eq!(peak_source_influence(summit, 8, 0), 230);
        let lower_profile = (8..=20)
            .map(|distance| peak_source_influence(summit, distance, 0))
            .collect::<Vec<_>>();
        assert!(lower_profile
            .windows(2)
            .all(|pair| pair[0].saturating_sub(pair[1]) == PEAK_LOWER_BODY_SLOPE));
        assert!(peak_source_influence(summit, 16, 0) > summit.saturating_sub(16 * PEAK_BODY_SLOPE));

        let field = reference_field();
        let profile = V3GrandV3BasicTerrainProfile::canonical();
        for component in &field.peak_authority.components {
            for (patch, body) in &component.expected_peak_bodies {
                let (summit_coord, summit_level) = component
                    .summit_pins
                    .iter()
                    .find(|(coord, _)| body.contains_key(coord))
                    .map(|(coord, level)| (*coord, *level))
                    .expect("each peak body retains one summit pin");
                let broad_middle = body
                    .iter()
                    .filter(|(coord, level)| {
                        (9..=15).contains(&summit_coord.distance(**coord))
                            && **level > profile.sharp_peak_bench_max.saturating_add(12)
                            && **level < summit_level
                    })
                    .count();
                assert!(
                    broad_middle >= 6,
                    "peak patch {} retained only {broad_middle} broad lower/middle-body witnesses",
                    patch.0
                );
            }
        }
    }

    #[test]
    fn peak_lobe_validator_rejects_a_counterfactual_primary_cone() {
        let field = reference_field();
        let component = field
            .peak_authority
            .components
            .first()
            .expect("reference map retains its first six-peak chain");
        let error = validate_peak_upper_lobe_contribution(
            &component.patch_masks,
            &component.expected_ridge_profile,
            &component.expected_ridge_profile,
        )
        .expect_err("a peak chain with no surviving upper-lobe contribution must fail");
        assert!(error.to_string().contains("upper lobes"));
    }

    #[test]
    fn inner_peak_foundation_joins_a_four_column_frozen_ingress_to_the_88_58_saddle() {
        let field = reference_field();
        let component = field
            .peak_authority
            .components
            .iter()
            .find(|component| {
                component
                    .patch_masks
                    .contains_key(&INNER_PEAK_INGRESS_PEAK_PATCH)
            })
            .expect("one peak chain owns exact endpoint Patch 88");
        assert_eq!(component.expected_external_ingress_swaths.len(), 1);
        let ingress = component
            .expected_external_ingress_swaths
            .get(&(
                INNER_PEAK_INGRESS_PEAK_PATCH,
                INNER_PEAK_INGRESS_FROZEN_PATCH,
            ))
            .expect("inner peak chain publishes the exact Frozen-123/Peak-88 swath");
        assert!(ingress.len() >= 4 && connected(ingress));
        assert!(ingress.iter().all(|coord| {
            let ceiling = field
                .frozen_plateau
                .halo_distance
                .get(coord)
                .copied()
                .map(|depth| {
                    FROZEN_PLATEAU_MAX.saturating_add(
                        i32::try_from(depth)
                            .unwrap_or(i32::MAX)
                            .saturating_mul(FROZEN_PLATEAU_MAXIMUM_STEP),
                    )
                });
            component
                .expected_ridge_profile
                .get(coord)
                .zip(ceiling)
                .is_some_and(|(level, ceiling)| *level <= ceiling)
        }));

        let patch_mask = &component.patch_masks[&INNER_PEAK_INGRESS_PEAK_PATCH];
        let internal = component
            .expected_saddle_swaths
            .get(&(PatchId(58), INNER_PEAK_INGRESS_PEAK_PATCH))
            .expect("inner peak chain retains its exact 88/58 saddle")
            .intersection(patch_mask)
            .copied()
            .collect::<BTreeSet<_>>();
        let low = patch_mask
            .iter()
            .copied()
            .filter(|coord| {
                component
                    .expected_ridge_profile
                    .get(coord)
                    .is_some_and(|level| *level < PEAK_VISUAL_WALL_THRESHOLD)
            })
            .collect::<BTreeSet<_>>();
        let shared = fine_components(&low)
            .into_iter()
            .find(|candidate| ingress.iter().all(|coord| candidate.contains(coord)))
            .expect("the complete external ingress lies in one sub-240 component");
        assert!(internal.iter().all(|coord| shared.contains(coord)));
        assert!(internal.iter().all(|coord| {
            component
                .expected_ridge_profile
                .get(coord)
                .is_some_and(|level| *level <= INNER_PEAK_ROUTE_SADDLE_CEILING)
        }));
        assert_eq!(
            field
                .peak_authority
                .components
                .iter()
                .map(|candidate| candidate.expected_external_ingress_swaths.len())
                .sum::<usize>(),
            1
        );
    }

    #[test]
    fn inner_peak_foundation_retains_one_ordered_patch_59_transit_spine() {
        let field = reference_field();
        let spines = field
            .peak_authority
            .components
            .iter()
            .flat_map(|component| component.ordered_saddle_spines.values())
            .collect::<Vec<_>>();
        let [spine] = spines.as_slice() else {
            panic!(
                "reference foundation retained {} ordered saddle spines instead of one",
                spines.len()
            );
        };
        let component = field
            .peak_authority
            .components
            .iter()
            .find(|component| component.patch_masks.contains_key(&spine.owner))
            .expect("the ordered spine retains its peak component");
        let owner_mask = &component.patch_masks[&spine.owner];
        let ingress_mask = &component.patch_masks[&spine.ingress_from];
        let egress_mask = &component.patch_masks[&spine.egress_to];
        let unique = spine.centerline.iter().copied().collect::<BTreeSet<_>>();

        assert_eq!(spine.owner, INNER_PEAK_TRANSIT_OWNER);
        assert_eq!(spine.ingress_from, INNER_PEAK_TRANSIT_INGRESS.0);
        assert_eq!(spine.egress_to, INNER_PEAK_TRANSIT_EGRESS.0);
        assert!(spine.centerline.len() >= 2);
        assert_eq!(unique.len(), spine.centerline.len());
        assert!(spine
            .centerline
            .windows(2)
            .all(|pair| pair[0].distance(pair[1]) == 1));
        assert_eq!(spine.required_grade_coords(), unique);
        assert_eq!(
            spine
                .authored_grades
                .keys()
                .copied()
                .collect::<BTreeSet<_>>(),
            unique
        );
        assert_eq!(
            spine.authored_grades.get(&spine.centerline[0]),
            Some(&INNER_PEAK_TRANSIT_RUNWAY_ENDPOINT_LEVEL)
        );
        assert_eq!(
            spine
                .authored_grades
                .get(spine.centerline.last().expect("ordered spine has an end")),
            Some(&INNER_PEAK_TRANSIT_RUNWAY_ENDPOINT_LEVEL)
        );
        assert!(spine.centerline.windows(2).all(|pair| {
            spine.authored_grades[&pair[0]].abs_diff(spine.authored_grades[&pair[1]]) <= 1
        }));
        assert!(spine.authored_grades.values().all(|level| {
            *level <= INNER_PEAK_ROUTE_SADDLE_CEILING
                && *level
                    >= INNER_PEAK_TRANSIT_RUNWAY_ENDPOINT_LEVEL.saturating_sub(PEAK_SADDLE_DEPTH)
        }));
        assert!(unique.is_subset(owner_mask));
        assert!(unique.is_subset(&spine.support_domain));
        assert!(connected(&spine.support_domain));
        assert_eq!(
            spine.authored_grades.get(&INNER_PEAK_TRANSIT_WEST_NOTCH),
            Some(&INNER_PEAK_ROUTE_SADDLE_CEILING)
        );
        let reauthored_high_cells = spine
            .centerline
            .iter()
            .filter(|coord| {
                component
                    .expected_ridge_profile
                    .get(coord)
                    .is_some_and(|level| *level >= PEAK_VISUAL_WALL_THRESHOLD)
            })
            .copied()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            reauthored_high_cells,
            BTreeSet::from([INNER_PEAK_TRANSIT_WEST_NOTCH])
        );
        let reauthored_high_support = spine
            .support_domain
            .iter()
            .filter(|coord| {
                component
                    .expected_ridge_profile
                    .get(coord)
                    .is_some_and(|level| *level >= PEAK_VISUAL_WALL_THRESHOLD)
            })
            .copied()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            reauthored_high_support,
            BTreeSet::from([INNER_PEAK_TRANSIT_WEST_NOTCH])
        );
        assert!(spine.centerline.iter().all(|coord| {
            component
                .expected_ridge_profile
                .get(coord)
                .is_some_and(|level| {
                    *level < PEAK_VISUAL_WALL_THRESHOLD
                        || ordered_peak_saddle_is_bounded_west_notch(
                            *coord,
                            *level,
                            &component.expected_ridge_profile,
                        )
                })
                && !component.summit_pins.contains_key(coord)
        }));
        let first = spine.centerline[0];
        let last = *spine.centerline.last().expect("ordered spine has an end");
        assert!(spine.ingress_portals.iter().all(|(from, to)| {
            *to == first && ingress_mask.contains(from) && from.distance(*to) == 1
        }));
        assert!(spine.egress_portals.iter().all(|(from, to)| {
            *from == last && egress_mask.contains(to) && from.distance(*to) == 1
        }));
    }

    #[test]
    fn ordered_peak_west_notch_is_one_bounded_seed_stable_exception() {
        let mut levels = INNER_PEAK_TRANSIT_WEST_NOTCH
            .neighbors()
            .into_iter()
            .map(|coord| (coord, INNER_PEAK_TRANSIT_WEST_NOTCH_RAW_MAX))
            .collect::<BTreeMap<_, _>>();
        for raw_level in [
            PEAK_VISUAL_WALL_THRESHOLD.saturating_sub(1),
            PEAK_VISUAL_WALL_THRESHOLD.saturating_add(1),
            INNER_PEAK_TRANSIT_WEST_NOTCH_RAW_MAX,
        ] {
            levels.insert(INNER_PEAK_TRANSIT_WEST_NOTCH, raw_level);
            assert!(ordered_peak_saddle_is_bounded_west_notch(
                INNER_PEAK_TRANSIT_WEST_NOTCH,
                raw_level,
                &levels,
            ));
        }
        assert!(!ordered_peak_saddle_is_bounded_west_notch(
            INNER_PEAK_TRANSIT_WEST_NOTCH,
            INNER_PEAK_TRANSIT_WEST_NOTCH_RAW_MAX.saturating_add(1),
            &levels,
        ));
        let high_neighbor = INNER_PEAK_TRANSIT_WEST_NOTCH
            .neighbors()
            .into_iter()
            .next()
            .expect("a hex has six neighbors");
        levels.insert(
            high_neighbor,
            INNER_PEAK_TRANSIT_WEST_NOTCH_RAW_MAX.saturating_add(1),
        );
        assert!(!ordered_peak_saddle_is_bounded_west_notch(
            INNER_PEAK_TRANSIT_WEST_NOTCH,
            PEAK_VISUAL_WALL_THRESHOLD.saturating_add(1),
            &levels,
        ));
        assert!(!ordered_peak_saddle_is_bounded_west_notch(
            HexCoord::ORIGIN,
            PEAK_VISUAL_WALL_THRESHOLD.saturating_add(1),
            &levels,
        ));
    }

    #[test]
    fn every_intermediate_peak_patch_joins_its_saddle_swaths_below_the_crowns() {
        let field = reference_field();
        let mut checked = BTreeSet::new();
        for component in &field.peak_authority.components {
            for (patch, patch_mask) in &component.patch_masks {
                let groups = component
                    .expected_saddle_swaths
                    .iter()
                    .filter_map(|(edge, swath)| {
                        (edge.0 == *patch || edge.1 == *patch).then(|| {
                            (
                                *edge,
                                swath
                                    .intersection(patch_mask)
                                    .copied()
                                    .collect::<BTreeSet<_>>(),
                                PEAK_VISUAL_WALL_THRESHOLD.saturating_sub(1),
                            )
                        })
                    })
                    .filter(|(_, group, _)| !group.is_empty())
                    .collect::<Vec<_>>();
                if groups.len() < 2 {
                    continue;
                }
                validate_peak_patch_saddle_connectivity(
                    *patch,
                    patch_mask,
                    &groups,
                    &component.expected_ridge_profile,
                )
                .expect("every intermediate peak joins all seam swaths below 240");
                let route_safe = if *patch == INNER_PEAK_TRANSIT_OWNER {
                    ordered_peak_saddle_route_ready_mask(
                        patch_mask,
                        &component.expected_ridge_profile,
                    )
                    .expect("Patch59 retains its bounded authored west notch")
                } else {
                    peak_saddle_route_ready_mask(patch_mask, &component.expected_ridge_profile)
                };
                let route_safe_components = fine_components(&route_safe);
                assert!(
                    route_safe_components.iter().any(|connected| {
                        groups.iter().all(|(_, group, _)| {
                            group.iter().any(|coord| connected.contains(coord))
                        })
                    }),
                    "peak patch {} saddle seams do not share one route-safe-low component",
                    patch.0
                );
                checked.insert(*patch);
            }
        }
        assert!(
            checked.contains(&PatchId(59)),
            "Patch59 must remain the defining two-saddle fixture"
        );
        assert!(
            checked.len() >= 8,
            "both six-peak chains must exercise intermediate saddle patches"
        );
    }

    #[test]
    fn peak_patch_saddle_connectivity_rejects_reintroduced_internal_barrier() {
        let field = reference_field();
        let component = field
            .peak_authority
            .components
            .iter()
            .find(|component| component.patch_masks.contains_key(&PatchId(59)))
            .expect("reference highlands retain Patch59");
        let patch = PatchId(59);
        let patch_mask = &component.patch_masks[&patch];
        let groups = component
            .expected_saddle_swaths
            .iter()
            .filter_map(|(edge, swath)| {
                (edge.0 == patch || edge.1 == patch).then(|| {
                    let other = if edge.0 == patch { edge.1 } else { edge.0 };
                    let other_mask = &component.patch_masks[&other];
                    (
                        *edge,
                        swath
                            .intersection(patch_mask)
                            .filter(|coord| {
                                coord
                                    .neighbors()
                                    .into_iter()
                                    .any(|neighbor| other_mask.contains(&neighbor))
                            })
                            .copied()
                            .collect::<BTreeSet<_>>(),
                        PEAK_VISUAL_WALL_THRESHOLD.saturating_sub(1),
                    )
                })
            })
            .filter(|(_, group, _)| !group.is_empty())
            .collect::<Vec<_>>();
        assert!(groups.len() >= 2);
        let protected_swaths = groups
            .iter()
            .flat_map(|(_, group, _)| group.iter().copied())
            .collect::<BTreeSet<_>>();
        let mut blocked = component.expected_ridge_profile.clone();
        for coord in patch_mask.difference(&protected_swaths) {
            blocked.insert(*coord, PEAK_VISUAL_WALL_THRESHOLD);
        }
        let error = validate_peak_patch_saddle_connectivity(patch, patch_mask, &groups, &blocked)
            .expect_err("restoring a high internal Patch59 barrier must fail");
        assert!(error.to_string().contains("disconnected sub-240"));
    }

    #[test]
    fn peak_chains_continue_through_owned_mountain_feathers_without_boundary_cliffs() {
        let template = hex_schematic::grand_v3_reference_template().expect("template parses");
        let reference = hex_schematic::reference_plan(&template, 0).expect("reference validates");
        let plan = reference.plan;
        let settings = ProceduralV3Settings {
            layout: V3LayoutSettings::Schematic(V3SchematicLayoutSettings {
                template: V3SchematicTemplate::GrandV3,
                template_revision: crate::settings::V3_GRAND_V3_TEMPLATE_REVISION,
                cell_pitch: 22,
                terrain_profile: V3SchematicTerrainProfile::GrandV3BasicV1(
                    V3GrandV3BasicTerrainProfile::canonical(),
                ),
            }),
        };
        let mut layout = super::super::layout::resolve_layout(V3_SCHEMATIC_GRID_RADIUS, &settings)
            .expect("reference layout resolves");
        super::super::schematic_crystal::claim_site(&plan, &mut layout, 22)
            .expect("Crystal site claim validates");
        let field =
            GrandHighlandField::build(&plan, &layout, V3GrandV3BasicTerrainProfile::canonical())
                .expect("reference highland field builds");
        let cells = plan
            .cells
            .iter()
            .map(|cell| (PatchId(u32::from(cell.id.get())), cell))
            .collect::<BTreeMap<_, _>>();

        for component in &field.peak_authority.components {
            let component_mask = component
                .patch_masks
                .values()
                .flat_map(|mask| mask.iter().copied())
                .collect::<BTreeSet<_>>();
            let feather_mask = component
                .feather_owners
                .keys()
                .copied()
                .collect::<BTreeSet<_>>();
            assert!(!feather_mask.is_empty());
            assert!(feather_mask.is_disjoint(&component_mask));
            assert!(connected(
                &component_mask
                    .union(&feather_mask)
                    .copied()
                    .collect::<BTreeSet<_>>()
            ));
            assert!(component.feather_owners.iter().all(|(coord, owner)| {
                layout
                    .patches
                    .get(owner)
                    .is_some_and(|patch| patch.mask.contains(coord))
                    && cells.get(owner).is_some_and(|cell| {
                        cell.facts.surface == SurfaceKind::Land
                            && cell.facts.landform == LandformKind::Mountain
                            && cell.facts.overlays.is_empty()
                    })
            }));
            let mut inner_edges = 0_usize;
            let mut outer_edges = 0_usize;
            for (first, second) in &component.feather_boundary_edges {
                assert_eq!(first.distance(*second), 1);
                if component_mask.contains(first) || component_mask.contains(second) {
                    inner_edges = inner_edges.saturating_add(1);
                } else {
                    outer_edges = outer_edges.saturating_add(1);
                }
                let level_at = |coord| {
                    field
                        .peak_levels
                        .get(&coord)
                        .copied()
                        .or_else(|| {
                            field
                                .peak_feather
                                .get(&coord)
                                .copied()
                                .map(|contribution| contribution.resolve(140))
                        })
                        .unwrap_or(140)
                };
                let first_level = level_at(*first);
                let second_level = level_at(*second);
                assert!(
                    field.massif.mask.contains(first)
                        || field.massif.mask.contains(second)
                        || first_level.abs_diff(second_level) <= 9,
                    "feather edge {first:?} {first_level} -> {second:?} {second_level}; first-component={}, second-component={}, first-feather={}, second-feather={}",
                    component_mask.contains(first),
                    component_mask.contains(second),
                    feather_mask.contains(first),
                    feather_mask.contains(second),
                );
            }
            assert!(inner_edges > 0 && outer_edges > 0);
        }
    }

    #[test]
    fn massif_crest_is_the_interior_world_high_point_away_from_crystal() {
        let field = reference_field();
        // Crest placement is intentionally measured against the semantic
        // Massif plus its one-cell connectivity corridor.  The visual mask
        // also contains two low Mountain feather rings; including those rings
        // in this comparison can pull the selected crest toward the feather
        // instead of keeping it centered in the authored Massif body.
        let crest_selection_mask = field
            .massif
            .semantic_owner_mask
            .union(&field.massif.connector_mask)
            .copied()
            .collect::<BTreeSet<_>>();
        let crest_selection_depths = boundary_depth(&crest_selection_mask);
        let crest_depth = crest_selection_depths
            .get(&field.massif.crest)
            .copied()
            .unwrap_or_default();
        // The complete connected visual body includes semantic cells adjacent
        // to Crystal which are deliberately ineligible for the summit.  Do not
        // compare the crest to those deeper-but-adjacent coordinates here; the
        // focused selection test below reconstructs the exact eligible owner
        // mask and proves the maximum within it.
        assert!(crest_depth >= 24);
        assert!(!field.crystal_mask.contains(&field.massif.crest));
        assert!(field
            .massif
            .crest
            .neighbors()
            .into_iter()
            .all(|neighbor| !field.crystal_mask.contains(&neighbor)));
        assert!((MASSIF_SUMMIT_MIN..=MASSIF_SUMMIT_MAX).contains(&field.massif.summit));
        assert!(field.massif.summit > PEAK_SUMMIT_MAX);
        assert!(field.massif.summit < MAX_V3_LEVEL);
    }

    #[test]
    fn massif_has_a_broad_irregular_summit_body_without_a_pencil_crest() {
        let field = reference_field();
        let baseline = V3GrandV3BasicTerrainProfile::canonical().high_core_level;
        assert_eq!(
            field.massif.resolve(field.massif.crest, baseline),
            field.massif.summit
        );
        let summit_cells = field
            .massif
            .mask
            .iter()
            .copied()
            .filter(|coord| field.massif.resolve(*coord, baseline) == field.massif.summit)
            .collect::<BTreeSet<_>>();
        assert!((1..=7).contains(&summit_cells.len()));
        assert!(summit_cells.contains(&field.massif.crest));
        assert_eq!(field.massif.summit_sources.len(), 7);
        assert_eq!(
            field.massif.summit_sources.get(&field.massif.crest),
            Some(&field.massif.summit)
        );
        assert!(field
            .massif
            .summit_sources
            .iter()
            .filter(|(coord, _)| **coord != field.massif.crest)
            .all(|(coord, level)| {
                (5..=14).contains(&field.massif.crest.distance(*coord))
                    && *level < field.massif.summit
                    && *level >= field.massif.summit.saturating_sub(30)
            }));
        assert_eq!(field.massif.shoulder_sources.len(), 6);
        assert!(
            field.massif.shoulder_sources.iter().all(|(coord, level)| {
                (MASSIF_SHOULDER_MIN_DISTANCE..=MASSIF_SHOULDER_MAX_DISTANCE)
                    .contains(&field.massif.crest.distance(*coord))
                    && *level <= field.massif.summit.saturating_sub(28)
                    && *level >= field.massif.summit.saturating_sub(87)
            }),
            "invalid Massif shoulder sources for summit {}: {:?}",
            field.massif.summit,
            field.massif.shoulder_sources
        );
        let shoulder_witnesses = field
            .massif
            .mask
            .iter()
            .filter(|coord| {
                let depth = field
                    .massif
                    .boundary_depth
                    .get(coord)
                    .copied()
                    .unwrap_or_default();
                let edge_cap = MASSIF_ABSOLUTE_BODY_BASE.saturating_add(
                    i32::try_from(depth)
                        .unwrap_or(i32::MAX)
                        .saturating_mul(MASSIF_SHOULDER_EDGE_RISE_PER_HEX),
                );
                let shoulder = field
                    .massif
                    .shoulder_support
                    .get(coord)
                    .copied()
                    .unwrap_or(field.massif.floor)
                    .min(edge_cap)
                    .max(baseline);
                let crown = field
                    .massif
                    .summit_support
                    .get(coord)
                    .copied()
                    .unwrap_or(field.massif.floor)
                    .max(baseline);
                shoulder > crown && field.massif.crest.distance(**coord) > 10
            })
            .count();
        assert!(
            shoulder_witnesses >= 24,
            "distant Massif shoulder sources affected only {shoulder_witnesses} mid-body columns"
        );
        assert_eq!(field.massif.summit_core.len(), 631);
        assert!(connected(&field.massif.summit_core));
        let resolved = field
            .massif
            .mask
            .iter()
            .map(|coord| field.massif.resolve(*coord, baseline))
            .collect::<BTreeSet<_>>();
        assert!(
            resolved.len() >= 20,
            "massif collapsed into broad level bands"
        );
        assert!(resolved.contains(&baseline));
        assert!(resolved.iter().any(|level| *level > PEAK_SUMMIT_MAX));
        let crown = field
            .massif
            .crest
            .within_radius(MASSIF_SUMMIT_BODY_RADIUS)
            .into_iter()
            .map(|coord| (coord, field.massif.resolve(coord, baseline)))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(crown.len(), 631);
        assert!(crown.values().copied().collect::<BTreeSet<_>>().len() >= 8);
        assert!(
            crown
                .values()
                .filter(|level| **level >= field.massif.summit.saturating_sub(8))
                .count()
                <= 25
        );
        assert!(crown.iter().all(|(coord, level)| {
            coord.neighbors().into_iter().all(|neighbor| {
                !field.massif.mask.contains(&neighbor)
                    || level.abs_diff(field.massif.resolve(neighbor, baseline)) <= 9
            })
        }));
        let broad_shoulders = crown
            .values()
            .filter(|level| **level >= field.massif.summit.saturating_sub(40))
            .count();
        assert!(broad_shoulders >= 50);
        for direction in 0..6 {
            let profile = (0..=MASSIF_SUMMIT_BODY_RADIUS)
                .map(|distance| {
                    field.massif.resolve(
                        step_in_direction(field.massif.crest, direction, distance),
                        baseline,
                    )
                })
                .collect::<Vec<_>>();
            assert!(profile
                .first()
                .zip(profile.last())
                .is_some_and(|(inner, outer)| outer < inner));
            assert!(profile.windows(5).all(|window| window
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                > 1));
        }
        let radial_profiles = (0..6)
            .map(|direction| {
                (0..=MASSIF_SUMMIT_BODY_RADIUS)
                    .map(|distance| {
                        field.massif.resolve(
                            step_in_direction(field.massif.crest, direction, distance),
                            baseline,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<BTreeSet<_>>();
        assert!(
            radial_profiles.len() >= 4,
            "offset summit lobes collapsed back into a rotationally uniform cone"
        );
        for direction in 0..6 {
            let full_ray = (0..=96_u32)
                .map(|distance| step_in_direction(field.massif.crest, direction, distance))
                .take_while(|coord| field.massif.mask.contains(coord))
                .map(|coord| field.massif.resolve(coord, baseline))
                .collect::<Vec<_>>();
            assert!(
                full_ray.len() > usize::try_from(MASSIF_SUMMIT_BODY_RADIUS).unwrap_or_default(),
                "Massif direction {direction} did not extend beyond the protected crown"
            );
            let elevated = full_ray
                .into_iter()
                .take_while(|level| *level > baseline.saturating_add(9))
                .collect::<Vec<_>>();
            assert!(
                elevated.windows(5).all(|window| window
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .len()
                    > 1),
                "Massif direction {direction} retained a high shelf outside the crown: {elevated:?}"
            );
        }
        let distributed_high_sources = field
            .massif
            .summit_sources
            .iter()
            .filter(|(coord, level)| {
                **coord != field.massif.crest
                    && field.massif.crest.distance(**coord) >= 8
                    && **level >= field.massif.summit.saturating_sub(30)
            })
            .count();
        assert!(
            distributed_high_sources >= 3,
            "massif has too few distributed high lobes: {distributed_high_sources}"
        );
    }

    #[test]
    fn massif_source_validator_rejects_a_single_cone_and_inert_shoulders() {
        let field = reference_field();
        let baseline = V3GrandV3BasicTerrainProfile::canonical().high_core_level;

        let mut cone = field.massif.clone();
        cone.summit_sources.retain(|coord, _| *coord == cone.crest);
        cone.summit_support =
            massif_summit_support(&cone.mask, &cone.summit_sources, cone.crest, cone.floor);
        let cone_error = validate_massif_irregular_source_contribution(&cone, baseline)
            .expect_err("one perfect central cone must not satisfy the Massif contract");
        assert!(cone_error.to_string().contains("offset summits"));

        let mut inert_shoulders = field.massif.clone();
        inert_shoulders.shoulder_support.clear();
        let shoulder_error =
            validate_massif_irregular_source_contribution(&inert_shoulders, baseline)
                .expect_err("nominal shoulder metadata without final influence must fail");
        assert!(shoulder_error.to_string().contains("shoulder sources"));
    }

    #[test]
    fn massif_shared_field_keeps_the_deep_reference_owner_seam_gradual() {
        let field = reference_field();
        let first = HexCoord::from_axial(-66, -77);
        let second = HexCoord::from_axial(-65, -78);
        let first_level = field.massif.resolve(first, 84);
        let second_level = field.massif.resolve(second, 87);
        assert!(first_level > 84 && second_level > 87);
        assert_ne!(first_level - 84, second_level - 87);
        assert!(
            first_level.abs_diff(second_level) <= 9,
            "adjacent semantic-owner levels {first_level} and {second_level} formed a seam"
        );
    }

    #[test]
    fn massif_crest_selection_enforces_the_same_crystal_separation_as_final_validation() {
        let massif_center = HexCoord::from_axial(50, 0);
        let mask = massif_center
            // The production field owns a sixteen-row Mountain feather around
            // its semantic Massif. Give this isolated selector fixture the
            // same physical shoulder budget so the low absolute datum can
            // place honest offset summits rather than testing an obsolete
            // radius-thirty cliff.
            .within_radius(46)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let crystal_mask = HexCoord::ORIGIN
            .within_radius(CRYSTAL_SITE_RADIUS)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let crest_owner_mask = massif_center
            .within_radius(5)
            .into_iter()
            .filter(|coord| mask.contains(coord))
            .collect::<BTreeSet<_>>();
        let field = build_massif_field(
            &mask,
            &mask,
            &BTreeSet::new(),
            &crest_owner_mask,
            &crystal_mask,
            V3GrandV3BasicTerrainProfile::canonical(),
            41,
        )
        .expect("a broad massif retains separated central crest candidates");
        let separation = crystal_mask
            .iter()
            .map(|crystal| field.crest.distance(*crystal))
            .min()
            .expect("Crystal fixture is nonempty");
        assert!(crest_owner_mask.contains(&field.crest));
        assert!(separation >= CELL_PITCH.unsigned_abs() / 2);
        let crest_selection_depths = boundary_depth(&field.mask);
        let maximum_eligible_depth = crest_owner_mask
            .iter()
            .filter(|coord| {
                crystal_mask
                    .iter()
                    .map(|crystal| coord.distance(*crystal))
                    .min()
                    .is_some_and(|distance| distance >= CELL_PITCH.unsigned_abs() / 2)
            })
            .filter_map(|coord| crest_selection_depths.get(coord).copied())
            .max();
        assert_eq!(
            crest_selection_depths.get(&field.crest).copied(),
            maximum_eligible_depth
        );
    }

    #[test]
    fn exact_crystal_disk_formula_matches_set_distance() {
        let center = HexCoord::from_axial(17, -9);
        let crystal_mask = center
            .within_radius(CRYSTAL_SITE_RADIUS)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let recovered =
            super::super::schematic::exact_hex_disk_center(&crystal_mask, CRYSTAL_SITE_RADIUS)
                .expect("translated exact Crystal disk retains one exact centre");
        assert_eq!(recovered, center);

        let mut samples = center.within_radius(4).into_iter().collect::<BTreeSet<_>>();
        for distance in 0_i32..=64 {
            for (q, r) in [
                (distance, 0),
                (0, distance),
                (-distance, distance),
                (-distance, 0),
                (0, -distance),
                (distance, -distance),
                (distance / 2, distance),
                (-distance, distance / 2),
            ] {
                samples.insert(HexCoord::from_axial(
                    center.x().saturating_add(q),
                    center.y().saturating_add(r),
                ));
            }
        }
        for coord in samples {
            let exact_set_distance = crystal_mask
                .iter()
                .map(|crystal| coord.distance(*crystal))
                .min();
            assert_eq!(
                exact_set_distance,
                Some(
                    coord
                        .distance(recovered)
                        .saturating_sub(CRYSTAL_SITE_RADIUS)
                ),
                "exact disk distance formula drifted at {coord:?}"
            );
        }
    }

    #[test]
    fn massif_field_rejects_a_malformed_crystal_disk() {
        let mut crystal_mask = HexCoord::ORIGIN
            .within_radius(CRYSTAL_SITE_RADIUS)
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert!(crystal_mask.remove(&HexCoord::ORIGIN));
        assert!(
            super::super::schematic::exact_hex_disk_center(&crystal_mask, CRYSTAL_SITE_RADIUS)
                .is_none()
        );

        let massif_center = HexCoord::from_axial(50, 0);
        let mask = massif_center
            .within_radius(30)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let crest_owner_mask = massif_center
            .within_radius(5)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let error = build_massif_field(
            &mask,
            &mask,
            &BTreeSet::new(),
            &crest_owner_mask,
            &crystal_mask,
            V3GrandV3BasicTerrainProfile::canonical(),
            41,
        )
        .expect_err("malformed Crystal authority must fail before massif shaping");
        let V3GenerationError::RecipeContract(detail) = error else {
            panic!("malformed Crystal disk returned the wrong error: {error:?}");
        };
        assert!(detail.contains("exact radius-32 Crystal site"));
    }

    #[test]
    fn seed_175_connects_the_visual_massif_without_changing_biome_ownership() {
        let template = hex_schematic::grand_v3_reference_template().expect("template parses");
        let plan = hex_schematic::generate(&template, 175)
            .expect("seed 175 schematic generates")
            .plan;
        let settings = ProceduralV3Settings {
            layout: V3LayoutSettings::Schematic(V3SchematicLayoutSettings {
                template: V3SchematicTemplate::GrandV3,
                template_revision: crate::settings::V3_GRAND_V3_TEMPLATE_REVISION,
                cell_pitch: 22,
                terrain_profile: V3SchematicTerrainProfile::GrandV3BasicV1(
                    V3GrandV3BasicTerrainProfile::canonical(),
                ),
            }),
        };
        let mut layout = super::super::layout::resolve_layout(V3_SCHEMATIC_GRID_RADIUS, &settings)
            .expect("seed 175 layout resolves");
        super::super::schematic_crystal::claim_site(&plan, &mut layout, 22)
            .expect("seed 175 Crystal site claim validates");
        let claimed_layout = layout.clone();
        let massif_patches = plan
            .cells
            .iter()
            .filter(|cell| cell.facts.landform == LandformKind::Massif)
            .map(|cell| PatchId(u32::from(cell.id.get())))
            .collect::<BTreeSet<_>>();
        let massif_owner_mask = union_masks(&layout, massif_patches.iter().copied())
            .expect("seed 175 has Massif ownership");
        assert!(
            fine_components(&massif_owner_mask).len() > 1,
            "seed 175 remains the defining Crystal-split Massif fixture"
        );

        let field =
            GrandHighlandField::build(&plan, &layout, V3GrandV3BasicTerrainProfile::canonical())
                .expect("seed 175 highland visual field builds");

        assert_eq!(
            layout, claimed_layout,
            "visual highland construction must not mutate biome ownership"
        );
        assert_eq!(
            field.massif_visual_authority.semantic_owner_mask,
            massif_owner_mask
        );
        assert_eq!(field.massif_visual_authority.visual_mask, field.massif.mask);
        assert!(connected(&field.massif.mask));
        assert!(massif_owner_mask.is_subset(&field.massif.mask));
        let connector = field
            .massif_visual_authority
            .connector_owners
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        assert!(
            !connector.is_empty(),
            "the split fixture must exercise a visual-only connector"
        );
        let feather = field
            .massif_visual_authority
            .feather_owners
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        assert!(!feather.is_empty());
        assert!(connector.is_disjoint(&feather));
        assert_eq!(
            field.massif.mask,
            massif_owner_mask
                .union(&connector)
                .copied()
                .chain(feather.iter().copied())
                .collect::<BTreeSet<_>>()
        );
        for coord in &connector {
            assert!(!field.crystal_mask.contains(coord));
            let owner = layout
                .patches
                .iter()
                .find_map(|(owner, patch)| patch.mask.contains(coord).then_some(*owner))
                .expect("connector coordinate retains one layout owner");
            let cell = plan
                .cells
                .iter()
                .find(|cell| u32::from(cell.id.get()) == owner.0)
                .expect("connector owner has a schematic cell");
            assert_eq!(cell.facts.surface, SurfaceKind::Land);
            assert_eq!(cell.facts.landform, LandformKind::Mountain);
            assert!(cell.facts.overlays.is_empty());
            assert_eq!(
                field.massif_visual_authority.connector_owners.get(coord),
                Some(&owner)
            );
        }
        let probe_coord = connector
            .first()
            .copied()
            .expect("seed 175 connector has a deterministic first coordinate");
        let probe_owner = layout
            .patches
            .iter()
            .find_map(|(owner, patch)| patch.mask.contains(&probe_coord).then_some(*owner))
            .expect("connector probe retains one layout owner");
        let probe_cell = plan
            .cells
            .iter()
            .find(|cell| u32::from(cell.id.get()) == probe_owner.0)
            .expect("connector probe owner has a schematic cell");
        let mut application_probe = field.clone();
        // Isolate the visual-Massif projection: this defining connector also
        // happens to sit beneath Crystal's independent higher mantle.
        application_probe.crystal_mantle.remove(&probe_coord);
        let baseline = 80;
        let expected = application_probe.massif.resolve(probe_coord, baseline);
        assert!(
            application_probe.massif.boundary_depth[&probe_coord]
                >= MASSIF_CONNECTOR_MINIMUM_TAPER_DEPTH,
            "the exact Crystal hole must not collapse the connector back onto the scalar-field edge"
        );
        assert!(expected > baseline);
        assert_eq!(
            application_probe.resolve_surface_level(probe_cell, probe_coord, baseline),
            expected,
            "semantic Mountain connector must receive the same widened scalar bridge as the Massif field"
        );
        let nearby_taper = feather
            .iter()
            .copied()
            .filter(|coord| {
                connector
                    .iter()
                    .map(|connector| connector.distance(*coord))
                    .min()
                    .is_some_and(|distance| distance <= MASSIF_CONNECTOR_MINIMUM_TAPER_DEPTH)
            })
            .filter(|coord| {
                application_probe
                    .massif
                    .boundary_depth
                    .get(coord)
                    .is_some_and(|depth| *depth > 0)
            })
            .collect::<BTreeSet<_>>();
        assert!(
            nearby_taper.len()
                >= usize::try_from(MASSIF_CONNECTOR_MINIMUM_TAPER_DEPTH).unwrap_or(usize::MAX),
            "the connector must widen into multiple nonsemantic Mountain columns"
        );
        assert!(nearby_taper
            .iter()
            .all(|coord| { application_probe.massif.resolve(*coord, baseline) > baseline }));
        let connector_boundary_edges = application_probe
            .massif
            .mask
            .iter()
            .copied()
            .flat_map(|coord| {
                coord
                    .neighbors()
                    .into_iter()
                    .filter(move |neighbor| coord < *neighbor)
                    .map(move |neighbor| (coord, neighbor))
            })
            .filter(|(first, second)| {
                application_probe.massif.mask.contains(second)
                    && application_probe
                        .massif
                        .connector_distance
                        .contains_key(first)
                        != application_probe
                            .massif
                            .connector_distance
                            .contains_key(second)
            })
            .collect::<Vec<_>>();
        assert!(
            connector_boundary_edges.len() >= 24,
            "seed 175 must exercise the complete lateral connector-support boundary"
        );
        let connector_boundary_cliffs = connector_boundary_edges
            .iter()
            .filter_map(|(first, second)| {
                let first_level = application_probe.massif.resolve(*first, baseline);
                let second_level = application_probe.massif.resolve(*second, baseline);
                (first_level.abs_diff(second_level) > 9).then_some((
                    *first,
                    first_level,
                    application_probe.massif.boundary_depth.get(first).copied(),
                    application_probe
                        .massif
                        .connector_distance
                        .get(first)
                        .copied(),
                    application_probe.massif.semantic_owner_mask.contains(first),
                    *second,
                    second_level,
                    application_probe.massif.boundary_depth.get(second).copied(),
                    application_probe
                        .massif
                        .connector_distance
                        .get(second)
                        .copied(),
                    application_probe
                        .massif
                        .semantic_owner_mask
                        .contains(second),
                ))
            })
            .collect::<Vec<_>>();
        assert!(
            connector_boundary_cliffs.is_empty(),
            "Massif connector support ends in lateral cliffs: {connector_boundary_cliffs:?}"
        );
        validate_massif_connector_scalar_bridge(&application_probe.massif)
            .expect("the widened seed-175 connector retains a descending scalar taper");

        let mut collapsed = application_probe.massif.clone();
        collapsed.boundary_depth.insert(probe_coord, 0);
        let error = validate_massif_connector_scalar_bridge(&collapsed)
            .expect_err("reintroducing the old depth-zero connector trench must fail");
        assert!(error.to_string().contains("narrow scalar seam"));
        let crest_owner = layout
            .patches
            .iter()
            .find_map(|(owner, patch)| patch.mask.contains(&field.massif.crest).then_some(*owner))
            .expect("Massif crest retains one layout owner");
        let crest_cell = plan
            .cells
            .iter()
            .find(|cell| u32::from(cell.id.get()) == crest_owner.0)
            .expect("Massif crest owner has a schematic cell");
        assert_eq!(crest_cell.facts.landform, LandformKind::Massif);
        assert!(massif_patches.contains(&crest_owner));
    }

    #[test]
    fn crystal_enclosure_raises_six_broad_neighboring_biome_sectors_without_a_ring_wall() {
        let field = reference_field();
        let authority = &field.crystal_mantle_authority;
        let attainable = authority.attainable_enclosure_band();
        assert!(field
            .crystal_mantle
            .keys()
            .all(|coord| !field.crystal_mask.contains(coord)));
        assert_eq!(authority.sector_pins.len(), 6);
        assert!(authority
            .uplift_core
            .is_subset(&authority.support_footprint));
        assert!(authority
            .enclosure_band
            .is_subset(&authority.support_footprint));
        assert!(authority.route_exclusion.is_subset(&authority.uplift_core));
        assert!(authority.route_exclusion.iter().all(|coord| {
            field
                .crystal_mantle_edge_depth
                .get(coord)
                .is_some_and(|depth| *depth >= 2)
        }));
        assert!(authority.route_exclusion.len() < authority.uplift_core.len());
        assert!(authority
            .opening_clearance
            .is_disjoint(&authority.uplift_core));
        assert!(authority
            .opening_clearance
            .is_disjoint(&authority.support_footprint));
        assert!(authority
            .opening_clearance
            .is_disjoint(&authority.enclosure_band));
        assert_eq!(authority.composite_crystal_top, 174);
        assert!(!authority.forced_low_frozen_halo.is_empty());
        assert!(authority
            .forced_low_exit_blend
            .iter()
            .all(|(coord, ceiling)| field.crystal_exit_ceiling.get(coord) == Some(ceiling)));
        assert!(authority
            .forced_low_frozen_halo
            .iter()
            .chain(&authority.forced_low_exit_blend)
            .all(|(coord, ceiling)| {
                authority.enclosure_band.contains(coord)
                    && *ceiling <= authority.composite_crystal_top
            }));
        assert!(authority.uplift_core.is_subset(&attainable));
        assert!(authority.uplift_core.len() * 5 >= attainable.len() * 3);
        authority
            .validate_attainable_coverage("raw mantle target", |coord| {
                field.crystal_mantle.get(&coord).copied()
            })
            .expect("the reference mantle is broad across every attainable sector");
        let transit_minimums = authority
            .transit_minimums("raw mantle transit snapshot", |coord| {
                field.crystal_mantle.get(&coord).copied()
            })
            .expect("the admitted raw mantle supplies live shared-transit floors");
        assert_eq!(
            transit_minimums.keys().copied().collect::<BTreeSet<_>>(),
            authority.uplift_core
        );
        assert!(transit_minimums
            .values()
            .all(|minimum| *minimum == authority.composite_crystal_top.saturating_add(1)));
        let transit_transition_edges = authority.transit_transition_edges(&field.crystal_mask);
        assert!(!transit_transition_edges.is_empty());
        assert!(transit_transition_edges.iter().all(|(inside, outside)| {
            authority.support_footprint.contains(inside)
                && !authority.support_footprint.contains(outside)
                && !field.crystal_mask.contains(outside)
                && authority.crystal_center.distance(*inside) == CRYSTAL_ENCLOSURE_OUTER_RADIUS
                && authority.crystal_center.distance(*outside) > CRYSTAL_ENCLOSURE_OUTER_RADIUS
        }));
        let (transition_inside, transition_outside) = transit_transition_edges
            .iter()
            .copied()
            .next()
            .expect("the reference mantle has a true exterior edge");
        let scenic_touch = BTreeSet::from([transition_inside]);
        authority
            .validate_transit_transition(
                "scenic counterexample",
                &field.crystal_mask,
                &scenic_touch,
                9,
                |coord| {
                    (coord == transition_inside)
                        .then_some(143)
                        .or_else(|| (coord == transition_outside).then_some(120))
                },
            )
            .expect("a one-sided scenic touch is not an authored transit crossing");
        let transit_crossing = BTreeSet::from([transition_inside, transition_outside]);
        let error = authority
            .validate_transit_transition(
                "counterexample",
                &field.crystal_mask,
                &transit_crossing,
                9,
                |coord| {
                    (coord == transition_inside)
                        .then_some(143)
                        .or_else(|| (coord == transition_outside).then_some(120))
                },
            )
            .expect_err("an ungraded shared-transit edge must fail closed");
        assert!(error.contains("143"));
        assert!(error.contains("120"));
        authority
            .validate_transit_transition(
                "graded counterexample",
                &field.crystal_mask,
                &transit_crossing,
                9,
                |coord| {
                    (coord == transition_inside)
                        .then_some(143)
                        .or_else(|| (coord == transition_outside).then_some(134))
                },
            )
            .expect("a nine-level shared-transit shoulder is admitted");
        let protected_lobes = fine_components(&authority.route_exclusion);
        assert_eq!(protected_lobes.len(), 6);
        assert!(protected_lobes.iter().all(|lobe| {
            authority
                .sector_pins
                .values()
                .filter(|(pin, _)| lobe.contains(pin))
                .count()
                == 1
        }));
        for sector in 0..6 {
            let sector_band = attainable
                .iter()
                .filter(|coord| enclosure_sector(field.crystal_center, **coord) == sector)
                .count();
            let sector_core = authority
                .uplift_core
                .iter()
                .copied()
                .filter(|coord| enclosure_sector(field.crystal_center, *coord) == sector)
                .collect::<BTreeSet<_>>();
            assert!(
                sector_core.len() >= sector_band.saturating_add(2) / 3,
                "sector {sector} has only {} high columns out of {sector_band}",
                sector_core.len()
            );
            assert!(
                sector_core
                    .iter()
                    .map(|coord| field.crystal_center.distance(*coord))
                    .collect::<BTreeSet<_>>()
                    .len()
                    >= crystal_enclosure_required_radial_depth(
                        attainable
                            .iter()
                            .filter(|coord| {
                                enclosure_sector(field.crystal_center, **coord) == sector
                            })
                            .map(|coord| field.crystal_center.distance(*coord))
                            .collect::<BTreeSet<_>>()
                            .len(),
                    )
            );
        }
        let represented_radii = authority
            .enclosure_band
            .iter()
            .map(|coord| field.crystal_center.distance(*coord))
            .collect::<BTreeSet<_>>();
        assert!(represented_radii.iter().all(|radius| {
            authority
                .uplift_core
                .iter()
                .filter(|coord| field.crystal_center.distance(**coord) == *radius)
                .count()
                < usize::try_from(6_u32.saturating_mul(*radius)).unwrap_or(usize::MAX)
        }));
    }

    #[test]
    fn crystal_transit_minimums_snapshot_the_live_admitted_high_set() {
        let field = reference_field();
        let authority = &field.crystal_mantle_authority;
        let extra = authority
            .attainable_enclosure_band()
            .difference(&authority.uplift_core)
            .next()
            .copied()
            .expect("the reference mantle retains attainable low feather terrain");
        let minimums = authority
            .transit_minimums("live transit snapshot", |coord| {
                (coord == extra)
                    .then_some(authority.composite_crystal_top.saturating_add(1))
                    .or_else(|| field.crystal_mantle.get(&coord).copied())
            })
            .expect("adding one live high witness preserves Crystal admission");
        assert!(minimums.contains_key(&extra));
        assert_eq!(
            minimums.len(),
            authority.uplift_core.len().saturating_add(1)
        );

        let error = authority
            .transit_minimums("collapsed transit snapshot", |_| {
                Some(authority.composite_crystal_top)
            })
            .expect_err("an inadmissible live enclosure cannot issue transit floors");
        assert!(error.contains("retained only 0 high columns"));
    }

    #[test]
    fn crystal_enclosure_radial_admission_scales_to_the_available_sector_depth() {
        assert_eq!(crystal_enclosure_required_radial_depth(25), 8);
        assert_eq!(crystal_enclosure_required_radial_depth(14), 4);

        let field = reference_field();
        let authority = &field.crystal_mantle_authority;
        let attainable = authority.attainable_enclosure_band();
        let full_sector = 0;
        let sector_coords = attainable
            .iter()
            .copied()
            .filter(|coord| enclosure_sector(authority.crystal_center, *coord) == full_sector)
            .collect::<BTreeSet<_>>();
        let available_radii = sector_coords
            .iter()
            .map(|coord| authority.crystal_center.distance(*coord))
            .collect::<BTreeSet<_>>();
        assert_eq!(available_radii.len(), 25);
        let retained_radii = (60..=66).collect::<BTreeSet<_>>();
        let retained_columns = sector_coords
            .iter()
            .filter(|coord| retained_radii.contains(&authority.crystal_center.distance(**coord)))
            .count();
        assert!(retained_columns >= sector_coords.len().saturating_add(2) / 3);
        let error = authority
            .validate_attainable_coverage("radial mutation", |coord| {
                let high = enclosure_sector(authority.crystal_center, coord) != full_sector
                    || retained_radii.contains(&authority.crystal_center.distance(coord));
                Some(if high {
                    authority.composite_crystal_top.saturating_add(1)
                } else {
                    authority.composite_crystal_top
                })
            })
            .expect_err("seven of twenty-five radial rows must fail despite enough high columns");
        assert!(error.contains("sector 0"));
        assert!(error.contains("radial-depth=7/25 required=8"));
    }

    #[test]
    fn massif_crystal_overlap_does_not_preserve_an_internal_ring_seam() {
        let field = reference_field();
        let mantle = &field.crystal_mantle_authority;
        let exclusion = field
            .massif_visual_authority
            .route_taper_exclusion(&mantle.support_footprint, &mantle.opening_clearance);
        let preservation = field
            .massif_visual_authority
            .route_taper_avoidance(&mantle.support_footprint, &mantle.opening_clearance);
        let deep_crystal_overlap_inside_massif = mantle
            .support_footprint
            .iter()
            .copied()
            .filter(|coord| field.massif_visual_authority.visual_mask.contains(coord))
            .filter(|coord| {
                coord.within_radius(2).into_iter().all(|neighbor| {
                    field
                        .massif_visual_authority
                        .visual_mask
                        .contains(&neighbor)
                })
            })
            .filter(|coord| {
                coord
                    .neighbors()
                    .into_iter()
                    .all(|neighbor| mantle.support_footprint.contains(&neighbor))
            })
            .filter(|coord| !mantle.opening_clearance.contains(coord))
            .collect::<BTreeSet<_>>();
        assert!(
            deep_crystal_overlap_inside_massif.len() >= 12,
            "reference fixture must exercise a substantial continuous Crystal/Massif overlap"
        );
        assert!(deep_crystal_overlap_inside_massif.is_disjoint(&exclusion));
        assert!(exclusion.is_disjoint(&mantle.opening_clearance));
        let complete_massif_boundary = field
            .massif_visual_authority
            .visual_mask
            .iter()
            .copied()
            .filter(|coord| {
                coord.neighbors().into_iter().any(|neighbor| {
                    !field
                        .massif_visual_authority
                        .visual_mask
                        .contains(&neighbor)
                })
            })
            .filter(|coord| !mantle.opening_clearance.contains(coord))
            .collect::<BTreeSet<_>>();
        assert!(complete_massif_boundary.is_subset(&preservation));
        assert!(deep_crystal_overlap_inside_massif.is_disjoint(&preservation));
        assert!(preservation.is_disjoint(&mantle.opening_clearance));
    }

    #[test]
    fn public_hero_enclosure_preserves_the_locked_tunnel_and_upper_exit_openings() {
        const HERO_SEED: u64 = 1_592_598_566;
        let template = hex_schematic::grand_v3_reference_template().expect("template parses");
        let generated =
            hex_schematic::generate(&template, HERO_SEED).expect("hero schematic generates");
        let field = field_for_plan(&generated.plan);
        let authority = &field.crystal_mantle_authority;
        let crystal_settings = V3CrystalAscentSettings {
            base_level: V3GrandV3BasicTerrainProfile::canonical().crystal_base_level,
            rise_levels: V3GrandV3BasicTerrainProfile::canonical().crystal_rise_levels,
        };
        let authored_top =
            super::super::crystal_ascent::macro_highest_authored_surface_level(&crystal_settings);
        assert!(authority.sector_pins.values().all(|(coord, level)| {
            authority.uplift_core.contains(coord)
                && *level > authored_top
                && *level >= CRYSTAL_ENCLOSURE_HIGH_MIN
        }));

        let tunnel_edge = generated
            .plan
            .networks
            .iter()
            .find(|network| network.kind == NetworkKind::Tunnel)
            .and_then(|network| {
                network
                    .edges
                    .iter()
                    .find(|edge| edge.id.as_str() == "edge/tunnel-complete")
            })
            .expect("hero schematic retains the locked tunnel edge");
        let fine_tunnel = tunnel_edge
            .path
            .windows(2)
            .flat_map(|pair| schematic_to_world(pair[0]).line_between(schematic_to_world(pair[1])))
            .collect::<BTreeSet<_>>();
        assert!(fine_tunnel
            .iter()
            .any(|coord| authority.opening_clearance.contains(coord)));
        assert!(fine_tunnel
            .iter()
            .all(|coord| !authority.uplift_core.contains(coord)));
        assert!(authority
            .opening_clearance
            .is_superset(&field.crystal_mantle_exit_clearance));
        assert!(authority.uplift_core.len() > 1_000);
    }

    #[test]
    fn crystal_enclosure_skins_the_inner_shell_and_feathers_at_real_outer_boundaries() {
        let field = reference_field();
        let authority = &field.crystal_mantle_authority;
        let boundary = authority
            .support_footprint
            .iter()
            .copied()
            .filter(|coord| {
                coord.neighbors().into_iter().any(|neighbor| {
                    !authority.support_footprint.contains(&neighbor)
                        && !field.crystal_mask.contains(&neighbor)
                })
            })
            .collect::<BTreeSet<_>>();
        assert!(!boundary.is_empty());
        assert!(boundary.iter().all(|coord| {
            field.crystal_mantle_edge_depth.get(coord) == Some(&0)
                && edge_blended_uplift(113, field.crystal_mantle[coord], 0) == 113
        }));
        assert_eq!(edge_blended_uplift(113, 208, 1), 120);
        assert_eq!(edge_blended_uplift(180, 160, 8), 180);
        let authored_top = super::super::crystal_ascent::macro_highest_authored_surface_level(
            &V3CrystalAscentSettings {
                base_level: V3GrandV3BasicTerrainProfile::canonical().crystal_base_level,
                rise_levels: V3GrandV3BasicTerrainProfile::canonical().crystal_rise_levels,
            },
        );
        let shell_apron = authority.shell_concealment_apron();
        assert!(authority
            .shell_concealment_floors
            .values()
            .all(|floor| *floor <= authored_top));
        assert!(authority
            .shell_concealment_floors
            .values()
            .any(|floor| *floor == authored_top));
        assert!(authority
            .shell_concealment_floors
            .keys()
            .eq(authority.shell_concealment_ceilings.keys()));
        assert!(authority
            .shell_concealment_floors
            .iter()
            .all(|(coord, floor)| { *floor <= authority.shell_concealment_ceilings[coord] }));
        assert!(!shell_apron.is_empty());
        assert!(shell_apron
            .iter()
            .all(|coord| field.crystal_center.distance(*coord) == CRYSTAL_SITE_RADIUS + 1));
        assert!(shell_apron.iter().all(|coord| coord
            .neighbors()
            .into_iter()
            .any(|neighbor| { authority.natural_shell_skin.contains(&neighbor) })));
        assert!(shell_apron.is_disjoint(&authority.opening_clearance));
        assert!(shell_apron.contains(&HexCoord::from_axial(40, -165)));
        let route_reservation = authority.shell_concealment_route_reservation();
        assert!(shell_apron.is_subset(&route_reservation));
        assert!(route_reservation.is_disjoint(&authority.opening_clearance));
        let frozen_transition = crystal_frozen_shell_transition_grades(
            &authority.shell_concealment_floors,
            &field.frozen_plateau.levels,
        );
        assert!(!frozen_transition.is_empty());
        assert!(frozen_transition.len() * 10 <= field.frozen_plateau.levels.len() * 3);
        assert!(frozen_transition.keys().all(|coord| {
            shell_apron
                .iter()
                .map(|apron| apron.distance(*coord))
                .min()
                .is_some_and(|distance| distance <= CRYSTAL_SHELL_FROZEN_TRANSITION_DEPTH)
        }));
        assert!(shell_apron.iter().all(|coord| {
            let floor = authority.shell_concealment_floors[coord];
            field.frozen_plateau.levels.get(coord).is_none_or(|level| {
                *level >= floor
                    || frozen_transition
                        .get(coord)
                        .is_some_and(|transition| *transition >= floor)
            })
        }));
        let suppressed_exit_conflicts = shell_apron
            .iter()
            .filter(|coord| {
                let floor = authority.shell_concealment_floors[coord];
                field
                    .crystal_exit_ceiling
                    .get(coord)
                    .is_some_and(|ceiling| *ceiling < floor)
            })
            .copied()
            .collect::<BTreeSet<_>>();
        assert!(!suppressed_exit_conflicts.is_empty());
        assert!(suppressed_exit_conflicts.is_subset(&route_reservation));
        let inner_skin = authority
            .support_footprint
            .iter()
            .filter(|coord| {
                (CRYSTAL_SITE_RADIUS.saturating_add(1)..=CRYSTAL_SITE_RADIUS.saturating_add(3))
                    .contains(&field.crystal_center.distance(**coord))
            })
            .collect::<Vec<_>>();
        assert!(!inner_skin.is_empty());
        assert!(inner_skin.iter().all(|coord| {
            field
                .crystal_mantle
                .get(coord)
                .is_some_and(|level| *level >= authored_top.saturating_add(2))
        }));
        let fully_skinned = inner_skin
            .iter()
            .filter(|coord| field.crystal_mantle_edge_depth.get(coord) != Some(&0))
            .count();
        assert!(fully_skinned * 5 >= inner_skin.len() * 4);
    }

    #[test]
    fn frozen_woods_is_a_level_152_plateau_with_a_six_row_mountain_blend() {
        let field = reference_field();
        assert!(!field.frozen_plateau.levels.is_empty());
        let in_band = field
            .frozen_plateau
            .levels
            .values()
            .filter(|level| (FROZEN_PLATEAU_MIN..=FROZEN_PLATEAU_MAX).contains(level))
            .count();
        assert!(in_band * 10 >= field.frozen_plateau.levels.len() * 7);
        assert!(field
            .frozen_plateau
            .halo_distance
            .values()
            .all(|distance| (1..=FROZEN_PLATEAU_HALO_DEPTH).contains(distance)));
    }

    #[test]
    fn frozen_plateau_halo_cannot_cross_an_excluded_barrier() {
        let core = BTreeSet::from([HexCoord::ORIGIN]);
        let adjacent = HexCoord::from_axial(0, 1);
        let remote = HexCoord::from_axial(2, 0);
        let eligible = BTreeSet::from([adjacent, remote]);

        let halo = frozen_halo_distances(&core, &eligible);

        assert_eq!(halo.get(&adjacent), Some(&1));
        assert!(!halo.contains_key(&remote));
    }

    #[test]
    fn crystal_mantle_clears_only_the_upper_exit_in_all_six_landmark_rotations() {
        let mask = HexCoord::ORIGIN
            .within_radius(CRYSTAL_SITE_RADIUS)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let footprint = HexCoord::ORIGIN
            .within_radius(84)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let profile = V3GrandV3BasicTerrainProfile::canonical();
        let mut rotated_clearances = BTreeSet::new();
        for rotation in 0..6 {
            let clearance = crystal_mantle_exit_clearance(&mask, rotation, profile, &footprint)
                .expect("rotated upper-exit clearance resolves");
            let lower = super::super::crystal_ascent::macro_lower_terminal_coords(
                &mask,
                rotation,
                profile.crystal_base_level,
            )
            .expect("rotated lower terminal resolves");
            let upper = super::super::crystal_ascent::macro_upper_terminal_outward_rows(
                &mask,
                rotation,
                profile
                    .crystal_base_level
                    .saturating_add(profile.crystal_rise_levels),
                CRYSTAL_MANTLE_EXIT_CLEARANCE_DEPTH,
            )
            .expect("rotated upper-exit rows resolve")
            .into_iter()
            .flatten()
            .collect::<BTreeSet<_>>();
            let expected = upper
                .iter()
                .copied()
                .flat_map(|coord| coord.within_radius(CRYSTAL_MANTLE_EXIT_CLEARANCE_BUFFER))
                .filter(|coord| footprint.contains(coord))
                .collect::<BTreeSet<_>>();
            assert_eq!(clearance, expected);
            assert!(lower.is_disjoint(&clearance));
            rotated_clearances.insert(clearance);
        }
        assert_eq!(rotated_clearances.len(), 6);
    }

    #[test]
    fn highland_field_is_stable_for_one_plan_and_seed() {
        assert_eq!(reference_field(), reference_field());
    }
}
