//! Whole-world Macro spanning features.
//!
//! A spanning feature is deliberately planned before ordinary patch decoration,
//! then applied once after all patch fragments have been merged.  Planning owns
//! exact horizontal reservations; application owns the one atomic semantic-volume
//! rewrite.  Keeping those phases separate prevents patch-local liquid, feature,
//! and route planners from each carving a slightly different version of the same
//! tunnel.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::fmt;

use hex_core::{
    HexCoord, IlluminationLevel, InteriorRegionId, Level, SpecialMovementRegion, TilePos,
};

use super::layout::{
    HexSide, LayoutKind, PatchId, ResolvedLayoutPlan, ResolvedMacroBoundaryTerminal,
    ResolvedMacroContracts, ResolvedMacroSpanningFeature, ResolvedMacroTunnel,
};
use super::routing::vertex_disjoint_paths;
use super::volume::{
    LevelInterval, SolidMass, SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeColumn,
    VolumeElement,
};
use super::world::{
    CaveCrystalKind, CaveCrystalPresentation, CaveCrystalSiteKind, GeneratedWorldPlan, LightId,
    PlannedGameplayLight, PlannedInterior, PlannedLightPresentation, ProtectedFeatureRoute,
};

const REQUIRED_TUNNEL_WIDTH: u32 = 4;
const REQUIRED_FLOOR_LEVEL: Level = 6;
const REQUIRED_CLEARANCE: u32 = 6;
const REQUIRED_ROOF_THICKNESS: u32 = 3;
const EXTERIOR_MOUTH_WIDTH: usize = 8;
const EXTERIOR_MOUTH_CLEARANCE: u32 = 12;
const EXTERIOR_MOUTH_ROUTE_ROWS: usize = 12;
const GOTHIC_ROW_COUNT: usize = 12;
// The paired Dim-18 gameplay sources may be much farther apart, but the matching
// presentation light has the established 4.5-world-unit cave-crystal range. Four
// centerline steps produces frequent physical pools without changing either
// authoritative illumination radius or the shared 4,500 lm / 4.5 rig. Exact route
// coverage remains the paired gameplay-light contract, not a renderer claim.
const LIGHT_SPACING_STEPS: usize = 4;
const MAX_RIBBON_RADIUS: u32 = 8;
const DIM_LIGHT_RADIUS: u32 = 18;
const BRIGHT_LIGHT_RADIUS: u32 = 4;
const MACRO_LOCAL_ID_BITS: u32 = 26;
const WORLD_NAMESPACE_PREFIX: u32 = 63;
const WORLD_NAMESPACE_BASE: u32 = WORLD_NAMESPACE_PREFIX << MACRO_LOCAL_ID_BITS;
const WORLD_LOCAL_ID_LIMIT: u32 = 1 << MACRO_LOCAL_ID_BITS;
const INTERIOR_LOCAL_BASE: u32 = 1;
const SEAM_CLOSURE_LOCAL_BASE: u32 = 512;
const LIGHT_LOCAL_BASE: u32 = 1_024;
const LIGHTS_PER_TUNNEL: u32 = 4_096;

const FOOT_APRON_ANCHOR: &str = "crystal_mountain.foot_apron";
const TUNNEL_MOUTH_ANCHOR: &str = "crystal_mountain.tunnel_mouth";
const TUNNEL_MIDPOINT_ANCHOR: &str = "crystal_mountain.midpoint";
const GOTHIC_TRANSITION_ANCHOR: &str = "crystal_mountain.gothic_transition";
const ASCENT_THRESHOLD_ANCHOR: &str = "crystal_mountain.ascent_threshold";

/// Raw authored facts extracted from the destination fragment before namespacing.
///
/// The anchor alone cannot identify which four authored surfaces form an aperture.
/// Requiring the exact terminal avoids guessing adjacent tiles and makes a rotated
/// landmark use the same contract as its unrotated source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawSpanningDestination {
    pub(crate) anchor: TilePos,
    pub(crate) terminal: BTreeSet<TilePos>,
    pub(crate) interior: Option<InteriorRegionId>,
    pub(crate) summit_threshold: BTreeSet<TilePos>,
}

/// Destination facts keyed by `(patch, patch-local anchor name)`.
pub(crate) type RawSpanningDestinations = BTreeMap<(PatchId, String), RawSpanningDestination>;

/// One nonblocking crystal alcove selected while the horizontal route is reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlannedTunnelLightSite {
    pub(crate) patch: PatchId,
    pub(crate) position: TilePos,
    pub(crate) kind: CaveCrystalKind,
    pub(crate) rotation: u8,
}

/// Complete exact data for one planned world-owned tunnel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlannedMacroTunnel {
    pub(crate) name: String,
    pub(crate) canonical_route: bool,
    pub(crate) instance_route: Vec<PatchId>,
    pub(crate) floor_level: Level,
    pub(crate) clearance: u32,
    pub(crate) roof_thickness: u32,
    /// Four stable lane identities, each ordered from the world boundary to the
    /// exact destination terminal. Seam crossings appear as adjacent coordinates.
    pub(crate) lanes: Vec<Vec<HexCoord>>,
    /// Exact declared crossings in instance order.
    pub(crate) seam_lanes: Vec<BTreeSet<(HexCoord, HexCoord)>>,
    /// One ordered representative path used by anchors, crystal spacing, and
    /// protected-route presentation. It never repeats a coordinate.
    pub(crate) centerline: Vec<HexCoord>,
    /// The exact four-lane route, excluding presentation alcoves.
    pub(crate) ribbon: BTreeSet<HexCoord>,
    /// The complete widened entrance treatment, including the open apron.
    pub(crate) mouth: BTreeSet<HexCoord>,
    /// Eight exterior boundary-row cells under the exact twelve-level-high mouth roof.
    pub(crate) exterior_apron: BTreeSet<HexCoord>,
    /// The first exact four-wide roofed row and the only lower entrance set.
    pub(crate) foot_threshold: BTreeSet<TilePos>,
    /// The final twelve route rows receiving worked Gothic masonry.
    pub(crate) gothic: BTreeSet<HexCoord>,
    /// Exact authored upper terminal and the only upper entrance set.
    pub(crate) summit_threshold: BTreeSet<TilePos>,
    pub(crate) destination_anchor: TilePos,
    pub(crate) destination_terminal: BTreeSet<TilePos>,
    pub(crate) destination_interior: InteriorRegionId,
    pub(crate) unified_interior: InteriorRegionId,
    pub(crate) light_sites: Vec<PlannedTunnelLightSite>,
    /// All carved or reserved columns, grouped by their declared instance owner.
    pub(crate) route_by_patch: BTreeMap<PatchId, BTreeSet<HexCoord>>,
    pub(crate) full_footprint: BTreeSet<HexCoord>,
    pub(crate) foot_apron_anchor: TilePos,
    pub(crate) tunnel_mouth_anchor: TilePos,
    pub(crate) midpoint_anchor: TilePos,
    pub(crate) gothic_transition_anchor: TilePos,
}

/// All spanning geometry plus the exact patch-local exclusions consumed before
/// liquids and decoration are planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlannedMacroSpanning {
    pub(crate) reservations_by_patch: BTreeMap<PatchId, BTreeSet<HexCoord>>,
    pub(crate) tunnels: BTreeMap<String, PlannedMacroTunnel>,
}

/// A spanning feature fails closed before a partially carved world can escape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MacroSpanningError {
    detail: String,
}

impl MacroSpanningError {
    fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for MacroSpanningError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for MacroSpanningError {}

/// Plans every resolved Macro tunnel against exact final masks.
pub(crate) fn plan_macro_spanning(
    layout: &ResolvedLayoutPlan,
    contracts: &ResolvedMacroContracts,
    raw_destinations: &RawSpanningDestinations,
) -> Result<PlannedMacroSpanning, MacroSpanningError> {
    if layout.kind != LayoutKind::Macro {
        return Err(MacroSpanningError::new(
            "spanning-feature planning requires a resolved Macro layout",
        ));
    }
    if contracts.spanning_features.is_empty() {
        return Ok(PlannedMacroSpanning {
            reservations_by_patch: BTreeMap::new(),
            tunnels: BTreeMap::new(),
        });
    }
    if contracts.spanning_features.len() != 1 {
        return Err(MacroSpanningError::new(
            "the initial Macro spanning pass supports exactly one tunnel",
        ));
    }

    let mut reservations_by_patch = BTreeMap::<PatchId, BTreeSet<HexCoord>>::new();
    let mut tunnels = BTreeMap::new();
    for (ordinal, (name, feature)) in contracts.spanning_features.iter().enumerate() {
        let ResolvedMacroSpanningFeature::Tunnel(contract) = feature;
        if name != &contract.name {
            return Err(MacroSpanningError::new(format!(
                "resolved spanning-feature key {name:?} disagrees with tunnel name {:?}",
                contract.name
            )));
        }
        let destination_key = (
            contract.destination_anchor.instance,
            contract.destination_anchor.anchor.clone(),
        );
        let raw_destination = raw_destinations.get(&destination_key).ok_or_else(|| {
            MacroSpanningError::new(format!(
                "tunnel {name:?} has no raw destination facts for {destination_key:?}"
            ))
        })?;
        let tunnel = plan_tunnel(layout, contract, raw_destination, ordinal)?;
        for (patch, reservation) in &tunnel.route_by_patch {
            let destination = reservations_by_patch.entry(*patch).or_default();
            if reservation.iter().any(|coord| !destination.insert(*coord)) {
                return Err(MacroSpanningError::new(format!(
                    "tunnel {name:?} overlaps another spanning reservation in patch {patch:?}"
                )));
            }
        }
        tunnels.insert(name.clone(), tunnel);
    }
    Ok(PlannedMacroSpanning {
        reservations_by_patch,
        tunnels,
    })
}

fn plan_tunnel(
    layout: &ResolvedLayoutPlan,
    contract: &ResolvedMacroTunnel,
    destination: &RawSpanningDestination,
    ordinal: usize,
) -> Result<PlannedMacroTunnel, MacroSpanningError> {
    validate_tunnel_contract(layout, contract, destination)?;
    let width = usize::try_from(contract.width)
        .map_err(|error| MacroSpanningError::new(format!("tunnel width overflowed: {error}")))?;
    let (lanes, center_lane) = route_complete_lane_bundle(layout, contract, destination, width)?;

    if lanes.len() != width
        || lanes.iter().any(|lane| {
            lane.is_empty()
                || lane
                    .windows(2)
                    .any(|pair| !matches!(pair, [first, second] if first.distance(*second) == 1))
                || lane.iter().copied().collect::<BTreeSet<_>>().len() != lane.len()
        })
    {
        return Err(MacroSpanningError::new(
            "planned tunnel lanes are empty, discontinuous, or self-intersecting",
        ));
    }
    let ribbon = lanes
        .iter()
        .flat_map(|lane| lane.iter().copied())
        .collect::<BTreeSet<_>>();
    let expected_lane_cells = lanes.iter().map(Vec::len).sum::<usize>();
    if ribbon.len() != expected_lane_cells {
        return Err(MacroSpanningError::new(
            "planned tunnel lanes overlap outside their exact seam crossings",
        ));
    }

    let centerline = lanes
        .get(center_lane)
        .cloned()
        .ok_or_else(|| MacroSpanningError::new("tunnel has no representative center lane"))?;
    if let Some(spread) = ribbon.iter().find(|coord| {
        centerline
            .iter()
            .all(|center| coord.distance(*center) > MAX_RIBBON_RADIUS)
    }) {
        let lane_summaries = lanes
            .iter()
            .map(|lane| {
                let nearest = lane
                    .iter()
                    .map(|coord| spread.distance(*coord))
                    .min()
                    .unwrap_or(u32::MAX);
                (lane.first().copied(), lane.last().copied(), nearest)
            })
            .collect::<Vec<_>>();
        let lane_scores = lanes
            .iter()
            .map(|lane| {
                ribbon
                    .iter()
                    .map(|coord| {
                        lane.iter()
                            .map(|center| coord.distance(*center))
                            .min()
                            .unwrap_or(u32::MAX)
                    })
                    .fold((0_u32, 0_u64), |(maximum, total), distance| {
                        (
                            maximum.max(distance),
                            total.saturating_add(u64::from(distance)),
                        )
                    })
            })
            .collect::<Vec<_>>();
        return Err(MacroSpanningError::new(format!(
            "planned tunnel ribbon spreads beyond width-four corridor at {spread:?}; \
             representative lane {center_lane}, scores {lane_scores:?}, \
             lane start/end/nearest-distance {lane_summaries:?}"
        )));
    }
    let (exterior_apron, mouth) = plan_mouth(layout, contract, &lanes, &ribbon)?;
    let foot_threshold = lanes
        .iter()
        .map(|lane| {
            lane.iter()
                .copied()
                .find(|coord| !exterior_apron.contains(coord))
                .map(|coord| TilePos::new(coord, contract.floor_level))
                .ok_or_else(|| {
                    MacroSpanningError::new("tunnel lane never leaves the open exterior apron")
                })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if foot_threshold.len() != width {
        return Err(MacroSpanningError::new(
            "the first roofed tunnel threshold is not exactly four-wide",
        ));
    }

    let mut gothic = BTreeSet::new();
    for lane in &lanes {
        gothic.extend(
            lane.iter()
                .rev()
                .copied()
                .filter(|coord| {
                    !destination
                        .terminal
                        .iter()
                        .any(|surface| surface.coord == *coord)
                        && !mouth.contains(coord)
                })
                .take(GOTHIC_ROW_COUNT),
        );
    }
    if gothic.is_empty() {
        return Err(MacroSpanningError::new(
            "tunnel has no route rows available for its Gothic transition",
        ));
    }

    let owner_by_coord = patch_owners(layout)?;
    let mut occupied = ribbon.union(&mouth).copied().collect::<BTreeSet<_>>();
    let light_forbidden = destination
        .summit_threshold
        .iter()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let light_sites = plan_light_sites(
        layout,
        contract,
        &centerline,
        &ribbon,
        &mouth,
        &light_forbidden,
        &mut occupied,
        &owner_by_coord,
    )?;
    if let Some(uncovered) = ribbon.iter().find(|coord| {
        light_sites
            .iter()
            .all(|site| coord.distance(site.position.coord) > DIM_LIGHT_RADIUS)
    }) {
        let center_distance = centerline
            .iter()
            .map(|center| uncovered.distance(*center))
            .min()
            .unwrap_or(u32::MAX);
        return Err(MacroSpanningError::new(format!(
            "planned tunnel crystals do not cover {uncovered:?}; its distance from the representative lane is {center_distance}"
        )));
    }

    let mut full_footprint = occupied;
    full_footprint.extend(light_sites.iter().map(|site| site.position.coord));
    let mut route_by_patch = BTreeMap::<PatchId, BTreeSet<HexCoord>>::new();
    for coord in &full_footprint {
        let owner = owner_by_coord.get(coord).copied().ok_or_else(|| {
            MacroSpanningError::new(format!("tunnel reservation {coord:?} has no patch owner"))
        })?;
        if !contract.instance_route.contains(&owner) {
            return Err(MacroSpanningError::new(format!(
                "tunnel reservation {coord:?} crosses undeclared patch {owner:?}"
            )));
        }
        route_by_patch.entry(owner).or_default().insert(*coord);
    }

    let raw_interior = destination.interior.ok_or_else(|| {
        MacroSpanningError::new("tunnel destination does not publish one authored interior")
    })?;
    let destination_interior =
        namespace_patch_local_interior(contract.destination_anchor.instance, raw_interior)?;
    let ordinal = u32::try_from(ordinal)
        .map_err(|error| MacroSpanningError::new(format!("tunnel ordinal overflowed: {error}")))?;
    let unified_interior =
        InteriorRegionId(world_owned_id(INTERIOR_LOCAL_BASE.saturating_add(ordinal))?);

    let foot_apron_anchor = TilePos::new(
        stable_middle(&exterior_apron)
            .ok_or_else(|| MacroSpanningError::new("exterior apron is empty"))?,
        contract.floor_level,
    );
    let tunnel_mouth_anchor = stable_middle(
        &foot_threshold
            .iter()
            .map(|position| position.coord)
            .collect(),
    )
    .map(|coord| TilePos::new(coord, contract.floor_level))
    .ok_or_else(|| MacroSpanningError::new("roofed mouth threshold is empty"))?;
    let midpoint_anchor = TilePos::new(
        *centerline
            .get(centerline.len() / 2)
            .ok_or_else(|| MacroSpanningError::new("tunnel centerline is empty"))?,
        contract.floor_level,
    );
    let gothic_transition_anchor = centerline
        .iter()
        .copied()
        .find(|coord| gothic.contains(coord))
        .map(|coord| TilePos::new(coord, contract.floor_level))
        .ok_or_else(|| MacroSpanningError::new("Gothic transition misses the centerline"))?;

    Ok(PlannedMacroTunnel {
        name: contract.name.clone(),
        canonical_route: contract.canonical_route,
        instance_route: contract.instance_route.clone(),
        floor_level: contract.floor_level,
        clearance: contract.clearance,
        roof_thickness: contract.roof_thickness,
        lanes,
        seam_lanes: contract
            .seams
            .iter()
            .map(|seam| seam.port.lanes.clone())
            .collect(),
        centerline,
        ribbon,
        mouth,
        exterior_apron,
        foot_threshold,
        gothic,
        summit_threshold: destination.summit_threshold.clone(),
        destination_anchor: destination.anchor,
        destination_terminal: destination.terminal.clone(),
        destination_interior,
        unified_interior,
        light_sites,
        route_by_patch,
        full_footprint,
        foot_apron_anchor,
        tunnel_mouth_anchor,
        midpoint_anchor,
        gothic_transition_anchor,
    })
}

/// Routes the complete ordered feature around one stable lane identity at a time.
///
/// A segment-local choice can switch between the two middle lanes at a seam and
/// produce two individually compact halves whose union is too wide. Retrying the
/// whole feature preserves one reference identity across every patch. The lower
/// middle lane remains first so existing canonical fingerprints do not move; the
/// other middle and outer identities are deterministic fallbacks for rotated,
/// concave boundary terminals.
fn route_complete_lane_bundle(
    layout: &ResolvedLayoutPlan,
    contract: &ResolvedMacroTunnel,
    destination: &RawSpanningDestination,
    width: usize,
) -> Result<(Vec<Vec<HexCoord>>, usize), MacroSpanningError> {
    let initial_starts = ordered_boundary_starts(&contract.boundary_terminal);
    let lower_middle = width.saturating_sub(1) / 2;
    let upper_middle = width / 2;
    let mut reference_order = vec![lower_middle];
    if upper_middle != lower_middle {
        reference_order.push(upper_middle);
    }
    let outer_references = (0..width)
        .filter(|index| !reference_order.contains(index))
        .collect::<Vec<_>>();
    reference_order.extend(outer_references);

    'reference: for reference_lane in reference_order {
        let mut current_starts = initial_starts.clone();
        let mut lanes = current_starts
            .iter()
            .copied()
            .map(|coord| vec![coord])
            .collect::<Vec<_>>();
        for (patch_index, patch_id) in contract.instance_route.iter().copied().enumerate() {
            let patch = layout.patches.get(&patch_id).ok_or_else(|| {
                MacroSpanningError::new(format!("tunnel names missing route patch {patch_id:?}"))
            })?;
            let target_coords = if let Some(seam) = contract.seams.get(patch_index) {
                if seam.source != patch_id {
                    return Err(MacroSpanningError::new(format!(
                        "tunnel seam {patch_index} starts in {:?}, expected {patch_id:?}",
                        seam.source
                    )));
                }
                ordered_coords(seam.port.lanes.iter().map(|(source, _)| *source))
            } else {
                ordered_coords(destination.terminal.iter().map(|surface| surface.coord))
            };
            if current_starts.len() != width || target_coords.len() != width {
                return Err(MacroSpanningError::new(format!(
                    "tunnel patch {patch_id:?} does not expose exactly {width} starts and targets"
                )));
            }
            let Some(segment) = route_lane_bundle_in_patch_frame(
                &patch.mask,
                &current_starts,
                &target_coords,
                reference_lane,
                patch.rotation_turns,
            ) else {
                continue 'reference;
            };
            for (lane, segment_path) in lanes.iter_mut().zip(&segment) {
                lane.extend(segment_path.iter().copied().skip(1));
            }

            if let Some(seam) = contract.seams.get(patch_index) {
                let crossing_by_source =
                    seam.port.lanes.iter().copied().collect::<BTreeMap<_, _>>();
                let mut next_starts = Vec::with_capacity(width);
                for lane in &mut lanes {
                    let source = lane.last().copied().ok_or_else(|| {
                        MacroSpanningError::new("a tunnel lane unexpectedly became empty")
                    })?;
                    let target = crossing_by_source.get(&source).copied().ok_or_else(|| {
                        MacroSpanningError::new(format!(
                            "routed source {source:?} is not one of seam {patch_index}'s exact lanes"
                        ))
                    })?;
                    if source.distance(target) != 1 {
                        return Err(MacroSpanningError::new(format!(
                            "tunnel seam lane {source:?} -> {target:?} is not adjacent"
                        )));
                    }
                    lane.push(target);
                    next_starts.push(target);
                }
                current_starts = next_starts;
            }
        }

        if lanes.len() != width
            || lanes.iter().any(|lane| {
                lane.is_empty()
                    || lane.windows(2).any(
                        |pair| !matches!(pair, [first, second] if first.distance(*second) == 1),
                    )
                    || lane.iter().copied().collect::<BTreeSet<_>>().len() != lane.len()
            })
        {
            continue;
        }
        let ribbon = lanes.iter().flatten().copied().collect::<BTreeSet<_>>();
        if ribbon.len() != lanes.iter().map(Vec::len).sum::<usize>() {
            continue;
        }
        let Some(reference) = lanes.get(reference_lane) else {
            continue;
        };
        if ribbon.iter().any(|coord| {
            reference
                .iter()
                .all(|center| coord.distance(*center) > MAX_RIBBON_RADIUS)
        }) {
            continue;
        }
        return Ok((lanes, reference_lane));
    }

    Err(MacroSpanningError::new(format!(
        "tunnel {:?} cannot route one compact four-lane bundle through its ordered patches",
        contract.name
    )))
}

fn validate_tunnel_contract(
    layout: &ResolvedLayoutPlan,
    contract: &ResolvedMacroTunnel,
    destination: &RawSpanningDestination,
) -> Result<(), MacroSpanningError> {
    if contract.width != REQUIRED_TUNNEL_WIDTH
        || contract.floor_level != REQUIRED_FLOOR_LEVEL
        || contract.clearance != REQUIRED_CLEARANCE
        || contract.roof_thickness < REQUIRED_ROOF_THICKNESS
    {
        return Err(MacroSpanningError::new(format!(
            "tunnel {:?} must be width {REQUIRED_TUNNEL_WIDTH}, level {REQUIRED_FLOOR_LEVEL}, with {REQUIRED_CLEARANCE} clear and at least {REQUIRED_ROOF_THICKNESS} roof levels",
            contract.name
        )));
    }
    if !contract.canonical_route {
        return Err(MacroSpanningError::new(format!(
            "the initial tunnel {:?} must own the canonical Macro route",
            contract.name
        )));
    }
    if contract.instance_route.len() < 2
        || contract.seams.len() != contract.instance_route.len().saturating_sub(1)
        || contract.instance_route.first().copied() != Some(contract.boundary_terminal.instance)
        || contract.instance_route.last().copied() != Some(contract.destination_anchor.instance)
        || contract
            .instance_route
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            != contract.instance_route.len()
    {
        return Err(MacroSpanningError::new(format!(
            "tunnel {:?} has a malformed ordered instance route",
            contract.name
        )));
    }
    let width = usize::try_from(contract.width).unwrap_or(usize::MAX);
    if contract.boundary_terminal.lanes.len() != width
        || destination.terminal.len() != width
        || destination.summit_threshold.len() != width
        || !destination.terminal.contains(&destination.anchor)
        || destination
            .terminal
            .iter()
            .any(|position| position.level != contract.floor_level)
    {
        return Err(MacroSpanningError::new(format!(
            "tunnel {:?} does not have exact four-wide boundary, destination, and summit terminals",
            contract.name
        )));
    }
    for (index, seam) in contract.seams.iter().enumerate() {
        let expected = contract.instance_route.windows(2).nth(index);
        if expected != Some(&[seam.source, seam.destination]) || seam.port.lanes.len() != width {
            return Err(MacroSpanningError::new(format!(
                "tunnel {:?} seam {index} disagrees with the ordered instance route",
                contract.name
            )));
        }
        let source_mask = &layout
            .patches
            .get(&seam.source)
            .ok_or_else(|| MacroSpanningError::new("tunnel seam source patch is missing"))?
            .mask;
        let destination_mask = &layout
            .patches
            .get(&seam.destination)
            .ok_or_else(|| MacroSpanningError::new("tunnel seam destination patch is missing"))?
            .mask;
        if seam.port.lanes.iter().any(|(source, target)| {
            !source_mask.contains(source)
                || !destination_mask.contains(target)
                || source.distance(*target) != 1
        }) {
            return Err(MacroSpanningError::new(format!(
                "tunnel {:?} seam {index} leaves its declared masks",
                contract.name
            )));
        }
    }
    let destination_mask = &layout
        .patches
        .get(&contract.destination_anchor.instance)
        .ok_or_else(|| MacroSpanningError::new("tunnel destination patch is missing"))?
        .mask;
    if destination
        .terminal
        .iter()
        .chain(&destination.summit_threshold)
        .any(|position| !destination_mask.contains(&position.coord))
    {
        return Err(MacroSpanningError::new(
            "tunnel destination facts leave the declared destination mask",
        ));
    }
    Ok(())
}

fn ordered_coords(coords: impl IntoIterator<Item = HexCoord>) -> Vec<HexCoord> {
    coords
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Orders the boundary lanes in the authored west-facing frame.
///
/// Raw coordinate ordering reverses or reshuffles a curved four-cell terminal
/// under sixty-degree rotation. Normalizing the terminal back to the west-facing
/// frame preserves lane identity, so the same middle lane remains the compact
/// routing reference in every global orientation.
fn ordered_boundary_starts(terminal: &ResolvedMacroBoundaryTerminal) -> Vec<HexCoord> {
    let turns = match terminal.side {
        HexSide::West => 0,
        HexSide::SouthWest => 1,
        HexSide::SouthEast => 2,
        HexSide::East => 3,
        HexSide::NorthEast => 4,
        HexSide::NorthWest => 5,
    };
    let inverse_turns = (6_u8.saturating_sub(turns)) % 6;
    let mut starts = terminal
        .lanes
        .iter()
        .map(|(inside, _)| (rotate_coord(*inside, inverse_turns), *inside))
        .collect::<Vec<_>>();
    starts.sort_unstable();
    starts.into_iter().map(|(_, inside)| inside).collect()
}

fn rotate_coord(mut coord: HexCoord, turns: u8) -> HexCoord {
    for _ in 0..turns % 6 {
        let [x, y, z] = coord.to_cubic_array();
        coord = HexCoord::new_cubic(-z, -x, -y);
    }
    coord
}

fn patch_owners(
    layout: &ResolvedLayoutPlan,
) -> Result<BTreeMap<HexCoord, PatchId>, MacroSpanningError> {
    let mut owners = BTreeMap::new();
    for (patch, resolved) in &layout.patches {
        for coord in &resolved.mask {
            if owners.insert(*coord, *patch).is_some() {
                return Err(MacroSpanningError::new(format!(
                    "resolved Macro masks overlap at {coord:?}"
                )));
            }
        }
    }
    if owners.keys().copied().collect::<BTreeSet<_>>() != layout.footprint {
        return Err(MacroSpanningError::new(
            "resolved Macro masks do not exactly cover the world footprint",
        ));
    }
    Ok(owners)
}

pub(super) fn namespace_patch_local_interior(
    patch: PatchId,
    local: InteriorRegionId,
) -> Result<InteriorRegionId, MacroSpanningError> {
    if patch.0 >= WORLD_NAMESPACE_PREFIX || local.0 >= WORLD_LOCAL_ID_LIMIT {
        return Err(MacroSpanningError::new(format!(
            "destination interior {:?}/{local:?} cannot fit the Macro namespace",
            patch
        )));
    }
    Ok(InteriorRegionId((patch.0 << MACRO_LOCAL_ID_BITS) | local.0))
}

fn world_owned_id(local: u32) -> Result<u32, MacroSpanningError> {
    if local >= WORLD_LOCAL_ID_LIMIT {
        return Err(MacroSpanningError::new(format!(
            "world-owned Macro local ID {local} exceeds its 26-bit namespace"
        )));
    }
    Ok(WORLD_NAMESPACE_BASE | local)
}

fn route_lane_bundle(
    allowed: &BTreeSet<HexCoord>,
    starts: &[HexCoord],
    targets: &[HexCoord],
    reference_lane: usize,
) -> Option<Vec<Vec<HexCoord>>> {
    if starts.len() != targets.len()
        || starts.is_empty()
        || reference_lane >= starts.len()
        || starts.iter().any(|coord| !allowed.contains(coord))
        || targets.iter().any(|coord| !allowed.contains(coord))
        || starts.iter().copied().collect::<BTreeSet<_>>().len() != starts.len()
        || targets.iter().copied().collect::<BTreeSet<_>>().len() != targets.len()
    {
        return None;
    }

    let mut target_permutations = Vec::new();
    permutations(targets.to_vec(), 0, &mut target_permutations);
    let mut fixed_order = vec![reference_lane];
    fixed_order.extend(
        (0..starts.len())
            .filter(|lane| *lane != reference_lane)
            .collect::<Vec<_>>(),
    );
    let mut best: Option<((u32, u32, Vec<Vec<HexCoord>>), Vec<Vec<HexCoord>>)> = None;
    for target_order in &target_permutations {
        if let Some(paths) =
            route_lane_assignment(allowed, starts, target_order, &fixed_order, reference_lane)
        {
            let score = bundle_score(&paths);
            if best.as_ref().is_none_or(|(current, _)| score < *current) {
                best = Some((score, paths));
            }
        }
    }

    // Broad authored masks normally resolve under stable lane priority. If a
    // concavity makes that priority consume another lane's only cell, exhaust the
    // other four-lane priorities before failing the feature closed.
    if best.is_none() {
        let mut remaining = fixed_order.iter().copied().skip(1).collect::<Vec<_>>();
        let mut remaining_orders = Vec::new();
        permutations(std::mem::take(&mut remaining), 0, &mut remaining_orders);
        for target_order in &target_permutations {
            for remaining_order in &remaining_orders {
                let lane_order = std::iter::once(reference_lane)
                    .chain(remaining_order.iter().copied())
                    .collect::<Vec<_>>();
                let Some(paths) = route_lane_assignment(
                    allowed,
                    starts,
                    target_order,
                    &lane_order,
                    reference_lane,
                ) else {
                    continue;
                };
                let score = bundle_score(&paths);
                if best.as_ref().is_none_or(|(current, _)| score < *current) {
                    best = Some((score, paths));
                }
            }
        }
    }

    if best.is_none() {
        for paths in flow_bundles_around_reference(allowed, starts, targets, reference_lane) {
            let score = bundle_score(&paths);
            if best.as_ref().is_none_or(|(current, _)| score < *current) {
                best = Some((score, paths));
            }
        }
    }
    if best.is_none() {
        if let Some(paths) = vertex_disjoint_paths(allowed, starts, targets) {
            let paths = tighten_flow_bundle(allowed, paths);
            let score = bundle_score(&paths);
            best = Some((score, paths));
        }
    }
    best.map(|(_, paths)| paths)
}

fn route_lane_bundle_in_patch_frame(
    allowed: &BTreeSet<HexCoord>,
    starts: &[HexCoord],
    targets: &[HexCoord],
    reference_lane: usize,
    rotation_turns: u8,
) -> Option<Vec<Vec<HexCoord>>> {
    let inverse_turns = (6_u8.saturating_sub(rotation_turns % 6)) % 6;
    let normalized_allowed = allowed
        .iter()
        .map(|coord| rotate_coord(*coord, inverse_turns))
        .collect::<BTreeSet<_>>();
    let normalized_starts = starts
        .iter()
        .map(|coord| rotate_coord(*coord, inverse_turns))
        .collect::<Vec<_>>();
    let normalized_targets = targets
        .iter()
        .map(|coord| rotate_coord(*coord, inverse_turns))
        .collect::<Vec<_>>();
    route_lane_bundle(
        &normalized_allowed,
        &normalized_starts,
        &normalized_targets,
        reference_lane,
    )
    .map(|paths| {
        paths
            .into_iter()
            .map(|path| {
                path.into_iter()
                    .map(|coord| rotate_coord(coord, rotation_turns))
                    .collect()
            })
            .collect()
    })
}

fn tighten_flow_bundle(
    allowed: &BTreeSet<HexCoord>,
    mut paths: Vec<Vec<HexCoord>>,
) -> Vec<Vec<HexCoord>> {
    for _ in 0..2 {
        let mut occupied = paths
            .iter()
            .flat_map(|path| path.iter().copied())
            .collect::<BTreeSet<_>>();
        let mut changed = false;
        for path in &mut paths {
            let Some(start) = path.first().copied() else {
                continue;
            };
            let Some(target) = path.last().copied() else {
                continue;
            };
            for coord in path.iter() {
                occupied.remove(coord);
            }
            let replacement =
                shortest_then_fewest_turns(allowed, &occupied, start, target).filter(|candidate| {
                    (candidate.len(), path_turns(candidate)) < (path.len(), path_turns(path))
                });
            if let Some(replacement) = replacement {
                *path = replacement;
                changed = true;
            }
            occupied.extend(path.iter().copied());
        }
        if !changed {
            break;
        }
    }
    paths
}

fn flow_bundles_around_reference(
    allowed: &BTreeSet<HexCoord>,
    starts: &[HexCoord],
    targets: &[HexCoord],
    reference_lane: usize,
) -> Vec<Vec<Vec<HexCoord>>> {
    let Some(reference_start) = starts.get(reference_lane).copied() else {
        return Vec::new();
    };
    let remaining_starts = starts
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(index, coord)| (index != reference_lane).then_some((index, coord)))
        .collect::<Vec<_>>();
    let mut bundles = Vec::new();
    for reference_target in targets {
        let blocked = starts
            .iter()
            .chain(targets)
            .copied()
            .filter(|coord| *coord != reference_start && *coord != *reference_target)
            .collect::<BTreeSet<_>>();
        let Some(reference) =
            shortest_then_fewest_turns(allowed, &blocked, reference_start, *reference_target)
        else {
            continue;
        };
        let remaining_targets = targets
            .iter()
            .copied()
            .filter(|target| *target != *reference_target)
            .collect::<Vec<_>>();
        for radius in [4_u32, 6, MAX_RIBBON_RADIUS] {
            let mut corridor = allowed
                .iter()
                .copied()
                .filter(|coord| {
                    reference
                        .iter()
                        .any(|center| coord.distance(*center) <= radius)
                })
                .collect::<BTreeSet<_>>();
            for coord in &reference {
                corridor.remove(coord);
            }
            corridor.extend(remaining_starts.iter().map(|(_, coord)| *coord));
            corridor.extend(remaining_targets.iter().copied());
            let Some(remaining_paths) = vertex_disjoint_paths(
                &corridor,
                &remaining_starts
                    .iter()
                    .map(|(_, coord)| *coord)
                    .collect::<Vec<_>>(),
                &remaining_targets,
            ) else {
                continue;
            };
            let mut paths = vec![Vec::new(); starts.len()];
            let Some(reference_slot) = paths.get_mut(reference_lane) else {
                return bundles;
            };
            *reference_slot = reference.clone();
            for ((lane, _), path) in remaining_starts.iter().zip(remaining_paths) {
                let Some(slot) = paths.get_mut(*lane) else {
                    return bundles;
                };
                *slot = path;
            }
            bundles.push(paths);
        }
    }
    bundles
}

fn route_lane_assignment(
    allowed: &BTreeSet<HexCoord>,
    starts: &[HexCoord],
    targets: &[HexCoord],
    lane_order: &[usize],
    reference_lane: usize,
) -> Option<Vec<Vec<HexCoord>>> {
    let mut occupied = starts
        .iter()
        .chain(targets)
        .copied()
        .collect::<BTreeSet<_>>();
    let mut paths = vec![Vec::new(); starts.len()];
    let mut reference_path = None;
    for lane in lane_order {
        let start = *starts.get(*lane)?;
        let target = *targets.get(*lane)?;
        occupied.remove(&start);
        occupied.remove(&target);
        let compact_allowed = reference_path.as_ref().map(|reference: &Vec<HexCoord>| {
            allowed
                .iter()
                .copied()
                .filter(|coord| {
                    *coord == start
                        || *coord == target
                        || reference
                            .iter()
                            .any(|center| coord.distance(*center) <= MAX_RIBBON_RADIUS)
                })
                .collect::<BTreeSet<_>>()
        });
        let path = shortest_then_fewest_turns(
            compact_allowed.as_ref().unwrap_or(allowed),
            &occupied,
            start,
            target,
        )?;
        if *lane == reference_lane {
            reference_path = Some(path.clone());
        }
        occupied.extend(path.iter().copied());
        *paths.get_mut(*lane)? = path;
    }
    Some(paths)
}

fn permutations<T: Clone>(mut values: Vec<T>, index: usize, output: &mut Vec<Vec<T>>) {
    if index == values.len() {
        output.push(values);
        return;
    }
    for next in index..values.len() {
        values.swap(index, next);
        permutations(values.clone(), index.saturating_add(1), output);
        values.swap(index, next);
    }
}

fn bundle_score(paths: &[Vec<HexCoord>]) -> (u32, u32, Vec<Vec<HexCoord>>) {
    let steps = paths
        .iter()
        .map(|path| u32::try_from(path.len().saturating_sub(1)).unwrap_or(u32::MAX))
        .fold(0_u32, u32::saturating_add);
    let turns = paths
        .iter()
        .map(|path| path_turns(path))
        .fold(0_u32, u32::saturating_add);
    (steps, turns, paths.to_vec())
}

fn path_turns(path: &[HexCoord]) -> u32 {
    path.windows(3)
        .filter(|window| {
            let [first, second, third] = window else {
                return false;
            };
            direction_between(*first, *second) != direction_between(*second, *third)
        })
        .count()
        .try_into()
        .unwrap_or(u32::MAX)
}

fn direction_between(first: HexCoord, second: HexCoord) -> Option<u8> {
    HexSide::ALL
        .iter()
        .position(|side| side.neighbor(first) == second)
        .and_then(|index| u8::try_from(index).ok())
}

fn shortest_then_fewest_turns(
    allowed: &BTreeSet<HexCoord>,
    blocked: &BTreeSet<HexCoord>,
    start: HexCoord,
    goal: HexCoord,
) -> Option<Vec<HexCoord>> {
    const NO_DIRECTION: u8 = 6;
    if !allowed.contains(&start)
        || !allowed.contains(&goal)
        || blocked.contains(&start)
        || blocked.contains(&goal)
    {
        return None;
    }
    let start_state = (start, NO_DIRECTION);
    let mut best = BTreeMap::from([(start_state, (0_u32, 0_u32))]);
    let mut previous = BTreeMap::<(HexCoord, u8), (HexCoord, u8)>::new();
    let mut pending = BinaryHeap::from([Reverse((0_u32, 0_u32, start, NO_DIRECTION))]);
    let mut goal_state = None;
    while let Some(Reverse((steps, turns, coord, prior_direction))) = pending.pop() {
        if best.get(&(coord, prior_direction)).copied() != Some((steps, turns)) {
            continue;
        }
        if coord == goal {
            goal_state = Some((coord, prior_direction));
            break;
        }
        for (index, side) in HexSide::ALL.into_iter().enumerate() {
            let neighbor = side.neighbor(coord);
            if !allowed.contains(&neighbor) || blocked.contains(&neighbor) {
                continue;
            }
            let direction = u8::try_from(index).ok()?;
            let next = (
                steps.saturating_add(1),
                turns.saturating_add(u32::from(
                    prior_direction != NO_DIRECTION && prior_direction != direction,
                )),
            );
            let state = (neighbor, direction);
            let candidate_predecessor = (coord, prior_direction);
            let replace = match best.get(&state).copied() {
                None => true,
                Some(current) if next < current => true,
                Some(current) if next == current => previous
                    .get(&state)
                    .is_none_or(|existing| candidate_predecessor < *existing),
                Some(_) => false,
            };
            if replace {
                best.insert(state, next);
                previous.insert(state, candidate_predecessor);
                pending.push(Reverse((next.0, next.1, neighbor, direction)));
            }
        }
    }

    let mut state = goal_state?;
    let mut path = vec![state.0];
    while state != start_state {
        state = *previous.get(&state)?;
        path.push(state.0);
    }
    path.reverse();
    Some(path)
}

fn plan_mouth(
    layout: &ResolvedLayoutPlan,
    contract: &ResolvedMacroTunnel,
    lanes: &[Vec<HexCoord>],
    ribbon: &BTreeSet<HexCoord>,
) -> Result<(BTreeSet<HexCoord>, BTreeSet<HexCoord>), MacroSpanningError> {
    let patch = layout
        .patches
        .get(&contract.boundary_terminal.instance)
        .ok_or_else(|| MacroSpanningError::new("tunnel boundary patch is missing"))?;
    let boundary_cells = patch
        .mask
        .iter()
        .copied()
        .filter(|coord| {
            !layout
                .footprint
                .contains(&contract.boundary_terminal.side.neighbor(*coord))
        })
        .collect::<BTreeSet<_>>();
    let starts = contract
        .boundary_terminal
        .lanes
        .iter()
        .map(|(inside, _)| *inside)
        .collect::<BTreeSet<_>>();
    let ordered_boundary = ordered_line_component(&boundary_cells, &starts).ok_or_else(|| {
        MacroSpanningError::new("the tunnel boundary terminal is not on one simple boundary line")
    })?;
    if ordered_boundary.len() < EXTERIOR_MOUTH_WIDTH {
        return Err(MacroSpanningError::new(
            "the tunnel boundary cannot fit an eight-wide exterior mouth",
        ));
    }
    let exterior_apron = ordered_boundary
        .windows(EXTERIOR_MOUTH_WIDTH)
        .filter(|window| starts.iter().all(|coord| window.contains(coord)))
        .map(|window| window.iter().copied().collect::<BTreeSet<_>>())
        .min_by_key(|window| {
            let distance = window
                .iter()
                .map(|coord| {
                    starts
                        .iter()
                        .map(|start| coord.distance(*start))
                        .min()
                        .unwrap_or(u32::MAX)
                })
                .sum::<u32>();
            (distance, window.clone())
        })
        .ok_or_else(|| {
            MacroSpanningError::new(
                "the exact four boundary lanes cannot expand into one contiguous eight-wide mouth",
            )
        })?;

    let mut mouth = exterior_apron.clone();
    let inward = contract.boundary_terminal.side.opposite();
    let ordered_apron = ordered_line_component(&exterior_apron, &starts).ok_or_else(|| {
        MacroSpanningError::new("the exterior mouth apron is not one simple line")
    })?;
    for (depth, width) in [(1_u32, 6_usize), (2_u32, 4_usize)] {
        let trim = ordered_apron.len().saturating_sub(width) / 2;
        for coord in ordered_apron.iter().skip(trim).take(width) {
            let shifted = step_side(*coord, inward, depth);
            if !patch.mask.contains(&shifted) {
                return Err(MacroSpanningError::new(format!(
                    "the widened mouth leaves its boundary patch at {shifted:?}"
                )));
            }
            mouth.insert(shifted);
        }
    }
    // Continue the 8 -> 6 -> 4 taper into a monumental twelve-row, four-lane
    // authored entrance. The low outer slope intentionally has no natural
    // overburden here, so the complete approach receives the same exact masonry
    // arch as the widened facade rather than depending on terrain height.
    mouth.extend(
        lanes
            .iter()
            .flat_map(|lane| lane.iter().copied().take(EXTERIOR_MOUTH_ROUTE_ROWS)),
    );
    if !starts.is_subset(&exterior_apron)
        || !ribbon.is_superset(&starts)
        || mouth.iter().any(|coord| !patch.mask.contains(coord))
    {
        return Err(MacroSpanningError::new(
            "the widened mouth disagrees with its exact boundary lanes or owner",
        ));
    }
    Ok((exterior_apron, mouth))
}

fn ordered_line_component(
    cells: &BTreeSet<HexCoord>,
    required: &BTreeSet<HexCoord>,
) -> Option<Vec<HexCoord>> {
    let start = *required.first()?;
    if !required.is_subset(cells) {
        return None;
    }
    let mut component = BTreeSet::from([start]);
    let mut pending = vec![start];
    while let Some(coord) = pending.pop() {
        for neighbor in coord.neighbors() {
            if cells.contains(&neighbor) && component.insert(neighbor) {
                pending.push(neighbor);
            }
        }
    }
    if !required.is_subset(&component)
        || component.iter().any(|coord| {
            coord
                .neighbors()
                .into_iter()
                .filter(|neighbor| component.contains(neighbor))
                .count()
                > 2
        })
    {
        return None;
    }
    let first = component
        .iter()
        .copied()
        .find(|coord| {
            coord
                .neighbors()
                .into_iter()
                .filter(|neighbor| component.contains(neighbor))
                .count()
                <= 1
        })
        .or_else(|| component.first().copied())?;
    let mut ordered = vec![first];
    let mut previous = None;
    while ordered.len() < component.len() {
        let current = *ordered.last()?;
        let next = current
            .neighbors()
            .into_iter()
            .filter(|neighbor| component.contains(neighbor) && Some(*neighbor) != previous)
            .min()?;
        previous = Some(current);
        if ordered.contains(&next) {
            return None;
        }
        ordered.push(next);
    }
    Some(ordered)
}

fn step_side(mut coord: HexCoord, side: HexSide, steps: u32) -> HexCoord {
    for _ in 0..steps {
        coord = side.neighbor(coord);
    }
    coord
}

#[expect(
    clippy::too_many_arguments,
    reason = "alcove selection must honor the complete reserved route contract"
)]
fn plan_light_sites(
    layout: &ResolvedLayoutPlan,
    contract: &ResolvedMacroTunnel,
    centerline: &[HexCoord],
    ribbon: &BTreeSet<HexCoord>,
    mouth: &BTreeSet<HexCoord>,
    forbidden: &BTreeSet<HexCoord>,
    occupied: &mut BTreeSet<HexCoord>,
    owner_by_coord: &BTreeMap<HexCoord, PatchId>,
) -> Result<Vec<PlannedTunnelLightSite>, MacroSpanningError> {
    if centerline.is_empty() {
        return Err(MacroSpanningError::new(
            "cannot place tunnel lights along an empty centerline",
        ));
    }
    // The monumental mouth receives its own exact masonry roof, but an adjacent
    // alcove candidate can fall on the exposed foothill outside that footprint.
    // Sample only beyond the mouth in the naturally roofed body so a light
    // reservation never extends the entrance roof into an unrelated exterior column.
    let light_centerline = centerline
        .iter()
        .copied()
        .filter(|coord| !mouth.contains(coord))
        .collect::<Vec<_>>();
    if light_centerline.is_empty() {
        return Err(MacroSpanningError::new(
            "cannot place tunnel lights without a roofed centerline",
        ));
    }
    let indices = light_sample_indices(light_centerline.len());

    let mut sites = Vec::new();
    for (fixture_index, requested) in indices.into_iter().enumerate() {
        let mut search_indices = (0..light_centerline.len()).collect::<Vec<_>>();
        search_indices
            .sort_unstable_by_key(|candidate| (candidate.abs_diff(requested), *candidate));
        let selected = search_indices.into_iter().find_map(|candidate_index| {
            let spine = *light_centerline.get(candidate_index)?;
            let owner = *owner_by_coord.get(&spine)?;
            let patch = layout.patches.get(&owner)?;
            let mut candidates = spine
                .within_radius(2)
                .into_iter()
                .filter(|candidate| {
                    *candidate != spine
                        && patch.mask.contains(candidate)
                        && owner_by_coord.get(candidate) == Some(&owner)
                        && contract.instance_route.contains(&owner)
                        && !occupied.contains(candidate)
                        && !mouth.contains(candidate)
                        && !forbidden.contains(candidate)
                        && candidate.neighbors().into_iter().all(|neighbor| {
                            owner_by_coord
                                .get(&neighbor)
                                .is_none_or(|neighbor_owner| *neighbor_owner == owner)
                        })
                        && candidate
                            .neighbors()
                            .into_iter()
                            .any(|neighbor| ribbon.contains(&neighbor))
                })
                .collect::<Vec<_>>();
            candidates.sort_unstable_by_key(|candidate| {
                (
                    candidate.distance(spine),
                    candidate.distance(HexCoord::ORIGIN),
                    *candidate,
                )
            });
            candidates.first().copied().map(|coord| (owner, coord))
        });
        let Some((patch, coord)) = selected else {
            return Err(MacroSpanningError::new(format!(
                "tunnel {:?} cannot reserve crystal alcove {fixture_index}",
                contract.name
            )));
        };
        occupied.insert(coord);
        let kind = match fixture_index % 3 {
            0 => CaveCrystalKind::LowCluster,
            1 => CaveCrystalKind::Branched,
            _ => CaveCrystalKind::Spire,
        };
        sites.push(PlannedTunnelLightSite {
            patch,
            position: TilePos::new(coord, contract.floor_level),
            kind,
            rotation: u8::try_from(fixture_index % 6).unwrap_or_default(),
        });
    }
    Ok(sites)
}

fn light_sample_indices(centerline_len: usize) -> Vec<usize> {
    if centerline_len == 0 {
        return Vec::new();
    }
    let last = centerline_len.saturating_sub(1);
    let mut indices = Vec::new();
    let mut index = last.min(LIGHT_SPACING_STEPS / 2);
    loop {
        indices.push(index);
        if index.saturating_add(LIGHT_SPACING_STEPS) >= last {
            break;
        }
        index = index.saturating_add(LIGHT_SPACING_STEPS);
    }
    if last.saturating_sub(*indices.last().unwrap_or(&0)) > LIGHT_SPACING_STEPS / 2 {
        indices.push(last.saturating_sub(LIGHT_SPACING_STEPS / 2));
    }
    indices.sort_unstable();
    indices.dedup();
    indices
}

fn stable_middle<T: Copy + Ord>(values: &BTreeSet<T>) -> Option<T> {
    values
        .iter()
        .copied()
        .nth(values.len().saturating_sub(1) / 2)
}

/// Applies the already-reserved tunnels to an owned merged world.
///
/// Taking and returning the complete plan is intentional: any error drops the
/// private working value, so callers can never observe half-rewritten interior or
/// volume state.
pub(crate) fn apply_macro_spanning(
    mut world: GeneratedWorldPlan,
    planned: &PlannedMacroSpanning,
) -> Result<GeneratedWorldPlan, MacroSpanningError> {
    if planned.tunnels.is_empty() {
        return Ok(world);
    }
    if world.layout.kind != LayoutKind::Macro || planned.tunnels.len() != 1 {
        return Err(MacroSpanningError::new(
            "the initial spanning application requires one tunnel in one Macro world",
        ));
    }
    validate_planned_reservations(&world.layout, planned)?;
    reject_world_namespace_collisions(&world)?;
    for tunnel in planned.tunnels.values() {
        preflight_application(&world, tunnel)?;
    }
    for (ordinal, tunnel) in planned.tunnels.values().enumerate() {
        apply_tunnel(&mut world, tunnel, ordinal)?;
    }
    Ok(world)
}

fn validate_planned_reservations(
    layout: &ResolvedLayoutPlan,
    planned: &PlannedMacroSpanning,
) -> Result<(), MacroSpanningError> {
    let owners = patch_owners(layout)?;
    let expected = planned
        .tunnels
        .values()
        .flat_map(|tunnel| {
            tunnel
                .route_by_patch
                .iter()
                .flat_map(|(patch, coords)| coords.iter().copied().map(|coord| (*patch, coord)))
        })
        .fold(
            BTreeMap::<PatchId, BTreeSet<HexCoord>>::new(),
            |mut by_patch, (patch, coord)| {
                by_patch.entry(patch).or_default().insert(coord);
                by_patch
            },
        );
    if expected != planned.reservations_by_patch {
        return Err(MacroSpanningError::new(
            "spanning reservations disagree with the planned per-patch route",
        ));
    }
    for (patch, coords) in &planned.reservations_by_patch {
        if coords.iter().any(|coord| owners.get(coord) != Some(patch)) {
            return Err(MacroSpanningError::new(format!(
                "spanning reservation leaves declared patch {patch:?}"
            )));
        }
    }
    Ok(())
}

fn reject_world_namespace_collisions(world: &GeneratedWorldPlan) -> Result<(), MacroSpanningError> {
    let world_owned = |id: u32| id >> MACRO_LOCAL_ID_BITS == WORLD_NAMESPACE_PREFIX;
    if world.features.by_id.keys().any(|id| world_owned(id.0))
        || world.structures.by_id.keys().any(|id| world_owned(id.0))
        || world.liquids.bodies.keys().any(|id| world_owned(id.0))
        || world.lights.keys().any(|id| world_owned(id.0))
        || world.interiors.by_id.keys().any(|id| world_owned(id.0))
        || world.volume.surfaces.values().any(|metadata| {
            matches!(
                metadata.access,
                SurfaceAccess::SpecialMovement(SpecialMovementRegion(id)) if world_owned(id)
            )
        })
    {
        return Err(MacroSpanningError::new(
            "merged fragments already occupy reserved Macro namespace prefix 63",
        ));
    }
    Ok(())
}

fn preflight_application(
    world: &GeneratedWorldPlan,
    tunnel: &PlannedMacroTunnel,
) -> Result<(), MacroSpanningError> {
    if world.interiors.by_id.contains_key(&tunnel.unified_interior) {
        return Err(MacroSpanningError::new(format!(
            "world-owned interior {:?} already exists",
            tunnel.unified_interior
        )));
    }
    if !world
        .interiors
        .by_id
        .contains_key(&tunnel.destination_interior)
    {
        return Err(MacroSpanningError::new(format!(
            "destination interior {:?} was not published by the merged landmark",
            tunnel.destination_interior
        )));
    }
    if world.features.protected_routes.contains_key(&tunnel.name) {
        return Err(MacroSpanningError::new(format!(
            "protected route {:?} already exists",
            tunnel.name
        )));
    }
    if tunnel.full_footprint.iter().any(|coord| {
        !world.volume.mask.contains(coord) || !world.volume.columns.contains_key(coord)
    }) {
        return Err(MacroSpanningError::new(format!(
            "tunnel {:?} leaves the merged semantic volume",
            tunnel.name
        )));
    }
    if world.liquids.bodies.values().any(|body| {
        body.nodes
            .keys()
            .any(|position| tunnel.full_footprint.contains(&position.coord))
    }) {
        return Err(MacroSpanningError::new(format!(
            "tunnel {:?} collides with a planned liquid body",
            tunnel.name
        )));
    }
    if tunnel.full_footprint.iter().any(|coord| {
        world.volume.columns.get(coord).is_some_and(|column| {
            column
                .elements
                .iter()
                .any(|element| matches!(element, VolumeElement::Fill(_)))
        })
    }) {
        return Err(MacroSpanningError::new(format!(
            "tunnel {:?} intersects a non-solid fill run",
            tunnel.name
        )));
    }

    let apron_features = world.features.by_id.values().any(|feature| {
        tunnel.exterior_apron.contains(&feature.root.coord)
            || feature
                .blocker_footprint
                .iter()
                .any(|position| tunnel.exterior_apron.contains(&position.coord))
    });
    if apron_features {
        return Err(MacroSpanningError::new(format!(
            "tunnel {:?} exterior aperture would delete an authored feature",
            tunnel.name
        )));
    }
    let apron_removes = |position: TilePos| {
        tunnel.exterior_apron.contains(&position.coord)
            && removed_by_tunnel(tunnel, position).unwrap_or(true)
    };
    if world.anchors.values().copied().any(apron_removes)
        || world
            .lights
            .values()
            .any(|light| apron_removes(light.origin))
        || world
            .features
            .protected_routes
            .values()
            .any(|route| route.surfaces.iter().copied().any(apron_removes))
        || world
            .features
            .clearings
            .values()
            .any(|clearing| clearing.surfaces.iter().copied().any(apron_removes))
        || world.interiors.by_id.values().any(|interior| {
            interior.floors.iter().copied().any(apron_removes)
                || interior.roof_voxels.iter().copied().any(apron_removes)
        })
    {
        return Err(MacroSpanningError::new(format!(
            "tunnel {:?} exterior aperture would delete referenced semantic state",
            tunnel.name
        )));
    }
    for blocker in &world.blockers {
        let clearance_top = clearance_top(tunnel, blocker.coord)?;
        if tunnel.full_footprint.contains(&blocker.coord) && blocker.level < clearance_top {
            return Err(MacroSpanningError::new(format!(
                "tunnel {:?} intersects traversal blocker {blocker:?}",
                tunnel.name
            )));
        }
    }
    for (structure_id, structure) in &world.structures.by_id {
        if let Some(voxel) = structure
            .voxels
            .iter()
            .copied()
            .find(|voxel| removed_by_tunnel(tunnel, *voxel).unwrap_or(true))
        {
            return Err(MacroSpanningError::new(format!(
                "tunnel {:?} would remove authored structure {structure_id:?} voxel {voxel:?} (ribbon={}, mouth={}, light_alcove={})",
                tunnel.name,
                tunnel.ribbon.contains(&voxel.coord),
                tunnel.mouth.contains(&voxel.coord),
                tunnel
                    .light_sites
                    .iter()
                    .any(|site| site.position.coord == voxel.coord),
            )));
        }
    }

    for position in tunnel
        .destination_terminal
        .iter()
        .chain(&tunnel.summit_threshold)
    {
        if !matches!(
            world
                .volume
                .surfaces
                .get(position)
                .map(|metadata| metadata.access),
            Some(SurfaceAccess::Ordinary)
        ) {
            return Err(MacroSpanningError::new(format!(
                "authored tunnel threshold {position:?} is not ordinary footing"
            )));
        }
    }

    for coord in &tunnel.full_footprint {
        let column = world.volume.columns.get(coord).ok_or_else(|| {
            MacroSpanningError::new(format!("tunnel column {coord:?} is missing"))
        })?;
        let floor_mass = solid_mass_at(column, tunnel.floor_level).ok_or_else(|| {
            MacroSpanningError::new(format!(
                "tunnel floor level {} is not solid at {coord:?}",
                tunnel.floor_level
            ))
        })?;
        if floor_mass.material == SolidMaterialRole::Bedrock {
            return Err(MacroSpanningError::new(format!(
                "tunnel floor at {coord:?} breaches authored bedrock"
            )));
        }
        if is_destination_terminal_coord(tunnel, *coord) {
            if !existing_clear_floor(world, tunnel, *coord)? {
                return Err(MacroSpanningError::new(format!(
                    "authored destination terminal {coord:?} is not a clear supported level-{} floor",
                    tunnel.floor_level
                )));
            }
            continue;
        }
        let clear_top = clearance_top(tunnel, *coord)?;
        let roof_end = clear_top
            .checked_add(i32::try_from(tunnel.roof_thickness).map_err(|error| {
                MacroSpanningError::new(format!("roof thickness overflowed: {error}"))
            })?)
            .ok_or_else(|| MacroSpanningError::new("tunnel roof level overflowed"))?;
        if column.elements.iter().any(|element| {
            matches!(
                element,
                VolumeElement::Solid(mass)
                    if mass.material == SolidMaterialRole::Bedrock
                        && mass.levels.bottom < roof_end
                        && tunnel.floor_level < mass.levels.top
            )
        }) {
            return Err(MacroSpanningError::new(format!(
                "tunnel at {coord:?} overwrites authored bedrock between levels {} and {roof_end}",
                tunnel.floor_level
            )));
        }
        if !tunnel.mouth.contains(coord) {
            for level in clear_top..roof_end {
                if solid_mass_at(column, level).is_none() {
                    return Err(MacroSpanningError::new(format!(
                        "tunnel {coord:?} lacks solid roof at level {level}"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn removed_by_tunnel(
    tunnel: &PlannedMacroTunnel,
    voxel: TilePos,
) -> Result<bool, MacroSpanningError> {
    if !tunnel.full_footprint.contains(&voxel.coord) {
        return Ok(false);
    }
    if is_destination_terminal_coord(tunnel, voxel.coord) {
        return Ok(false);
    }
    let clear_top = clearance_top(tunnel, voxel.coord)?;
    Ok(tunnel.floor_level < voxel.level && voxel.level < clear_top)
}

fn is_destination_terminal_coord(tunnel: &PlannedMacroTunnel, coord: HexCoord) -> bool {
    tunnel
        .destination_terminal
        .iter()
        .any(|position| position.coord == coord)
}

fn clearance_top(
    tunnel: &PlannedMacroTunnel,
    coord: HexCoord,
) -> Result<Level, MacroSpanningError> {
    let clearance = if tunnel.mouth.contains(&coord) {
        EXTERIOR_MOUTH_CLEARANCE
    } else {
        tunnel.clearance
    };
    tunnel
        .floor_level
        .checked_add(1)
        .and_then(|level| level.checked_add(i32::try_from(clearance).ok()?))
        .ok_or_else(|| MacroSpanningError::new("tunnel clearance level overflowed"))
}

fn solid_mass_at(column: &VolumeColumn, level: Level) -> Option<SolidMass> {
    column.elements.iter().find_map(|element| {
        let VolumeElement::Solid(mass) = *element else {
            return None;
        };
        (mass.levels.bottom <= level && level < mass.levels.top).then_some(mass)
    })
}

fn existing_clear_floor(
    world: &GeneratedWorldPlan,
    tunnel: &PlannedMacroTunnel,
    coord: HexCoord,
) -> Result<bool, MacroSpanningError> {
    let floor = TilePos::new(coord, tunnel.floor_level);
    if !world.volume.surfaces.contains_key(&floor) {
        return Ok(false);
    }
    let clear_bottom = tunnel.floor_level.saturating_add(1);
    let clear_top = clearance_top(tunnel, coord)?;
    let column = world
        .volume
        .columns
        .get(&coord)
        .ok_or_else(|| MacroSpanningError::new(format!("missing tunnel column {coord:?}")))?;
    Ok(column.elements.iter().all(|element| {
        let (bottom, top) = element_interval(element);
        top <= clear_bottom || bottom >= clear_top
    }))
}

fn apply_tunnel(
    world: &mut GeneratedWorldPlan,
    tunnel: &PlannedMacroTunnel,
    ordinal: usize,
) -> Result<(), MacroSpanningError> {
    rewrite_destination_interior(world, tunnel)?;

    let mut coords = tunnel.full_footprint.iter().copied().collect::<Vec<_>>();
    coords.sort_unstable();
    for coord in coords {
        if is_destination_terminal_coord(tunnel, coord) {
            continue;
        }
        let material = if tunnel.gothic.contains(&coord) {
            SolidMaterialRole::WorkedStone
        } else {
            SolidMaterialRole::Stone
        };
        carve_column(
            world,
            tunnel,
            coord,
            clearance_top(tunnel, coord)?,
            material,
        )?;
    }

    for coord in &tunnel.full_footprint {
        let coord = *coord;
        let position = TilePos::new(coord, tunnel.floor_level);
        let metadata = world.volume.surfaces.get_mut(&position).ok_or_else(|| {
            MacroSpanningError::new(format!("tunnel floor {position:?} was not exposed"))
        })?;
        metadata.access = SurfaceAccess::Ordinary;
        metadata.interior =
            (!tunnel.exterior_apron.contains(&coord)).then_some(tunnel.unified_interior);
    }
    for position in &tunnel.summit_threshold {
        let metadata = world.volume.surfaces.get_mut(position).ok_or_else(|| {
            MacroSpanningError::new(format!(
                "summit threshold {position:?} disappeared before interior publication"
            ))
        })?;
        metadata.access = SurfaceAccess::Ordinary;
        metadata.interior = Some(tunnel.unified_interior);
    }

    validate_applied_tunnel_geometry(world, tunnel)?;

    close_undeclared_tunnel_seams(world, tunnel, ordinal)?;
    publish_tunnel_lights(world, tunnel, ordinal)?;
    publish_tunnel_route(world, tunnel)?;
    rebuild_unified_interior(world, tunnel)?;
    publish_tunnel_anchors(world, tunnel)?;
    Ok(())
}

/// Verifies the semantic volume produced by the atomic carve before any route,
/// light, or interior publication can make it observable.
///
/// Crystal Ascent's exact four authored destination columns deliberately retain
/// their taller pointed aperture. Every other reserved column has an exact empty
/// run followed immediately by the configured solid roof; a naturally open
/// mountain column is never allowed to masquerade as a valid tunnel.
fn validate_applied_tunnel_geometry(
    world: &GeneratedWorldPlan,
    tunnel: &PlannedMacroTunnel,
) -> Result<(), MacroSpanningError> {
    if tunnel.destination_terminal.len() != 4 {
        return Err(MacroSpanningError::new(format!(
            "tunnel {:?} no longer has exactly four authored destination columns",
            tunnel.name
        )));
    }

    for coord in &tunnel.full_footprint {
        let position = TilePos::new(*coord, tunnel.floor_level);
        let metadata = world.volume.surfaces.get(&position).ok_or_else(|| {
            MacroSpanningError::new(format!(
                "applied tunnel floor {position:?} is not an exposed surface"
            ))
        })?;
        if metadata.access != SurfaceAccess::Ordinary {
            return Err(MacroSpanningError::new(format!(
                "applied tunnel floor {position:?} is not ordinary footing"
            )));
        }
        let expected_interior =
            (!tunnel.exterior_apron.contains(coord)).then_some(tunnel.unified_interior);
        if metadata.interior != expected_interior {
            return Err(MacroSpanningError::new(format!(
                "applied tunnel floor {position:?} has interior {:?}, expected {expected_interior:?}",
                metadata.interior
            )));
        }

        let column = world.volume.columns.get(coord).ok_or_else(|| {
            MacroSpanningError::new(format!("applied tunnel column {coord:?} is missing"))
        })?;
        if is_destination_terminal_coord(tunnel, *coord) {
            if !existing_clear_floor(world, tunnel, *coord)? {
                return Err(MacroSpanningError::new(format!(
                    "authored destination terminal {coord:?} was not preserved as a clear supported floor"
                )));
            }
            continue;
        }

        let expected_material = if tunnel.gothic.contains(coord) {
            SolidMaterialRole::WorkedStone
        } else {
            SolidMaterialRole::Stone
        };
        let floor_mass = solid_mass_at(column, tunnel.floor_level).ok_or_else(|| {
            MacroSpanningError::new(format!(
                "applied tunnel floor level {} is unsupported at {coord:?}",
                tunnel.floor_level
            ))
        })?;
        if floor_mass.material != expected_material {
            return Err(MacroSpanningError::new(format!(
                "applied tunnel floor at {coord:?} has material {:?}, expected {expected_material:?}",
                floor_mass.material
            )));
        }

        let clear_bottom = tunnel
            .floor_level
            .checked_add(1)
            .ok_or_else(|| MacroSpanningError::new("applied tunnel clearance bottom overflowed"))?;
        let clear_top = clearance_top(tunnel, *coord)?;
        for level in clear_bottom..clear_top {
            if column_occupied_at(column, level) {
                return Err(MacroSpanningError::new(format!(
                    "applied tunnel {coord:?} is occupied inside its clearance at level {level}"
                )));
            }
        }

        let roof_end = clear_top
            .checked_add(i32::try_from(tunnel.roof_thickness).map_err(|error| {
                MacroSpanningError::new(format!("roof thickness overflowed: {error}"))
            })?)
            .ok_or_else(|| MacroSpanningError::new("applied tunnel roof level overflowed"))?;
        for level in clear_top..roof_end {
            let roof_mass = solid_mass_at(column, level).ok_or_else(|| {
                MacroSpanningError::new(format!(
                    "applied tunnel {coord:?} lacks its exact roof at level {level}; column is {column:?}"
                ))
            })?;
            if roof_mass.material != expected_material
                || roof_mass.cutaway_for != Some(tunnel.unified_interior)
            {
                return Err(MacroSpanningError::new(format!(
                    "applied tunnel {coord:?} roof at level {level} has material/cutaway {:?}/{:?}, expected {expected_material:?}/{:?}",
                    roof_mass.material, roof_mass.cutaway_for, tunnel.unified_interior
                )));
            }
        }
    }
    Ok(())
}

fn close_undeclared_tunnel_seams(
    world: &mut GeneratedWorldPlan,
    tunnel: &PlannedMacroTunnel,
    ordinal: usize,
) -> Result<(), MacroSpanningError> {
    let ordinal = u32::try_from(ordinal)
        .map_err(|error| MacroSpanningError::new(format!("tunnel ordinal overflowed: {error}")))?;
    let region = SpecialMovementRegion(world_owned_id(
        SEAM_CLOSURE_LOCAL_BASE
            .checked_add(ordinal)
            .ok_or_else(|| MacroSpanningError::new("seam-closure namespace overflowed"))?,
    )?);
    let route = tunnel
        .ribbon
        .union(&tunnel.mouth)
        .copied()
        .collect::<BTreeSet<_>>();
    let mut close = BTreeSet::new();
    for edge in world.layout.shared_edges.values() {
        for (first, second) in &edge.boundary_pairs {
            let first_position = TilePos::new(*first, tunnel.floor_level);
            let second_position = TilePos::new(*second, tunnel.floor_level);
            let first_ordinary = world
                .volume
                .surfaces
                .get(&first_position)
                .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary);
            let second_ordinary = world
                .volume
                .surfaces
                .get(&second_position)
                .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary);
            if !first_ordinary || !second_ordinary {
                continue;
            }
            match (route.contains(first), route.contains(second)) {
                (true, false) => {
                    close.insert(second_position);
                }
                (false, true) => {
                    close.insert(first_position);
                }
                (true, true) | (false, false) => {}
            }
        }
    }
    for position in close {
        let metadata = world.volume.surfaces.get_mut(&position).ok_or_else(|| {
            MacroSpanningError::new(format!("tunnel seam closure lost surface {position:?}"))
        })?;
        metadata.access = SurfaceAccess::SpecialMovement(region);
    }
    Ok(())
}

fn rewrite_destination_interior(
    world: &mut GeneratedWorldPlan,
    tunnel: &PlannedMacroTunnel,
) -> Result<(), MacroSpanningError> {
    let _old = world
        .interiors
        .by_id
        .remove(&tunnel.destination_interior)
        .ok_or_else(|| MacroSpanningError::new("destination interior disappeared before apply"))?;
    for metadata in world.volume.surfaces.values_mut() {
        if metadata.interior == Some(tunnel.destination_interior) {
            metadata.interior = Some(tunnel.unified_interior);
        }
    }
    for column in world.volume.columns.values_mut() {
        for element in &mut column.elements {
            let VolumeElement::Solid(mass) = element else {
                continue;
            };
            if mass.cutaway_for == Some(tunnel.destination_interior) {
                mass.cutaway_for = Some(tunnel.unified_interior);
            }
        }
    }
    Ok(())
}

fn carve_column(
    world: &mut GeneratedWorldPlan,
    tunnel: &PlannedMacroTunnel,
    coord: HexCoord,
    clear_top: Level,
    material: SolidMaterialRole,
) -> Result<(), MacroSpanningError> {
    let original =
        world.volume.columns.get(&coord).cloned().ok_or_else(|| {
            MacroSpanningError::new(format!("cannot carve missing column {coord:?}"))
        })?;
    let _floor_mass = solid_mass_at(&original, tunnel.floor_level).ok_or_else(|| {
        MacroSpanningError::new(format!("cannot carve unsupported floor at {coord:?}"))
    })?;
    let roof_end = clear_top
        .checked_add(i32::try_from(tunnel.roof_thickness).map_err(|error| {
            MacroSpanningError::new(format!("roof thickness overflowed: {error}"))
        })?)
        .ok_or_else(|| MacroSpanningError::new("tunnel roof level overflowed"))?;
    let mut elements = Vec::new();
    for element in original.elements {
        let VolumeElement::Solid(mass) = element else {
            return Err(MacroSpanningError::new(format!(
                "tunnel column {coord:?} contains a fill run after preflight"
            )));
        };
        let below_top = mass.levels.top.min(tunnel.floor_level);
        if mass.levels.bottom < below_top {
            elements.push(VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(mass.levels.bottom, below_top),
                material: mass.material,
                cutaway_for: mass.cutaway_for,
            }));
        }
        let above_bottom = mass.levels.bottom.max(roof_end);
        if above_bottom >= mass.levels.top {
            continue;
        }
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(above_bottom, mass.levels.top),
            material: mass.material,
            cutaway_for: Some(tunnel.unified_interior),
        }));
    }
    elements.push(VolumeElement::Solid(SolidMass {
        levels: LevelInterval::new(clear_top, roof_end),
        material,
        cutaway_for: Some(tunnel.unified_interior),
    }));
    elements.push(VolumeElement::Solid(SolidMass {
        levels: LevelInterval::new(tunnel.floor_level, tunnel.floor_level.saturating_add(1)),
        material,
        cutaway_for: None,
    }));
    elements.sort_unstable_by_key(element_interval);
    let elements = coalesce_elements(elements);
    replace_column_and_surfaces(world, coord, elements, tunnel)
}

fn element_interval(element: &VolumeElement) -> (Level, Level) {
    match element {
        VolumeElement::Solid(mass) => (mass.levels.bottom, mass.levels.top),
        VolumeElement::Fill(fill) => (fill.levels.bottom, fill.levels.top),
    }
}

fn column_occupied_at(column: &VolumeColumn, level: Level) -> bool {
    column.elements.iter().any(|element| {
        let (bottom, top) = element_interval(element);
        bottom <= level && level < top
    })
}

fn coalesce_elements(elements: Vec<VolumeElement>) -> Vec<VolumeElement> {
    let mut output = Vec::new();
    for element in elements {
        let merged = match (output.last_mut(), &element) {
            (Some(VolumeElement::Solid(before)), VolumeElement::Solid(after))
                if before.levels.top == after.levels.bottom
                    && before.material == after.material
                    && before.cutaway_for == after.cutaway_for =>
            {
                before.levels.top = after.levels.top;
                true
            }
            (Some(VolumeElement::Fill(before)), VolumeElement::Fill(after))
                if before.levels.top == after.levels.bottom
                    && before.material == after.material =>
            {
                before.levels.top = after.levels.top;
                true
            }
            _ => false,
        };
        if !merged {
            output.push(element);
        }
    }
    output
}

fn replace_column_and_surfaces(
    world: &mut GeneratedWorldPlan,
    coord: HexCoord,
    elements: Vec<VolumeElement>,
    tunnel: &PlannedMacroTunnel,
) -> Result<(), MacroSpanningError> {
    let old_surfaces = world
        .volume
        .surfaces
        .iter()
        .filter(|(position, _)| position.coord == coord)
        .map(|(position, metadata)| (*position, *metadata))
        .collect::<BTreeMap<_, _>>();
    let old_biomes = old_surfaces
        .keys()
        .filter_map(|position| {
            world
                .biome_regions
                .get(position)
                .copied()
                .map(|region| (*position, region))
        })
        .collect::<BTreeMap<_, _>>();
    world
        .volume
        .surfaces
        .retain(|position, _| position.coord != coord);
    world
        .biome_regions
        .retain(|position, _| position.coord != coord);
    world
        .volume
        .columns
        .insert(coord, VolumeColumn { elements });

    let column = world
        .volume
        .columns
        .get(&coord)
        .ok_or_else(|| MacroSpanningError::new("carved column was not stored"))?;
    let exposed = exposed_solid_tops(coord, column);
    let owner_region = world
        .layout
        .patches
        .values()
        .find(|patch| patch.mask.contains(&coord))
        .map(|patch| patch.biome_region)
        .ok_or_else(|| MacroSpanningError::new(format!("carved column {coord:?} has no owner")))?;
    let authored_mouth_roof = if tunnel.mouth.contains(&coord) {
        let clear_top = clearance_top(tunnel, coord)?;
        let roof_top = clear_top
            .checked_add(i32::try_from(tunnel.roof_thickness).map_err(|error| {
                MacroSpanningError::new(format!("roof thickness overflowed: {error}"))
            })?)
            .and_then(|level| level.checked_sub(1))
            .ok_or_else(|| MacroSpanningError::new("tunnel roof surface level overflowed"))?;
        Some(TilePos::new(coord, roof_top))
    } else {
        None
    };
    for position in exposed {
        let metadata = if position.level == tunnel.floor_level {
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: (!tunnel.exterior_apron.contains(&coord))
                    .then_some(tunnel.unified_interior),
            }
        } else if Some(position) == authored_mouth_roof {
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            }
        } else {
            old_surfaces.get(&position).copied().ok_or_else(|| {
                MacroSpanningError::new(format!(
                    "carving {coord:?} unexpectedly exposed new non-floor surface {position:?}"
                ))
            })?
        };
        let biome = old_biomes.get(&position).copied().unwrap_or(owner_region);
        world.volume.surfaces.insert(position, metadata);
        world.biome_regions.insert(position, biome);
    }
    Ok(())
}

fn exposed_solid_tops(coord: HexCoord, column: &VolumeColumn) -> Vec<TilePos> {
    let mut positions = Vec::new();
    for (index, element) in column.elements.iter().enumerate() {
        let VolumeElement::Solid(mass) = *element else {
            continue;
        };
        let covered = column
            .elements
            .get(index.saturating_add(1))
            .is_some_and(|next| {
                matches!(next, VolumeElement::Solid(_))
                    && element_interval(next).0 == mass.levels.top
            });
        if !covered && mass.levels.bottom < mass.levels.top {
            positions.push(TilePos::new(coord, mass.levels.top.saturating_sub(1)));
        }
    }
    positions
}

fn publish_tunnel_lights(
    world: &mut GeneratedWorldPlan,
    tunnel: &PlannedMacroTunnel,
    ordinal: usize,
) -> Result<(), MacroSpanningError> {
    let ordinal = u32::try_from(ordinal)
        .map_err(|error| MacroSpanningError::new(format!("tunnel ordinal overflowed: {error}")))?;
    let base = LIGHT_LOCAL_BASE
        .checked_add(ordinal.saturating_mul(LIGHTS_PER_TUNNEL))
        .ok_or_else(|| MacroSpanningError::new("tunnel light namespace overflowed"))?;
    for (index, site) in tunnel.light_sites.iter().enumerate() {
        let index = u32::try_from(index)
            .map_err(|error| MacroSpanningError::new(format!("light index overflowed: {error}")))?;
        let bright_id = LightId(world_owned_id(
            base.checked_add(index.saturating_mul(2))
                .ok_or_else(|| MacroSpanningError::new("bright light ID overflowed"))?,
        )?);
        let dim_id = LightId(world_owned_id(
            base.checked_add(index.saturating_mul(2).saturating_add(1))
                .ok_or_else(|| MacroSpanningError::new("dim light ID overflowed"))?,
        )?);
        let bright = PlannedGameplayLight {
            origin: site.position,
            level: IlluminationLevel::Bright,
            radius: BRIGHT_LIGHT_RADIUS,
            presentation: Some(PlannedLightPresentation::CaveCrystal(
                CaveCrystalPresentation {
                    kind: site.kind,
                    site: CaveCrystalSiteKind::InteriorAlcove,
                    rotation: site.rotation,
                },
            )),
        };
        let dim = PlannedGameplayLight {
            origin: site.position,
            level: IlluminationLevel::Dim,
            radius: DIM_LIGHT_RADIUS,
            presentation: None,
        };
        if world.lights.insert(bright_id, bright).is_some()
            || world.lights.insert(dim_id, dim).is_some()
        {
            return Err(MacroSpanningError::new(
                "world-owned tunnel light ID collided during publication",
            ));
        }
    }
    Ok(())
}

fn publish_tunnel_route(
    world: &mut GeneratedWorldPlan,
    tunnel: &PlannedMacroTunnel,
) -> Result<(), MacroSpanningError> {
    let surfaces = tunnel
        .ribbon
        .union(&tunnel.mouth)
        .copied()
        .map(|coord| TilePos::new(coord, tunnel.floor_level))
        .collect::<BTreeSet<_>>();
    let centerline = tunnel
        .centerline
        .iter()
        .copied()
        .map(|coord| TilePos::new(coord, tunnel.floor_level))
        .collect::<Vec<_>>();
    if surfaces.iter().any(|position| {
        !matches!(
            world
                .volume
                .surfaces
                .get(position)
                .map(|metadata| metadata.access),
            Some(SurfaceAccess::Ordinary)
        )
    }) {
        return Err(MacroSpanningError::new(
            "tunnel protected route contains a non-ordinary floor",
        ));
    }
    if world
        .features
        .protected_routes
        .insert(
            tunnel.name.clone(),
            ProtectedFeatureRoute {
                centerline,
                surfaces,
            },
        )
        .is_some()
    {
        return Err(MacroSpanningError::new(
            "tunnel protected route collided during publication",
        ));
    }
    Ok(())
}

fn rebuild_unified_interior(
    world: &mut GeneratedWorldPlan,
    tunnel: &PlannedMacroTunnel,
) -> Result<(), MacroSpanningError> {
    let floors = world
        .volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.interior == Some(tunnel.unified_interior)).then_some(*position)
        })
        .collect::<BTreeSet<_>>();
    let roof_voxels = world
        .volume
        .columns
        .iter()
        .flat_map(|(coord, column)| {
            column.elements.iter().flat_map(move |element| {
                let VolumeElement::Solid(mass) = *element else {
                    return Vec::new().into_iter();
                };
                if mass.cutaway_for != Some(tunnel.unified_interior) {
                    return Vec::new().into_iter();
                }
                (mass.levels.bottom..mass.levels.top)
                    .map(|level| TilePos::new(*coord, level))
                    .collect::<Vec<_>>()
                    .into_iter()
            })
        })
        .collect::<BTreeSet<_>>();
    let entrances = tunnel
        .foot_threshold
        .union(&tunnel.summit_threshold)
        .copied()
        .collect::<BTreeSet<_>>();
    if !entrances.is_subset(&floors) || entrances.len() != 8 {
        return Err(MacroSpanningError::new(
            "unified tunnel interior does not have exactly its two four-wide thresholds",
        ));
    }
    if world
        .interiors
        .by_id
        .insert(
            tunnel.unified_interior,
            PlannedInterior {
                floors,
                entrances,
                roof_voxels,
            },
        )
        .is_some()
    {
        return Err(MacroSpanningError::new(
            "unified tunnel interior collided during publication",
        ));
    }
    Ok(())
}

fn publish_tunnel_anchors(
    world: &mut GeneratedWorldPlan,
    tunnel: &PlannedMacroTunnel,
) -> Result<(), MacroSpanningError> {
    for (name, position) in [
        (FOOT_APRON_ANCHOR, tunnel.foot_apron_anchor),
        (TUNNEL_MOUTH_ANCHOR, tunnel.tunnel_mouth_anchor),
        (TUNNEL_MIDPOINT_ANCHOR, tunnel.midpoint_anchor),
        (GOTHIC_TRANSITION_ANCHOR, tunnel.gothic_transition_anchor),
        (ASCENT_THRESHOLD_ANCHOR, tunnel.destination_anchor),
    ] {
        match world.anchors.get(name).copied() {
            Some(existing) if existing == position => {}
            Some(existing) => {
                return Err(MacroSpanningError::new(format!(
                    "stable anchor {name:?} conflicts at {existing:?} versus {position:?}"
                )));
            }
            None => {
                world.anchors.insert(name.to_owned(), position);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::{BiomeRegionId, MapViewHint};

    use super::super::layout::{
        resolve_layout, resolve_macro_contracts, ResolvedEdgeReference,
        ResolvedMacroAnchorReference, ResolvedMacroBoundaryTerminal, ResolvedMacroSubsurfaceSeam,
        ResolvedPatch, ResolvedPort,
    };
    use super::super::liquid::LiquidPlan;
    use super::super::volume::{SolidMaterialRole, VolumePlan};
    use super::super::world::{FeaturePlan, InteriorPlan, StructurePlan};
    use crate::settings::{MapSettings, ProceduralSettings, TerrainSettings, V3LayoutSettings};

    const OLD_LOCAL_INTERIOR: InteriorRegionId = InteriorRegionId(40);

    fn coord(x: i32, y: i32) -> HexCoord {
        HexCoord::from_axial(x, y)
    }

    fn rectangular_mask(min_x: i32, max_x: i32) -> BTreeSet<HexCoord> {
        (min_x..=max_x)
            .flat_map(|x| (-4..=4).map(move |y| coord(x, y)))
            .collect()
    }

    fn fixture_layout() -> ResolvedLayoutPlan {
        let masks = [
            rectangular_mask(-20, -3),
            rectangular_mask(-2, 1),
            rectangular_mask(2, 8),
        ];
        let footprint = masks.iter().flatten().copied().collect::<BTreeSet<_>>();
        let patches = masks
            .into_iter()
            .enumerate()
            .map(|(index, mask)| {
                let id = PatchId(u32::try_from(index).unwrap_or(u32::MAX));
                (
                    id,
                    ResolvedPatch {
                        biome_region: BiomeRegionId(id.0),
                        rotation_turns: 0,
                        mask,
                        edges: HexSide::ALL
                            .into_iter()
                            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
                            .collect(),
                    },
                )
            })
            .collect();
        ResolvedLayoutPlan {
            kind: LayoutKind::Macro,
            grid_radius: 77,
            footprint,
            patches,
            shared_edges: BTreeMap::new(),
            boundary_liquid_outlets: BTreeMap::new(),
        }
    }

    fn lanes(source_x: i32, destination_x: i32) -> BTreeSet<(HexCoord, HexCoord)> {
        (-1..=2)
            .map(|y| (coord(source_x, y), coord(destination_x, y)))
            .collect()
    }

    fn empty_port(lanes: BTreeSet<(HexCoord, HexCoord)>) -> ResolvedPort {
        ResolvedPort {
            lanes,
            first_approach: BTreeSet::new(),
            second_approach: BTreeSet::new(),
        }
    }

    fn fixture_contracts() -> ResolvedMacroContracts {
        let tunnel = ResolvedMacroTunnel {
            name: "crystal_mountain.tunnel".to_owned(),
            canonical_route: true,
            instance_route: vec![PatchId(0), PatchId(1), PatchId(2)],
            boundary_terminal: ResolvedMacroBoundaryTerminal {
                instance: PatchId(0),
                side: HexSide::West,
                lanes: lanes(-20, -21),
                inward_approach: BTreeSet::new(),
            },
            destination_anchor: ResolvedMacroAnchorReference {
                instance: PatchId(2),
                anchor: "crystal_ascent.lower_entry".to_owned(),
            },
            floor_level: 6,
            width: 4,
            clearance: 6,
            roof_thickness: 3,
            seams: vec![
                ResolvedMacroSubsurfaceSeam {
                    source: PatchId(0),
                    destination: PatchId(1),
                    port: empty_port(lanes(-3, -2)),
                },
                ResolvedMacroSubsurfaceSeam {
                    source: PatchId(1),
                    destination: PatchId(2),
                    port: empty_port(lanes(1, 2)),
                },
            ],
        };
        ResolvedMacroContracts {
            walker_connections: Vec::new(),
            spanning_features: BTreeMap::from([(
                tunnel.name.clone(),
                ResolvedMacroSpanningFeature::Tunnel(tunnel),
            )]),
            anchor_aliases: BTreeMap::new(),
        }
    }

    fn fixture_destination() -> RawSpanningDestination {
        let terminal = (-1..=2)
            .map(|y| TilePos::new(coord(7, y), 6))
            .collect::<BTreeSet<_>>();
        let summit_threshold = (-1..=2).map(|y| TilePos::new(coord(8, y), 20)).collect();
        RawSpanningDestination {
            anchor: TilePos::new(coord(7, 0), 6),
            terminal,
            interior: Some(OLD_LOCAL_INTERIOR),
            summit_threshold,
        }
    }

    fn fixture_plan() -> PlannedMacroSpanning {
        let destinations = BTreeMap::from([(
            (PatchId(2), "crystal_ascent.lower_entry".to_owned()),
            fixture_destination(),
        )]);
        plan_macro_spanning(&fixture_layout(), &fixture_contracts(), &destinations)
            .expect("straight three-instance tunnel should plan")
    }

    fn fixture_world() -> GeneratedWorldPlan {
        let layout = fixture_layout();
        let mut volume = VolumePlan::new(layout.footprint.clone());
        let terminal = fixture_destination().terminal;
        let summit = fixture_destination().summit_threshold;
        let chamber = TilePos::new(coord(8, 3), 6);
        for coord in &layout.footprint {
            let terminal_position = TilePos::new(*coord, 6);
            let summit_position = TilePos::new(*coord, 20);
            let (top, material, interior) = if terminal.contains(&terminal_position) {
                (7, SolidMaterialRole::WorkedStone, None)
            } else if summit.contains(&summit_position) {
                (21, SolidMaterialRole::Grass, None)
            } else if chamber.coord == *coord {
                (
                    7,
                    SolidMaterialRole::WorkedStone,
                    Some(namespaced_old_interior()),
                )
            } else {
                (31, SolidMaterialRole::Stone, None)
            };
            volume
                .columns
                .get_mut(coord)
                .expect("fixture mask has every column")
                .elements = vec![
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(0, 1),
                    material: SolidMaterialRole::Bedrock,
                    cutaway_for: None,
                }),
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(1, top),
                    material,
                    cutaway_for: None,
                }),
            ];
            volume.surfaces.insert(
                TilePos::new(*coord, top - 1),
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior,
                },
            );
        }
        let biome_regions = volume
            .surfaces
            .keys()
            .copied()
            .map(|surface| {
                let region = layout
                    .patches
                    .values()
                    .find(|patch| patch.mask.contains(&surface.coord))
                    .expect("fixture surface has one patch")
                    .biome_region;
                (surface, region)
            })
            .collect();
        GeneratedWorldPlan {
            source_schematic_fingerprint: None,
            layout,
            volume,
            liquids: LiquidPlan::default(),
            features: FeaturePlan::default(),
            structures: StructurePlan::default(),
            blockers: BTreeSet::new(),
            lights: BTreeMap::new(),
            biome_regions,
            interiors: InteriorPlan {
                by_id: BTreeMap::from([(
                    namespaced_old_interior(),
                    PlannedInterior {
                        floors: BTreeSet::from([chamber]),
                        entrances: BTreeSet::from([chamber]),
                        roof_voxels: BTreeSet::new(),
                    },
                )]),
            },
            anchors: BTreeMap::from([(
                ASCENT_THRESHOLD_ANCHOR.to_owned(),
                fixture_destination().anchor,
            )]),
            observation_anchors: BTreeMap::new(),
            view_hint: MapViewHint::new((1.0, 4.0, 2.0), (0.0, 0.0, 0.0)),
        }
    }

    fn namespaced_old_interior() -> InteriorRegionId {
        namespace_patch_local_interior(PatchId(2), OLD_LOCAL_INTERIOR)
            .expect("fixture interior fits")
    }

    fn authored_ring(radius: u32) -> Vec<HexCoord> {
        if radius == 0 {
            return vec![HexCoord::ORIGIN];
        }
        let directions = [
            coord(1, -1),
            coord(1, 0),
            coord(0, 1),
            coord(-1, 1),
            coord(-1, 0),
            coord(0, -1),
        ];
        let radius_i32 = i32::try_from(radius).expect("authored radius fits i32");
        let mut current = HexCoord::new_cubic(-radius_i32, 0, radius_i32);
        let mut ring = Vec::new();
        for direction in directions {
            for _ in 0..radius {
                ring.push(current);
                let [x, y, z] = current.to_cubic_array();
                let [dx, dy, dz] = direction.to_cubic_array();
                current = HexCoord::new_cubic(x + dx, y + dy, z + dz);
            }
        }
        ring
    }

    fn authored_radial_pad(
        radius: u32,
        side: usize,
        width: usize,
        level: Level,
    ) -> BTreeSet<TilePos> {
        let ring = authored_ring(radius);
        let center = (usize::try_from(radius / 2).expect("radius fits usize")
            + side.saturating_mul(usize::try_from(radius).expect("radius fits usize")))
            % ring.len();
        let before = width / 2;
        (0..width)
            .filter_map(|offset| {
                let raw = (center + ring.len() + offset).saturating_sub(before) % ring.len();
                ring.get(raw).copied()
            })
            .map(|coord| TilePos::new(coord, level))
            .collect()
    }

    #[test]
    fn deterministic_bundle_uses_all_four_exact_seam_lanes() {
        let first = fixture_plan();
        let second = fixture_plan();
        assert_eq!(first, second);
        let tunnel = first
            .tunnels
            .get("crystal_mountain.tunnel")
            .expect("fixture tunnel is named");
        assert_eq!(tunnel.lanes.len(), 4);
        assert!(tunnel.lanes.iter().all(|lane| path_turns(lane) == 0));
        for seam in &tunnel.seam_lanes {
            assert_eq!(seam.len(), 4);
            for crossing in seam {
                assert!(tunnel.lanes.iter().any(|lane| {
                    lane.windows(2)
                        .any(|window| window == [crossing.0, crossing.1])
                }));
            }
        }
        for (patch, reservation) in &first.reservations_by_patch {
            let layout = fixture_layout();
            let mask = &layout
                .patches
                .get(patch)
                .expect("reservation patch exists")
                .mask;
            assert!(reservation.is_subset(mask));
        }
        assert_eq!(tunnel.exterior_apron.len(), 8);
        assert_eq!(tunnel.foot_threshold.len(), 4);
        assert!(tunnel.lanes.iter().all(|lane| lane
            .iter()
            .take(EXTERIOR_MOUTH_ROUTE_ROWS)
            .all(|coord| tunnel.mouth.contains(coord))));
    }

    #[test]
    fn boundary_lane_identity_is_stable_under_global_rotation() {
        let west_inside = [coord(-77, 0), coord(-77, 1), coord(-76, -1), coord(-75, -2)];
        let west = ResolvedMacroBoundaryTerminal {
            instance: PatchId(0),
            side: HexSide::West,
            lanes: west_inside
                .into_iter()
                .map(|inside| (inside, HexSide::West.neighbor(inside)))
                .collect(),
            inward_approach: BTreeSet::new(),
        };
        let expected = ordered_boundary_starts(&west);

        let rotated = ResolvedMacroBoundaryTerminal {
            instance: PatchId(0),
            side: HexSide::SouthWest,
            lanes: west
                .lanes
                .iter()
                .map(|(inside, outside)| (rotate_coord(*inside, 1), rotate_coord(*outside, 1)))
                .collect(),
            inward_approach: BTreeSet::new(),
        };
        assert_eq!(
            ordered_boundary_starts(&rotated),
            expected
                .into_iter()
                .map(|inside| rotate_coord(inside, 1))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn shipped_crystal_mountain_resolves_one_complete_reserved_tunnel() {
        let map: MapSettings = ron::from_str(include_str!(
            "../../../../assets/config/worlds/procedural-crystal-mountain.ron"
        ))
        .expect("shipped Crystal Mountain settings parse");
        let TerrainSettings::Procedural(ProceduralSettings::V3(settings)) = &map.terrain else {
            panic!("Crystal Mountain remains V3");
        };
        let V3LayoutSettings::Macro(macro_settings) = &settings.layout else {
            panic!("Crystal Mountain remains Macro");
        };
        let layout = resolve_layout(map.grid_radius, settings)
            .expect("shipped Crystal Mountain masks resolve");
        let contracts = resolve_macro_contracts(macro_settings, &layout)
            .expect("shipped Crystal Mountain contracts resolve");
        let lower = authored_radial_pad(32, 0, 4, 6);
        let upper = authored_radial_pad(31, 3, 4, 150);
        let destination = RawSpanningDestination {
            anchor: *lower.iter().nth(1).expect("lower pad has four surfaces"),
            terminal: lower,
            interior: Some(OLD_LOCAL_INTERIOR),
            summit_threshold: upper,
        };
        let raw = BTreeMap::from([(
            (PatchId(0), "crystal_ascent.lower_entry".to_owned()),
            destination,
        )]);
        let planned = plan_macro_spanning(&layout, &contracts, &raw)
            .expect("shipped Crystal Mountain tunnel plans");
        let tunnel = planned
            .tunnels
            .get("crystal_mountain.tunnel")
            .expect("one stable tunnel exists");
        assert_eq!(tunnel.lanes.len(), 4);
        assert_eq!(tunnel.seam_lanes.len(), 2);
        assert_eq!(tunnel.exterior_apron.len(), 8);
        assert_eq!(tunnel.foot_threshold.len(), 4);
        assert!(tunnel.ribbon.len() > 100);
        assert_eq!(
            tunnel.light_sites.len(),
            13,
            "the shipped roofed body should retain thirteen physical light pools"
        );
        assert_eq!(
            tunnel.gothic.len(),
            GOTHIC_ROW_COUNT * 4,
            "the final twelve rows must remain worked across all four lanes"
        );
        for lane in &tunnel.lanes {
            let mut transition = lane.iter().rev().copied().filter(|coord| {
                !tunnel
                    .destination_terminal
                    .iter()
                    .any(|surface| surface.coord == *coord)
                    && !tunnel.mouth.contains(coord)
            });
            assert!(
                transition
                    .by_ref()
                    .take(GOTHIC_ROW_COUNT)
                    .all(|coord| tunnel.gothic.contains(&coord)),
                "each lane must contribute its final twelve rows to the Gothic transition"
            );
            let preceding = transition
                .next()
                .expect("the shipped tunnel must retain a rough row before the transition");
            assert!(
                !tunnel.gothic.contains(&preceding),
                "the row immediately before the Gothic transition must remain rough"
            );
        }
        assert!(planned.reservations_by_patch.keys().copied().eq([
            PatchId(0),
            PatchId(2),
            PatchId(3)
        ]));
    }

    #[test]
    fn routing_prefers_fewer_turns_after_shortest_length() {
        let allowed = (-3..=3)
            .flat_map(|x| (-3..=3).map(move |y| coord(x, y)))
            .collect::<BTreeSet<_>>();
        let path =
            shortest_then_fewest_turns(&allowed, &BTreeSet::new(), coord(-3, 0), coord(3, 0))
                .expect("straight path resolves");
        assert_eq!(path_turns(&path), 0);
        assert_eq!(path.len(), 7);
    }

    #[test]
    fn physical_light_sampling_bounds_requested_centerline_gaps() {
        assert!(light_sample_indices(0).is_empty());
        assert_eq!(light_sample_indices(1), vec![0]);

        let indices = light_sample_indices(63);
        assert_eq!(indices.len(), 16);
        assert_eq!(indices.first().copied(), Some(LIGHT_SPACING_STEPS / 2));
        assert_eq!(indices.last().copied(), Some(62 - LIGHT_SPACING_STEPS / 2));
        assert!(indices.windows(2).all(|window| {
            matches!(window, [first, second] if second.saturating_sub(*first) <= LIGHT_SPACING_STEPS)
        }));
    }

    #[test]
    fn apply_carves_exact_clearance_and_roofs_the_exterior_mouth() {
        let plan = fixture_plan();
        let tunnel = plan.tunnels.values().next().expect("one tunnel").clone();
        let mut original = fixture_world();
        let original_terminal_columns = tunnel
            .destination_terminal
            .iter()
            .map(|position| {
                (
                    position.coord,
                    original
                        .volume
                        .columns
                        .get(&position.coord)
                        .expect("authored destination column exists")
                        .clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        for apron in &tunnel.exterior_apron {
            original
                .volume
                .columns
                .get_mut(apron)
                .expect("low-slope exterior column exists")
                .elements = vec![
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(0, 1),
                    material: SolidMaterialRole::Bedrock,
                    cutaway_for: None,
                }),
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(1, 7),
                    material: SolidMaterialRole::Stone,
                    cutaway_for: None,
                }),
            ];
            original
                .volume
                .surfaces
                .retain(|position, _| position.coord != *apron);
            original
                .biome_regions
                .retain(|position, _| position.coord != *apron);
            let floor = TilePos::new(*apron, tunnel.floor_level);
            original.volume.surfaces.insert(
                floor,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
            let region = original
                .layout
                .patches
                .values()
                .find(|patch| patch.mask.contains(apron))
                .expect("exterior mouth column has one owner")
                .biome_region;
            original.biome_regions.insert(floor, region);
        }
        assert!(original.volume.validate().is_ok());
        let world = apply_macro_spanning(original, &plan)
            .expect("valid planned tunnel should apply atomically");
        let core = tunnel
            .centerline
            .iter()
            .copied()
            .find(|coord| {
                !tunnel.mouth.contains(coord)
                    && !tunnel
                        .destination_terminal
                        .iter()
                        .any(|position| position.coord == *coord)
            })
            .expect("fixture exposes natural tunnel core");
        let column = world
            .volume
            .columns
            .get(&core)
            .expect("carved core column exists");
        assert!(solid_mass_at(column, 6).is_some());
        assert!((7..13).all(|level| solid_mass_at(column, level).is_none()));
        assert!((13..16).all(|level| {
            solid_mass_at(column, level)
                .is_some_and(|mass| mass.cutaway_for == Some(tunnel.unified_interior))
        }));

        let apron = *tunnel.exterior_apron.first().expect("open apron exists");
        let apron_column = world
            .volume
            .columns
            .get(&apron)
            .expect("exterior mouth column exists");
        assert!(solid_mass_at(apron_column, 6).is_some());
        assert!((7..19).all(|level| !column_occupied_at(apron_column, level)));
        assert!((19..22).all(|level| {
            solid_mass_at(apron_column, level).is_some_and(|mass| {
                mass.cutaway_for == Some(tunnel.unified_interior)
                    && mass.material == SolidMaterialRole::Stone
            })
        }));
        for apron in &tunnel.exterior_apron {
            let floor = TilePos::new(*apron, tunnel.floor_level);
            assert_eq!(
                world
                    .volume
                    .surfaces
                    .get(&floor)
                    .expect("every exterior mouth floor remains exposed")
                    .interior,
                None
            );
        }
        assert!(tunnel
            .mouth
            .difference(&tunnel.exterior_apron)
            .all(|coord| {
                world
                    .volume
                    .surfaces
                    .get(&TilePos::new(*coord, tunnel.floor_level))
                    .is_some_and(|metadata| metadata.interior == Some(tunnel.unified_interior))
            }));
        assert_eq!(tunnel.destination_terminal.len(), 4);
        for (coord, original_column) in original_terminal_columns {
            assert_eq!(
                world
                    .volume
                    .columns
                    .get(&coord)
                    .expect("authored destination column remains present"),
                &original_column,
                "only the exact authored destination terminal keeps its taller aperture"
            );
        }
        assert!(world.volume.validate().is_ok());
    }

    #[test]
    fn naturally_clear_nonterminal_column_cannot_bypass_exact_roof_preflight() {
        let plan = fixture_plan();
        let tunnel = plan.tunnels.values().next().expect("one tunnel");
        let core = tunnel
            .centerline
            .iter()
            .copied()
            .find(|coord| {
                !tunnel.mouth.contains(coord) && !is_destination_terminal_coord(tunnel, *coord)
            })
            .expect("fixture exposes a nonterminal tunnel core");
        let mut world = fixture_world();
        world
            .volume
            .columns
            .get_mut(&core)
            .expect("core column exists")
            .elements = vec![
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(0, 1),
                material: SolidMaterialRole::Bedrock,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(1, 7),
                material: SolidMaterialRole::Stone,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(20, 31),
                material: SolidMaterialRole::Stone,
                cutaway_for: None,
            }),
        ];
        let floor = TilePos::new(core, 6);
        world.volume.surfaces.insert(
            floor,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );
        let region = world
            .layout
            .patches
            .values()
            .find(|patch| patch.mask.contains(&core))
            .expect("core has one owner")
            .biome_region;
        world.biome_regions.insert(floor, region);

        let error = apply_macro_spanning(world, &plan)
            .expect_err("a naturally open body column without the exact roof must fail closed");
        assert!(error.to_string().contains("lacks solid roof at level 13"));
    }

    #[test]
    fn authored_bedrock_in_the_overwritten_tunnel_volume_fails_before_carving() {
        let plan = fixture_plan();
        let tunnel = plan.tunnels.values().next().expect("one tunnel");
        let core = tunnel
            .centerline
            .iter()
            .copied()
            .find(|coord| {
                !tunnel.mouth.contains(coord) && !is_destination_terminal_coord(tunnel, *coord)
            })
            .expect("fixture exposes a nonterminal tunnel core");
        let mut floor_world = fixture_world();
        let floor_mass = floor_world
            .volume
            .columns
            .get_mut(&core)
            .expect("core column exists")
            .elements
            .iter_mut()
            .find_map(|element| {
                let VolumeElement::Solid(mass) = element else {
                    return None;
                };
                (mass.levels.bottom <= tunnel.floor_level && tunnel.floor_level < mass.levels.top)
                    .then_some(mass)
            })
            .expect("core floor has one supporting mass");
        floor_mass.material = SolidMaterialRole::Bedrock;

        let error = apply_macro_spanning(floor_world, &plan)
            .expect_err("authored bedrock at the tunnel floor must fail closed");
        assert!(error.to_string().contains("breaches authored bedrock"));

        let mut clearance_world = fixture_world();
        clearance_world
            .volume
            .columns
            .get_mut(&core)
            .expect("core column exists")
            .elements = vec![
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(0, 1),
                material: SolidMaterialRole::Bedrock,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(1, 14),
                material: SolidMaterialRole::Stone,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(14, 15),
                material: SolidMaterialRole::Bedrock,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(15, 31),
                material: SolidMaterialRole::Stone,
                cutaway_for: None,
            }),
        ];
        let error = apply_macro_spanning(clearance_world, &plan)
            .expect_err("authored bedrock in the synthesized tunnel roof must fail closed");
        assert!(error.to_string().contains("overwrites authored bedrock"));
    }

    #[test]
    fn post_carve_validator_detects_corrupted_body_roof() {
        let plan = fixture_plan();
        let tunnel = plan.tunnels.values().next().expect("one tunnel");
        let mut world = apply_macro_spanning(fixture_world(), &plan)
            .expect("fixture tunnel applies before deliberate corruption");
        let core = tunnel
            .centerline
            .iter()
            .copied()
            .find(|coord| {
                !tunnel.mouth.contains(coord) && !is_destination_terminal_coord(tunnel, *coord)
            })
            .expect("fixture exposes a nonterminal tunnel core");
        let roof = world
            .volume
            .columns
            .get_mut(&core)
            .expect("carved core exists")
            .elements
            .iter_mut()
            .find_map(|element| {
                let VolumeElement::Solid(mass) = element else {
                    return None;
                };
                (mass.levels.bottom <= 13 && 13 < mass.levels.top).then_some(mass)
            })
            .expect("core has its exact roof");
        roof.cutaway_for = None;

        let error = validate_applied_tunnel_geometry(&world, tunnel)
            .expect_err("post-carve validation must reject corrupted roof semantics");
        assert!(error.to_string().contains("roof at level 13"));
    }

    #[test]
    fn apply_rewrites_one_interior_publishes_paired_lights_and_stable_anchors() {
        let plan = fixture_plan();
        let tunnel = plan.tunnels.values().next().expect("one tunnel").clone();
        let world = apply_macro_spanning(fixture_world(), &plan)
            .expect("valid planned tunnel should apply atomically");
        assert!(!world
            .interiors
            .by_id
            .contains_key(&tunnel.destination_interior));
        let interior = world
            .interiors
            .by_id
            .get(&tunnel.unified_interior)
            .expect("one unified interior is published");
        assert_eq!(interior.entrances.len(), 8);
        assert!(tunnel.foot_threshold.is_subset(&interior.entrances));
        assert!(tunnel.summit_threshold.is_subset(&interior.entrances));
        assert_eq!(world.lights.len(), tunnel.light_sites.len() * 2);
        for site in &tunnel.light_sites {
            let at_site = world
                .lights
                .values()
                .filter(|light| light.origin == site.position)
                .collect::<Vec<_>>();
            assert_eq!(at_site.len(), 2);
            assert!(at_site.iter().any(|light| {
                light.level == IlluminationLevel::Bright
                    && light.radius == 4
                    && light.presentation.is_some()
            }));
            assert!(at_site.iter().any(|light| {
                light.level == IlluminationLevel::Dim
                    && light.radius == 18
                    && light.presentation.is_none()
            }));
        }
        for name in [
            FOOT_APRON_ANCHOR,
            TUNNEL_MOUTH_ANCHOR,
            TUNNEL_MIDPOINT_ANCHOR,
            GOTHIC_TRANSITION_ANCHOR,
            ASCENT_THRESHOLD_ANCHOR,
        ] {
            assert!(world.anchors.contains_key(name), "missing {name}");
        }
        assert!(world
            .volume
            .surfaces
            .values()
            .all(|metadata| metadata.interior != Some(tunnel.destination_interior)));
        assert!(world.volume.columns.values().all(|column| {
            column.elements.iter().all(|element| {
                !matches!(
                    element,
                    VolumeElement::Solid(mass)
                        if mass.cutaway_for == Some(tunnel.destination_interior)
                )
            })
        }));
    }

    #[test]
    fn prefix_63_collision_fails_before_mutation() {
        let plan = fixture_plan();
        let mut world = fixture_world();
        world.lights.insert(
            LightId(WORLD_NAMESPACE_BASE | 99),
            PlannedGameplayLight {
                origin: fixture_destination().anchor,
                level: IlluminationLevel::Dim,
                radius: 1,
                presentation: None,
            },
        );
        let error = apply_macro_spanning(world, &plan)
            .expect_err("reserved namespace collision must fail closed");
        assert!(error.to_string().contains("namespace prefix 63"));
    }

    #[test]
    fn malformed_width_is_rejected_before_routing() {
        let layout = fixture_layout();
        let mut contracts = fixture_contracts();
        let ResolvedMacroSpanningFeature::Tunnel(tunnel) = contracts
            .spanning_features
            .get_mut("crystal_mountain.tunnel")
            .expect("fixture tunnel exists");
        tunnel.width = 3;
        let destinations = BTreeMap::from([(
            (PatchId(2), "crystal_ascent.lower_entry".to_owned()),
            fixture_destination(),
        )]);
        let error = plan_macro_spanning(&layout, &contracts, &destinations)
            .expect_err("non-four-wide tunnel must fail");
        assert!(error.to_string().contains("must be width 4"));
    }
}
