//! Authored traversal cuts through Grand V3's locked peak chains.
//!
//! These routes are deliberately resolved before the generic coarse-hub
//! network.  A connected natural peak field is much steeper than a walker can
//! cross, so its Ordinary schematic cells need one narrow, explicitly owned
//! ledge without granting the generic connector permission to flatten a ridge.

use super::*;

const INNER_CHAIN: [((i32, i32, i32), u16); 6] = [
    ((2, -5, 3), 88),
    ((1, -4, 3), 58),
    ((2, -4, 2), 59),
    ((2, -3, 1), 36),
    ((3, -3, 0), 19),
    ((4, -3, -1), 38),
];
const FROZEN_EXIT_OWNER: ((i32, i32, i32), u16) = ((3, -7, 4), 165);
const FROZEN_FIRST_WAYPOINT: ((i32, i32, i32), u16) = ((2, -6, 4), 123);
const FROZEN_PERIMETER_WAYPOINT: ((i32, i32, i32), u16) = ((3, -6, 3), 124);
const TUNNEL_UPPER_OWNER: ((i32, i32, i32), u16) = ((1, -5, 4), 87);
const INNER_CHAIN_ROUTE: [u16; 9] = [165, 124, 123, 88, 58, 59, 36, 19, 38];
const INNER_PEAK_TRANSIT_PATCH: PatchId = PatchId(59);
const INNER_PEAK_TRANSIT_INGRESS: (PatchId, PatchId) = (PatchId(58), PatchId(59));
const INNER_PEAK_TRANSIT_EGRESS: (PatchId, PatchId) = (PatchId(36), PatchId(59));
const MAXIMUM_LEDGE_LEVEL: Level = 239;
const MAXIMUM_PEAK_NEIGHBOR_STEP: Level = 9;
const INNER_PEAK_TRANSIT_HANDOFF_LIMIT: usize = 64;
const INNER_PEAK_TRANSIT_LOCAL_HANDOFF_LIMIT: usize = 32;
const INNER_PEAK_SUFFIX_STATE_LIMIT: usize = 500_000;

struct InnerPeakTransitSearchBudget {
    remaining_handoffs: usize,
    remaining_recovery_work: usize,
    saw_incomplete_search: bool,
}

impl InnerPeakTransitSearchBudget {
    fn new() -> Self {
        Self {
            remaining_handoffs: INNER_PEAK_TRANSIT_HANDOFF_LIMIT,
            remaining_recovery_work: EXACT_RECOVERY_WORK_BUDGET,
            saw_incomplete_search: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct BoundaryPortal {
    from: HexCoord,
    to: HexCoord,
}

/// Bounded route authority through the one broad peak cell whose two scenic
/// saddles need a physical climbing runway rather than interchangeable
/// boundary portals. The ingress and egress retain their authored direction;
/// `runway_domain` is one connected candidate domain admitted after every route
/// exclusion and shoulder bound. Only the solver-selected ordered segment is
/// retained as terrain mutation authority.
#[derive(Debug, Clone, PartialEq, Eq)]
struct InnerPeakTransitAuthority {
    runway_domain: BTreeSet<HexCoord>,
    ingress: BTreeSet<BoundaryPortal>,
    egress: BTreeSet<BoundaryPortal>,
    suffix_reachability: std::sync::Arc<InnerPeakSuffixReachability>,
    ordered_runway: Option<Vec<TilePos>>,
}

/// Complete reverse exact-state evidence for the authored suffix, or an
/// explicit refusal to use a partial index as proof that an ingress is dead.
#[derive(Debug, Clone, PartialEq, Eq)]
enum InnerPeakSuffixReachability {
    Complete {
        sequence: Vec<u16>,
        distance_by_entry: BTreeMap<TilePos, u32>,
        explored_states: usize,
        possible_states: usize,
        diagnostic: String,
    },
    Incomplete {
        sequence: Vec<u16>,
        possible_states: usize,
        state_limit: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct InnerPeakSuffixState {
    stage: usize,
    position: TilePos,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectedInnerPeakTransitAdmission {
    ingress: BoundaryPortal,
    runway: Vec<TilePos>,
    egress: BoundaryPortal,
}

impl SelectedInnerPeakTransitAdmission {
    fn is_retained_by(&self, route: &[TilePos]) -> bool {
        let retains_portal = |portal: BoundaryPortal| {
            route.windows(2).any(|pair| {
                let [first, second] = pair else {
                    return false;
                };
                first.coord == portal.from && second.coord == portal.to
            })
        };
        retains_portal(self.ingress)
            && retains_portal(self.egress)
            && route
                .windows(self.runway.len())
                .any(|window| window == self.runway)
    }
}

impl InnerPeakTransitAuthority {
    fn permits_portal(&self, from_id: u16, to_id: u16, portal: BoundaryPortal) -> bool {
        match (from_id, to_id) {
            (58, 59) => self.ingress.contains(&portal),
            (59, 36) => self.egress.contains(&portal),
            _ => true,
        }
    }

    fn validate_route(
        &self,
        route: &[TilePos],
        masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    ) -> Result<SelectedInnerPeakTransitAdmission, String> {
        let patch = masks
            .get(&INNER_PEAK_TRANSIT_PATCH)
            .ok_or_else(|| "inner peak transit contract lost Patch 59".to_owned())?;
        let runway = route
            .iter()
            .filter(|position| patch.contains(&position.coord))
            .copied()
            .collect::<Vec<_>>();
        let patch_route = runway
            .iter()
            .map(|position| position.coord)
            .collect::<BTreeSet<_>>();
        if patch_route.is_empty() || !patch_route.is_subset(&self.runway_domain) {
            return Err(format!(
                "inner peak route escaped its typed Patch-59 runway domain: route={}, domain={}",
                patch_route.len(),
                self.runway_domain.len()
            ));
        }
        let crossings = |from: PatchId, to: PatchId| {
            let from_mask = masks.get(&from);
            let to_mask = masks.get(&to);
            route
                .windows(2)
                .filter_map(|pair| {
                    let [first, second] = pair else {
                        return None;
                    };
                    (from_mask.is_some_and(|mask| mask.contains(&first.coord))
                        && to_mask.is_some_and(|mask| mask.contains(&second.coord)))
                    .then_some(BoundaryPortal {
                        from: first.coord,
                        to: second.coord,
                    })
                })
                .collect::<BTreeSet<_>>()
        };
        let ingress = crossings(INNER_PEAK_TRANSIT_INGRESS.0, INNER_PEAK_TRANSIT_INGRESS.1);
        let egress = crossings(INNER_PEAK_TRANSIT_EGRESS.1, INNER_PEAK_TRANSIT_EGRESS.0);
        if ingress.len() != 1
            || egress.len() != 1
            || !ingress.is_subset(&self.ingress)
            || !egress.is_subset(&self.egress)
        {
            return Err(format!(
                "inner peak route did not use one exact typed 58->59 ingress and 59->36 egress: ingress={ingress:?}, egress={egress:?}"
            ));
        }
        let ingress = ingress.first().copied().ok_or_else(|| {
            "inner peak route lost its selected 58->59 ingress after validation".to_owned()
        })?;
        let egress = egress.first().copied().ok_or_else(|| {
            "inner peak route lost its selected 59->36 egress after validation".to_owned()
        })?;
        if runway.first().map(|position| position.coord) != Some(ingress.to)
            || runway.last().map(|position| position.coord) != Some(egress.from)
        {
            return Err(
                "inner peak route's ordered Patch-59 runway does not span its selected portals"
                    .to_owned(),
            );
        }
        if self
            .ordered_runway
            .as_ref()
            .is_some_and(|expected| runway != *expected)
        {
            return Err(
                "inner peak route changed the exact ordered Patch-59 foundation spine or grade"
                    .to_owned(),
            );
        }
        Ok(SelectedInnerPeakTransitAdmission {
            ingress,
            runway,
            egress,
        })
    }
}

pub(super) struct InnerPeakLedgeCompilation {
    pub(super) route: ProtectedFeatureRoute,
    pub(super) side_routes: BTreeMap<String, ProtectedFeatureRoute>,
    pub(super) anchor: Option<TilePos>,
    /// Natural shoulder columns changed only to support the narrow one-level
    /// walker ledge. They are terrain authority, not additional walker lanes.
    pub(super) support_coords: BTreeSet<HexCoord>,
}

/// Exact read/write authorities needed to add the one internal peak ledge.
pub(super) struct InnerPeakLedgeContext<'a> {
    pub(super) plan: &'a SchematicPlanV1,
    pub(super) layout: &'a ResolvedLayoutPlan,
    pub(super) fine_index: &'a FineWorldIndex,
    pub(super) water_coords: &'a BTreeSet<HexCoord>,
    pub(super) existing_features: &'a FeaturePlan,
    pub(super) additional_routes: [&'a ProtectedFeatureRoute; 2],
    pub(super) structures: &'a StructurePlan,
    pub(super) blockers: &'a BTreeSet<TilePos>,
    pub(super) surface_route_exclusion: &'a BTreeSet<HexCoord>,
    pub(super) peak_ridges: &'a super::super::schematic_highlands::PeakRidgeAuthority,
    pub(super) bench_level: Level,
    pub(super) volume: &'a mut VolumePlan,
    pub(super) biome_regions: &'a mut BTreeMap<TilePos, BiomeRegionId>,
}

/// Carves one connected Upper-band ledge from Frozen Woods through the inner
/// six-peak chain.
///
/// The route begins on ordinary plateau footing immediately beside the
/// already-published `frozen_exit` in its outward Frozen owner 165, then
/// crosses Frozen shore 124 before retained Frozen cell 123. This order follows
/// the actual outward side of the protected summit-exit band; the nominal
/// 165/123 handoff is on its disconnected inner side. The radius-32 Crystal
/// claim removes the complete 123/87 boundary, so the route then uses the exact
/// 123/88 seam and healthy direct 88/58 saddle before reaching peaks
/// 59/36/19/38. The ledge
/// never consumes the level-6 tunnel as its seed, opens a lower-region portal,
/// or enters an immutable summit crown or waterfall-gorge surface.
pub(super) fn compile_inner_peak_ledge(
    context: InnerPeakLedgeContext<'_>,
) -> Result<InnerPeakLedgeCompilation, V3GenerationError> {
    let InnerPeakLedgeContext {
        plan,
        layout,
        fine_index,
        water_coords,
        existing_features,
        additional_routes,
        structures,
        blockers,
        surface_route_exclusion,
        peak_ridges,
        bench_level,
        volume,
        biome_regions,
    } = context;
    let (mut patch_masks, component) = inner_chain_authority(plan, layout, peak_ridges)?;
    let ordered_transit_spine = component
        .ordered_saddle_spines
        .get(&INNER_PEAK_TRANSIT_PATCH)
        .ok_or_else(|| {
            schematic_contract("inner peak ledge has no retained ordered Patch-59 foundation spine")
        })?;
    insert_route_support_mask(
        &mut patch_masks,
        plan,
        layout,
        FROZEN_EXIT_OWNER,
        LandformKind::Mountain,
        ClimateKind::Frozen,
        SchematicFeature::FrozenWoods,
        "outward Frozen-exit owner",
    )?;
    insert_route_support_mask(
        &mut patch_masks,
        plan,
        layout,
        FROZEN_FIRST_WAYPOINT,
        LandformKind::Mountain,
        ClimateKind::Frozen,
        SchematicFeature::FrozenWoods,
        "retained Frozen first waypoint",
    )?;
    insert_route_support_mask(
        &mut patch_masks,
        plan,
        layout,
        FROZEN_PERIMETER_WAYPOINT,
        LandformKind::Shore,
        ClimateKind::Frozen,
        SchematicFeature::FrozenWoods,
        "Frozen-shore perimeter waypoint",
    )?;
    let frozen_exit = existing_features
        .protected_routes
        .get("grand_v3.frozen_exit")
        .ok_or_else(|| schematic_contract("inner peak ledge has no published Frozen-exit route"))?;
    validate_protected_route_integrity("preexisting Frozen exit", frozen_exit, volume)?;
    if !grand_v3_structural_review_draft_enabled() {
        for (name, existing) in &existing_features.protected_routes {
            validate_protected_route_integrity(
                &format!("{name} before inner peak ledge construction"),
                existing,
                volume,
            )?;
        }
        for (index, existing) in additional_routes.iter().enumerate() {
            validate_protected_route_integrity(
                &format!("preexisting peak route {index} before inner ledge construction"),
                existing,
                volume,
            )?;
        }
    }
    let frozen_route_root = frozen_exit
        .centerline
        .first()
        .copied()
        .ok_or_else(|| schematic_contract("published Frozen exit has no exact route root"))?;
    if OrdinaryRegionBand::containing(frozen_route_root.level) != OrdinaryRegionBand::Upper
        || !ordinary_surface_is_node(volume, Some(blockers), frozen_route_root)
    {
        return Err(schematic_contract(format!(
            "published Frozen exit has no exact Upper-band root: {frozen_route_root:?}"
        )));
    }

    let high_band = component
        .expected_high_band
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    let summit_pins = component
        .summit_pins
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    let protected_coords = existing_features
        .protected_routes
        .values()
        .flat_map(|route| route.surfaces.iter().map(|surface| surface.coord))
        .chain(
            additional_routes
                .iter()
                .flat_map(|route| route.surfaces.iter().map(|surface| surface.coord)),
        )
        .collect::<BTreeSet<_>>();
    let mut protected_route_owners = BTreeMap::<HexCoord, Vec<String>>::new();
    for (name, route) in &existing_features.protected_routes {
        for surface in &route.surfaces {
            protected_route_owners
                .entry(surface.coord)
                .or_default()
                .push(name.clone());
        }
    }
    for (name, route) in ["grand_v3.natural_pass", "grand_v3.peak_saddle"]
        .into_iter()
        .zip(additional_routes)
    {
        for surface in &route.surfaces {
            protected_route_owners
                .entry(surface.coord)
                .or_default()
                .push(name.to_owned());
        }
    }
    let structure_coords = structures
        .by_id
        .values()
        .flat_map(|structure| structure.voxels.iter().map(|voxel| voxel.coord))
        .collect::<BTreeSet<_>>();
    let blocker_coords = blockers
        .iter()
        .map(|blocker| blocker.coord)
        .collect::<BTreeSet<_>>();
    let bank_minimums = recessed_water_bank_minimums(volume);

    let all_route_coords = patch_masks
        .values()
        .flat_map(|mask| mask.iter().copied())
        .collect::<BTreeSet<_>>();
    let mut mutable_allowed = all_route_coords
        .iter()
        .copied()
        .filter(|coord| {
            !water_coords.contains(coord)
                && !protected_coords.contains(coord)
                && !structure_coords.contains(coord)
                && !blocker_coords.contains(coord)
                && !surface_route_exclusion.contains(coord)
                && !high_band.contains(coord)
                && !summit_pins.contains(coord)
                && volume
                    .top_surface_at_coord(*coord)
                    .is_some_and(|(surface, metadata)| {
                        OrdinaryRegionBand::Upper.accepts_existing(surface.level)
                            && metadata.access == SurfaceAccess::Ordinary
                            && ordinary_surface_is_node(volume, Some(blockers), surface)
                    })
        })
        .collect::<BTreeSet<_>>();
    // A mutable column may itself sit between incompatible immutable
    // authorities even though it is not a route candidate. If it remains in
    // the shoulder set, search can legally route nearby and projection only
    // later discovers an impossible support interval (for example level
    // 195..=153 beside the mountain lake). Remove exactly those local support
    // columns, then repeat because each removed column becomes an immutable
    // boundary for its remaining neighbors. This fixed point is intentionally
    // local: unlike propagating every immutable bound through the whole peak
    // component, it does not erase healthy portals on a distant saddle.
    loop {
        let rejected = mutable_allowed
            .iter()
            .copied()
            .filter(|coord| {
                inner_peak_ledge_bounds(*coord, &mutable_allowed, &bank_minimums, volume).is_err()
            })
            .collect::<Vec<_>>();
        if rejected.is_empty() {
            break;
        }
        for coord in rejected {
            mutable_allowed.remove(&coord);
        }
    }
    let centerline_allowed = mutable_allowed
        .iter()
        .copied()
        .filter(|coord| {
            let mut minimum = UPPER_REGION_THRESHOLD
                .saturating_add(1)
                .max(bank_minimums.get(coord).copied().unwrap_or(Level::MIN));
            let mut maximum = MAXIMUM_LEDGE_LEVEL;
            for neighbor in coord
                .neighbors()
                .into_iter()
                .filter(|neighbor| !mutable_allowed.contains(neighbor))
            {
                if let Some((surface, _)) = volume.top_surface_at_coord(neighbor) {
                    minimum = minimum.max(surface.level.saturating_sub(MAXIMUM_PEAK_NEIGHBOR_STEP));
                    maximum = maximum.min(surface.level.saturating_add(MAXIMUM_PEAK_NEIGHBOR_STEP));
                }
            }
            minimum <= maximum
        })
        .collect::<BTreeSet<_>>();
    let routing_penalties = centerline_allowed
        .iter()
        .copied()
        .map(|coord| (coord, ledge_routing_penalty(coord, &bank_minimums, volume)))
        .collect::<BTreeMap<_, _>>();
    let local_route_bounds = centerline_allowed
        .iter()
        .copied()
        .filter_map(|coord| {
            inner_peak_ledge_bounds(coord, &mutable_allowed, &bank_minimums, volume)
                .ok()
                .map(|bounds| (coord, bounds))
        })
        .collect::<BTreeMap<_, _>>();
    let mut route_bounds =
        shoulder_taper_safe_route_bounds(local_route_bounds.clone(), &mutable_allowed, volume);
    // The highland phase owns this one ordered low spine and reserves its
    // exact shoulder before competing route phases run. The whole-component
    // taper has known false negatives because distant immutable authorities
    // constrain Patch 59 through terrain the selected route never changes.
    // Recompute the taper inside the four-row reservation instead: this keeps
    // every boundary constraint that the eventual shoulder must satisfy while
    // preventing unrelated remote terrain from erasing the typed runway.
    let ordered_support_allowed = ordered_transit_spine
        .support_domain
        .intersection(&mutable_allowed)
        .copied()
        .collect::<BTreeSet<_>>();
    let ordered_support_bounds = local_route_bounds
        .iter()
        .filter(|(coord, _)| ordered_support_allowed.contains(coord))
        .map(|(coord, bounds)| (*coord, *bounds))
        .collect::<BTreeMap<_, _>>();
    let ordered_route_bounds =
        shoulder_taper_safe_route_bounds(ordered_support_bounds, &ordered_support_allowed, volume);
    for coord in &ordered_transit_spine.centerline {
        if !mutable_allowed.contains(coord) {
            return Err(schematic_contract(format!(
                "ordered Patch-59 foundation spine lost mutable route authority at {coord:?}"
            )));
        }
        let bounds = ordered_route_bounds.get(coord).copied().ok_or_else(|| {
            schematic_contract(format!(
                "ordered Patch-59 foundation spine has no support-taper-safe bounds at {coord:?}"
            ))
        })?;
        let authored_level = ordered_transit_spine
            .authored_grades
            .get(coord)
            .copied()
            .ok_or_else(|| {
                schematic_contract(format!(
                    "ordered Patch-59 foundation spine has no authored grade at {coord:?}"
                ))
            })?;
        let actual_level = volume
            .top_surface_at_coord(*coord)
            .map(|(surface, _)| surface.level);
        if !(bounds.0..=bounds.1).contains(&authored_level) || actual_level != Some(authored_level)
        {
            return Err(schematic_contract(format!(
                "ordered Patch-59 foundation grade {coord:?}@{authored_level} escaped its support-taper bounds {}..={} or materialized as {actual_level:?}",
                bounds.0, bounds.1
            )));
        }
        route_bounds.insert(*coord, (authored_level, authored_level));
    }
    let route_search_allowed = route_bounds.keys().copied().collect::<BTreeSet<_>>();

    let graph = OrdinaryGraph::from_volume(volume, Some(blockers));
    let upper_distances =
        ordinary_band_distances(&graph, frozen_route_root, OrdinaryRegionBand::Upper);
    let frozen_junction_mask = patch_masks
        .get(&PatchId(u32::from(FROZEN_EXIT_OWNER.1)))
        .ok_or_else(|| {
            schematic_contract("inner peak ledge lost its authored cell-165 Frozen junction mask")
        })?;
    let first_waypoint_mask = patch_masks
        .get(&PatchId(u32::from(FROZEN_FIRST_WAYPOINT.1)))
        .ok_or_else(|| schematic_contract("inner peak ledge lost its retained Frozen waypoint"))?;
    let frozen_exit_coords = frozen_exit
        .surfaces
        .iter()
        .map(|surface| surface.coord)
        .collect::<BTreeSet<_>>();
    // The four-wide Frozen exit is a protected route in its own right. Starting
    // the ledge on one of those protected cells made a single admitted cell sit
    // inside a two-row protected band, so it could not reach either side. Start
    // on exact ordinary plateau footing beside the outer route instead: the
    // existing graph proves the adjacency is already a legal Upper connection,
    // while both route identities remain disjoint and immutable.
    let mut junctions = frozen_exit_coords
        .iter()
        .flat_map(|coord| coord.neighbors())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|coord| {
            let (surface, metadata) = volume.top_surface_at_coord(coord)?;
            (frozen_junction_mask.contains(&coord)
                && mutable_allowed.contains(&coord)
                && OrdinaryRegionBand::Upper.accepts_existing(surface.level)
                && upper_distances.contains_key(&surface)
                && metadata.access == SurfaceAccess::Ordinary
                && ordinary_surface_is_node(volume, Some(blockers), surface))
            .then_some(surface)
        })
        .map(|surface| {
            let approach_gap = first_waypoint_mask
                .iter()
                .map(|candidate| surface.coord.distance(*candidate))
                .min()
                .unwrap_or(u32::MAX);
            let distance = upper_distances.get(&surface).copied().unwrap_or(u32::MAX);
            (
                approach_gap,
                distance,
                surface.level.abs_diff(bench_level),
                surface,
            )
        })
        .collect::<Vec<_>>();
    junctions.sort_unstable();
    if junctions.is_empty() {
        let adjacent_plateau_surfaces = frozen_exit_coords
            .iter()
            .flat_map(|coord| coord.neighbors())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|coord| frozen_junction_mask.contains(coord))
            .filter_map(|coord| {
                volume
                    .top_surface_at_coord(coord)
                    .map(|(surface, _)| surface)
            })
            .collect::<Vec<_>>();
        let published_owners = frozen_exit
            .surfaces
            .iter()
            .map(|surface| (*surface, fine_index.patch(surface.coord)))
            .collect::<Vec<_>>();
        return Err(schematic_contract(format!(
            "inner peak ledge has no admissible already-reachable plateau junction beside the Frozen exit in outward cell 165; adjacent candidates={adjacent_plateau_surfaces:?}; published owners={published_owners:?}; cell-165 mask-size={}; neither the Crystal shell nor level-6 tunnel is a valid ledge seed",
            frozen_junction_mask.len()
        )));
    }

    let main_ids = INNER_CHAIN_ROUTE;
    let suffix_ids = main_ids
        .get(5..)
        .ok_or_else(|| schematic_contract("inner peak route lost its authored suffix"))?;
    let transit_authorities = [ordered_inner_peak_transit_authority(
        &patch_masks,
        ordered_transit_spine,
        &route_search_allowed,
        &route_bounds,
        suffix_ids,
    )?];
    let transit_patch = patch_masks
        .get(&INNER_PEAK_TRANSIT_PATCH)
        .ok_or_else(|| schematic_contract("inner peak ledge lost its Patch-59 transit owner"))?;
    let main_allowed_base = patch_union(&patch_masks, &main_ids)?
        .intersection(&route_search_allowed)
        .copied()
        .collect::<BTreeSet<_>>();
    let main_zones = INNER_CHAIN_ROUTE
        .iter()
        .skip(1)
        .map(|id| {
            patch_masks
                .get(&PatchId(u32::from(*id)))
                .map(|mask| {
                    mask.intersection(&mutable_allowed)
                        .filter(|coord| route_search_allowed.contains(coord))
                        .copied()
                        .collect::<BTreeSet<_>>()
                })
                .ok_or_else(|| {
                    schematic_contract(format!("inner peak ledge lost authored main-arm cell {id}"))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if main_zones.iter().any(BTreeSet::is_empty) {
        let empty_main = INNER_CHAIN_ROUTE
            .iter()
            .skip(1)
            .zip(&main_zones)
            .filter_map(|(id, zone)| zone.is_empty().then_some(*id))
            .collect::<Vec<_>>();
        return Err(schematic_contract(format!(
            "inner peak ledge has no dry Upper corridor outside summit/hydrology authority in required authored cells {empty_main:?}"
        )));
    }
    let shoulder_authority = ShoulderAuthorityDiagnostics {
        plan,
        fine_index,
        mutable_allowed: &mutable_allowed,
        water: water_coords,
        protected_routes: &protected_coords,
        protected_route_owners: &protected_route_owners,
        structures: &structure_coords,
        blockers: &blocker_coords,
        surface_route_exclusion,
        high_band: &high_band,
        summit_pins: &summit_pins,
    };

    let junction_positions = junctions
        .iter()
        .map(|(_, _, _, junction)| *junction)
        .collect::<Vec<_>>();
    let mut transit_search_budget = InnerPeakTransitSearchBudget::new();
    let mut diagnostics = Vec::new();
    let mut selected = None;
    'junctions: for (_, _, _, junction) in junctions {
        for (transit_index, transit_authority) in transit_authorities.iter().enumerate() {
            let mut main_allowed = main_allowed_base.clone();
            main_allowed.retain(|coord| {
                !transit_patch.contains(coord) || transit_authority.runway_domain.contains(coord)
            });
            main_allowed.insert(junction.coord);
            let search = segmented_inner_peak_route_ranked(
                junction.coord,
                &patch_masks,
                &main_allowed,
                &main_ids,
                &routing_penalties,
                junction.level,
                &route_bounds,
                transit_authority,
                &mut transit_search_budget,
            );
            let centerline = match search {
                Ok(centerline) => centerline,
                Err(error) => {
                    if diagnostics.len() < 8 {
                        diagnostics.push(format!(
                            "{junction:?} transit-{transit_index}/{}: {error}",
                            transit_authority.runway_domain.len(),
                        ));
                    }
                    continue;
                }
            };
            let transit_admission =
                match transit_authority.validate_route(&centerline, &patch_masks) {
                    Ok(admission) => admission,
                    Err(diagnostic) => {
                        if diagnostics.len() < 8 {
                            diagnostics.push(format!(
                                "{junction:?} transit-{transit_index}/{}: {diagnostic}",
                                transit_authority.runway_domain.len()
                            ));
                        }
                        continue;
                    }
                };
            let grading = grade_authored_inner_peak_ledge(
                junction,
                &centerline,
                &mutable_allowed,
                Some(&shoulder_authority),
                &bank_minimums,
                volume,
            )
            .and_then(|mut graded| {
                // The natural shoulder is solved as one continuous field, but
                // Patch 59 grants publication authority only inside its typed
                // reservation. Keep the valid route and legal shoulder; an
                // abrupt edge at this ownership seam is accepted as scenery.
                graded.support_levels.retain(|coord, _| {
                    !transit_patch.contains(coord)
                        || ordered_transit_spine.support_domain.contains(coord)
                });
                // Optional shoulder cells may not lift an already-valid
                // scenic saddle. Omitting them restores the unchanged
                // highland surface; route levels are deliberately untouched.
                omit_overheight_saddle_support(component, &mut graded.support_levels)?;
                if graded.support_levels.keys().any(|coord| {
                    transit_patch.contains(coord)
                        && !ordered_transit_spine.support_domain.contains(coord)
                }) {
                    return Err(
                        "ordered Patch-59 route requires support outside its reserved foundation domain"
                            .to_owned(),
                    );
                }
                planned_saddles_remain_scenic(component, &graded.all_levels(), volume)?;
                Ok(graded)
            });
            match grading {
                Ok(graded) => {
                    selected = Some((junction, centerline, graded, transit_admission));
                    break 'junctions;
                }
                Err(diagnostic) if grand_v3_structural_review_draft_enabled() => {
                    // The structural-review launcher exists so terrain authors can
                    // inspect today's highland geometry while the natural support
                    // projection for this optional scenic ledge is still being
                    // reconciled. Keep the exact one-level, coordinate-simple
                    // centerline selected by the bounded solver, but omit only its
                    // unfinished nine-level shoulder. Normal generation never
                    // takes this path and remains fail-closed.
                    bevy::log::warn!(
                        "Grand V3 structural-review draft: omitting inner-peak ledge shoulder: {diagnostic}"
                    );
                    let route_levels = centerline
                        .iter()
                        .map(|position| (position.coord, position.level))
                        .collect::<BTreeMap<_, _>>();
                    selected = Some((
                        junction,
                        centerline,
                        GradedInnerPeakLedge {
                            route_levels,
                            support_levels: BTreeMap::new(),
                        },
                        transit_admission,
                    ));
                    break 'junctions;
                }
                Err(diagnostic) if diagnostics.len() < 8 => {
                    diagnostics.push(format!(
                        "{junction:?} transit-{transit_index}/{}: {diagnostic}",
                        transit_authority.runway_domain.len()
                    ));
                }
                Err(_) => {}
            }
        }
    }
    let (junction, centerline, graded, transit_admission) = selected.ok_or_else(|| {
        let entry_diagnostic = boundary_route_diagnostic(
            BoundaryDiagnosticRequest {
                from_id: 165,
                to_id: 124,
                previous_id: None,
                junctions: &junction_positions,
                component: None,
            },
            &patch_masks,
            &route_search_allowed,
            volume,
            &shoulder_authority,
        );
        let saddle_diagnostic = boundary_route_diagnostic(
            BoundaryDiagnosticRequest {
                from_id: 59,
                to_id: 36,
                previous_id: Some(58),
                junctions: &[],
                component: Some(component),
            },
            &patch_masks,
            &route_search_allowed,
            volume,
            &shoulder_authority,
        );
        let direct_peak_diagnostic = boundary_route_diagnostic(
            BoundaryDiagnosticRequest {
                from_id: 88,
                to_id: 58,
                previous_id: Some(123),
                junctions: &[],
                component: Some(component),
            },
            &patch_masks,
            &route_search_allowed,
            volume,
            &shoulder_authority,
        );
        schematic_contract(format!(
            "no exact plateau junction beside the Frozen exit supports the non-Crystal perimeter ledge through Frozen cells 165/124/123 and all six inner peaks: {}; entry={entry_diagnostic}; saddle={saddle_diagnostic}; direct-88-58={direct_peak_diagnostic}",
            diagnostics.join("; "),
        ))
    })?;

    let mut surfaces = BTreeSet::new();
    for coord in graded.all_coords() {
        let level = graded.level(coord).ok_or_else(|| {
            schematic_contract(format!(
                "inner peak ledge omitted graded level at {coord:?}"
            ))
        })?;
        let position = TilePos::new(*coord, level);
        let current = volume.top_surface_at_coord(*coord).ok_or_else(|| {
            schematic_contract(format!("inner peak ledge lost source surface at {coord:?}"))
        })?;
        if current.0 != position {
            let biome = fine_index.biome(*coord).ok_or_else(|| {
                schematic_contract(format!("inner peak ledge {coord:?} has no biome owner"))
            })?;
            let material = volume
                .columns
                .get(coord)
                .map(top_solid_material)
                .unwrap_or(SolidMaterialRole::Stone);
            replace_column_surface(
                volume,
                biome_regions,
                *coord,
                land_column(level, material),
                position,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
                biome,
            );
        }
        if graded.route_levels.contains_key(coord) {
            surfaces.insert(position);
        }
    }

    if !surfaces.contains(&junction)
        || surfaces.iter().any(|surface| {
            surface.level >= super::super::schematic_highlands::PEAK_VISUAL_WALL_THRESHOLD
                || high_band.contains(&surface.coord)
                || summit_pins.contains(&surface.coord)
        })
    {
        return Err(schematic_contract(
            "inner peak ledge changed its exact junction or entered immutable summit authority",
        ));
    }
    if centerline
        .iter()
        .any(|position| graded.route_levels.get(&position.coord).copied() != Some(position.level))
    {
        return Err(schematic_contract(
            "inner peak ledge changed one exact search-selected centerline level",
        ));
    }
    let route = ProtectedFeatureRoute {
        centerline,
        surfaces,
    };
    if !transit_admission.is_retained_by(&route.centerline) {
        return Err(schematic_contract(
            "inner peak ledge publication changed its exact selected Patch-59 transit admission",
        ));
    }
    validate_protected_route_integrity("inner peak ledge", &route, volume)?;

    // The locked tunnel owner sits above a level-six spanning feature. Repair
    // its unrelated mountain cap here as an Upper-only side branch rather than
    // letting the generic grader replace a stacked tunnel column.
    let mut side_occupied = protected_coords.clone();
    side_occupied.extend(route.surfaces.iter().map(|surface| surface.coord));
    // The main ledge's natural shoulder is deliberately reusable terrain, not
    // a second route authority. A branch must be able to leave through that
    // already-feathered rock; its exact trunk surfaces remain immutable and
    // the branch projection re-proves the same nine-level lateral bound.
    let tunnel_branch = compile_upper_ledge_side_branch(
        "upper tunnel-owner ledge",
        TUNNEL_UPPER_OWNER,
        &[88, 58],
        plan,
        layout,
        fine_index,
        water_coords,
        &route,
        &side_occupied,
        &structure_coords,
        &blocker_coords,
        surface_route_exclusion,
        &high_band,
        &summit_pins,
        component,
        volume,
        biome_regions,
    )?;
    let side_routes = BTreeMap::from([(
        "grand_v3.upper_tunnel_owner_ledge".to_owned(),
        tunnel_branch.route,
    )]);
    let side_support_coords = tunnel_branch.support_coords;
    if !grand_v3_structural_review_draft_enabled() {
        for (name, existing) in &existing_features.protected_routes {
            validate_protected_route_integrity(
                &format!("{name} after inner peak ledge construction"),
                existing,
                volume,
            )?;
        }
        for (index, existing) in additional_routes.iter().enumerate() {
            validate_protected_route_integrity(
                &format!("preexisting peak route {index} after inner ledge construction"),
                existing,
                volume,
            )?;
        }
    }
    // The authored route terminates in inner-chain cell 38, facing both the
    // outer peak chain and the waterfall opening. Its midpoint remains valid
    // traversal footing but sits too far back along the Frozen approach to be
    // the semantic ridge-composition overlook.
    let anchor = route.centerline.last().copied();
    Ok(InnerPeakLedgeCompilation {
        route,
        side_routes,
        anchor,
        support_coords: graded
            .support_levels
            .keys()
            .copied()
            .chain(side_support_coords)
            .collect(),
    })
}

struct UpperLedgeSideBranchCompilation {
    route: ProtectedFeatureRoute,
    support_coords: BTreeSet<HexCoord>,
}

struct UpperLedgeSideBranchContract<'a> {
    label: &'a str,
    centerline: &'a [TilePos],
    support_levels: &'a BTreeMap<HexCoord, Level>,
    trunk: &'a ProtectedFeatureRoute,
    water_coords: &'a BTreeSet<HexCoord>,
    structure_coords: &'a BTreeSet<HexCoord>,
    blocker_coords: &'a BTreeSet<HexCoord>,
    surface_route_exclusion: &'a BTreeSet<HexCoord>,
    high_band: &'a BTreeSet<HexCoord>,
    summit_pins: &'a BTreeSet<HexCoord>,
}

fn validate_upper_ledge_side_branch_contract(
    contract: UpperLedgeSideBranchContract<'_>,
) -> Result<(), String> {
    let UpperLedgeSideBranchContract {
        label,
        centerline,
        support_levels,
        trunk,
        water_coords,
        structure_coords,
        blocker_coords,
        surface_route_exclusion,
        high_band,
        summit_pins,
    } = contract;
    let [shared_start, junction, first_off_trunk, ..] = centerline else {
        return Err(format!(
            "{label} must publish a two-node trunk prefix followed by an off-trunk surface"
        ));
    };
    let prefix_is_real = trunk.centerline.windows(2).any(|pair| {
        let [first, second] = pair else {
            return false;
        };
        (*first == *shared_start && *second == *junction)
            || (*first == *junction && *second == *shared_start)
    });
    if !prefix_is_real {
        return Err(format!(
            "{label} prefix {shared_start:?}->{junction:?} is not one exact consecutive trunk span"
        ));
    }
    if centerline
        .iter()
        .map(|surface| surface.coord)
        .collect::<BTreeSet<_>>()
        .len()
        != centerline.len()
        || centerline.windows(2).any(|pair| {
            let [first, second] = pair else {
                return true;
            };
            first.coord.distance(second.coord) != 1 || first.level.abs_diff(second.level) > 1
        })
    {
        return Err(format!(
            "{label} is not one coordinate-simple adjacent one-level centerline"
        ));
    }

    let trunk_coords = trunk
        .surfaces
        .iter()
        .map(|surface| surface.coord)
        .collect::<BTreeSet<_>>();
    let route_surfaces = centerline.iter().copied().collect::<BTreeSet<_>>();
    let trunk_intersections = route_surfaces
        .intersection(&trunk.surfaces)
        .copied()
        .collect::<BTreeSet<_>>();
    if trunk_intersections != BTreeSet::from([*shared_start, *junction]) {
        return Err(format!(
            "{label} must intersect its trunk at exactly the real two-node prefix"
        ));
    }
    let first_contacts = first_off_trunk
        .coord
        .neighbors()
        .into_iter()
        .filter(|neighbor| trunk_coords.contains(neighbor))
        .collect::<BTreeSet<_>>();
    if first_contacts != BTreeSet::from([junction.coord]) {
        return Err(format!(
            "{label} first off-trunk surface {first_off_trunk:?} contacts trunk coordinates {first_contacts:?}, expected only {:?}",
            junction.coord
        ));
    }
    if let Some((surface, contacts)) = centerline.iter().skip(3).find_map(|surface| {
        let contacts = surface
            .coord
            .neighbors()
            .into_iter()
            .filter(|neighbor| trunk_coords.contains(neighbor))
            .collect::<BTreeSet<_>>();
        (!contacts.is_empty()).then_some((*surface, contacts))
    }) {
        return Err(format!(
            "{label} re-enters or runs beside its trunk at {surface:?} through {contacts:?}"
        ));
    }
    if let Some((first, second)) = centerline.iter().enumerate().find_map(|(index, first)| {
        centerline
            .iter()
            .enumerate()
            .skip(index.saturating_add(2))
            .find_map(|(_, second)| {
                (first.coord.distance(second.coord) == 1).then_some((*first, *second))
            })
    }) {
        return Err(format!(
            "{label} folds into a nonconsecutive self-chord between {first:?} and {second:?}"
        ));
    }

    let upper_floor = UPPER_REGION_THRESHOLD.saturating_add(1);
    if let Some(surface) = centerline
        .iter()
        .copied()
        .find(|surface| surface.level < upper_floor)
    {
        return Err(format!(
            "{label} route surface {surface:?} falls below Upper-only level {upper_floor}"
        ));
    }
    if let Some((coord, level)) = support_levels
        .iter()
        .find(|(_, level)| **level < upper_floor)
    {
        return Err(format!(
            "{label} shoulder support {coord:?}@{level} falls below Upper-only level {upper_floor}"
        ));
    }

    let route_coords = centerline
        .iter()
        .map(|surface| surface.coord)
        .collect::<BTreeSet<_>>();
    let support_coords = support_levels.keys().copied().collect::<BTreeSet<_>>();
    if let Some(coord) = route_coords.intersection(&support_coords).next() {
        return Err(format!(
            "{label} duplicates route and shoulder support authority at {coord:?}"
        ));
    }
    let owned_coords = route_coords
        .union(&support_coords)
        .copied()
        .collect::<BTreeSet<_>>();
    for (authority, excluded) in [
        ("water", water_coords),
        ("structure", structure_coords),
        ("blocker", blocker_coords),
        (
            "waterfall/Crystal/highland route exclusion",
            surface_route_exclusion,
        ),
        ("peak high band", high_band),
        ("summit pin", summit_pins),
    ] {
        if let Some(coord) = owned_coords.intersection(excluded).next() {
            return Err(format!(
                "{label} entered {authority} authority at {coord:?}"
            ));
        }
    }

    let trunk_levels = trunk.surfaces.iter().fold(
        BTreeMap::<HexCoord, Vec<Level>>::new(),
        |mut levels, surface| {
            levels.entry(surface.coord).or_default().push(surface.level);
            levels
        },
    );
    if let Some((support, trunk_surface)) = support_levels.iter().find_map(|(coord, level)| {
        coord.neighbors().into_iter().find_map(|neighbor| {
            trunk_levels.get(&neighbor).and_then(|levels| {
                levels.iter().copied().find_map(|trunk_level| {
                    (neighbor != junction.coord && level.abs_diff(trunk_level) <= 1).then_some((
                        TilePos::new(*coord, *level),
                        TilePos::new(neighbor, trunk_level),
                    ))
                })
            })
        })
    }) {
        return Err(format!(
            "{label} shoulder creates an extra walker departure from {trunk_surface:?} to {support:?}"
        ));
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "the authored side branch receives explicit disjoint world authorities so it cannot infer or collapse them"
)]
fn compile_upper_ledge_side_branch(
    label: &str,
    ((q, r, s), expected_id): ((i32, i32, i32), u16),
    source_ids: &[u16],
    plan: &SchematicPlanV1,
    layout: &ResolvedLayoutPlan,
    fine_index: &FineWorldIndex,
    water_coords: &BTreeSet<HexCoord>,
    trunk: &ProtectedFeatureRoute,
    occupied: &BTreeSet<HexCoord>,
    structure_coords: &BTreeSet<HexCoord>,
    blocker_coords: &BTreeSet<HexCoord>,
    surface_route_exclusion: &BTreeSet<HexCoord>,
    high_band: &BTreeSet<HexCoord>,
    summit_pins: &BTreeSet<HexCoord>,
    component: &super::super::schematic_highlands::PeakRidgeComponentAuthority,
    volume: &mut VolumePlan,
    biome_regions: &mut BTreeMap<TilePos, BiomeRegionId>,
) -> Result<UpperLedgeSideBranchCompilation, V3GenerationError> {
    let schematic_coord =
        SchematicCoord::new(q, r, s).map_err(|error| schematic_contract(error.to_string()))?;
    let target_cell = plan.cell(schematic_coord).ok_or_else(|| {
        schematic_contract(format!(
            "{label} is missing locked schematic cell {schematic_coord:?}"
        ))
    })?;
    let expected_overlay = SchematicFeature::Tunnel;
    if target_cell.id.get() != expected_id
        || target_cell.facts.surface != SurfaceKind::Land
        || target_cell.facts.landform != LandformKind::Mountain
        || target_cell.facts.climate != ClimateKind::Alpine
        || target_cell.facts.access != AccessIntent::Ordinary
        || !has_overlay(target_cell, expected_overlay)
    {
        return Err(schematic_contract(format!(
            "{label} cell {expected_id} lost its exact Land/Mountain/Alpine/Ordinary/{expected_overlay:?} contract"
        )));
    }
    let target_mask = layout
        .patches
        .get(&PatchId(u32::from(expected_id)))
        .map(|patch| patch.mask.clone())
        .ok_or_else(|| schematic_contract(format!("{label} target has no resolved patch")))?;
    let source_masks = source_ids
        .iter()
        .map(|id| {
            layout
                .patches
                .get(&PatchId(u32::from(*id)))
                .map(|patch| (*id, patch.mask.clone()))
                .ok_or_else(|| {
                    schematic_contract(format!("{label} source cell {id} has no resolved patch"))
                })
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let bank_minimums = recessed_water_bank_minimums(volume);
    let trunk_coords = trunk
        .surfaces
        .iter()
        .map(|surface| surface.coord)
        .collect::<BTreeSet<_>>();
    let route_footprint = target_mask
        .iter()
        .copied()
        .chain(source_masks.values().flat_map(|mask| mask.iter().copied()))
        .collect::<BTreeSet<_>>();
    let mut mutable_base = route_footprint
        .iter()
        .copied()
        .filter(|coord| {
            !water_coords.contains(coord)
                && !occupied.contains(coord)
                && !structure_coords.contains(coord)
                && !blocker_coords.contains(coord)
                && !surface_route_exclusion.contains(coord)
                && !high_band.contains(coord)
                && !summit_pins.contains(coord)
                && volume
                    .top_surface_at_coord(*coord)
                    .is_some_and(|(surface, metadata)| {
                        OrdinaryRegionBand::Upper.accepts_existing(surface.level)
                            && metadata.access == SurfaceAccess::Ordinary
                            && ordinary_surface_is_node(volume, None, surface)
                    })
        })
        .collect::<BTreeSet<_>>();
    loop {
        let rejected = mutable_base
            .iter()
            .copied()
            .filter(|coord| {
                inner_peak_ledge_bounds(*coord, &mutable_base, &bank_minimums, volume).is_err()
            })
            .collect::<Vec<_>>();
        if rejected.is_empty() {
            break;
        }
        for coord in rejected {
            mutable_base.remove(&coord);
        }
    }
    let mut attempts = Vec::new();
    let mut source_summaries = Vec::new();
    let target_exclusion_summary = (
        target_mask
            .iter()
            .filter(|coord| water_coords.contains(coord))
            .count(),
        target_mask
            .iter()
            .filter(|coord| occupied.contains(coord))
            .count(),
        target_mask
            .iter()
            .filter(|coord| surface_route_exclusion.contains(coord))
            .count(),
        target_mask
            .iter()
            .filter(|coord| high_band.contains(coord))
            .count(),
        target_mask
            .iter()
            .filter(|coord| summit_pins.contains(coord))
            .count(),
        target_mask
            .iter()
            .filter(|coord| {
                volume
                    .top_surface_at_coord(**coord)
                    .is_some_and(|(surface, metadata)| {
                        OrdinaryRegionBand::Upper.accepts_existing(surface.level)
                            && metadata.access == SurfaceAccess::Ordinary
                            && ordinary_surface_is_node(volume, None, surface)
                    })
            })
            .count(),
    );
    let mut selected = None;
    for source_id in source_ids {
        let Some(source_mask) = source_masks.get(source_id) else {
            continue;
        };
        // On a hex grid, leaving the side of a straight one-cell path normally
        // touches both endpoints of the crossed edge. Model that as one
        // contiguous two-surface junction span rather than pretending a
        // one-voxel branch can avoid the second legal adjacency. The branch
        // shares that exact prefix, leaves once, and can never touch the trunk
        // again.
        let mut attachments = trunk
            .centerline
            .windows(2)
            .filter_map(|pair| {
                let [first, second] = pair else {
                    return None;
                };
                Some([(*first, *second), (*second, *first)])
            })
            .flatten()
            .filter(|(shared_start, junction)| {
                source_mask.contains(&shared_start.coord)
                    && source_mask.contains(&junction.coord)
                    && OrdinaryRegionBand::Upper.accepts_existing(shared_start.level)
                    && OrdinaryRegionBand::Upper.accepts_existing(junction.level)
            })
            .collect::<Vec<_>>();
        attachments.sort_unstable_by_key(|(shared_start, junction)| {
            let target_gap = target_mask
                .iter()
                .map(|coord| junction.coord.distance(*coord))
                .min()
                .unwrap_or(u32::MAX);
            (target_gap, *junction, *shared_start)
        });
        let source_attempt_start = attempts.len();
        source_summaries.push(format!(
            "source={source_id} attachments={} mutable-source={} mutable-target={} raw-seam-portals={}",
            attachments.len(),
            source_mask.intersection(&mutable_base).count(),
            target_mask.intersection(&mutable_base).count(),
            source_mask
                .iter()
                .flat_map(|coord| coord.neighbors())
                .filter(|coord| target_mask.contains(coord))
                .count(),
        ));
        for (shared_start, junction) in attachments {
            let attachment_coords = BTreeSet::from([shared_start.coord, junction.coord]);
            // A side branch owns one exact contiguous junction span. Exclude
            // every candidate column that would touch any other trunk node,
            // preventing a hidden chord or route re-entry after grading.
            let mutable_allowed = mutable_base
                .iter()
                .copied()
                .filter(|coord| {
                    coord.neighbors().into_iter().all(|neighbor| {
                        !trunk_coords.contains(&neighbor) || attachment_coords.contains(&neighbor)
                    })
                })
                .collect::<BTreeSet<_>>();
            let mut bounds = mutable_allowed
                .iter()
                .copied()
                .filter_map(|coord| {
                    inner_peak_ledge_bounds(coord, &mutable_allowed, &bank_minimums, volume)
                        .ok()
                        .map(|range| (coord, range))
                })
                .collect::<BTreeMap<_, _>>();
            bounds = shoulder_taper_safe_route_bounds(bounds, &mutable_allowed, volume);
            bounds.insert(junction.coord, (junction.level, junction.level));
            // Taper reconciliation can remove coordinates that passed the
            // first local bound check. Keep the portal graph and its exact
            // elevation domain identical; admitting a coordinate after its
            // bound was removed makes the ranked search fail nondeterministically
            // at the first such cell instead of trying the remaining safe
            // corridor.
            let bounded_allowed = mutable_allowed
                .iter()
                .copied()
                .filter(|coord| bounds.contains_key(coord))
                .collect::<BTreeSet<_>>();
            let source_zone = source_mask
                .intersection(&bounded_allowed)
                .copied()
                .chain(std::iter::once(junction.coord))
                .collect::<BTreeSet<_>>();
            let target_zone = target_mask
                .intersection(&bounded_allowed)
                .copied()
                .collect::<BTreeSet<_>>();
            if target_zone.is_empty() {
                continue;
            }
            let masks = BTreeMap::from([
                (PatchId(u32::from(*source_id)), source_zone.clone()),
                (PatchId(u32::from(expected_id)), target_zone.clone()),
            ]);
            let allowed = source_zone
                .union(&target_zone)
                .copied()
                .collect::<BTreeSet<_>>();
            let penalties = allowed
                .iter()
                .copied()
                .map(|coord| (coord, ledge_routing_penalty(coord, &bank_minimums, volume)))
                .collect::<BTreeMap<_, _>>();
            let search = segmented_portal_route_ranked(
                junction.coord,
                &masks,
                &allowed,
                &[*source_id, expected_id],
                &penalties,
                Some(junction.level),
                Some(&bounds),
            );
            let centerline = match search {
                Ok(value) => value,
                Err(error) => {
                    if attempts.len().saturating_sub(source_attempt_start) < 6 {
                        attempts.push(format!("source={source_id} junction={junction:?}: {error}"));
                    }
                    continue;
                }
            };
            if centerline.len() < 2
                || centerline
                    .last()
                    .is_none_or(|end| !target_mask.contains(&end.coord))
                || centerline.iter().skip(1).any(|position| {
                    position.coord.neighbors().into_iter().any(|neighbor| {
                        trunk_coords.contains(&neighbor) && neighbor != junction.coord
                    })
                })
            {
                continue;
            }
            let grading = grade_authored_inner_peak_ledge(
                junction,
                &centerline,
                &mutable_allowed,
                None,
                &bank_minimums,
                volume,
            )
            .and_then(|graded| {
                planned_saddles_remain_scenic(component, &graded.all_levels(), volume)?;
                Ok(graded)
            });
            let graded = match grading {
                Ok(value) => value,
                Err(error) => {
                    if attempts.len().saturating_sub(source_attempt_start) < 6 {
                        attempts.push(format!("source={source_id} junction={junction:?}: {error}"));
                    }
                    continue;
                }
            };
            let prospective_centerline = std::iter::once(shared_start)
                .chain(centerline.iter().copied())
                .collect::<Vec<_>>();
            if let Err(error) =
                validate_upper_ledge_side_branch_contract(UpperLedgeSideBranchContract {
                    label,
                    centerline: &prospective_centerline,
                    support_levels: &graded.support_levels,
                    trunk,
                    water_coords,
                    structure_coords,
                    blocker_coords,
                    surface_route_exclusion,
                    high_band,
                    summit_pins,
                })
            {
                if attempts.len().saturating_sub(source_attempt_start) < 6 {
                    attempts.push(format!("source={source_id} junction={junction:?}: {error}"));
                }
                continue;
            }
            let score = (
                graded.support_levels.len(),
                centerline.len().saturating_add(1),
                centerline.last().copied(),
                junction,
                shared_start,
            );
            if selected
                .as_ref()
                .is_none_or(|(current, _, _, _)| score < *current)
            {
                selected = Some((score, shared_start, centerline, graded));
            }
        }
    }
    let (_, shared_start, centerline, graded) = selected.ok_or_else(|| {
        schematic_contract(format!(
            "{label} has no dry Upper-only one-junction branch into cell {expected_id} outside exact feature/highland authority: {}",
            std::iter::once(format!("target-exclusions=(water,occupied,feature,high-band,summit,upper-ordinary)={target_exclusion_summary:?}"))
                .chain(source_summaries)
                .chain(attempts)
                .collect::<Vec<_>>()
                .join("; ")
        ))
    })?;
    for coord in graded.all_coords() {
        let level = graded.level(coord).ok_or_else(|| {
            schematic_contract(format!("{label} omitted graded level at {coord:?}"))
        })?;
        let position = TilePos::new(*coord, level);
        let (current, metadata) = volume.top_surface_at_coord(*coord).ok_or_else(|| {
            schematic_contract(format!("{label} lost source surface at {coord:?}"))
        })?;
        if current != position {
            let biome = fine_index.biome(*coord).ok_or_else(|| {
                schematic_contract(format!("{label} {coord:?} has no biome owner"))
            })?;
            let material = volume
                .columns
                .get(coord)
                .map(top_solid_material)
                .unwrap_or(SolidMaterialRole::Stone);
            replace_column_surface(
                volume,
                biome_regions,
                *coord,
                land_column(level, material),
                position,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: metadata.interior,
                },
                biome,
            );
        }
    }
    let centerline = std::iter::once(shared_start)
        .chain(centerline)
        .collect::<Vec<_>>();
    let surfaces = centerline.iter().copied().collect::<BTreeSet<_>>();
    let branch = ProtectedFeatureRoute {
        centerline,
        surfaces,
    };
    validate_protected_route_integrity(label, &branch, volume)?;
    validate_upper_ledge_side_branch_contract(UpperLedgeSideBranchContract {
        label,
        centerline: &branch.centerline,
        support_levels: &graded.support_levels,
        trunk,
        water_coords,
        structure_coords,
        blocker_coords,
        surface_route_exclusion,
        high_band,
        summit_pins,
    })
    .map_err(schematic_contract)?;
    for (coord, level) in &graded.support_levels {
        let support = TilePos::new(*coord, *level);
        if volume
            .top_surface_at_coord(*coord)
            .is_none_or(|(surface, _)| surface != support)
            || !ordinary_surface_is_node(volume, None, support)
        {
            return Err(schematic_contract(format!(
                "{label} shoulder support {support:?} was not published as its exact ordinary top surface"
            )));
        }
    }
    if !branch
        .surfaces
        .iter()
        .any(|surface| target_mask.contains(&surface.coord))
    {
        return Err(schematic_contract(format!(
            "{label} did not publish exact footing in target cell {expected_id}"
        )));
    }
    Ok(UpperLedgeSideBranchCompilation {
        route: branch,
        support_coords: graded.support_levels.keys().copied().collect(),
    })
}

fn insert_route_support_mask(
    masks: &mut BTreeMap<PatchId, BTreeSet<HexCoord>>,
    plan: &SchematicPlanV1,
    layout: &ResolvedLayoutPlan,
    ((q, r, s), expected_id): ((i32, i32, i32), u16),
    expected_landform: LandformKind,
    expected_climate: ClimateKind,
    expected_overlay: SchematicFeature,
    label: &str,
) -> Result<(), V3GenerationError> {
    let coord =
        SchematicCoord::new(q, r, s).map_err(|error| schematic_contract(error.to_string()))?;
    let cell = plan.cell(coord).ok_or_else(|| {
        schematic_contract(format!(
            "{label} is missing locked schematic cell {coord:?}"
        ))
    })?;
    if cell.id.get() != expected_id
        || cell.facts.surface != SurfaceKind::Land
        || cell.facts.landform != expected_landform
        || cell.facts.climate != expected_climate
        || cell.facts.access != AccessIntent::Ordinary
        || !has_overlay(cell, expected_overlay)
    {
        return Err(schematic_contract(format!(
            "{label} cell {expected_id} lost its exact Land/{expected_landform:?}/{expected_climate:?}/Ordinary/{expected_overlay:?} contract"
        )));
    }
    let patch_id = PatchId(u32::from(expected_id));
    let mask = layout
        .patches
        .get(&patch_id)
        .map(|patch| patch.mask.clone())
        .ok_or_else(|| schematic_contract(format!("{label} cell {expected_id} has no patch")))?;
    if masks.insert(patch_id, mask).is_some() {
        return Err(schematic_contract(format!(
            "{label} cell {expected_id} duplicated existing ledge authority"
        )));
    }
    Ok(())
}

fn peak_saddle_nominal_scenic_ceiling(
    component: &super::super::schematic_highlands::PeakRidgeComponentAuthority,
    first: &PatchId,
    second: &PatchId,
) -> Result<Level, String> {
    let summit_level = |owner: &PatchId| {
        component.summit_pins.iter().find_map(|(pin, level)| {
            component
                .expected_peak_bodies
                .get(owner)
                .is_some_and(|body| body.contains_key(pin))
                .then_some(*level)
        })
    };
    let (Some(first_level), Some(second_level)) = (summit_level(first), summit_level(second))
    else {
        return Err(format!(
            "peak saddle {}-{} lost one summit owner",
            first.0, second.0
        ));
    };
    Ok(first_level
        .min(second_level)
        .saturating_sub(30)
        .min(super::super::schematic_highlands::PEAK_VISUAL_WALL_THRESHOLD.saturating_sub(1)))
}

fn omit_overheight_saddle_support(
    component: &super::super::schematic_highlands::PeakRidgeComponentAuthority,
    support_levels: &mut BTreeMap<HexCoord, Level>,
) -> Result<(), String> {
    for ((first, second), swath) in &component.expected_saddle_swaths {
        let nominal_ceiling = peak_saddle_nominal_scenic_ceiling(component, first, second)?;
        let admitted_ceilings = swath
            .iter()
            .map(|coord| {
                component
                    .expected_ridge_profile
                    .get(coord)
                    .copied()
                    .map(|foundation| (*coord, nominal_ceiling.max(foundation)))
                    .ok_or_else(|| {
                        format!("peak saddle support lost its sealed foundation at {coord:?}")
                    })
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        support_levels.retain(|coord, level| {
            admitted_ceilings
                .get(coord)
                .is_none_or(|ceiling| *level <= *ceiling)
        });
    }
    Ok(())
}

fn planned_saddles_remain_scenic(
    component: &super::super::schematic_highlands::PeakRidgeComponentAuthority,
    planned_levels: &BTreeMap<HexCoord, Level>,
    volume: &VolumePlan,
) -> Result<(), String> {
    for ((first, second), swath) in &component.expected_saddle_swaths {
        if swath.len() < 4 || !coords_connected(swath) {
            return Err(format!(
                "peak saddle {}-{} lost its four-column connected swath before ledge grading",
                first.0, second.0
            ));
        }
        let nominal_ceiling = peak_saddle_nominal_scenic_ceiling(component, first, second)?;
        let admitted_ceilings = swath
            .iter()
            .map(|coord| {
                component
                    .expected_ridge_profile
                    .get(coord)
                    .copied()
                    .map(|foundation| (*coord, nominal_ceiling.max(foundation)))
                    .ok_or_else(|| {
                        format!("peak saddle validation lost its sealed foundation at {coord:?}")
                    })
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        if let Some((coord, level, admitted_ceiling)) = swath.iter().find_map(|coord| {
            let admitted_ceiling = admitted_ceilings.get(coord).copied()?;
            planned_levels
                .get(coord)
                .copied()
                .or_else(|| {
                    volume
                        .top_surface_at_coord(*coord)
                        .map(|(surface, _)| surface.level)
                })
                .filter(|level| *level > admitted_ceiling)
                .map(|level| (*coord, level, admitted_ceiling))
        }) {
            return Err(format!(
                "planned ledge raised saddle {}-{} above its admitted scenic ceiling {admitted_ceiling} (nominal {nominal_ceiling}) at {coord:?} level {level}",
                first.0, second.0
            ));
        }
    }
    Ok(())
}

fn inner_chain_authority<'a>(
    plan: &SchematicPlanV1,
    layout: &ResolvedLayoutPlan,
    peak_ridges: &'a super::super::schematic_highlands::PeakRidgeAuthority,
) -> Result<
    (
        BTreeMap<PatchId, BTreeSet<HexCoord>>,
        &'a super::super::schematic_highlands::PeakRidgeComponentAuthority,
    ),
    V3GenerationError,
> {
    let mut masks = BTreeMap::new();
    for ((q, r, s), expected_id) in INNER_CHAIN {
        let coord =
            SchematicCoord::new(q, r, s).map_err(|error| schematic_contract(error.to_string()))?;
        let cell = plan.cell(coord).ok_or_else(|| {
            schematic_contract(format!("inner peak chain is missing locked cell {coord:?}"))
        })?;
        if cell.id.get() != expected_id
            || cell.facts.surface != SurfaceKind::Land
            || cell.facts.landform != LandformKind::SharpPeak
            || cell.facts.access != AccessIntent::Ordinary
            || !has_overlay(cell, SchematicFeature::PeakRing)
        {
            return Err(schematic_contract(format!(
                "inner peak cell {expected_id} lost its exact locked Land/SharpPeak/Ordinary/PeakRing contract"
            )));
        }
        let patch_id = PatchId(u32::from(expected_id));
        let mask = layout
            .patches
            .get(&patch_id)
            .map(|patch| patch.mask.clone())
            .ok_or_else(|| {
                schematic_contract(format!(
                    "inner peak cell {expected_id} has no resolved patch"
                ))
            })?;
        masks.insert(patch_id, mask);
    }
    let expected_patches = masks.keys().copied().collect::<BTreeSet<_>>();
    let component = peak_ridges
        .components
        .iter()
        .find(|component| {
            component
                .patch_masks
                .keys()
                .copied()
                .collect::<BTreeSet<_>>()
                == expected_patches
        })
        .ok_or_else(|| {
            schematic_contract("inner peak ledge has no exact six-patch connected-ridge authority")
        })?;
    Ok((masks, component))
}

fn patch_union(
    masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    ids: &[u16],
) -> Result<BTreeSet<HexCoord>, V3GenerationError> {
    let mut union = BTreeSet::new();
    for id in ids {
        let mask = masks.get(&PatchId(u32::from(*id))).ok_or_else(|| {
            schematic_contract(format!("inner peak ledge lost resolved patch {id}"))
        })?;
        union.extend(mask.iter().copied());
    }
    Ok(union)
}

/// Builds a complete reverse reachability map for the exact authored suffix.
///
/// Coordinate history is deliberately relaxed here, so membership is only a
/// necessary condition for a physical route. The forward history-aware solver
/// and typed admission remain authoritative. Absence is used as a proof only
/// when every finite exact state fit below `state_limit` and the traversal
/// completed; an oversized graph is returned as typed incomplete evidence.
fn inner_peak_suffix_reachability(
    sequence: &[u16],
    masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    route_search_allowed: &BTreeSet<HexCoord>,
    route_bounds: &BTreeMap<HexCoord, (Level, Level)>,
    runway_domain: &BTreeSet<HexCoord>,
    egress: &BTreeSet<BoundaryPortal>,
    state_limit: usize,
) -> Result<InnerPeakSuffixReachability, String> {
    if sequence.len() < 2
        || sequence.first().copied() != Some(59)
        || sequence.iter().copied().collect::<BTreeSet<_>>().len() != sequence.len()
    {
        return Err(format!(
            "inner peak suffix must begin at Patch 59 and contain at least two unique stages: {sequence:?}"
        ));
    }
    let mut allowed_by_stage = Vec::with_capacity(sequence.len());
    let mut possible_states = 0_usize;
    for id in sequence.iter().copied() {
        let patch = masks
            .get(&PatchId(u32::from(id)))
            .ok_or_else(|| format!("inner peak suffix lost resolved patch {id}"))?;
        let admitted = patch
            .intersection(route_search_allowed)
            .filter(|coord| id != 59 || runway_domain.contains(coord))
            .copied()
            .collect::<BTreeSet<_>>();
        if admitted.is_empty() {
            return Err(format!(
                "inner peak suffix patch {id} has no admitted coordinate"
            ));
        }
        for coord in &admitted {
            let (minimum, maximum) = route_bounds.get(coord).copied().ok_or_else(|| {
                format!("inner peak suffix has no exact elevation bounds at {coord:?}")
            })?;
            if minimum > maximum {
                return Err(format!(
                    "inner peak suffix has inverted elevation bounds at {coord:?}: {minimum}..={maximum}"
                ));
            }
            let width = i64::from(maximum)
                .saturating_sub(i64::from(minimum))
                .saturating_add(1);
            let width = usize::try_from(width).map_err(|error| {
                format!(
                    "inner peak suffix bounds at {coord:?} exceed addressable state space: {error}"
                )
            })?;
            possible_states = possible_states.saturating_add(width);
        }
        allowed_by_stage.push(admitted);
    }
    if possible_states > state_limit {
        return Ok(InnerPeakSuffixReachability::Incomplete {
            sequence: sequence.to_vec(),
            possible_states,
            state_limit,
        });
    }

    let mut portals = Vec::with_capacity(sequence.len().saturating_sub(1));
    for (stage, pair) in sequence.windows(2).enumerate() {
        let [from_id, to_id] = pair else {
            continue;
        };
        let from_allowed = allowed_by_stage
            .get(stage)
            .ok_or_else(|| format!("inner peak suffix lost admitted patch {from_id}"))?;
        let to_allowed = allowed_by_stage
            .get(stage.saturating_add(1))
            .ok_or_else(|| format!("inner peak suffix lost admitted patch {to_id}"))?;
        let mut edge_portals = BTreeSet::new();
        for from in from_allowed {
            for to in from.neighbors() {
                let portal = BoundaryPortal { from: *from, to };
                if to_allowed.contains(&to)
                    && (*from_id != 59 || *to_id != 36 || egress.contains(&portal))
                {
                    edge_portals.insert(portal);
                }
            }
        }
        if edge_portals.is_empty() {
            return Err(format!(
                "inner peak suffix has no exact admitted boundary handoff {from_id}->{to_id}"
            ));
        }
        portals.push(edge_portals);
    }

    let final_stage = sequence.len().saturating_sub(1);
    let final_allowed = allowed_by_stage
        .get(final_stage)
        .ok_or_else(|| "inner peak suffix lost its final authored patch".to_owned())?;
    let mut distances = BTreeMap::<InnerPeakSuffixState, u32>::new();
    let mut frontier = VecDeque::new();
    for coord in final_allowed {
        let (minimum, maximum) = route_bounds
            .get(coord)
            .copied()
            .ok_or_else(|| format!("inner peak suffix lost final bounds at {coord:?}"))?;
        for level in minimum..=maximum {
            let state = InnerPeakSuffixState {
                stage: final_stage,
                position: TilePos::new(*coord, level),
            };
            distances.insert(state, 0);
            frontier.push_back(state);
        }
    }

    while let Some(current) = frontier.pop_front() {
        let distance = distances
            .get(&current)
            .copied()
            .ok_or_else(|| format!("inner peak suffix lost reverse state {current:?}"))?;
        let current_allowed = allowed_by_stage
            .get(current.stage)
            .ok_or_else(|| "inner peak suffix lost a reverse stage".to_owned())?;
        let mut predecessor_coords = current
            .position
            .coord
            .neighbors()
            .into_iter()
            .filter(|coord| current_allowed.contains(coord))
            .map(|coord| (current.stage, coord))
            .collect::<BTreeSet<_>>();
        if current.stage > 0 {
            let previous_stage = current.stage.saturating_sub(1);
            let previous_allowed = allowed_by_stage
                .get(previous_stage)
                .ok_or_else(|| "inner peak suffix lost a previous reverse stage".to_owned())?;
            let edge_portals = portals
                .get(previous_stage)
                .ok_or_else(|| "inner peak suffix lost reverse boundary portals".to_owned())?;
            for coord in current.position.coord.neighbors() {
                if previous_allowed.contains(&coord)
                    && edge_portals.contains(&BoundaryPortal {
                        from: coord,
                        to: current.position.coord,
                    })
                {
                    predecessor_coords.insert((previous_stage, coord));
                }
            }
        }

        for (stage, coord) in predecessor_coords {
            let (minimum, maximum) = route_bounds
                .get(&coord)
                .copied()
                .ok_or_else(|| format!("inner peak suffix lost reverse bounds at {coord:?}"))?;
            let lowest = minimum.max(current.position.level.saturating_sub(1));
            let highest = maximum.min(current.position.level.saturating_add(1));
            if lowest > highest {
                continue;
            }
            for level in lowest..=highest {
                let predecessor = InnerPeakSuffixState {
                    stage,
                    position: TilePos::new(coord, level),
                };
                if distances.contains_key(&predecessor) {
                    continue;
                }
                distances.insert(predecessor, distance.saturating_add(1));
                frontier.push_back(predecessor);
            }
        }
    }

    let distance_by_entry = distances
        .iter()
        .filter_map(|(state, distance)| (state.stage == 0).then_some((state.position, *distance)))
        .collect::<BTreeMap<_, _>>();
    let reachable_by_stage = sequence
        .iter()
        .copied()
        .enumerate()
        .map(|(stage, patch)| {
            let states = distances
                .keys()
                .filter(|state| state.stage == stage)
                .collect::<Vec<_>>();
            let levels = states.iter().map(|state| state.position.level);
            let minimum = levels.clone().min();
            let maximum = levels.max();
            (patch, states.len(), minimum, maximum)
        })
        .collect::<Vec<_>>();
    let egress_diagnostic = egress
        .iter()
        .map(|portal| {
            let from_bounds = route_bounds.get(&portal.from).copied();
            let to_bounds = route_bounds.get(&portal.to).copied();
            let reachable_to_levels = distances
                .keys()
                .filter(|state| state.stage == 1 && state.position.coord == portal.to)
                .map(|state| state.position.level)
                .collect::<BTreeSet<_>>();
            (
                *portal,
                from_bounds,
                to_bounds,
                reachable_to_levels.first().copied(),
                reachable_to_levels.last().copied(),
                reachable_to_levels.len(),
            )
        })
        .collect::<Vec<_>>();
    Ok(InnerPeakSuffixReachability::Complete {
        sequence: sequence.to_vec(),
        distance_by_entry,
        explored_states: distances.len(),
        possible_states,
        diagnostic: format!(
            "reachable-by-stage={reachable_by_stage:?}; egress={egress_diagnostic:?}"
        ),
    })
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Regression fixtures retain the alternate transit search; production uses ordered_inner_peak_transit_authority."
    )
)]
fn inner_peak_transit_authorities(
    masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    saddle_swaths: &BTreeMap<(PatchId, PatchId), BTreeSet<HexCoord>>,
    route_search_allowed: &BTreeSet<HexCoord>,
    route_bounds: &BTreeMap<HexCoord, (Level, Level)>,
    suffix_sequence: &[u16],
) -> Result<Vec<InnerPeakTransitAuthority>, V3GenerationError> {
    let mask = |patch: PatchId| {
        masks.get(&patch).ok_or_else(|| {
            schematic_contract(format!(
                "inner peak transit contract lost Patch {}",
                patch.0
            ))
        })
    };
    let patch_58 = mask(INNER_PEAK_TRANSIT_INGRESS.0)?;
    let patch_59 = mask(INNER_PEAK_TRANSIT_PATCH)?;
    let patch_36 = mask(INNER_PEAK_TRANSIT_EGRESS.0)?;
    let ingress_swath = saddle_swaths
        .get(&INNER_PEAK_TRANSIT_INGRESS)
        .ok_or_else(|| {
            schematic_contract("inner peak transit contract lost the exact 58/59 saddle swath")
        })?;
    let egress_swath = saddle_swaths
        .get(&INNER_PEAK_TRANSIT_EGRESS)
        .ok_or_else(|| {
            schematic_contract("inner peak transit contract lost the exact 59/36 saddle swath")
        })?;
    let boundary_portals = |from: &BTreeSet<HexCoord>,
                            to: &BTreeSet<HexCoord>,
                            swath: &BTreeSet<HexCoord>| {
        from.iter()
            .flat_map(|from_coord| {
                from_coord
                    .neighbors()
                    .into_iter()
                    .filter(|to_coord| to.contains(to_coord))
                    .map(|to_coord| BoundaryPortal {
                        from: *from_coord,
                        to: to_coord,
                    })
            })
            .filter(|portal| {
                swath.contains(&portal.from)
                    && swath.contains(&portal.to)
                    && route_search_allowed.contains(&portal.from)
                    && route_search_allowed.contains(&portal.to)
                    && route_bounds
                        .get(&portal.from)
                        .copied()
                        .zip(route_bounds.get(&portal.to).copied())
                        .is_some_and(|((from_minimum, from_maximum), (to_minimum, to_maximum))| {
                            from_minimum <= to_maximum.saturating_add(1)
                                && to_minimum <= from_maximum.saturating_add(1)
                        })
            })
            .collect::<BTreeSet<_>>()
    };
    let ingress = boundary_portals(patch_58, patch_59, ingress_swath);
    let egress = boundary_portals(patch_59, patch_36, egress_swath);
    if ingress.is_empty() || egress.is_empty() {
        return Err(schematic_contract(format!(
            "inner peak transit contract has no admitted scenic boundary portal: ingress={}, egress={}",
            ingress.len(),
            egress.len()
        )));
    }

    let scenic_domain = ingress_swath
        .union(egress_swath)
        .copied()
        .filter(|coord| patch_59.contains(coord) && route_search_allowed.contains(coord))
        .collect::<BTreeSet<_>>();
    let full_domain = patch_59
        .intersection(route_search_allowed)
        .copied()
        .collect::<BTreeSet<_>>();
    let qualifies = |component: &BTreeSet<HexCoord>| {
        ingress.iter().any(|portal| component.contains(&portal.to))
            && egress.iter().any(|portal| component.contains(&portal.from))
    };
    let mut domains = Vec::<BTreeSet<HexCoord>>::new();
    for domain in [&scenic_domain, &full_domain] {
        let mut components = coord_components(domain)
            .into_iter()
            .filter(qualifies)
            .collect::<Vec<_>>();
        components.sort_by_key(|component| {
            (
                component.len(),
                component.first().copied().unwrap_or(HexCoord::ORIGIN),
            )
        });
        for component in components {
            if !domains.contains(&component) {
                domains.push(component);
            }
        }
    }
    let mut authorities = Vec::new();
    for runway_domain in domains {
        let component_ingress = ingress
            .iter()
            .copied()
            .filter(|portal| runway_domain.contains(&portal.to))
            .collect::<BTreeSet<_>>();
        let component_egress = egress
            .iter()
            .copied()
            .filter(|portal| runway_domain.contains(&portal.from))
            .collect::<BTreeSet<_>>();
        if runway_domain.is_empty()
            || !coords_connected(&runway_domain)
            || component_ingress.is_empty()
            || component_egress.is_empty()
        {
            continue;
        }
        let suffix_reachability = inner_peak_suffix_reachability(
            suffix_sequence,
            masks,
            route_search_allowed,
            route_bounds,
            &runway_domain,
            &component_egress,
            INNER_PEAK_SUFFIX_STATE_LIMIT,
        )
        .map_err(schematic_contract)?;
        authorities.push(InnerPeakTransitAuthority {
            runway_domain,
            ingress: component_ingress,
            egress: component_egress,
            suffix_reachability: std::sync::Arc::new(suffix_reachability),
            ordered_runway: None,
        });
    }
    if authorities.is_empty() {
        return Err(schematic_contract(format!(
            "inner peak transit contract has no connected Patch-59 domain between exact grade-compatible saddle portals: scenic={}, full={}, ingress={}, egress={}",
            scenic_domain.len(),
            full_domain.len(),
            ingress.len(),
            egress.len()
        )));
    }
    Ok(authorities)
}

fn ordered_inner_peak_transit_authority(
    masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    authored: &super::super::schematic_highlands::OrderedPeakSaddleSpineAuthority,
    route_search_allowed: &BTreeSet<HexCoord>,
    route_bounds: &BTreeMap<HexCoord, (Level, Level)>,
    suffix_sequence: &[u16],
) -> Result<InnerPeakTransitAuthority, V3GenerationError> {
    if authored.owner != INNER_PEAK_TRANSIT_PATCH
        || authored.ingress_from != INNER_PEAK_TRANSIT_INGRESS.0
        || authored.egress_to != INNER_PEAK_TRANSIT_EGRESS.0
    {
        return Err(schematic_contract(format!(
            "ordered inner-peak transit has wrong typed owners: ingress={}, owner={}, egress={}",
            authored.ingress_from.0, authored.owner.0, authored.egress_to.0
        )));
    }
    let ingress_mask = masks
        .get(&authored.ingress_from)
        .ok_or_else(|| schematic_contract("ordered inner-peak transit lost Patch 58"))?;
    let owner_mask = masks
        .get(&authored.owner)
        .ok_or_else(|| schematic_contract("ordered inner-peak transit lost Patch 59"))?;
    let egress_mask = masks
        .get(&authored.egress_to)
        .ok_or_else(|| schematic_contract("ordered inner-peak transit lost Patch 36"))?;
    let runway_domain = authored.centerline.iter().copied().collect::<BTreeSet<_>>();
    let authored_grade_coords = authored
        .authored_grades
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    let has_chord = authored
        .centerline
        .iter()
        .enumerate()
        .any(|(index, coord)| {
            authored
                .centerline
                .iter()
                .skip(index.saturating_add(2))
                .any(|other| coord.distance(*other) == 1)
        });
    if authored.centerline.len() < 2
        || runway_domain.len() != authored.centerline.len()
        || authored.centerline.windows(2).any(|pair| {
            let [first, second] = pair else {
                return true;
            };
            first.distance(*second) != 1
        })
        || has_chord
        || authored_grade_coords != runway_domain
        || authored.centerline.windows(2).any(|pair| {
            let [first, second] = pair else {
                return true;
            };
            authored
                .authored_grades
                .get(first)
                .zip(authored.authored_grades.get(second))
                .is_none_or(|(first_level, second_level)| first_level.abs_diff(*second_level) > 1)
        })
        || runway_domain.iter().any(|coord| {
            !owner_mask.contains(coord)
                || !route_search_allowed.contains(coord)
                || route_bounds.get(coord).is_none_or(|(minimum, maximum)| {
                    authored
                        .authored_grades
                        .get(coord)
                        .is_none_or(|level| !(*minimum..=*maximum).contains(level))
                })
        })
    {
        let excluded = runway_domain
            .iter()
            .filter(|coord| {
                !owner_mask.contains(coord)
                    || !route_search_allowed.contains(coord)
                    || !route_bounds.contains_key(coord)
            })
            .copied()
            .collect::<Vec<_>>();
        return Err(schematic_contract(format!(
            "ordered inner-peak transit spine is not one induced admitted graded Patch-59 path: length={}, unique={}, grades={}, excluded={excluded:?}",
            authored.centerline.len(),
            runway_domain.len(),
            authored_grade_coords.len()
        )));
    }
    let compatible = |portal: BoundaryPortal| {
        route_search_allowed.contains(&portal.from)
            && route_search_allowed.contains(&portal.to)
            && route_bounds
                .get(&portal.from)
                .copied()
                .zip(route_bounds.get(&portal.to).copied())
                .is_some_and(|((from_minimum, from_maximum), (to_minimum, to_maximum))| {
                    from_minimum <= to_maximum.saturating_add(1)
                        && to_minimum <= from_maximum.saturating_add(1)
                })
    };
    let first = authored
        .centerline
        .first()
        .copied()
        .ok_or_else(|| schematic_contract("ordered inner-peak transit lost its ingress"))?;
    let last = *authored
        .centerline
        .last()
        .ok_or_else(|| schematic_contract("ordered inner-peak transit lost its egress"))?;
    let ingress = authored
        .ingress_portals
        .iter()
        .filter_map(|(from, to)| {
            let portal = BoundaryPortal {
                from: *from,
                to: *to,
            };
            (*to == first
                && ingress_mask.contains(from)
                && owner_mask.contains(to)
                && from.distance(*to) == 1
                && compatible(portal))
            .then_some(portal)
        })
        .collect::<BTreeSet<_>>();
    let egress = authored
        .egress_portals
        .iter()
        .filter_map(|(from, to)| {
            let portal = BoundaryPortal {
                from: *from,
                to: *to,
            };
            (*from == last
                && owner_mask.contains(from)
                && egress_mask.contains(to)
                && from.distance(*to) == 1
                && compatible(portal))
            .then_some(portal)
        })
        .collect::<BTreeSet<_>>();
    if ingress.is_empty() || egress.is_empty() {
        return Err(schematic_contract(format!(
            "ordered inner-peak transit spine has no grade-compatible typed portal: ingress={}, egress={}",
            ingress.len(),
            egress.len()
        )));
    }
    let mut suffix_reachability = inner_peak_suffix_reachability(
        suffix_sequence,
        masks,
        route_search_allowed,
        route_bounds,
        &runway_domain,
        &egress,
        INNER_PEAK_SUFFIX_STATE_LIMIT,
    )
    .map_err(schematic_contract)?;
    if let InnerPeakSuffixReachability::Complete { diagnostic, .. } = &mut suffix_reachability {
        let runway_bounds = authored
            .centerline
            .iter()
            .map(|coord| (*coord, route_bounds.get(coord).copied()))
            .collect::<Vec<_>>();
        diagnostic.push_str(&format!("; ordered-runway-bounds={runway_bounds:?}"));
    }
    if matches!(
        &suffix_reachability,
        InnerPeakSuffixReachability::Complete {
            distance_by_entry,
            ..
        } if distance_by_entry.is_empty()
    ) {
        let alternate_egress = owner_mask
            .iter()
            .flat_map(|from| {
                from.neighbors()
                    .into_iter()
                    .filter(|to| egress_mask.contains(to))
                    .map(|to| BoundaryPortal { from: *from, to })
            })
            .filter(|portal| compatible(*portal))
            .collect::<BTreeSet<_>>();
        let alternate_runway = owner_mask
            .intersection(route_search_allowed)
            .copied()
            .collect::<BTreeSet<_>>();
        let alternate_reachability = inner_peak_suffix_reachability(
            suffix_sequence,
            masks,
            route_search_allowed,
            route_bounds,
            &alternate_runway,
            &alternate_egress,
            INNER_PEAK_SUFFIX_STATE_LIMIT,
        )
        .map_err(schematic_contract)?;
        let alternate_diagnostic = match &alternate_reachability {
            InnerPeakSuffixReachability::Complete {
                distance_by_entry,
                explored_states,
                possible_states,
                diagnostic,
                ..
            } => {
                let reachable_portals = alternate_egress
                    .iter()
                    .filter_map(|portal| {
                        let levels = distance_by_entry
                            .keys()
                            .filter(|position| position.coord == portal.from)
                            .map(|position| position.level)
                            .collect::<BTreeSet<_>>();
                        (!levels.is_empty()).then_some((
                            *portal,
                            route_bounds.get(&portal.from).copied(),
                            route_bounds.get(&portal.to).copied(),
                            levels.first().copied(),
                            levels.last().copied(),
                            levels.len(),
                        ))
                    })
                    .take(24)
                    .collect::<Vec<_>>();
                format!(
                    "complete explored={explored_states}/{possible_states}, entries={}, reachable-portals={reachable_portals:?}; {diagnostic}",
                    distance_by_entry.len()
                )
            }
            InnerPeakSuffixReachability::Incomplete {
                possible_states,
                state_limit,
                ..
            } => format!("incomplete possible={possible_states}, state-limit={state_limit}"),
        };
        return Err(schematic_contract(format!(
            "ordered inner-peak egress is suffix-dead; authored={:?}; alternate-egress-count={}; alternate-runway={}; alternate={alternate_diagnostic}",
            authored.egress_portals,
            alternate_egress.len(),
            alternate_runway.len()
        )));
    }
    Ok(InnerPeakTransitAuthority {
        runway_domain,
        ingress,
        egress,
        suffix_reachability: std::sync::Arc::new(suffix_reachability),
        ordered_runway: Some(
            authored
                .centerline
                .iter()
                .map(|coord| {
                    authored
                        .authored_grades
                        .get(coord)
                        .copied()
                        .map(|level| TilePos::new(*coord, level))
                        .ok_or_else(|| {
                            schematic_contract(format!(
                                "ordered inner-peak transit lost its authored grade at {coord:?}"
                            ))
                        })
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
    })
}

struct BoundaryDiagnosticRequest<'a> {
    from_id: u16,
    to_id: u16,
    previous_id: Option<u16>,
    junctions: &'a [TilePos],
    component: Option<&'a super::super::schematic_highlands::PeakRidgeComponentAuthority>,
}

fn boundary_route_diagnostic(
    request: BoundaryDiagnosticRequest<'_>,
    masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    centerline_allowed: &BTreeSet<HexCoord>,
    volume: &VolumePlan,
    authority: &ShoulderAuthorityDiagnostics<'_>,
) -> String {
    let BoundaryDiagnosticRequest {
        from_id,
        to_id,
        previous_id,
        junctions,
        component,
    } = request;
    let Some(from_mask) = masks.get(&PatchId(u32::from(from_id))) else {
        return format!("{from_id}->{to_id} missing source mask");
    };
    let Some(to_mask) = masks.get(&PatchId(u32::from(to_id))) else {
        return format!("{from_id}->{to_id} missing destination mask");
    };
    let raw = from_mask
        .iter()
        .flat_map(|from| {
            from.neighbors()
                .into_iter()
                .filter(|to| to_mask.contains(to))
                .map(|to| BoundaryPortal { from: *from, to })
        })
        .collect::<BTreeSet<_>>();
    let admitted = raw
        .iter()
        .filter(|portal| {
            centerline_allowed.contains(&portal.from) && centerline_allowed.contains(&portal.to)
        })
        .copied()
        .collect::<Vec<_>>();
    let level_rows = raw
        .iter()
        .filter_map(|portal| {
            let from_level = volume
                .top_surface_at_coord(portal.from)
                .map(|(surface, _)| surface.level)?;
            let to_level = volume
                .top_surface_at_coord(portal.to)
                .map(|(surface, _)| surface.level)?;
            Some((portal.from, from_level, portal.to, to_level))
        })
        .collect::<Vec<_>>();
    let minimum_step = level_rows
        .iter()
        .map(|(_, from, _, to)| from.abs_diff(*to))
        .min();
    let excluded = raw
        .iter()
        .flat_map(|portal| [portal.from, portal.to])
        .filter(|coord| !centerline_allowed.contains(coord))
        .collect::<BTreeSet<_>>();
    let excluded_summary = excluded
        .iter()
        .take(12)
        .map(|coord| authority.coord(*coord, volume))
        .collect::<Vec<_>>();
    let reachability = junctions
        .iter()
        .map(|junction| {
            let mut allowed = from_mask
                .intersection(centerline_allowed)
                .copied()
                .collect::<BTreeSet<_>>();
            allowed.insert(junction.coord);
            let reachable = coord_distances(junction.coord, &allowed)
                .map(|distances| {
                    raw.iter()
                        .filter(|portal| {
                            distances.contains_key(&portal.from)
                                && centerline_allowed.contains(&portal.to)
                        })
                        .count()
                })
                .unwrap_or_default();
            (*junction, reachable)
        })
        .collect::<Vec<_>>();
    let admitted_from_mask = from_mask
        .intersection(centerline_allowed)
        .copied()
        .collect::<BTreeSet<_>>();
    let components = coord_components(&admitted_from_mask);
    let outgoing_components = components
        .iter()
        .enumerate()
        .filter_map(|(index, component)| {
            raw.iter()
                .any(|portal| component.contains(&portal.from))
                .then_some(index)
        })
        .collect::<BTreeSet<_>>();
    let incoming_components = previous_id
        .and_then(|previous_id| masks.get(&PatchId(u32::from(previous_id))))
        .map(|previous_mask| {
            let incoming = previous_mask
                .iter()
                .flat_map(|from| {
                    from.neighbors()
                        .into_iter()
                        .filter(|to| from_mask.contains(to))
                })
                .collect::<BTreeSet<_>>();
            components
                .iter()
                .enumerate()
                .filter_map(|(index, component)| {
                    incoming
                        .iter()
                        .any(|coord| component.contains(coord))
                        .then_some(index)
                })
                .collect::<BTreeSet<_>>()
        });
    let excluded_from_summary = from_mask
        .difference(centerline_allowed)
        .take(12)
        .map(|coord| {
            let surface = volume.top_surface_at_coord(*coord);
            let neighbor_levels = coord
                .neighbors()
                .into_iter()
                .filter_map(|neighbor| {
                    (!authority.mutable_allowed.contains(&neighbor)).then(|| {
                        volume
                            .top_surface_at_coord(neighbor)
                            .map(|(surface, _)| (neighbor, surface.level))
                    })?
                })
                .collect::<Vec<_>>();
            format!(
                "{coord:?}@{:?} access={:?} mutable={} immutable-neighbor-levels={neighbor_levels:?}",
                surface.map(|(surface, _)| surface.level),
                surface.map(|(_, metadata)| metadata.access),
                authority.mutable_allowed.contains(coord),
            )
        })
        .collect::<Vec<_>>();
    let blocked_junction_neighbors = junctions
        .iter()
        .filter_map(|junction| {
            let details = junction
                .coord
                .neighbors()
                .into_iter()
                .filter(|neighbor| from_mask.contains(neighbor))
                .map(|neighbor| {
                    if centerline_allowed.contains(&neighbor) {
                        format!("{neighbor:?}=admitted")
                    } else {
                        authority.coord(neighbor, volume)
                    }
                })
                .collect::<Vec<_>>();
            (!details.is_empty()).then_some((*junction, details))
        })
        .collect::<Vec<_>>();
    let saddle = component.and_then(|component| {
        let key = if from_id < to_id {
            (PatchId(u32::from(from_id)), PatchId(u32::from(to_id)))
        } else {
            (PatchId(u32::from(to_id)), PatchId(u32::from(from_id)))
        };
        component.expected_saddle_swaths.get(&key).map(|swath| {
            let below = swath
                .iter()
                .filter(|coord| {
                    volume
                        .top_surface_at_coord(**coord)
                        .is_some_and(|(surface, _)| surface.level < MAXIMUM_LEDGE_LEVEL + 1)
                })
                .copied()
                .collect::<BTreeSet<_>>();
            format!(
                "swath={} below240={} below-connected={} admitted={}",
                swath.len(),
                below.len(),
                coords_connected(&below),
                below.intersection(centerline_allowed).count(),
            )
        })
    });
    format!(
        "{from_id}->{to_id} raw={} admitted-pairs={} admitted-from={} admitted-to={} min-current-step={minimum_step:?} reachability={reachability:?} components={:?} incoming-components={incoming_components:?} outgoing-components={outgoing_components:?} junction-neighbors={blocked_junction_neighbors:?} saddle={saddle:?} seam-excluded={excluded_summary:?} from-excluded={excluded_from_summary:?}",
        raw.len(),
        admitted.len(),
        raw.iter()
            .filter(|portal| centerline_allowed.contains(&portal.from))
            .count(),
        raw.iter()
            .filter(|portal| centerline_allowed.contains(&portal.to))
            .count(),
        components.iter().map(BTreeSet::len).collect::<Vec<_>>(),
    )
}

fn coord_components(coords: &BTreeSet<HexCoord>) -> Vec<BTreeSet<HexCoord>> {
    let mut remaining = coords.clone();
    let mut components = Vec::new();
    while let Some(start) = remaining.pop_first() {
        let mut component = BTreeSet::from([start]);
        let mut frontier = VecDeque::from([start]);
        while let Some(coord) = frontier.pop_front() {
            for neighbor in coord.neighbors() {
                if remaining.remove(&neighbor) {
                    component.insert(neighbor);
                    frontier.push_back(neighbor);
                }
            }
        }
        components.push(component);
    }
    components
}

#[cfg(test)]
pub(super) fn raw_boundary_portal_count(
    masks: &BTreeMap<PatchId, super::super::layout::ResolvedPatch>,
    from_id: u16,
    to_id: u16,
) -> usize {
    let Some(from_mask) = masks
        .get(&PatchId(u32::from(from_id)))
        .map(|patch| &patch.mask)
    else {
        return 0;
    };
    let Some(to_mask) = masks
        .get(&PatchId(u32::from(to_id)))
        .map(|patch| &patch.mask)
    else {
        return 0;
    };
    from_mask
        .iter()
        .flat_map(|from| from.neighbors())
        .filter(|to| to_mask.contains(to))
        .count()
}

#[cfg(test)]
fn segmented_portal_path(
    start: HexCoord,
    masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    allowed: &BTreeSet<HexCoord>,
    sequence: &[u16],
) -> Result<Vec<HexCoord>, String> {
    segmented_portal_path_ranked(
        start,
        masks,
        allowed,
        sequence,
        &BTreeMap::new(),
        None,
        None,
    )
}

/// Resolves the fixed authored coarse sequence exactly in a finite layered
/// graph. A state is `(coarse stage, coordinate, exact feasible level)`, so a
/// geometrically short handoff cannot outrank the longer runway required to
/// climb from the Frozen plateau into a scenic saddle. Each spatial coordinate
/// has at most one state per admissible level, which bounds both work and memory
/// independently of the number of simple paths through a patch.
#[cfg(test)]
fn segmented_portal_path_ranked(
    start: HexCoord,
    masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    allowed: &BTreeSet<HexCoord>,
    sequence: &[u16],
    penalties: &BTreeMap<HexCoord, u32>,
    start_level: Option<Level>,
    bounds: Option<&BTreeMap<HexCoord, (Level, Level)>>,
) -> Result<Vec<HexCoord>, String> {
    segmented_portal_route_ranked(
        start,
        masks,
        allowed,
        sequence,
        penalties,
        start_level,
        bounds,
    )
    .map(|route| route.into_iter().map(|position| position.coord).collect())
}

/// Production form of [`segmented_portal_path_ranked`]. Retaining the exact
/// level chosen for each coordinate is essential: those levels are the proof
/// that the physical runway satisfies every local bound. Re-grading only the
/// returned coordinates can select a different, support-incompatible profile.
fn segmented_portal_route_ranked(
    start: HexCoord,
    masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    allowed: &BTreeSet<HexCoord>,
    sequence: &[u16],
    penalties: &BTreeMap<HexCoord, u32>,
    start_level: Option<Level>,
    bounds: Option<&BTreeMap<HexCoord, (Level, Level)>>,
) -> Result<Vec<TilePos>, String> {
    segmented_portal_route_ranked_with_transit(
        start,
        masks,
        allowed,
        sequence,
        penalties,
        start_level,
        bounds,
        None,
    )
}

/// Resolves the authored inner-peak chain across its typed Patch-59 transit
/// boundary without carrying every Frozen-prefix history through the broad
/// climbing runway. The prefix must finish by taking one exact 58->59 portal;
/// the suffix starts from that exact position and level, must leave through an
/// exact 59->36 portal, and then completes the remaining authored chain. The
/// two independently bounded proofs are accepted only when their stitched
/// centerline remains one coordinate-simple, adjacent, one-level route.
#[expect(
    clippy::too_many_arguments,
    reason = "the split solver receives the complete immutable route contract explicitly"
)]
fn segmented_inner_peak_route_ranked(
    start: HexCoord,
    masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    allowed: &BTreeSet<HexCoord>,
    sequence: &[u16],
    penalties: &BTreeMap<HexCoord, u32>,
    start_level: Level,
    bounds: &BTreeMap<HexCoord, (Level, Level)>,
    transit: &InnerPeakTransitAuthority,
    search_budget: &mut InnerPeakTransitSearchBudget,
) -> Result<Vec<TilePos>, String> {
    if sequence.iter().copied().collect::<BTreeSet<_>>().len() != sequence.len() {
        return Err(format!(
            "inner peak route sequence repeats an authored patch: {sequence:?}"
        ));
    }
    let transit_stage = sequence
        .iter()
        .position(|id| u32::from(*id) == INNER_PEAK_TRANSIT_PATCH.0)
        .ok_or_else(|| "inner peak route sequence omitted its Patch-59 transit stage".to_owned())?;
    if transit_stage == 0
        || transit_stage.saturating_add(1) >= sequence.len()
        || sequence.get(transit_stage.saturating_sub(1)).copied() != Some(58)
        || sequence.get(transit_stage.saturating_add(1)).copied() != Some(36)
    {
        return Err(format!(
            "inner peak route sequence does not retain the exact 58->59->36 transit contract: {sequence:?}"
        ));
    }
    let prefix_sequence = sequence
        .get(..=transit_stage)
        .ok_or_else(|| "inner peak transit prefix is outside the authored sequence".to_owned())?;
    let suffix_sequence = sequence
        .get(transit_stage..)
        .ok_or_else(|| "inner peak transit suffix is outside the authored sequence".to_owned())?;
    if prefix_sequence.len() < 2 || suffix_sequence.len() < 2 {
        return Err("inner peak transit split produced an empty prefix or suffix".to_owned());
    }

    let egress_portals = transit.egress.iter().copied().collect::<Vec<_>>();
    let (suffix_distance_by_entry, suffix_reachability_diagnostic) = match transit
        .suffix_reachability
        .as_ref()
    {
        InnerPeakSuffixReachability::Complete {
            sequence: indexed_sequence,
            distance_by_entry,
            explored_states,
            possible_states,
            diagnostic,
        } if indexed_sequence.as_slice() == suffix_sequence => (
            Some(distance_by_entry),
            format!(
                "complete explored={explored_states}/{possible_states} reachable-entry-states={}; {diagnostic}",
                distance_by_entry.len(),
            ),
        ),
        InnerPeakSuffixReachability::Complete {
            sequence: indexed_sequence,
            ..
        } => {
            search_budget.saw_incomplete_search = true;
            (
                None,
                format!(
                    "incomplete indexed-sequence={indexed_sequence:?} requested={suffix_sequence:?}; filtering-disabled"
                ),
            )
        }
        InnerPeakSuffixReachability::Incomplete {
            sequence: indexed_sequence,
            possible_states,
            state_limit,
        } => {
            search_budget.saw_incomplete_search = true;
            (
                None,
                format!(
                    "incomplete sequence={indexed_sequence:?} possible={possible_states} state-limit={state_limit}; filtering-disabled"
                ),
            )
        }
    };
    let mut raw_handoff_count = 0_usize;
    let mut handoffs_by_ingress = Vec::new();
    for ingress in transit.ingress.iter().copied() {
        let Some((from_minimum, from_maximum)) = bounds.get(&ingress.from).copied() else {
            handoffs_by_ingress.push(Vec::new());
            continue;
        };
        let Some((to_minimum, to_maximum)) = bounds.get(&ingress.to).copied() else {
            handoffs_by_ingress.push(Vec::new());
            continue;
        };
        let lowest = to_minimum.max(from_minimum.saturating_sub(1));
        let highest = to_maximum.min(from_maximum.saturating_add(1));
        if lowest > highest {
            handoffs_by_ingress.push(Vec::new());
            continue;
        }
        let mut ingress_handoffs = Vec::new();
        for level in lowest..=highest {
            raw_handoff_count = raw_handoff_count.saturating_add(1);
            let entry = TilePos::new(ingress.to, level);
            let suffix_distance = match suffix_distance_by_entry {
                Some(distances) => distances.get(&entry).copied(),
                None => Some(u32::MAX),
            };
            let Some(suffix_distance) = suffix_distance else {
                continue;
            };
            let runway_lower_bound = recovery_portal_lower_bound(entry, &egress_portals, bounds);
            ingress_handoffs.push((suffix_distance, runway_lower_bound, ingress, level));
        }
        handoffs_by_ingress.push(ingress_handoffs);
    }
    for handoffs in &mut handoffs_by_ingress {
        handoffs.sort_unstable();
    }
    handoffs_by_ingress.sort_unstable_by_key(|handoffs| handoffs.first().copied());
    let mut handoffs = Vec::new();
    let maximum_levels = handoffs_by_ingress
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or_default();
    for level_index in 0..maximum_levels {
        for ingress_handoffs in &handoffs_by_ingress {
            if let Some(handoff) = ingress_handoffs.get(level_index).copied() {
                handoffs.push(handoff);
            }
        }
    }
    let suffix_viable_handoff_count = handoffs.len();
    let local_limit = search_budget
        .remaining_handoffs
        .min(INNER_PEAK_TRANSIT_LOCAL_HANDOFF_LIMIT);
    if handoffs.len() > local_limit {
        search_budget.saw_incomplete_search = true;
        handoffs.truncate(local_limit);
    }

    let mut diagnostics = Vec::new();
    for (_, _, ingress, ingress_level) in handoffs.iter().copied() {
        search_budget.remaining_handoffs = search_budget.remaining_handoffs.saturating_sub(1);
        let exact_transit = InnerPeakTransitAuthority {
            runway_domain: transit.runway_domain.clone(),
            ingress: BTreeSet::from([ingress]),
            egress: transit.egress.clone(),
            suffix_reachability: transit.suffix_reachability.clone(),
            ordered_runway: transit.ordered_runway.clone(),
        };
        let mut exact_bounds = bounds.clone();
        exact_bounds.insert(ingress.to, (ingress_level, ingress_level));
        let handoff = TilePos::new(ingress.to, ingress_level);
        // The reverse index is a necessary-condition filter, not admission.
        // Prove its physical coordinate-simple suffix before spending work on
        // a junction-specific Frozen prefix.
        let suffix = match segmented_portal_route_ranked_with_transit_budgeted(
            handoff.coord,
            masks,
            allowed,
            suffix_sequence,
            penalties,
            Some(handoff.level),
            Some(&exact_bounds),
            Some(&exact_transit),
            &mut search_budget.remaining_recovery_work,
            &mut search_budget.saw_incomplete_search,
        ) {
            Ok(suffix) => suffix,
            Err(diagnostic) => {
                if diagnostics.len() < 8 {
                    diagnostics.push(format!(
                        "ingress {ingress:?}@{ingress_level} suffix: {}",
                        diagnostic.chars().take(768).collect::<String>()
                    ));
                }
                continue;
            }
        };
        if suffix.first().copied() != Some(handoff) {
            if diagnostics.len() < 8 {
                diagnostics.push(format!(
                    "ingress {ingress:?}@{ingress_level} suffix did not retain its exact entry state"
                ));
            }
            continue;
        }

        let prefix = match segmented_portal_route_ranked_with_transit_budgeted(
            start,
            masks,
            allowed,
            prefix_sequence,
            penalties,
            Some(start_level),
            Some(&exact_bounds),
            Some(&exact_transit),
            &mut search_budget.remaining_recovery_work,
            &mut search_budget.saw_incomplete_search,
        ) {
            Ok(prefix) => prefix,
            Err(diagnostic) => {
                if diagnostics.len() < 8 {
                    diagnostics.push(format!(
                        "ingress {ingress:?}@{ingress_level} prefix: {}",
                        diagnostic.chars().take(768).collect::<String>()
                    ));
                }
                continue;
            }
        };
        let Some(prefix_handoff) = prefix.last().copied() else {
            if diagnostics.len() < 8 {
                diagnostics.push(format!("ingress {ingress:?} produced an empty prefix"));
            }
            continue;
        };
        if prefix_handoff != handoff
            || !prefix.windows(2).any(|pair| {
                let [first, second] = pair else {
                    return false;
                };
                first.coord == ingress.from && second.coord == ingress.to
            })
        {
            if diagnostics.len() < 8 {
                diagnostics.push(format!(
                    "ingress {ingress:?}@{ingress_level} prefix ended at {prefix_handoff:?} without retaining its exact portal and level"
                ));
            }
            continue;
        }
        let route = prefix
            .into_iter()
            .chain(suffix.into_iter().skip(1))
            .collect::<Vec<_>>();
        let coordinates = route
            .iter()
            .map(|position| position.coord)
            .collect::<BTreeSet<_>>();
        if coordinates.len() != route.len()
            || route.windows(2).any(|pair| {
                let [first, second] = pair else {
                    return true;
                };
                first.coord.distance(second.coord) != 1 || first.level.abs_diff(second.level) > 1
            })
        {
            if diagnostics.len() < 8 {
                diagnostics.push(format!(
                    "ingress {ingress:?} produced a repeated, disjoint, or over-steep stitched route"
                ));
            }
            continue;
        }
        if let Err(diagnostic) = exact_transit.validate_route(&route, masks) {
            if diagnostics.len() < 8 {
                diagnostics.push(format!(
                    "ingress {ingress:?} failed stitched transit admission: {diagnostic}"
                ));
            }
            continue;
        }
        return Ok(route);
    }

    let search_incomplete = search_budget.saw_incomplete_search
        || handoffs.len() < suffix_viable_handoff_count
        || search_budget.remaining_handoffs == 0
        || search_budget.remaining_recovery_work == 0;
    Err(format!(
        "inner peak split transit exhausted {} of {} suffix-viable exact ingress-level contracts ({raw_handoff_count} raw; suffix-reachability={suffix_reachability_diagnostic}) without a valid prefix/runway/suffix route; search-incomplete={search_incomplete}, global-handoffs-remaining={}, global-recovery-work-remaining={}: {}",
        handoffs.len(),
        suffix_viable_handoff_count,
        search_budget.remaining_handoffs,
        search_budget.remaining_recovery_work,
        diagnostics.join("; ")
    ))
}

#[expect(
    clippy::too_many_arguments,
    reason = "the optional typed transit authority is an independent route-admission input"
)]
fn segmented_portal_route_ranked_with_transit(
    start: HexCoord,
    masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    allowed: &BTreeSet<HexCoord>,
    sequence: &[u16],
    penalties: &BTreeMap<HexCoord, u32>,
    start_level: Option<Level>,
    bounds: Option<&BTreeMap<HexCoord, (Level, Level)>>,
    transit: Option<&InnerPeakTransitAuthority>,
) -> Result<Vec<TilePos>, String> {
    let mut recovery_work_remaining = EXACT_RECOVERY_WORK_BUDGET;
    let mut search_incomplete = false;
    segmented_portal_route_ranked_with_transit_budgeted(
        start,
        masks,
        allowed,
        sequence,
        penalties,
        start_level,
        bounds,
        transit,
        &mut recovery_work_remaining,
        &mut search_incomplete,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the route search receives its immutable contract and shared recovery meter explicitly"
)]
fn segmented_portal_route_ranked_with_transit_budgeted(
    start: HexCoord,
    masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    allowed: &BTreeSet<HexCoord>,
    sequence: &[u16],
    penalties: &BTreeMap<HexCoord, u32>,
    start_level: Option<Level>,
    bounds: Option<&BTreeMap<HexCoord, (Level, Level)>>,
    transit: Option<&InnerPeakTransitAuthority>,
    recovery_work_remaining: &mut usize,
    search_incomplete: &mut bool,
) -> Result<Vec<TilePos>, String> {
    let Some(first) = sequence.first().copied() else {
        return Err("portal route has no authored coarse-cell sequence".to_owned());
    };
    if sequence.len() < 2
        || sequence.iter().copied().collect::<BTreeSet<_>>().len() != sequence.len()
    {
        return Err(format!(
            "portal route sequence must contain at least two unique cells: {sequence:?}"
        ));
    }
    let mut allowed_by_patch = BTreeMap::new();
    for id in sequence {
        let patch = masks
            .get(&PatchId(u32::from(*id)))
            .ok_or_else(|| format!("portal route lost resolved patch {id}"))?;
        let admitted = patch
            .intersection(allowed)
            .copied()
            .collect::<BTreeSet<_>>();
        if admitted.is_empty() {
            return Err(format!("portal route cell {id} has no admitted coordinate"));
        }
        allowed_by_patch.insert(*id, admitted);
    }
    let mut claimed = BTreeSet::new();
    for id in sequence {
        let admitted = allowed_by_patch
            .get(id)
            .ok_or_else(|| format!("portal route lost admitted cell {id}"))?;
        if !claimed.is_disjoint(admitted) {
            return Err(format!(
                "portal route requires pairwise-disjoint admitted cell masks; cell {id} overlaps an earlier stage"
            ));
        }
        claimed.extend(admitted.iter().copied());
    }
    if allowed_by_patch
        .get(&first)
        .is_none_or(|first_allowed| !first_allowed.contains(&start))
    {
        return Err(format!(
            "portal route start {start:?} is not admitted by first cell {first}"
        ));
    }

    let mut portals: Vec<Vec<BoundaryPortal>> =
        Vec::with_capacity(sequence.len().saturating_sub(1));
    for pair in sequence.windows(2) {
        let [from_id, to_id] = pair else {
            continue;
        };
        let from_allowed = allowed_by_patch
            .get(from_id)
            .ok_or_else(|| format!("portal route lost admitted cell {from_id}"))?;
        let to_allowed = allowed_by_patch
            .get(to_id)
            .ok_or_else(|| format!("portal route lost admitted cell {to_id}"))?;
        let mut edge_portals = BTreeSet::new();
        for from in from_allowed {
            for to in from.neighbors() {
                if to_allowed.contains(&to) {
                    edge_portals.insert(BoundaryPortal { from: *from, to });
                }
            }
        }
        if let Some(transit) = transit {
            edge_portals.retain(|portal| transit.permits_portal(*from_id, *to_id, *portal));
        }
        if edge_portals.is_empty() {
            let from_mask = masks
                .get(&PatchId(u32::from(*from_id)))
                .ok_or_else(|| format!("portal route lost raw source mask {from_id}"))?;
            let to_mask = masks
                .get(&PatchId(u32::from(*to_id)))
                .ok_or_else(|| format!("portal route lost raw destination mask {to_id}"))?;
            let raw_portals = from_mask
                .iter()
                .flat_map(|from| {
                    from.neighbors()
                        .into_iter()
                        .filter(|to| to_mask.contains(to))
                        .map(|to| BoundaryPortal { from: *from, to })
                })
                .collect::<BTreeSet<_>>();
            let admitted_from = raw_portals
                .iter()
                .filter(|portal| from_allowed.contains(&portal.from))
                .count();
            let admitted_to = raw_portals
                .iter()
                .filter(|portal| to_allowed.contains(&portal.to))
                .count();
            return Err(format!(
                "portal route has no exact admitted boundary handoff {from_id}->{to_id}; raw-portals={}, admitted-from={admitted_from}, admitted-to={admitted_to}",
                raw_portals.len()
            ));
        }
        portals.push(edge_portals.into_iter().collect());
    }

    let geometric_only = start_level.is_none() && bounds.is_none();
    if !geometric_only && (start_level.is_none() || bounds.is_none()) {
        return Err(
            "portal route elevation search requires both an exact start level and coordinate bounds"
                .to_owned(),
        );
    }
    let initial_level = start_level.unwrap_or(0);
    let level_bounds = |coord: HexCoord| -> Result<(Level, Level), String> {
        if geometric_only {
            return Ok((0, 0));
        }
        let (minimum, maximum) = match bounds.and_then(|bounds| bounds.get(&coord).copied()) {
            Some(bounds) => bounds,
            None if coord == start => (initial_level, initial_level),
            None => {
                return Err(format!(
                    "portal route has no exact elevation bounds at {coord:?}"
                ));
            }
        };
        if minimum > maximum {
            return Err(format!(
                "portal route has inverted elevation bounds at {coord:?}: {minimum}..={maximum}"
            ));
        }
        Ok((minimum, maximum))
    };
    for coord in allowed_by_patch
        .values()
        .flat_map(|coords| coords.iter().copied())
    {
        let _ = level_bounds(coord)?;
    }

    let (start_minimum, start_maximum) = level_bounds(start)?;
    if !(start_minimum..=start_maximum).contains(&initial_level) {
        return Err(format!(
            "portal route start {start:?} level {initial_level} is outside exact bounds {start_minimum}..={start_maximum}"
        ));
    }

    let start_key = ExactRouteKey {
        stage: 0,
        coord: start,
        level: initial_level,
    };
    let mut labels = BTreeMap::from([(
        start_key,
        ExactRouteLabel {
            score: 0,
            steps: 0,
            predecessor: None,
        },
    )]);
    let mut frontier = BTreeSet::from([(0_u64, 0_u32, start_key)]);
    // A popped label is final under the `(penalty, steps, key)` ordering: every
    // edge has a non-negative penalty and advances `steps` by one. Freezing it
    // matters for more than ordinary Dijkstra correctness here. Descendant
    // labels retain predecessor keys, so rewriting an already-expanded
    // ancestor could retroactively introduce a repeated coordinate into a
    // path which was simple when it was admitted.
    let mut settled = BTreeSet::new();
    let final_stage = sequence.len().saturating_sub(1);
    let mut repeated_goal_walks = 0_usize;

    while let Some((score, steps, key)) = frontier.pop_first() {
        let Some(label) = labels.get(&key) else {
            continue;
        };
        if (label.score, label.steps) != (score, steps) {
            continue;
        }
        if !settled.insert(key) {
            continue;
        }
        if key.stage == final_stage {
            match reconstruct_exact_route(key, &labels) {
                Ok(route) => return Ok(route),
                Err(error) if error == REPEATED_COORDINATE_ROUTE => {
                    // Exact-level search may find a short walk which loops on
                    // one coordinate merely to gain altitude. That is not a
                    // physical runway. Keep the final state closed and allow
                    // the deterministic frontier to reach the next-ranked,
                    // coordinate-simple goal instead of failing the complete
                    // Frozen junction on the first artificial loop.
                    repeated_goal_walks = repeated_goal_walks.saturating_add(1);
                    continue;
                }
                Err(error) => return Err(error),
            }
        }

        let current_id = sequence
            .get(key.stage)
            .copied()
            .ok_or_else(|| "portal route lost its current coarse stage".to_owned())?;
        let current_allowed = allowed_by_patch
            .get(&current_id)
            .ok_or_else(|| format!("portal route lost admitted cell {current_id}"))?;
        let next_allowed = sequence
            .get(key.stage.saturating_add(1))
            .and_then(|id| allowed_by_patch.get(id));

        for neighbor in key.coord.neighbors() {
            let same_stage = current_allowed.contains(&neighbor);
            let forward_stage = next_allowed.is_some_and(|coords| coords.contains(&neighbor))
                && portals.get(key.stage).is_some_and(|edge_portals| {
                    edge_portals
                        .binary_search(&BoundaryPortal {
                            from: key.coord,
                            to: neighbor,
                        })
                        .is_ok()
                });
            for next_stage in [
                same_stage.then_some(key.stage),
                forward_stage.then_some(key.stage.saturating_add(1)),
            ]
            .into_iter()
            .flatten()
            {
                if exact_route_predecessors_contain_coordinate(key, neighbor, &labels)? {
                    continue;
                }
                let (minimum, maximum) = level_bounds(neighbor)?;
                let lowest = minimum.max(key.level.saturating_sub(1));
                let highest = maximum.min(key.level.saturating_add(1));
                if lowest > highest {
                    continue;
                }
                for level in lowest..=highest {
                    let next_key = ExactRouteKey {
                        stage: next_stage,
                        coord: neighbor,
                        level,
                    };
                    // Settled labels are immutable. Besides being the normal
                    // Dijkstra invariant, this keeps every predecessor chain
                    // checked above permanently coordinate-simple.
                    if settled.contains(&next_key) {
                        continue;
                    }
                    let next_score = score
                        .saturating_add(u64::from(penalties.get(&neighbor).copied().unwrap_or(0)));
                    let next_steps = steps.saturating_add(1);
                    let candidate_rank = (next_score, next_steps);
                    let improves = labels
                        .get(&next_key)
                        .is_none_or(|current| candidate_rank < (current.score, current.steps));
                    if !improves {
                        continue;
                    }
                    if let Some(previous) = labels.insert(
                        next_key,
                        ExactRouteLabel {
                            score: next_score,
                            steps: next_steps,
                            predecessor: Some(key),
                        },
                    ) {
                        frontier.remove(&(previous.score, previous.steps, next_key));
                    }
                    frontier.insert((next_score, next_steps, next_key));
                }
            }
        }
    }

    // A broad authored cell can require a deliberately winding runway before
    // one of its outgoing portals becomes elevation-feasible. The compact
    // `(stage, coord, level)` labels above cannot retain every distinct visited-
    // coordinate history for one state: the cheapest history can consume a
    // coordinate needed by the only simple continuation. Replay reached stages
    // from latest to earliest with one globally bounded history-aware search.
    // Trying earlier stages matters when the compact search entered a later
    // stage through a dead handoff. Enumerating exact handoff levels matters
    // when only a higher profile can continue through the following cell.
    // This fallback is a penalty-guided feasibility recovery, not a second
    // global optimum proof: the ordinary compact search above remains the
    // source of every minimum-ranked route it can represent.
    let recovery_diagnostic = if geometric_only {
        None
    } else {
        let exact_bounds = bounds
            .ok_or_else(|| "portal route lost elevation bounds during stage recovery".to_owned())?;
        match recover_coordinate_simple_route(
            sequence,
            &allowed_by_patch,
            &portals,
            penalties,
            exact_bounds,
            start_key,
            final_stage,
            &settled,
            &labels,
            recovery_work_remaining,
        )? {
            ExactRecoveryOutcome::Found(route) => return Ok(route),
            ExactRecoveryOutcome::SearchExhausted(stats) => {
                *search_incomplete = true;
                Some(format!("search-exhausted {stats:?}"))
            }
            ExactRecoveryOutcome::ProvenDisconnected(stats) => {
                Some(format!("proven-disconnected {stats:?}"))
            }
        }
    };

    let stage_reachability = sequence
        .iter()
        .enumerate()
        .map(|(stage, id)| {
            let states = settled
                .iter()
                .filter(|key| key.stage == stage)
                .copied()
                .collect::<Vec<_>>();
            let coordinates = states
                .iter()
                .map(|key| key.coord)
                .collect::<BTreeSet<_>>()
                .len();
            let minimum_level = states.iter().map(|key| key.level).min();
            let maximum_level = states.iter().map(|key| key.level).max();
            (*id, states.len(), coordinates, minimum_level, maximum_level)
        })
        .collect::<Vec<_>>();
    let portal_reachability = portals
        .iter()
        .enumerate()
        .map(|(stage, edge_portals)| {
            let reachable_from = edge_portals
                .iter()
                .filter(|portal| {
                    settled
                        .iter()
                        .any(|key| key.stage == stage && key.coord == portal.from)
                })
                .count();
            let elevation_feasible = edge_portals
                .iter()
                .filter(|portal| {
                    level_bounds(portal.from)
                        .ok()
                        .zip(level_bounds(portal.to).ok())
                        .is_some_and(|((from_min, from_max), (to_min, to_max))| {
                            from_min <= to_max.saturating_add(1)
                                && to_min <= from_max.saturating_add(1)
                        })
                })
                .count();
            let reachable_transitions = edge_portals
                .iter()
                .filter(|portal| {
                    let Some((to_minimum, to_maximum)) = level_bounds(portal.to).ok() else {
                        return false;
                    };
                    settled.iter().any(|key| {
                        key.stage == stage
                            && key.coord == portal.from
                            && to_minimum <= key.level.saturating_add(1)
                            && to_maximum >= key.level.saturating_sub(1)
                    })
                })
                .count();
            (
                sequence.get(stage).copied(),
                sequence.get(stage.saturating_add(1)).copied(),
                edge_portals.len(),
                reachable_from,
                elevation_feasible,
                reachable_transitions,
            )
        })
        .collect::<Vec<_>>();
    Err(format!(
        "portal route found no coordinate-simple elevation-feasible authored path across {sequence:?}; rejected-loop-goals={repeated_goal_walks}; bounded-recovery={recovery_diagnostic:?}; stage-reachability=(cell,states,coords,min,max){stage_reachability:?}; portal-reachability=(from,to,portals,reachable-from,elevation-feasible,reachable-transitions){portal_reachability:?}"
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ExactRouteKey {
    stage: usize,
    coord: HexCoord,
    level: Level,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExactRouteLabel {
    score: u64,
    steps: u32,
    predecessor: Option<ExactRouteKey>,
}

const REPEATED_COORDINATE_ROUTE: &str = "portal route's shortest elevation-feasible walk repeats a coordinate instead of using a physical runway";

fn exact_route_predecessors_contain_coordinate(
    mut current: ExactRouteKey,
    candidate: HexCoord,
    labels: &BTreeMap<ExactRouteKey, ExactRouteLabel>,
) -> Result<bool, String> {
    let mut remaining = labels.len().saturating_add(1);
    loop {
        if current.coord == candidate {
            return Ok(true);
        }
        let label = labels
            .get(&current)
            .ok_or_else(|| format!("portal route lost predecessor state at {current:?}"))?;
        let Some(previous) = label.predecessor else {
            return Ok(false);
        };
        if remaining == 0 {
            return Err("portal route predecessor graph contains a cycle".to_owned());
        }
        remaining = remaining.saturating_sub(1);
        current = previous;
    }
}

fn reconstruct_exact_route(
    mut current: ExactRouteKey,
    labels: &BTreeMap<ExactRouteKey, ExactRouteLabel>,
) -> Result<Vec<TilePos>, String> {
    let mut reversed = Vec::new();
    loop {
        reversed.push(TilePos::new(current.coord, current.level));
        let label = labels
            .get(&current)
            .ok_or_else(|| format!("portal route lost predecessor state at {current:?}"))?;
        let Some(previous) = label.predecessor else {
            break;
        };
        current = previous;
    }
    reversed.reverse();
    if reversed
        .iter()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>()
        .len()
        != reversed.len()
    {
        return Err(REPEATED_COORDINATE_ROUTE.to_owned());
    }
    Ok(reversed)
}

const EXACT_RECOVERY_WORK_BUDGET: usize = 500_000;
const EXACT_RECOVERY_EARLIER_STAGE_RESERVE: usize = 8_192;
const EXACT_RECOVERY_ENTRY_LIMIT: usize = 64;
const EXACT_RECOVERY_BEAM_WIDTH: usize = 32;
const EXACT_RECOVERY_RUNWAY_LIMIT: usize = 512;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ExactRecoveryStats {
    work_units: usize,
    neighbor_expansions: usize,
    handoffs_considered: usize,
    continuations_attempted: usize,
    stages_attempted: usize,
    entries_attempted: usize,
    entry_states_pruned: usize,
    beam_states_pruned: usize,
    runway_candidates_pruned: usize,
    stage_enumerations_truncated: usize,
    budget_exhausted: bool,
}

impl ExactRecoveryStats {
    fn absorb(&mut self, other: Self) {
        self.work_units = self.work_units.saturating_add(other.work_units);
        self.neighbor_expansions = self
            .neighbor_expansions
            .saturating_add(other.neighbor_expansions);
        self.handoffs_considered = self
            .handoffs_considered
            .saturating_add(other.handoffs_considered);
        self.continuations_attempted = self
            .continuations_attempted
            .saturating_add(other.continuations_attempted);
        self.stages_attempted = self.stages_attempted.saturating_add(other.stages_attempted);
        self.entries_attempted = self
            .entries_attempted
            .saturating_add(other.entries_attempted);
        self.entry_states_pruned = self
            .entry_states_pruned
            .saturating_add(other.entry_states_pruned);
        self.beam_states_pruned = self
            .beam_states_pruned
            .saturating_add(other.beam_states_pruned);
        self.runway_candidates_pruned = self
            .runway_candidates_pruned
            .saturating_add(other.runway_candidates_pruned);
        self.stage_enumerations_truncated = self
            .stage_enumerations_truncated
            .saturating_add(other.stage_enumerations_truncated);
        self.budget_exhausted |= other.budget_exhausted;
    }
}

struct ExactRecoverySearch {
    remaining_work: usize,
    stats: ExactRecoveryStats,
}

impl ExactRecoverySearch {
    fn new(work_budget: usize) -> Self {
        Self {
            remaining_work: work_budget,
            stats: ExactRecoveryStats::default(),
        }
    }

    fn spend_work(&mut self) -> bool {
        if self.remaining_work == 0 {
            self.stats.budget_exhausted = true;
            return false;
        }
        self.remaining_work = self.remaining_work.saturating_sub(1);
        self.stats.work_units = self.stats.work_units.saturating_add(1);
        true
    }

    fn expand_neighbor(&mut self) -> bool {
        if !self.spend_work() {
            return false;
        }
        self.stats.neighbor_expansions = self.stats.neighbor_expansions.saturating_add(1);
        true
    }

    fn consider_handoff(&mut self) -> bool {
        if !self.spend_work() {
            return false;
        }
        self.stats.handoffs_considered = self.stats.handoffs_considered.saturating_add(1);
        true
    }

    fn attempt_continuation(&mut self) -> bool {
        if !self.spend_work() {
            return false;
        }
        self.stats.continuations_attempted = self.stats.continuations_attempted.saturating_add(1);
        true
    }

    fn incomplete(&self) -> bool {
        self.stats.budget_exhausted
            || self.stats.entry_states_pruned > 0
            || self.stats.beam_states_pruned > 0
            || self.stats.runway_candidates_pruned > 0
            || self.stats.stage_enumerations_truncated > 0
    }

    fn no_route_outcome(self) -> ExactRecoveryOutcome {
        if self.incomplete() {
            ExactRecoveryOutcome::SearchExhausted(self.stats)
        } else {
            ExactRecoveryOutcome::ProvenDisconnected(self.stats)
        }
    }
}

enum ExactRecoveryOutcome {
    Found(Vec<TilePos>),
    SearchExhausted(ExactRecoveryStats),
    ProvenDisconnected(ExactRecoveryStats),
}

fn recovery_stage_work_budget(remaining_work: usize, remaining_stages: usize) -> usize {
    if remaining_work == 0 || remaining_stages == 0 {
        return 0;
    }
    let complete_reserve = EXACT_RECOVERY_EARLIER_STAGE_RESERVE.saturating_mul(remaining_stages);
    if remaining_work >= complete_reserve {
        remaining_work.saturating_sub(
            EXACT_RECOVERY_EARLIER_STAGE_RESERVE.saturating_mul(remaining_stages.saturating_sub(1)),
        )
    } else {
        remaining_work / remaining_stages
    }
}

fn recovery_runway_work_budget(remaining_work: usize, remaining_transitions: usize) -> usize {
    let continuation_reserve = EXACT_RECOVERY_EARLIER_STAGE_RESERVE
        .saturating_mul(remaining_transitions)
        .min(remaining_work / 2);
    remaining_work.saturating_sub(continuation_reserve)
}

#[expect(
    clippy::too_many_arguments,
    reason = "recovery keeps the admitted route graph and compact-search evidence explicit"
)]
fn recover_coordinate_simple_route(
    sequence: &[u16],
    allowed_by_patch: &BTreeMap<u16, BTreeSet<HexCoord>>,
    portals: &[Vec<BoundaryPortal>],
    penalties: &BTreeMap<HexCoord, u32>,
    bounds: &BTreeMap<HexCoord, (Level, Level)>,
    start_key: ExactRouteKey,
    final_stage: usize,
    settled: &BTreeSet<ExactRouteKey>,
    labels: &BTreeMap<ExactRouteKey, ExactRouteLabel>,
    recovery_work_remaining: &mut usize,
) -> Result<ExactRecoveryOutcome, String> {
    let work_budget = *recovery_work_remaining;
    let reached_stages = settled
        .iter()
        .map(|key| key.stage)
        .filter(|stage| *stage < final_stage)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    let mut aggregate = ExactRecoveryStats::default();

    for (stage_index, stage) in reached_stages.iter().copied().enumerate() {
        // A large dead handoff in the latest reached stage may consume most of
        // the work cap, but it cannot starve every earlier replay. Unused work
        // is carried forward, so the final stage still receives the remainder.
        let remaining_work = work_budget.saturating_sub(aggregate.work_units);
        let remaining_stages = reached_stages.len().saturating_sub(stage_index);
        let stage_budget = recovery_stage_work_budget(remaining_work, remaining_stages);
        let mut search = ExactRecoverySearch::new(stage_budget);
        search.stats.stages_attempted = 1;
        let mut entry_keys = if stage == 0 {
            vec![start_key]
        } else {
            settled
                .iter()
                .copied()
                .filter(|key| {
                    key.stage == stage
                        && labels
                            .get(key)
                            .and_then(|label| label.predecessor)
                            .is_some_and(|previous| previous.stage < stage)
                })
                .collect::<Vec<_>>()
        };
        let stage_portals = portals
            .get(stage)
            .ok_or_else(|| "portal route lost exact recovery portals".to_owned())?;
        entry_keys.sort_unstable_by_key(|key| {
            let portal_lower_bound = recovery_portal_lower_bound(
                TilePos::new(key.coord, key.level),
                stage_portals,
                bounds,
            );
            labels
                .get(key)
                .map(|label| (portal_lower_bound, label.score, label.steps, *key))
                .unwrap_or((portal_lower_bound, u64::MAX, u32::MAX, *key))
        });
        if entry_keys.len() > EXACT_RECOVERY_ENTRY_LIMIT {
            search.stats.entry_states_pruned = search
                .stats
                .entry_states_pruned
                .saturating_add(entry_keys.len().saturating_sub(EXACT_RECOVERY_ENTRY_LIMIT));
            entry_keys.truncate(EXACT_RECOVERY_ENTRY_LIMIT);
        }

        'entries: for entry in entry_keys {
            search.stats.entries_attempted = search.stats.entries_attempted.saturating_add(1);
            let prefix = reconstruct_exact_route(entry, labels)?;
            let prefix_coords = prefix
                .iter()
                .map(|position| position.coord)
                .collect::<BTreeSet<_>>();
            let Some(suffix) = recover_exact_stage_suffix(
                TilePos::new(entry.coord, entry.level),
                stage,
                final_stage,
                sequence,
                allowed_by_patch,
                portals,
                penalties,
                bounds,
                &prefix_coords,
                &mut search,
            ) else {
                if search.stats.budget_exhausted {
                    break 'entries;
                }
                continue;
            };
            let recovered = prefix
                .into_iter()
                .chain(suffix.into_iter().skip(1))
                .collect::<Vec<_>>();
            if exact_recovered_route_is_valid(
                &recovered,
                sequence,
                allowed_by_patch,
                bounds,
                final_stage,
            ) {
                aggregate.absorb(search.stats);
                *recovery_work_remaining =
                    (*recovery_work_remaining).saturating_sub(aggregate.work_units);
                return Ok(ExactRecoveryOutcome::Found(recovered));
            }
        }
        aggregate.absorb(search.stats);
    }

    *recovery_work_remaining = (*recovery_work_remaining).saturating_sub(aggregate.work_units);
    Ok(ExactRecoverySearch {
        remaining_work: work_budget.saturating_sub(aggregate.work_units),
        stats: aggregate,
    }
    .no_route_outcome())
}

#[expect(
    clippy::too_many_arguments,
    reason = "recursive recovery carries one explicit immutable routing contract and shared budget"
)]
fn recover_exact_stage_suffix(
    start: TilePos,
    stage: usize,
    final_stage: usize,
    sequence: &[u16],
    allowed_by_patch: &BTreeMap<u16, BTreeSet<HexCoord>>,
    portals: &[Vec<BoundaryPortal>],
    penalties: &BTreeMap<HexCoord, u32>,
    bounds: &BTreeMap<HexCoord, (Level, Level)>,
    forbidden: &BTreeSet<HexCoord>,
    search: &mut ExactRecoverySearch,
) -> Option<Vec<TilePos>> {
    if stage == final_stage {
        return Some(vec![start]);
    }
    if search.stats.budget_exhausted {
        return None;
    }
    let current_id = sequence.get(stage).copied()?;
    let stage_allowed = allowed_by_patch.get(&current_id)?;
    let stage_portals = portals.get(stage)?;
    // Enumerating every self-avoiding runway can consume the complete shared
    // cap before one already-discovered handoff is tried. Reserve bounded work
    // for every downstream transition; a truncated enumeration remains an
    // honest SearchExhausted result if none of those continuations succeeds.
    let runway_work_budget =
        recovery_runway_work_budget(search.remaining_work, final_stage.saturating_sub(stage));
    let mut runways = exact_simple_stage_runways(
        start,
        stage_allowed,
        stage_portals,
        bounds,
        forbidden,
        penalties,
        runway_work_budget,
        search,
    );
    runways.sort_unstable_by_key(|runway| {
        let handoff = runway.last().copied().unwrap_or(start);
        let penalty = runway.iter().fold(0_u64, |score, position| {
            score.saturating_add(u64::from(
                penalties.get(&position.coord).copied().unwrap_or(0),
            ))
        });
        let continuation = portals
            .get(stage.saturating_add(1))
            .map_or((0, 0, 0), |next_portals| {
                recovery_portal_lower_bound(handoff, next_portals, bounds)
            });
        (continuation, penalty, runway.len(), handoff)
    });
    if runways.len() > EXACT_RECOVERY_RUNWAY_LIMIT {
        search.stats.runway_candidates_pruned = search
            .stats
            .runway_candidates_pruned
            .saturating_add(runways.len().saturating_sub(EXACT_RECOVERY_RUNWAY_LIMIT));
        runways.truncate(EXACT_RECOVERY_RUNWAY_LIMIT);
    }

    for runway in runways {
        if !search.attempt_continuation() {
            return None;
        }
        let handoff = runway.last().copied()?;
        let mut next_forbidden = forbidden.clone();
        next_forbidden.extend(runway.iter().map(|position| position.coord));
        let Some(suffix) = recover_exact_stage_suffix(
            handoff,
            stage.saturating_add(1),
            final_stage,
            sequence,
            allowed_by_patch,
            portals,
            penalties,
            bounds,
            &next_forbidden,
            search,
        ) else {
            if search.stats.budget_exhausted {
                return None;
            }
            continue;
        };
        return Some(
            runway
                .into_iter()
                .chain(suffix.into_iter().skip(1))
                .collect(),
        );
    }
    None
}

fn recovery_portal_lower_bound(
    start: TilePos,
    portals: &[BoundaryPortal],
    bounds: &BTreeMap<HexCoord, (Level, Level)>,
) -> (u32, u32, u32) {
    portals
        .iter()
        .filter_map(|portal| {
            let (from_minimum, from_maximum) = bounds.get(&portal.from).copied()?;
            let (to_minimum, to_maximum) = bounds.get(&portal.to).copied()?;
            let feasible_minimum = from_minimum.max(to_minimum.saturating_sub(1));
            let feasible_maximum = from_maximum.min(to_maximum.saturating_add(1));
            if feasible_minimum > feasible_maximum {
                return None;
            }
            let vertical = if start.level < feasible_minimum {
                start.level.abs_diff(feasible_minimum)
            } else if start.level > feasible_maximum {
                start.level.abs_diff(feasible_maximum)
            } else {
                0
            };
            let horizontal = start.coord.distance(portal.from);
            Some((vertical.max(horizontal), vertical, horizontal))
        })
        .min()
        .unwrap_or((u32::MAX, u32::MAX, u32::MAX))
}

fn recovery_interval_portal_lower_bound(
    coord: HexCoord,
    minimum: Level,
    maximum: Level,
    portals: &[BoundaryPortal],
    bounds: &BTreeMap<HexCoord, (Level, Level)>,
) -> (u32, u32, u32) {
    portals
        .iter()
        .filter_map(|portal| {
            let (from_minimum, from_maximum) = bounds.get(&portal.from).copied()?;
            let (to_minimum, to_maximum) = bounds.get(&portal.to).copied()?;
            let feasible_minimum = from_minimum.max(to_minimum.saturating_sub(1));
            let feasible_maximum = from_maximum.min(to_maximum.saturating_add(1));
            if feasible_minimum > feasible_maximum {
                return None;
            }
            let vertical = if maximum < feasible_minimum {
                maximum.abs_diff(feasible_minimum)
            } else if minimum > feasible_maximum {
                minimum.abs_diff(feasible_maximum)
            } else {
                0
            };
            let horizontal = coord.distance(portal.from);
            Some((vertical.max(horizontal), vertical, horizontal))
        })
        .min()
        .unwrap_or((u32::MAX, u32::MAX, u32::MAX))
}

fn exact_recovered_route_is_valid(
    route: &[TilePos],
    sequence: &[u16],
    allowed_by_patch: &BTreeMap<u16, BTreeSet<HexCoord>>,
    bounds: &BTreeMap<HexCoord, (Level, Level)>,
    final_stage: usize,
) -> bool {
    let owner_stages = route
        .iter()
        .map(|position| {
            sequence.iter().enumerate().find_map(|(owner_stage, id)| {
                allowed_by_patch
                    .get(id)
                    .is_some_and(|coords| coords.contains(&position.coord))
                    .then_some(owner_stage)
            })
        })
        .collect::<Option<Vec<_>>>();
    let exact_owner_trace = owner_stages.is_some_and(|owners| {
        owners.first() == Some(&0)
            && owners.last() == Some(&final_stage)
            && owners.windows(2).all(|pair| {
                let [first, second] = pair else {
                    return false;
                };
                first <= second && second.saturating_sub(*first) <= 1
            })
    });
    route
        .iter()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>()
        .len()
        == route.len()
        && route.windows(2).all(|pair| {
            let [first, second] = pair else {
                return false;
            };
            first.coord.distance(second.coord) == 1
        })
        && route.windows(2).all(|pair| {
            let [first, second] = pair else {
                return false;
            };
            first.level.abs_diff(second.level) <= 1
        })
        && route.iter().all(|position| {
            bounds
                .get(&position.coord)
                .is_some_and(|(minimum, maximum)| (*minimum..=*maximum).contains(&position.level))
        })
        && exact_owner_trace
}

#[derive(Debug, Clone, Copy)]
struct StageRunwayStep {
    coord: HexCoord,
    minimum: Level,
    maximum: Level,
}

/// Test convenience for resolving the first ranked bounded runway.
#[cfg(test)]
fn exact_simple_stage_runway(
    start: TilePos,
    allowed: &BTreeSet<HexCoord>,
    portals: &[BoundaryPortal],
    bounds: &BTreeMap<HexCoord, (Level, Level)>,
    forbidden: &BTreeSet<HexCoord>,
    search_budget: usize,
) -> Option<Vec<TilePos>> {
    let mut search = ExactRecoverySearch::new(search_budget);
    exact_simple_stage_runways(
        start,
        allowed,
        portals,
        bounds,
        forbidden,
        &BTreeMap::new(),
        search_budget,
        &mut search,
    )
    .into_iter()
    .next()
}

/// Finds a bounded ranked set of self-avoiding runways inside one authored
/// patch. Feasible levels remain an interval at every step. Every distinct
/// exact handoff `(coordinate, level)` found within the shared budget retains
/// its best route, so later stages can reject a low or dead portal and try the
/// next candidate without weakening cell authority.
fn exact_simple_stage_runways(
    start: TilePos,
    allowed: &BTreeSet<HexCoord>,
    portals: &[BoundaryPortal],
    bounds: &BTreeMap<HexCoord, (Level, Level)>,
    forbidden: &BTreeSet<HexCoord>,
    penalties: &BTreeMap<HexCoord, u32>,
    runway_work_budget: usize,
    search: &mut ExactRecoverySearch,
) -> Vec<Vec<TilePos>> {
    if !allowed.contains(&start.coord) {
        return Vec::new();
    }
    let Some((start_minimum, start_maximum)) = bounds.get(&start.coord).copied() else {
        return Vec::new();
    };
    if !(start_minimum..=start_maximum).contains(&start.level) {
        return Vec::new();
    }

    fn portal_levels(
        coord: HexCoord,
        minimum: Level,
        maximum: Level,
        portals: &[BoundaryPortal],
        bounds: &BTreeMap<HexCoord, (Level, Level)>,
        runway_remaining_floor: usize,
        search: &mut ExactRecoverySearch,
    ) -> (Vec<(BoundaryPortal, Level, Level)>, bool) {
        let mut candidates = Vec::new();
        for portal in portals
            .iter()
            .copied()
            .filter(|portal| portal.from == coord)
        {
            let Some((to_minimum, to_maximum)) = bounds.get(&portal.to).copied() else {
                continue;
            };
            let reachable_minimum = to_minimum.max(minimum.saturating_sub(1));
            let reachable_maximum = to_maximum.min(maximum.saturating_add(1));
            if reachable_minimum > reachable_maximum {
                continue;
            }
            for to_level in reachable_minimum..=reachable_maximum {
                if search.remaining_work <= runway_remaining_floor {
                    return (candidates, true);
                }
                if !search.consider_handoff() {
                    return (candidates, true);
                }
                let current_level = to_level.clamp(minimum, maximum);
                candidates.push((portal, current_level, to_level));
            }
        }
        (candidates, false)
    }

    fn concrete_runway(
        start: TilePos,
        path: &[StageRunwayStep],
        portal: BoundaryPortal,
        current_level: Level,
        to_level: Level,
    ) -> Option<Vec<TilePos>> {
        let mut levels = vec![0; path.len()];
        *levels.last_mut()? = current_level;
        for index in (0..path.len().saturating_sub(1)).rev() {
            let next_level = *levels.get(index.saturating_add(1))?;
            let step = *path.get(index)?;
            *levels.get_mut(index)? = next_level.clamp(step.minimum, step.maximum);
        }
        let mut route = path
            .iter()
            .copied()
            .zip(levels)
            .map(|(step, level)| TilePos::new(step.coord, level))
            .collect::<Vec<_>>();
        route.push(TilePos::new(portal.to, to_level));
        (route.first().copied() == Some(start)
            && route.windows(2).all(|pair| {
                let [first, second] = pair else {
                    return false;
                };
                first.coord.distance(second.coord) == 1 && first.level.abs_diff(second.level) <= 1
            }))
        .then_some(route)
    }

    #[derive(Clone)]
    struct Candidate {
        path: Vec<StageRunwayStep>,
        visited: BTreeSet<HexCoord>,
        penalty: u64,
    }

    let mut initial_visited = forbidden.clone();
    initial_visited.remove(&start.coord);
    initial_visited.insert(start.coord);
    let mut beam = vec![Candidate {
        path: vec![StageRunwayStep {
            coord: start.coord,
            minimum: start.level,
            maximum: start.level,
        }],
        visited: initial_visited,
        penalty: u64::from(penalties.get(&start.coord).copied().unwrap_or(0)),
    }];
    let mut handoffs = BTreeMap::<TilePos, (u64, Vec<TilePos>)>::new();
    let runway_remaining_floor = search
        .remaining_work
        .saturating_sub(runway_work_budget.min(search.remaining_work));
    'search: while !beam.is_empty() {
        let mut next = Vec::new();
        for candidate in beam {
            let Some(current) = candidate.path.last().copied() else {
                continue;
            };
            let (portal_candidates, portal_search_truncated) = portal_levels(
                current.coord,
                current.minimum,
                current.maximum,
                portals,
                bounds,
                runway_remaining_floor,
                search,
            );
            for (portal, current_level, to_level) in portal_candidates {
                let Some(route) =
                    concrete_runway(start, &candidate.path, portal, current_level, to_level)
                else {
                    continue;
                };
                let handoff = TilePos::new(portal.to, to_level);
                let score = candidate
                    .penalty
                    .saturating_add(u64::from(penalties.get(&portal.to).copied().unwrap_or(0)));
                let replace =
                    handoffs
                        .get(&handoff)
                        .is_none_or(|(current_score, current_route)| {
                            (score, route.len()) < (*current_score, current_route.len())
                        });
                if replace {
                    handoffs.insert(handoff, (score, route));
                }
            }
            if portal_search_truncated {
                search.stats.stage_enumerations_truncated =
                    search.stats.stage_enumerations_truncated.saturating_add(1);
                break 'search;
            }
            if search.stats.budget_exhausted {
                break 'search;
            }
            let neighbors = current
                .coord
                .neighbors()
                .into_iter()
                .filter(|neighbor| {
                    allowed.contains(neighbor) && !candidate.visited.contains(neighbor)
                })
                .filter_map(|neighbor| {
                    let (bound_minimum, bound_maximum) = bounds.get(&neighbor).copied()?;
                    let minimum = bound_minimum.max(current.minimum.saturating_sub(1));
                    let maximum = bound_maximum.min(current.maximum.saturating_add(1));
                    if minimum > maximum {
                        return None;
                    }
                    let onward = neighbor
                        .neighbors()
                        .into_iter()
                        .filter(|next_coord| {
                            allowed.contains(next_coord) && !candidate.visited.contains(next_coord)
                        })
                        .count();
                    let portal_lower_bound = recovery_interval_portal_lower_bound(
                        neighbor, minimum, maximum, portals, bounds,
                    );
                    let penalty = candidate
                        .penalty
                        .saturating_add(u64::from(penalties.get(&neighbor).copied().unwrap_or(0)));
                    Some((
                        portal_lower_bound,
                        penalty,
                        std::cmp::Reverse(onward),
                        std::cmp::Reverse(maximum.saturating_sub(minimum)),
                        neighbor,
                        minimum,
                        maximum,
                    ))
                })
                .collect::<Vec<_>>();
            for (lower_bound, penalty, onward, width, neighbor, minimum, maximum) in neighbors {
                if search.remaining_work <= runway_remaining_floor {
                    search.stats.stage_enumerations_truncated =
                        search.stats.stage_enumerations_truncated.saturating_add(1);
                    break 'search;
                }
                if !search.expand_neighbor() {
                    break 'search;
                }
                let mut branch = candidate.clone();
                branch.visited.insert(neighbor);
                branch.path.push(StageRunwayStep {
                    coord: neighbor,
                    minimum,
                    maximum,
                });
                branch.penalty = penalty;
                next.push(((lower_bound, penalty, onward, width, neighbor), branch));
            }
        }
        if next.is_empty() {
            break;
        }
        next.sort_by_key(|(rank, _)| *rank);
        if next.len() > EXACT_RECOVERY_BEAM_WIDTH {
            search.stats.beam_states_pruned = search
                .stats
                .beam_states_pruned
                .saturating_add(next.len().saturating_sub(EXACT_RECOVERY_BEAM_WIDTH));
            next.truncate(EXACT_RECOVERY_BEAM_WIDTH);
        }
        beam = next.into_iter().map(|(_, candidate)| candidate).collect();
    }

    let mut routes = handoffs
        .into_iter()
        .map(|(handoff, (penalty, route))| (penalty, route.len(), handoff, route))
        .collect::<Vec<_>>();
    routes.sort_unstable_by_key(|(penalty, len, handoff, _)| (*penalty, *len, *handoff));
    routes.into_iter().map(|(_, _, _, route)| route).collect()
}

fn ledge_routing_penalty(
    coord: HexCoord,
    bank_minimums: &BTreeMap<HexCoord, Level>,
    volume: &VolumePlan,
) -> u32 {
    let mut minimum = UPPER_REGION_THRESHOLD
        .saturating_add(1)
        .max(bank_minimums.get(&coord).copied().unwrap_or(Level::MIN));
    let mut maximum = MAXIMUM_LEDGE_LEVEL;
    for neighbor in coord.neighbors() {
        if let Some((surface, _)) = volume.top_surface_at_coord(neighbor) {
            minimum = minimum.max(surface.level.saturating_sub(MAXIMUM_PEAK_NEIGHBOR_STEP));
            maximum = maximum.min(surface.level.saturating_add(MAXIMUM_PEAK_NEIGHBOR_STEP));
        }
    }
    u32::try_from(minimum.saturating_sub(maximum)).unwrap_or(u32::MAX)
}

fn inner_peak_ledge_bounds(
    coord: HexCoord,
    mutable_allowed: &BTreeSet<HexCoord>,
    bank_minimums: &BTreeMap<HexCoord, Level>,
    volume: &VolumePlan,
) -> Result<(Level, Level), String> {
    let mut minimum = UPPER_REGION_THRESHOLD
        .saturating_add(1)
        .max(bank_minimums.get(&coord).copied().unwrap_or(Level::MIN));
    let mut maximum = MAXIMUM_LEDGE_LEVEL;
    for neighbor in coord
        .neighbors()
        .into_iter()
        .filter(|neighbor| !mutable_allowed.contains(neighbor))
    {
        if let Some((surface, _)) = volume.top_surface_at_coord(neighbor) {
            minimum = minimum.max(surface.level.saturating_sub(MAXIMUM_PEAK_NEIGHBOR_STEP));
            maximum = maximum.min(surface.level.saturating_add(MAXIMUM_PEAK_NEIGHBOR_STEP));
        }
    }
    (minimum <= maximum)
        .then_some((minimum, maximum))
        .ok_or_else(|| {
            format!("authored centerline at {coord:?} has immutable bounds {minimum}..={maximum}")
        })
}

/// Tightens centerline bounds so its nine-level shoulder can taper back to the
/// existing terrain before reaching immutable authority. A mutable column at
/// that boundary is not itself fixed: it may be regraded anywhere within nine
/// levels of every adjacent immutable surface. Propagating those boundary
/// intervals inward at the same slope used by shoulder grading gives the exact
/// interval a route level may occupy. Pinning the mutable column to its current
/// level would reject valid long runways whose support naturally reaches the
/// authority edge, including the authored cell-88 to cell-58 handoff.
fn shoulder_taper_safe_route_bounds(
    mut route_bounds: BTreeMap<HexCoord, (Level, Level)>,
    mutable_allowed: &BTreeSet<HexCoord>,
    volume: &VolumePlan,
) -> BTreeMap<HexCoord, (Level, Level)> {
    let boundary_bounds = mutable_allowed
        .iter()
        .copied()
        .filter_map(|coord| {
            let mut minimum = Level::MIN;
            let mut maximum = Level::MAX;
            let mut has_immutable_surface = false;
            for neighbor in coord
                .neighbors()
                .into_iter()
                .filter(|neighbor| !mutable_allowed.contains(neighbor))
            {
                let Some((surface, _)) = volume.top_surface_at_coord(neighbor) else {
                    continue;
                };
                has_immutable_surface = true;
                minimum = minimum.max(surface.level.saturating_sub(MAXIMUM_PEAK_NEIGHBOR_STEP));
                maximum = maximum.min(surface.level.saturating_add(MAXIMUM_PEAK_NEIGHBOR_STEP));
            }
            (has_immutable_surface && minimum <= maximum).then_some((coord, (minimum, maximum)))
        })
        .collect::<BTreeMap<_, _>>();

    let mut taper_minimums = boundary_bounds
        .iter()
        .map(|(coord, (minimum, _))| (*coord, *minimum))
        .collect::<BTreeMap<_, _>>();
    let mut minimum_frontier = std::collections::BinaryHeap::from_iter(
        taper_minimums.iter().map(|(coord, level)| (*level, *coord)),
    );
    while let Some((level, coord)) = minimum_frontier.pop() {
        if taper_minimums.get(&coord).copied() != Some(level) {
            continue;
        }
        for neighbor in coord
            .neighbors()
            .into_iter()
            .filter(|neighbor| mutable_allowed.contains(neighbor))
        {
            let candidate = level.saturating_sub(MAXIMUM_PEAK_NEIGHBOR_STEP);
            if taper_minimums
                .get(&neighbor)
                .is_none_or(|current| candidate > *current)
            {
                taper_minimums.insert(neighbor, candidate);
                minimum_frontier.push((candidate, neighbor));
            }
        }
    }

    let mut taper_maximums = boundary_bounds
        .into_iter()
        .map(|(coord, (_, maximum))| (coord, maximum))
        .collect::<BTreeMap<_, _>>();
    let mut maximum_frontier = taper_maximums
        .iter()
        .map(|(coord, level)| (*level, *coord))
        .collect::<BTreeSet<_>>();
    while let Some((level, coord)) = maximum_frontier.pop_first() {
        if taper_maximums.get(&coord).copied() != Some(level) {
            continue;
        }
        for neighbor in coord
            .neighbors()
            .into_iter()
            .filter(|neighbor| mutable_allowed.contains(neighbor))
        {
            let candidate = level.saturating_add(MAXIMUM_PEAK_NEIGHBOR_STEP);
            if taper_maximums
                .get(&neighbor)
                .is_none_or(|current| candidate < *current)
            {
                taper_maximums.insert(neighbor, candidate);
                maximum_frontier.insert((candidate, neighbor));
            }
        }
    }

    route_bounds.retain(|coord, bounds| {
        if let Some(minimum) = taper_minimums.get(coord).copied() {
            bounds.0 = bounds.0.max(minimum);
        }
        if let Some(maximum) = taper_maximums.get(coord).copied() {
            bounds.1 = bounds.1.min(maximum);
        }
        bounds.0 <= bounds.1
    });
    route_bounds
}

#[derive(Debug)]
struct GradedInnerPeakLedge {
    route_levels: BTreeMap<HexCoord, Level>,
    support_levels: BTreeMap<HexCoord, Level>,
}

struct ShoulderAuthorityDiagnostics<'a> {
    plan: &'a SchematicPlanV1,
    fine_index: &'a FineWorldIndex,
    mutable_allowed: &'a BTreeSet<HexCoord>,
    water: &'a BTreeSet<HexCoord>,
    protected_routes: &'a BTreeSet<HexCoord>,
    protected_route_owners: &'a BTreeMap<HexCoord, Vec<String>>,
    structures: &'a BTreeSet<HexCoord>,
    blockers: &'a BTreeSet<HexCoord>,
    surface_route_exclusion: &'a BTreeSet<HexCoord>,
    high_band: &'a BTreeSet<HexCoord>,
    summit_pins: &'a BTreeSet<HexCoord>,
}

impl ShoulderAuthorityDiagnostics<'_> {
    fn coord(&self, coord: HexCoord, volume: &VolumePlan) -> String {
        let resolved_surface = volume.top_surface_at_coord(coord);
        let level = resolved_surface.map(|(surface, _)| surface.level);
        let access = resolved_surface.map(|(_, metadata)| metadata.access);
        let patch = self.fine_index.patch(coord);
        let cell = patch.and_then(|owner| {
            self.plan
                .cells
                .iter()
                .find(|cell| u32::from(cell.id.get()) == owner.0)
        });
        let mut reasons = Vec::new();
        if self.water.contains(&coord) {
            reasons.push("water");
        }
        if self.protected_routes.contains(&coord) {
            reasons.push("protected-route");
        }
        if self.structures.contains(&coord) {
            reasons.push("structure");
        }
        if self.blockers.contains(&coord) {
            reasons.push("blocker");
        }
        if self.surface_route_exclusion.contains(&coord) {
            reasons.push("surface-route-exclusion");
        }
        if self.high_band.contains(&coord) {
            reasons.push("peak-high-band");
        }
        if self.summit_pins.contains(&coord) {
            reasons.push("summit-pin");
        }
        if reasons.is_empty() {
            reasons.push("outside-declared-or-nonordinary-authority");
        }
        format!(
            "{coord:?}@{level:?} access={access:?} patch={patch:?} landform={:?} overlays={:?} exclusions={reasons:?} route_owners={:?}",
            cell.map(|cell| cell.facts.landform),
            cell.map(|cell| &cell.facts.overlays),
            self.protected_route_owners.get(&coord),
        )
    }

    fn immutable_neighbors(&self, coord: HexCoord, volume: &VolumePlan) -> String {
        coord
            .neighbors()
            .into_iter()
            .filter(|neighbor| !self.mutable_allowed.contains(neighbor))
            .map(|neighbor| self.coord(neighbor, volume))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl GradedInnerPeakLedge {
    fn all_levels(&self) -> BTreeMap<HexCoord, Level> {
        self.route_levels
            .iter()
            .chain(&self.support_levels)
            .map(|(coord, level)| (*coord, *level))
            .collect()
    }

    fn all_coords(&self) -> impl Iterator<Item = &HexCoord> {
        self.route_levels.keys().chain(self.support_levels.keys())
    }

    fn level(&self, coord: &HexCoord) -> Option<Level> {
        self.route_levels
            .get(coord)
            .or_else(|| self.support_levels.get(coord))
            .copied()
    }
}

/// Returns exactly the mutable shoulder columns whose existing grade cannot
/// absorb the authored route at the allowed natural-rock slope. Variable-level
/// multi-source propagation follows the admitted mask, so an excluded summit
/// crown cannot transmit route authority into an unrelated patch edge.
fn influenced_shoulder_coords(
    junction: TilePos,
    route_levels: &BTreeMap<HexCoord, Level>,
    mutable_allowed: &BTreeSet<HexCoord>,
    originals: &BTreeMap<HexCoord, Level>,
) -> BTreeSet<HexCoord> {
    let mut upper_envelope = BTreeMap::new();
    let mut upper_frontier = BTreeSet::new();
    let mut lower_envelope = BTreeMap::new();
    let mut lower_frontier = std::collections::BinaryHeap::new();
    for (coord, level) in route_levels {
        if mutable_allowed.contains(coord) {
            upper_envelope.insert(*coord, *level);
            upper_frontier.insert((*level, *coord));
            lower_envelope.insert(*coord, *level);
            lower_frontier.push((*level, *coord));
        }
    }
    for neighbor in junction.coord.neighbors() {
        if !mutable_allowed.contains(&neighbor) {
            continue;
        }
        let upper = junction.level.saturating_add(MAXIMUM_PEAK_NEIGHBOR_STEP);
        if upper_envelope
            .get(&neighbor)
            .is_none_or(|current| upper < *current)
        {
            upper_envelope.insert(neighbor, upper);
            upper_frontier.insert((upper, neighbor));
        }
        let lower = junction.level.saturating_sub(MAXIMUM_PEAK_NEIGHBOR_STEP);
        if lower_envelope
            .get(&neighbor)
            .is_none_or(|current| lower > *current)
        {
            lower_envelope.insert(neighbor, lower);
            lower_frontier.push((lower, neighbor));
        }
    }
    while let Some((level, coord)) = upper_frontier.pop_first() {
        if upper_envelope.get(&coord).copied() != Some(level) {
            continue;
        }
        for neighbor in coord
            .neighbors()
            .into_iter()
            .filter(|neighbor| mutable_allowed.contains(neighbor))
        {
            let candidate = level.saturating_add(MAXIMUM_PEAK_NEIGHBOR_STEP);
            if upper_envelope
                .get(&neighbor)
                .is_none_or(|current| candidate < *current)
            {
                upper_envelope.insert(neighbor, candidate);
                upper_frontier.insert((candidate, neighbor));
            }
        }
    }
    while let Some((level, coord)) = lower_frontier.pop() {
        if lower_envelope.get(&coord).copied() != Some(level) {
            continue;
        }
        for neighbor in coord
            .neighbors()
            .into_iter()
            .filter(|neighbor| mutable_allowed.contains(neighbor))
        {
            let candidate = level.saturating_sub(MAXIMUM_PEAK_NEIGHBOR_STEP);
            if lower_envelope
                .get(&neighbor)
                .is_none_or(|current| candidate > *current)
            {
                lower_envelope.insert(neighbor, candidate);
                lower_frontier.push((candidate, neighbor));
            }
        }
    }
    originals
        .iter()
        .filter_map(|(coord, original)| {
            let route = route_levels.contains_key(coord);
            let influenced = upper_envelope
                .get(coord)
                .is_some_and(|upper| original > upper)
                || lower_envelope
                    .get(coord)
                    .is_some_and(|lower| original < lower);
            (route || influenced).then_some(*coord)
        })
        .collect()
}

/// Projects the exact search-selected centerline onto a nine-level natural
/// shoulder. Route search has already solved the one-level grade through every
/// local bound; discarding those levels and greedily grading the coordinates a
/// second time can produce a different, support-incompatible profile.
fn grade_authored_inner_peak_ledge(
    junction: TilePos,
    centerline: &[TilePos],
    mutable_allowed: &BTreeSet<HexCoord>,
    authority: Option<&ShoulderAuthorityDiagnostics<'_>>,
    bank_minimums: &BTreeMap<HexCoord, Level>,
    volume: &VolumePlan,
) -> Result<GradedInnerPeakLedge, String> {
    if centerline.first().copied() != Some(junction)
        || centerline
            .iter()
            .map(|position| position.coord)
            .collect::<BTreeSet<_>>()
            .len()
            != centerline.len()
        || centerline.windows(2).any(|pair| {
            let [first, second] = pair else {
                return true;
            };
            first.coord.distance(second.coord) != 1 || first.level.abs_diff(second.level) > 1
        })
    {
        return Err(
            "authored ledge centerline is not one simple adjacent one-level path from its exact junction"
                .to_owned(),
        );
    }

    let mut route_levels = BTreeMap::new();
    for (index, position) in centerline.iter().copied().enumerate() {
        let (minimum, maximum) = if index == 0 {
            (junction.level, junction.level)
        } else {
            inner_peak_ledge_bounds(position.coord, mutable_allowed, bank_minimums, volume)?
        };
        if !(minimum..=maximum).contains(&position.level) {
            return Err(format!(
                "search-selected centerline {position:?} escaped exact immutable bounds {minimum}..={maximum}"
            ));
        }
        route_levels.insert(position.coord, position.level);
    }

    let route_coords = route_levels.keys().copied().collect::<BTreeSet<_>>();
    if route_coords
        .difference(&BTreeSet::from([junction.coord]))
        .any(|coord| !mutable_allowed.contains(coord))
    {
        return Err(
            "authored ledge centerline escaped its exact mutable patch authority".to_owned(),
        );
    }

    let originals = mutable_allowed
        .iter()
        .map(|coord| {
            volume
                .top_surface_at_coord(*coord)
                .map(|(surface, _)| (*coord, surface.level))
                .ok_or_else(|| format!("authored shoulder lost source surface at {coord:?}"))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let shoulder_allowed =
        influenced_shoulder_coords(junction, &route_levels, mutable_allowed, &originals);
    if route_coords
        .difference(&BTreeSet::from([junction.coord]))
        .any(|coord| !shoulder_allowed.contains(coord))
    {
        return Err("authored shoulder lost one exact centerline coordinate".to_owned());
    }

    let mut lower = BTreeMap::new();
    let mut upper = BTreeMap::new();
    for coord in &shoulder_allowed {
        let mut minimum = UPPER_REGION_THRESHOLD
            .saturating_add(1)
            .max(bank_minimums.get(coord).copied().unwrap_or(Level::MIN));
        let mut maximum = MAXIMUM_LEDGE_LEVEL;
        if let Some(route_level) = route_levels.get(coord).copied() {
            minimum = route_level;
            maximum = route_level;
        }
        for neighbor in coord
            .neighbors()
            .into_iter()
            .filter(|neighbor| !shoulder_allowed.contains(neighbor))
        {
            if let Some((surface, _)) = volume.top_surface_at_coord(neighbor) {
                minimum = minimum.max(surface.level.saturating_sub(MAXIMUM_PEAK_NEIGHBOR_STEP));
                maximum = maximum.min(surface.level.saturating_add(MAXIMUM_PEAK_NEIGHBOR_STEP));
            }
        }
        if minimum > maximum {
            let neighbors = authority
                .map(|authority| authority.immutable_neighbors(*coord, volume))
                .unwrap_or_default();
            return Err(format!(
                "authored shoulder at {coord:?} has immutable bounds {minimum}..={maximum}; neighbors=[{}]",
                neighbors,
            ));
        }
        lower.insert(*coord, minimum);
        upper.insert(*coord, maximum);
    }

    let mut frontier = std::collections::BinaryHeap::from_iter(
        lower.iter().map(|(coord, level)| (*level, *coord)),
    );
    while let Some((level, coord)) = frontier.pop() {
        if lower.get(&coord).copied() != Some(level) {
            continue;
        }
        for neighbor in coord
            .neighbors()
            .into_iter()
            .filter(|neighbor| shoulder_allowed.contains(neighbor))
        {
            let required = level.saturating_sub(MAXIMUM_PEAK_NEIGHBOR_STEP);
            if lower
                .get(&neighbor)
                .is_none_or(|current| required > *current)
            {
                lower.insert(neighbor, required);
                frontier.push((required, neighbor));
            }
        }
    }

    let mut frontier = upper
        .iter()
        .map(|(coord, level)| (*level, *coord))
        .collect::<BTreeSet<_>>();
    while let Some((level, coord)) = frontier.pop_first() {
        if upper.get(&coord).copied() != Some(level) {
            continue;
        }
        for neighbor in coord
            .neighbors()
            .into_iter()
            .filter(|neighbor| shoulder_allowed.contains(neighbor))
        {
            let required = level.saturating_add(MAXIMUM_PEAK_NEIGHBOR_STEP);
            if upper
                .get(&neighbor)
                .is_none_or(|current| required < *current)
            {
                upper.insert(neighbor, required);
                frontier.insert((required, neighbor));
            }
        }
    }

    if let Some((target, minimum, maximum)) = lower.iter().find_map(|(target, minimum)| {
        let maximum = upper.get(target).copied().unwrap_or(Level::MIN);
        (*minimum > maximum).then_some((*target, *minimum, maximum))
    }) {
        return Err(format!(
            "authored shoulder requires incompatible propagated bounds {minimum}..={maximum} at {target:?}"
        ));
    }

    let mut projected = BTreeMap::new();
    for coord in &shoulder_allowed {
        let original = originals
            .get(coord)
            .copied()
            .ok_or_else(|| format!("authored shoulder lost original level at {coord:?}"))?;
        let minimum = lower
            .get(coord)
            .copied()
            .ok_or_else(|| format!("authored shoulder lost lower bound at {coord:?}"))?;
        let maximum = upper
            .get(coord)
            .copied()
            .ok_or_else(|| format!("authored shoulder lost upper bound at {coord:?}"))?;
        let level = original.clamp(minimum, maximum);
        projected.insert(*coord, level);
    }
    for (coord, level) in &route_levels {
        if projected.insert(*coord, *level).is_none() && *coord != junction.coord {
            return Err(format!(
                "authored route grade escaped shoulder at {coord:?}"
            ));
        }
    }

    let changed = projected
        .iter()
        .filter_map(|(coord, level)| {
            (route_coords.contains(coord) || originals.get(coord) != Some(level))
                .then_some((*coord, *level))
        })
        .collect::<BTreeMap<_, _>>();
    if let Some((from, from_level, to, to_level)) = changed.iter().find_map(|(from, from_level)| {
        from.neighbors().into_iter().find_map(|to| {
            let to_level = projected.get(&to).copied().or_else(|| {
                volume
                    .top_surface_at_coord(to)
                    .map(|(surface, _)| surface.level)
            })?;
            (from_level.abs_diff(to_level)
                > u32::try_from(MAXIMUM_PEAK_NEIGHBOR_STEP).unwrap_or(u32::MAX))
            .then_some((*from, *from_level, to, to_level))
        })
    }) {
        return Err(format!(
            "authored shoulder edge {from:?}@{from_level}->{to:?}@{to_level} exceeds nine levels"
        ));
    }
    if route_levels.get(&junction.coord).copied() != Some(junction.level)
        || route_levels.values().any(|level| {
            !OrdinaryRegionBand::Upper.accepts_new(*level) || *level > MAXIMUM_LEDGE_LEVEL
        })
        || centerline.windows(2).any(|pair| {
            let [first, second] = pair else {
                return true;
            };
            route_levels
                .get(&first.coord)
                .zip(route_levels.get(&second.coord))
                .is_none_or(|(first_level, second_level)| first_level.abs_diff(*second_level) > 1)
        })
    {
        return Err(
            "authored ledge changed its junction, band, ceiling, or one-level route".to_owned(),
        );
    }

    let support_levels = changed
        .into_iter()
        .filter(|(coord, _)| !route_coords.contains(coord))
        .collect();
    Ok(GradedInnerPeakLedge {
        route_levels,
        support_levels,
    })
}

fn coord_distances(
    start: HexCoord,
    corridor: &BTreeSet<HexCoord>,
) -> Option<BTreeMap<HexCoord, u32>> {
    if !corridor.contains(&start) {
        return None;
    }
    let mut distances = BTreeMap::from([(start, 0_u32)]);
    let mut frontier = VecDeque::from([start]);
    while let Some(coord) = frontier.pop_front() {
        let distance = distances.get(&coord).copied().unwrap_or_default();
        let mut neighbors = coord.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if corridor.contains(&neighbor) && !distances.contains_key(&neighbor) {
                distances.insert(neighbor, distance.saturating_add(1));
                frontier.push_back(neighbor);
            }
        }
    }
    (distances.len() == corridor.len()).then_some(distances)
}

fn coords_connected(coords: &BTreeSet<HexCoord>) -> bool {
    coords
        .first()
        .copied()
        .and_then(|start| coord_distances(start, coords))
        .is_some_and(|distances| distances.len() == coords.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SideBranchContractFixture {
        centerline: Vec<TilePos>,
        support_levels: BTreeMap<HexCoord, Level>,
        trunk: ProtectedFeatureRoute,
        water_coords: BTreeSet<HexCoord>,
        structure_coords: BTreeSet<HexCoord>,
        blocker_coords: BTreeSet<HexCoord>,
        surface_route_exclusion: BTreeSet<HexCoord>,
        high_band: BTreeSet<HexCoord>,
        summit_pins: BTreeSet<HexCoord>,
    }

    impl SideBranchContractFixture {
        fn valid() -> Self {
            let trunk_centerline = [-1, 0, 1]
                .into_iter()
                .map(|q| TilePos::new(HexCoord::from_axial(q, 0), 130))
                .collect::<Vec<_>>();
            let trunk = ProtectedFeatureRoute {
                surfaces: trunk_centerline.iter().copied().collect(),
                centerline: trunk_centerline,
            };
            Self {
                centerline: [
                    HexCoord::from_axial(0, 0),
                    HexCoord::from_axial(1, 0),
                    HexCoord::from_axial(1, 1),
                    HexCoord::from_axial(2, 1),
                ]
                .into_iter()
                .map(|coord| TilePos::new(coord, 130))
                .collect(),
                support_levels: BTreeMap::from([(HexCoord::from_axial(2, 2), 130)]),
                trunk,
                water_coords: BTreeSet::new(),
                structure_coords: BTreeSet::new(),
                blocker_coords: BTreeSet::new(),
                surface_route_exclusion: BTreeSet::new(),
                high_band: BTreeSet::new(),
                summit_pins: BTreeSet::new(),
            }
        }

        fn validate(&self) -> Result<(), String> {
            validate_upper_ledge_side_branch_contract(UpperLedgeSideBranchContract {
                label: "test Upper ledge branch",
                centerline: &self.centerline,
                support_levels: &self.support_levels,
                trunk: &self.trunk,
                water_coords: &self.water_coords,
                structure_coords: &self.structure_coords,
                blocker_coords: &self.blocker_coords,
                surface_route_exclusion: &self.surface_route_exclusion,
                high_band: &self.high_band,
                summit_pins: &self.summit_pins,
            })
        }
    }

    #[test]
    fn upper_side_branch_contract_accepts_one_induced_upper_departure() {
        SideBranchContractFixture::valid()
            .validate()
            .expect("one exact two-node trunk prefix and induced Upper branch is valid");
    }

    #[test]
    fn upper_side_branch_contract_rejects_trunk_reentry_and_self_chords() {
        let mut reentry = SideBranchContractFixture::valid();
        reentry
            .centerline
            .push(TilePos::new(HexCoord::from_axial(2, 0), 130));
        let reentry_error = reentry
            .validate()
            .expect_err("a later surface beside the trunk is a second departure");
        assert!(reentry_error.contains("re-enters or runs beside"));

        let mut chord = SideBranchContractFixture::valid();
        chord.centerline.extend([
            TilePos::new(HexCoord::from_axial(2, 2), 130),
            TilePos::new(HexCoord::from_axial(1, 2), 130),
        ]);
        chord.support_levels.clear();
        let chord_error = chord
            .validate()
            .expect_err("an off-trunk loop may not touch a nonconsecutive route surface");
        assert!(chord_error.contains("nonconsecutive self-chord"));
    }

    #[test]
    fn upper_side_branch_contract_audits_support_departures_and_upper_floor() {
        let mut support_departure = SideBranchContractFixture::valid();
        support_departure.support_levels = BTreeMap::from([(HexCoord::from_axial(0, 1), 130)]);
        let departure_error = support_departure
            .validate()
            .expect_err("a graded support may not form a second trunk edge");
        assert!(departure_error.contains("extra walker departure"));

        let mut low_support = SideBranchContractFixture::valid();
        low_support
            .support_levels
            .insert(HexCoord::from_axial(2, 2), 121);
        let low_error = low_support
            .validate()
            .expect_err("every shoulder support must stay in the strict Upper band");
        assert!(low_error.contains("falls below Upper-only level 122"));
    }

    #[test]
    fn upper_side_branch_contract_rejects_every_forbidden_authority() {
        for authority in 0..6 {
            let mut fixture = SideBranchContractFixture::valid();
            let excluded = HexCoord::from_axial(2, 1);
            match authority {
                0 => fixture.water_coords.insert(excluded),
                1 => fixture.structure_coords.insert(excluded),
                2 => fixture.blocker_coords.insert(excluded),
                3 => fixture.surface_route_exclusion.insert(excluded),
                4 => fixture.high_band.insert(excluded),
                _ => fixture.summit_pins.insert(excluded),
            };
            fixture
                .validate()
                .expect_err("route/support authority must remain disjoint from every exclusion");
        }
    }

    #[test]
    fn inner_peak_transit_contract_selects_and_retains_typed_patch_59_crossings() {
        let ingress_from = HexCoord::from_axial(-1, 0);
        let alternate_ingress_from = HexCoord::from_axial(-1, 1);
        let ingress_to = HexCoord::from_axial(0, 0);
        let alternate_ingress_to = HexCoord::from_axial(0, 1);
        let egress_from = HexCoord::from_axial(1, 0);
        let alternate_egress_from = HexCoord::from_axial(1, 1);
        let egress_to = HexCoord::from_axial(2, 0);
        let alternate_egress_to = HexCoord::from_axial(2, 1);
        let masks = BTreeMap::from([
            (
                PatchId(58),
                BTreeSet::from([ingress_from, alternate_ingress_from]),
            ),
            (
                PatchId(59),
                BTreeSet::from([
                    ingress_to,
                    alternate_ingress_to,
                    egress_from,
                    alternate_egress_from,
                ]),
            ),
            (
                PatchId(36),
                BTreeSet::from([egress_to, alternate_egress_to]),
            ),
        ]);
        let swaths = BTreeMap::from([
            (
                INNER_PEAK_TRANSIT_INGRESS,
                BTreeSet::from([ingress_from, ingress_to, egress_from]),
            ),
            (
                INNER_PEAK_TRANSIT_EGRESS,
                BTreeSet::from([ingress_to, egress_from, egress_to]),
            ),
        ]);
        let allowed = masks
            .values()
            .flat_map(|mask| mask.iter().copied())
            .collect::<BTreeSet<_>>();
        let bounds = allowed
            .iter()
            .copied()
            .map(|coord| (coord, (0, 0)))
            .collect::<BTreeMap<_, _>>();

        let transits =
            inner_peak_transit_authorities(&masks, &swaths, &allowed, &bounds, &[59, 36])
                .expect("one exact scenic Patch-59 runway is admitted");
        let transit = transits
            .first()
            .expect("the scenic domain is tried before the broader fallback");

        assert_eq!(
            transit.runway_domain,
            BTreeSet::from([ingress_to, egress_from])
        );
        assert!(transit.permits_portal(
            58,
            59,
            BoundaryPortal {
                from: ingress_from,
                to: ingress_to,
            }
        ));
        assert!(!transit.permits_portal(
            58,
            59,
            BoundaryPortal {
                from: alternate_ingress_from,
                to: alternate_ingress_to,
            }
        ));
        assert!(transit.permits_portal(
            59,
            36,
            BoundaryPortal {
                from: egress_from,
                to: egress_to,
            }
        ));
        assert!(!transit.permits_portal(
            59,
            36,
            BoundaryPortal {
                from: alternate_egress_from,
                to: alternate_egress_to,
            }
        ));

        let mut scoped_allowed = allowed.clone();
        let patch_59 = masks
            .get(&INNER_PEAK_TRANSIT_PATCH)
            .expect("transit fixture retains Patch59");
        scoped_allowed
            .retain(|coord| !patch_59.contains(coord) || transit.runway_domain.contains(coord));
        let mut search_budget = InnerPeakTransitSearchBudget::new();
        let route = segmented_inner_peak_route_ranked(
            ingress_from,
            &masks,
            &scoped_allowed,
            &[58, 59, 36],
            &BTreeMap::new(),
            0,
            &bounds,
            transit,
            &mut search_budget,
        )
        .expect("the typed three-stage ingress/runway/egress contract resolves");
        let admission = transit
            .validate_route(&route, &masks)
            .expect("the exact solver result retains both typed portals");
        assert_eq!(
            admission.runway,
            vec![TilePos::new(ingress_to, 0), TilePos::new(egress_from, 0)]
        );
        assert!(admission.is_retained_by(&route));

        let escaped = [
            ingress_from,
            alternate_ingress_to,
            alternate_egress_from,
            egress_to,
        ]
        .into_iter()
        .map(|coord| TilePos::new(coord, 0))
        .collect::<Vec<_>>();
        assert!(transit.validate_route(&escaped, &masks).is_err());
    }

    #[test]
    fn ordered_inner_peak_transit_requires_the_retained_foundation_spine() {
        let ingress_from = HexCoord::from_axial(-1, 0);
        let first = HexCoord::from_axial(0, 0);
        let last = HexCoord::from_axial(1, 0);
        let egress_to = HexCoord::from_axial(2, 0);
        let masks = BTreeMap::from([
            (PatchId(58), BTreeSet::from([ingress_from])),
            (PatchId(59), BTreeSet::from([first, last])),
            (PatchId(36), BTreeSet::from([egress_to])),
        ]);
        let authored = super::super::super::schematic_highlands::OrderedPeakSaddleSpineAuthority {
            owner: PatchId(59),
            ingress_from: PatchId(58),
            egress_to: PatchId(36),
            ingress_portals: BTreeSet::from([(ingress_from, first)]),
            centerline: vec![first, last],
            egress_portals: BTreeSet::from([(last, egress_to)]),
            support_domain: BTreeSet::from([first, last]),
            authored_grades: BTreeMap::from([(first, 0), (last, 0)]),
        };
        let allowed = BTreeSet::from([ingress_from, first, last, egress_to]);
        let bounds = allowed
            .iter()
            .copied()
            .map(|coord| (coord, (0, 0)))
            .collect::<BTreeMap<_, _>>();
        let transit =
            ordered_inner_peak_transit_authority(&masks, &authored, &allowed, &bounds, &[59, 36])
                .expect("the exact retained two-coordinate spine is admitted");
        let route = [ingress_from, first, last, egress_to]
            .into_iter()
            .map(|coord| TilePos::new(coord, 0))
            .collect::<Vec<_>>();
        let admission = transit
            .validate_route(&route, &masks)
            .expect("the exact retained spine satisfies typed transit");
        assert_eq!(
            admission.runway.as_slice(),
            route
                .get(1..=2)
                .expect("four-point route retains the exact two-point runway")
        );

        let shortened = [ingress_from, first, egress_to]
            .into_iter()
            .map(|coord| TilePos::new(coord, 0))
            .collect::<Vec<_>>();
        let error = transit
            .validate_route(&shortened, &masks)
            .expect_err("omitting the retained egress endpoint must fail closed");
        assert!(error.contains("exact typed 58->59 ingress and 59->36 egress"));

        // Both exact boundary portals survive this mutation, so rejection must
        // come from the retained ordered runway rather than portal validation.
        let backtracked = [ingress_from, first, last, first, last, egress_to]
            .into_iter()
            .map(|coord| TilePos::new(coord, 0))
            .collect::<Vec<_>>();
        let error = transit
            .validate_route(&backtracked, &masks)
            .expect_err("backtracking inside the retained Patch-59 runway must fail closed");
        assert!(error.contains("ordered Patch-59 foundation spine"));

        let wrong_grade = [
            TilePos::new(ingress_from, 0),
            TilePos::new(first, 0),
            TilePos::new(last, 1),
            TilePos::new(egress_to, 0),
        ];
        let error = transit
            .validate_route(&wrong_grade, &masks)
            .expect_err("the same Patch-59 coordinates at a different grade must fail closed");
        assert!(error.contains("foundation spine or grade"));
    }

    #[test]
    fn inner_peak_transit_contract_uses_full_patch_runway_when_scenic_swaths_are_disjoint() {
        let ingress_from = HexCoord::from_axial(-1, 0);
        let ingress_to = HexCoord::from_axial(0, 0);
        let detour_a = HexCoord::from_axial(0, 1);
        let detour_b = HexCoord::from_axial(1, 1);
        let egress_from = HexCoord::from_axial(2, 0);
        let egress_to = HexCoord::from_axial(3, 0);
        let full_runway = BTreeSet::from([ingress_to, detour_a, detour_b, egress_from]);
        let masks = BTreeMap::from([
            (PatchId(58), BTreeSet::from([ingress_from])),
            (PatchId(59), full_runway.clone()),
            (PatchId(36), BTreeSet::from([egress_to])),
        ]);
        let swaths = BTreeMap::from([
            (
                INNER_PEAK_TRANSIT_INGRESS,
                BTreeSet::from([ingress_from, ingress_to]),
            ),
            (
                INNER_PEAK_TRANSIT_EGRESS,
                BTreeSet::from([egress_from, egress_to]),
            ),
        ]);
        let allowed = masks
            .values()
            .flat_map(|mask| mask.iter().copied())
            .collect::<BTreeSet<_>>();
        let bounds = allowed
            .iter()
            .copied()
            .map(|coord| (coord, (0, 0)))
            .collect::<BTreeMap<_, _>>();

        let transits =
            inner_peak_transit_authorities(&masks, &swaths, &allowed, &bounds, &[59, 36])
                .expect("the connected full Patch-59 domain supplies the missing scenic runway");
        assert_eq!(transits.len(), 1);
        let [transit] = transits.as_slice() else {
            panic!("fixture retains exactly one transit");
        };
        assert_eq!(transit.runway_domain, full_runway);

        let mut search_budget = InnerPeakTransitSearchBudget::new();
        let route = segmented_inner_peak_route_ranked(
            ingress_from,
            &masks,
            &allowed,
            &[58, 59, 36],
            &BTreeMap::new(),
            0,
            &bounds,
            transit,
            &mut search_budget,
        )
        .expect("the split transit proof follows the longer full-patch detour");
        let admission = transit
            .validate_route(&route, &masks)
            .expect("the longer detour retains both exact typed portals");
        assert_eq!(admission.runway.len(), 4);
        assert!(admission.is_retained_by(&route));
    }

    #[test]
    fn inner_peak_split_transit_retries_the_same_ingress_at_a_viable_exact_level() {
        let ingress_from = HexCoord::from_axial(-1, 0);
        let ingress_to = HexCoord::from_axial(0, 0);
        let climb = HexCoord::from_axial(1, 0);
        let descent = HexCoord::from_axial(2, 0);
        let egress_from = HexCoord::from_axial(3, 0);
        let egress_to = HexCoord::from_axial(4, 0);
        let masks = BTreeMap::from([
            (PatchId(58), BTreeSet::from([ingress_from])),
            (
                PatchId(59),
                BTreeSet::from([ingress_to, climb, descent, egress_from]),
            ),
            (PatchId(36), BTreeSet::from([egress_to])),
        ]);
        let swaths = BTreeMap::from([
            (
                INNER_PEAK_TRANSIT_INGRESS,
                BTreeSet::from([ingress_from, ingress_to]),
            ),
            (
                INNER_PEAK_TRANSIT_EGRESS,
                BTreeSet::from([egress_from, egress_to]),
            ),
        ]);
        let allowed = masks
            .values()
            .flat_map(|mask| mask.iter().copied())
            .collect::<BTreeSet<_>>();
        let bounds = BTreeMap::from([
            (ingress_from, (0, 0)),
            (ingress_to, (0, 1)),
            (climb, (2, 2)),
            (descent, (1, 1)),
            (egress_from, (0, 0)),
            (egress_to, (0, 0)),
        ]);
        let transit = inner_peak_transit_authorities(&masks, &swaths, &allowed, &bounds, &[59, 36])
            .expect("the full patch admits one exact climbing runway")
            .into_iter()
            .next()
            .expect("the full patch has one qualifying transit authority");
        let distance_by_entry = match transit.suffix_reachability.as_ref() {
            InnerPeakSuffixReachability::Complete {
                distance_by_entry, ..
            } => Some(distance_by_entry),
            InnerPeakSuffixReachability::Incomplete { .. } => None,
        }
        .expect("the small exact suffix index must be complete");
        assert!(!distance_by_entry.contains_key(&TilePos::new(ingress_to, 0)));
        assert!(distance_by_entry.contains_key(&TilePos::new(ingress_to, 1)));

        let mut search_budget = InnerPeakTransitSearchBudget::new();
        let route = segmented_inner_peak_route_ranked(
            ingress_from,
            &masks,
            &allowed,
            &[58, 59, 36],
            &BTreeMap::new(),
            0,
            &bounds,
            &transit,
            &mut search_budget,
        )
        .expect("the split contract selects the viable higher ingress level");
        assert_eq!(
            search_budget.remaining_handoffs,
            INNER_PEAK_TRANSIT_HANDOFF_LIMIT.saturating_sub(1),
            "the complete reverse proof must filter the dead lower level before one physical attempt"
        );
        assert!(route.contains(&TilePos::new(ingress_to, 1)));
        assert!(!route.contains(&TilePos::new(ingress_to, 0)));
        assert!(transit
            .validate_route(&route, &masks)
            .expect("the higher-level runway retains both typed portals")
            .is_retained_by(&route));

        let mut incomplete_transit = transit.clone();
        let incomplete_reachability = inner_peak_suffix_reachability(
            &[59, 36],
            &masks,
            &allowed,
            &bounds,
            &incomplete_transit.runway_domain,
            &incomplete_transit.egress,
            0,
        )
        .expect("the zero-cap reverse index is typed incomplete");
        incomplete_transit.suffix_reachability = std::sync::Arc::new(incomplete_reachability);
        let mut incomplete_budget = InnerPeakTransitSearchBudget::new();
        let fallback_route = segmented_inner_peak_route_ranked(
            ingress_from,
            &masks,
            &allowed,
            &[58, 59, 36],
            &BTreeMap::new(),
            0,
            &bounds,
            &incomplete_transit,
            &mut incomplete_budget,
        )
        .expect("an incomplete reverse index disables filtering and preserves fallback search");
        assert_eq!(
            incomplete_budget.remaining_handoffs,
            INNER_PEAK_TRANSIT_HANDOFF_LIMIT.saturating_sub(2)
        );
        assert!(incomplete_budget.saw_incomplete_search);
        assert!(incomplete_transit
            .validate_route(&fallback_route, &masks)
            .is_ok());
    }

    #[test]
    fn inner_peak_suffix_reachability_respects_typed_egress_and_incomplete_limit() {
        let admitted_from = HexCoord::from_axial(0, 0);
        let admitted_to = HexCoord::from_axial(1, 0);
        let unlisted_from = HexCoord::from_axial(10, 0);
        let unlisted_to = HexCoord::from_axial(11, 0);
        let masks = BTreeMap::from([
            (PatchId(59), BTreeSet::from([admitted_from, unlisted_from])),
            (PatchId(36), BTreeSet::from([admitted_to, unlisted_to])),
        ]);
        let allowed = masks
            .values()
            .flat_map(|mask| mask.iter().copied())
            .collect::<BTreeSet<_>>();
        let bounds = allowed
            .iter()
            .copied()
            .map(|coord| (coord, (0, 0)))
            .collect::<BTreeMap<_, _>>();
        let runway_domain = masks
            .get(&PatchId(59))
            .cloned()
            .expect("the fixture retains Patch 59");
        let egress = BTreeSet::from([BoundaryPortal {
            from: admitted_from,
            to: admitted_to,
        }]);

        let complete = inner_peak_suffix_reachability(
            &[59, 36],
            &masks,
            &allowed,
            &bounds,
            &runway_domain,
            &egress,
            4,
        )
        .expect("the four-state reverse graph is complete");
        let distance_by_entry = match &complete {
            InnerPeakSuffixReachability::Complete {
                distance_by_entry, ..
            } => Some(distance_by_entry),
            InnerPeakSuffixReachability::Incomplete { .. } => None,
        }
        .expect("the exact state count fits the supplied cap");
        assert!(distance_by_entry.contains_key(&TilePos::new(admitted_from, 0)));
        assert!(!distance_by_entry.contains_key(&TilePos::new(unlisted_from, 0)));

        let incomplete = inner_peak_suffix_reachability(
            &[59, 36],
            &masks,
            &allowed,
            &bounds,
            &runway_domain,
            &egress,
            3,
        )
        .expect("an oversized finite graph is a typed incomplete result");
        assert!(matches!(
            incomplete,
            InnerPeakSuffixReachability::Incomplete {
                possible_states: 4,
                state_limit: 3,
                ..
            }
        ));
    }

    #[test]
    fn inner_peak_suffix_reachability_never_admits_a_coordinate_repeating_witness() {
        let start = HexCoord::from_axial(0, 0);
        let ramp = HexCoord::from_axial(1, 0);
        let egress_from = HexCoord::from_axial(2, 0);
        let egress_to = HexCoord::from_axial(3, 0);
        let runway_domain = BTreeSet::from([start, ramp, egress_from]);
        let masks = BTreeMap::from([
            (PatchId(59), runway_domain.clone()),
            (PatchId(36), BTreeSet::from([egress_to])),
        ]);
        let allowed = masks
            .values()
            .flat_map(|mask| mask.iter().copied())
            .collect::<BTreeSet<_>>();
        let bounds = BTreeMap::from([
            (start, (0, 2)),
            (ramp, (1, 3)),
            (egress_from, (4, 4)),
            (egress_to, (4, 4)),
        ]);
        let egress = BTreeSet::from([BoundaryPortal {
            from: egress_from,
            to: egress_to,
        }]);
        let suffix_reachability = inner_peak_suffix_reachability(
            &[59, 36],
            &masks,
            &allowed,
            &bounds,
            &runway_domain,
            &egress,
            16,
        )
        .expect("the relaxed reverse graph is complete");
        let relaxed_contains_start = matches!(
            &suffix_reachability,
            InnerPeakSuffixReachability::Complete {
                distance_by_entry,
                ..
            } if distance_by_entry.contains_key(&TilePos::new(start, 0))
        );
        assert!(
            relaxed_contains_start,
            "the relaxed graph intentionally retains the A0-B1-A2-B3-C4 witness"
        );

        let transit = InnerPeakTransitAuthority {
            runway_domain,
            ingress: BTreeSet::new(),
            egress,
            suffix_reachability: std::sync::Arc::new(suffix_reachability),
            ordered_runway: None,
        };
        let mut recovery_work = EXACT_RECOVERY_WORK_BUDGET;
        let mut search_incomplete = false;
        let route = segmented_portal_route_ranked_with_transit_budgeted(
            start,
            &masks,
            &allowed,
            &[59, 36],
            &BTreeMap::new(),
            Some(0),
            Some(&bounds),
            Some(&transit),
            &mut recovery_work,
            &mut search_incomplete,
        );
        assert!(
            route.is_err(),
            "relaxed reverse membership must not authorize a repeated-coordinate route"
        );
    }

    #[test]
    fn portal_handoff_backtracks_from_a_canonical_dead_end() {
        let start = HexCoord::from_axial(-2, 0);
        let bad_entry = HexCoord::from_axial(0, -1);
        let good_entry = HexCoord::from_axial(0, 1);
        let masks = BTreeMap::from([
            (
                PatchId(1),
                BTreeSet::from([
                    start,
                    HexCoord::from_axial(-1, -1),
                    HexCoord::from_axial(-1, 0),
                    HexCoord::from_axial(-1, 1),
                ]),
            ),
            (
                PatchId(2),
                BTreeSet::from([
                    bad_entry,
                    HexCoord::from_axial(1, -1),
                    good_entry,
                    HexCoord::from_axial(1, 1),
                ]),
            ),
            (PatchId(3), BTreeSet::from([HexCoord::from_axial(2, 1)])),
        ]);
        let allowed = masks
            .values()
            .flat_map(|mask| mask.iter().copied())
            .collect::<BTreeSet<_>>();

        let path = segmented_portal_path(start, &masks, &allowed, &[1, 2, 3])
            .expect("the second exact A-B portal reaches the B-C handoff");

        assert!(path.contains(&good_entry));
        assert!(!path.contains(&bad_entry));
        assert!(path.windows(2).all(|pair| {
            let [first, second] = pair else {
                return false;
            };
            first.distance(*second) == 1
        }));
    }

    #[test]
    fn portal_segment_ranking_prefers_a_gradeable_longer_path() {
        let start = HexCoord::from_axial(-2, 0);
        let hazardous = HexCoord::from_axial(-1, 0);
        let safe_turn = HexCoord::from_axial(-2, 1);
        let safe_portal = HexCoord::from_axial(-1, 1);
        let masks = BTreeMap::from([
            (
                PatchId(1),
                BTreeSet::from([start, hazardous, safe_turn, safe_portal]),
            ),
            (
                PatchId(2),
                BTreeSet::from([HexCoord::from_axial(0, 0), HexCoord::from_axial(0, 1)]),
            ),
        ]);
        let allowed = masks
            .values()
            .flat_map(|mask| mask.iter().copied())
            .collect::<BTreeSet<_>>();
        let penalties = BTreeMap::new();
        let bounds = BTreeMap::from([
            (start, (0, 0)),
            (hazardous, (4, 4)),
            (safe_turn, (0, 0)),
            (safe_portal, (0, 0)),
            (HexCoord::from_axial(0, 0), (4, 4)),
            (HexCoord::from_axial(0, 1), (0, 0)),
        ]);

        let path = segmented_portal_path_ranked(
            start,
            &masks,
            &allowed,
            &[1, 2],
            &penalties,
            Some(0),
            Some(&bounds),
        )
        .expect("ranked segment search avoids the hazardous canonical shortcut");

        assert!(path.contains(&safe_turn));
        assert!(path.contains(&safe_portal));
        assert!(!path.contains(&hazardous));
    }

    #[test]
    fn portal_route_skips_a_short_loop_and_uses_a_coordinate_simple_runway() {
        let start = HexCoord::ORIGIN;
        let short_portal = HexCoord::from_axial(0, 1);
        let long_portal = HexCoord::from_axial(6, 0);
        let runway = (0_i32..=5)
            .map(|q| HexCoord::from_axial(q, 0))
            .collect::<BTreeSet<_>>();
        let masks = BTreeMap::from([
            (PatchId(1), runway.clone()),
            (PatchId(2), BTreeSet::from([short_portal, long_portal])),
        ]);
        let allowed = masks
            .values()
            .flat_map(|mask| mask.iter().copied())
            .collect::<BTreeSet<_>>();
        let mut bounds = runway
            .iter()
            .copied()
            .map(|coord| (coord, (0, 4)))
            .collect::<BTreeMap<_, _>>();
        bounds.insert(start, (0, 0));
        bounds.insert(short_portal, (4, 4));
        bounds.insert(long_portal, (4, 4));

        let path = segmented_portal_path_ranked(
            start,
            &masks,
            &allowed,
            &[1, 2],
            &BTreeMap::new(),
            Some(0),
            Some(&bounds),
        )
        .expect("the longer coordinate-simple runway outranks a rejected altitude loop");

        assert_eq!(path.last(), Some(&long_portal));
        assert!(!path.contains(&short_portal));
        assert_eq!(
            path.iter().copied().collect::<BTreeSet<_>>().len(),
            path.len()
        );
        assert!(path
            .windows(2)
            .all(|pair| matches!(pair, [first, second] if first.distance(*second) == 1)));
    }

    #[test]
    fn exact_stage_runway_winds_without_consuming_its_high_portal_early() {
        let start = HexCoord::from_axial(1, 0);
        let portal_from = HexCoord::from_axial(0, 1);
        let portal_to = HexCoord::from_axial(-1, 2);
        let allowed = BTreeSet::from([
            start,
            HexCoord::from_axial(1, -1),
            HexCoord::from_axial(0, -1),
            HexCoord::from_axial(-1, 0),
            HexCoord::from_axial(-1, 1),
            portal_from,
        ]);
        let mut bounds = allowed
            .iter()
            .copied()
            .map(|coord| (coord, (0, 5)))
            .collect::<BTreeMap<_, _>>();
        bounds.insert(start, (0, 0));
        bounds.insert(portal_to, (4, 4));

        let route = exact_simple_stage_runway(
            TilePos::new(start, 0),
            &allowed,
            &[BoundaryPortal {
                from: portal_from,
                to: portal_to,
            }],
            &bounds,
            &BTreeSet::from([start]),
            1_000,
        )
        .expect("the self-avoiding ring supplies enough runway to reach the high portal");

        assert_eq!(route.first(), Some(&TilePos::new(start, 0)));
        assert_eq!(route.last(), Some(&TilePos::new(portal_to, 4)));
        assert_eq!(route.len(), 7);
        assert_eq!(
            route
                .iter()
                .map(|position| position.coord)
                .collect::<BTreeSet<_>>()
                .len(),
            route.len()
        );
        assert!(route.windows(2).all(|pair| {
            matches!(pair, [first, second] if first.coord.distance(second.coord) == 1
                && first.level.abs_diff(second.level) <= 1)
        }));
    }

    #[test]
    fn exact_stage_runway_winds_without_consuming_its_low_portal_early() {
        let start = HexCoord::from_axial(1, 0);
        let portal_from = HexCoord::from_axial(0, 1);
        let portal_to = HexCoord::from_axial(-1, 2);
        let allowed = BTreeSet::from([
            start,
            HexCoord::from_axial(1, -1),
            HexCoord::from_axial(0, -1),
            HexCoord::from_axial(-1, 0),
            HexCoord::from_axial(-1, 1),
            portal_from,
        ]);
        let mut bounds = allowed
            .iter()
            .copied()
            .map(|coord| (coord, (0, 5)))
            .collect::<BTreeMap<_, _>>();
        bounds.insert(start, (5, 5));
        bounds.insert(portal_to, (1, 1));

        let route = exact_simple_stage_runway(
            TilePos::new(start, 5),
            &allowed,
            &[BoundaryPortal {
                from: portal_from,
                to: portal_to,
            }],
            &bounds,
            &BTreeSet::from([start]),
            1_000,
        )
        .expect("the self-avoiding ring supplies enough runway to reach the low portal");

        assert_eq!(route.first(), Some(&TilePos::new(start, 5)));
        assert_eq!(route.last(), Some(&TilePos::new(portal_to, 1)));
        assert_eq!(route.len(), 7);
        assert_eq!(
            route
                .iter()
                .map(|position| position.coord)
                .collect::<BTreeSet<_>>()
                .len(),
            route.len()
        );
        assert!(route.windows(2).all(|pair| {
            matches!(pair, [first, second] if first.coord.distance(second.coord) == 1
                && first.level.abs_diff(second.level) <= 1)
        }));
    }

    #[test]
    fn exact_stage_runway_can_cross_an_infeasible_portal_coordinate() {
        let start = HexCoord::ORIGIN;
        let early_portal = HexCoord::from_axial(1, 0);
        let later_portal = HexCoord::from_axial(2, 0);
        let unreachable_handoff = HexCoord::from_axial(1, 1);
        let destination = HexCoord::from_axial(3, 0);
        let allowed = BTreeSet::from([start, early_portal, later_portal]);
        let bounds = BTreeMap::from([
            (start, (0, 0)),
            (early_portal, (0, 0)),
            (later_portal, (0, 0)),
            (unreachable_handoff, (4, 4)),
            (destination, (0, 0)),
        ]);

        let route = exact_simple_stage_runway(
            TilePos::new(start, 0),
            &allowed,
            &[
                BoundaryPortal {
                    from: early_portal,
                    to: unreachable_handoff,
                },
                BoundaryPortal {
                    from: later_portal,
                    to: destination,
                },
            ],
            &bounds,
            &BTreeSet::new(),
            100,
        )
        .expect("an unusable early portal remains valid same-stage transit");

        assert_eq!(
            route,
            vec![
                TilePos::new(start, 0),
                TilePos::new(early_portal, 0),
                TilePos::new(later_portal, 0),
                TilePos::new(destination, 0),
            ]
        );
    }

    #[test]
    fn portal_route_recovers_missing_history_in_any_authored_stage() {
        let start = HexCoord::from_axial(-1, 1);
        let entry = HexCoord::ORIGIN;
        let reused_choke = HexCoord::from_axial(1, 0);
        let dominated_state = HexCoord::from_axial(2, 0);
        let detour = [
            HexCoord::from_axial(0, -1),
            HexCoord::from_axial(1, -2),
            HexCoord::from_axial(2, -2),
            HexCoord::from_axial(3, -2),
            HexCoord::from_axial(3, -1),
            HexCoord::from_axial(3, 0),
        ];
        let tail = [
            HexCoord::from_axial(0, 1),
            HexCoord::from_axial(0, 2),
            HexCoord::from_axial(0, 3),
        ];
        let destination = HexCoord::from_axial(0, 4);
        let middle = [entry, reused_choke, dominated_state]
            .into_iter()
            .chain(detour)
            .chain(tail)
            .collect::<BTreeSet<_>>();
        let masks = BTreeMap::from([
            (PatchId(401), BTreeSet::from([start])),
            (PatchId(777), middle),
            (PatchId(902), BTreeSet::from([destination])),
        ]);
        let allowed = masks
            .values()
            .flat_map(|mask| mask.iter().copied())
            .collect::<BTreeSet<_>>();
        let mut bounds = BTreeMap::from([
            (start, (0, 0)),
            (entry, (0, 0)),
            (reused_choke, (0, 2)),
            (dominated_state, (1, 1)),
            (tail[0], (3, 3)),
            (tail[1], (4, 4)),
            (tail[2], (5, 5)),
            (destination, (6, 6)),
        ]);
        bounds.extend(detour.into_iter().map(|coord| (coord, (0, 0))));

        let route = segmented_portal_route_ranked(
            start,
            &masks,
            &allowed,
            &[401, 777, 902],
            &BTreeMap::new(),
            Some(0),
            Some(&bounds),
        )
        .expect("generic stage recovery retains the alternate self-avoiding history");

        assert_eq!(route.first(), Some(&TilePos::new(start, 0)));
        assert_eq!(route.last(), Some(&TilePos::new(destination, 6)));
        assert_eq!(
            route
                .iter()
                .map(|position| position.coord)
                .collect::<BTreeSet<_>>()
                .len(),
            route.len()
        );
        let dominated_index = route
            .iter()
            .position(|position| *position == TilePos::new(dominated_state, 1))
            .expect("the valid history reaches the dominated exact state");
        let choke_index = route
            .iter()
            .position(|position| *position == TilePos::new(reused_choke, 2))
            .expect("the valid history saves the choke for the rising tail");
        assert!(dominated_index < choke_index);
        assert!(route.windows(2).all(|pair| {
            matches!(pair, [first, second] if first.coord.distance(second.coord) == 1
                && first.level.abs_diff(second.level) <= 1)
        }));
    }

    #[test]
    fn portal_route_replays_an_earlier_stage_after_a_dead_later_handoff() {
        let start = HexCoord::from_axial(-1, 1);
        let entry = HexCoord::ORIGIN;
        let reused_choke = HexCoord::from_axial(1, 0);
        let dominated_state = HexCoord::from_axial(2, 0);
        let detour = [
            HexCoord::from_axial(0, -1),
            HexCoord::from_axial(1, -2),
            HexCoord::from_axial(2, -2),
            HexCoord::from_axial(3, -2),
            HexCoord::from_axial(3, -1),
            HexCoord::from_axial(3, 0),
        ];
        let tail = [
            HexCoord::from_axial(0, 1),
            HexCoord::from_axial(0, 2),
            HexCoord::from_axial(0, 3),
        ];
        let dead_handoff = HexCoord::from_axial(2, 1);
        let live_handoff = HexCoord::from_axial(0, 4);
        let live_exit = HexCoord::from_axial(0, 5);
        let destination = HexCoord::from_axial(0, 6);
        let middle = [entry, reused_choke, dominated_state]
            .into_iter()
            .chain(detour)
            .chain(tail)
            .collect::<BTreeSet<_>>();
        let masks = BTreeMap::from([
            (PatchId(10), BTreeSet::from([start])),
            (PatchId(20), middle),
            (
                PatchId(30),
                BTreeSet::from([dead_handoff, live_handoff, live_exit]),
            ),
            (PatchId(40), BTreeSet::from([destination])),
        ]);
        let allowed = masks
            .values()
            .flat_map(|mask| mask.iter().copied())
            .collect::<BTreeSet<_>>();
        let mut bounds = BTreeMap::from([
            (start, (0, 0)),
            (entry, (0, 0)),
            (reused_choke, (0, 2)),
            (dominated_state, (1, 1)),
            (tail[0], (3, 3)),
            (tail[1], (4, 4)),
            (tail[2], (5, 5)),
            (dead_handoff, (1, 1)),
            (live_handoff, (6, 6)),
            (live_exit, (7, 7)),
            (destination, (8, 8)),
        ]);
        bounds.extend(detour.into_iter().map(|coord| (coord, (0, 0))));

        let route = segmented_portal_route_ranked(
            start,
            &masks,
            &allowed,
            &[10, 20, 30, 40],
            &BTreeMap::new(),
            Some(0),
            Some(&bounds),
        )
        .expect("recovery backs up from the dead stage-30 ingress to the live stage-20 runway");

        assert!(route.contains(&TilePos::new(live_handoff, 6)));
        assert!(!route.iter().any(|position| position.coord == dead_handoff));
        assert_eq!(route.last(), Some(&TilePos::new(destination, 8)));
    }

    #[test]
    fn portal_route_retries_exact_handoff_levels_before_rejecting_a_contract() {
        let start = HexCoord::from_axial(-1, 1);
        let entry = HexCoord::ORIGIN;
        let reused_choke = HexCoord::from_axial(1, 0);
        let dominated_state = HexCoord::from_axial(2, 0);
        let detour = [
            HexCoord::from_axial(0, -1),
            HexCoord::from_axial(1, -2),
            HexCoord::from_axial(2, -2),
            HexCoord::from_axial(3, -2),
            HexCoord::from_axial(3, -1),
            HexCoord::from_axial(3, 0),
        ];
        let tail = [
            HexCoord::from_axial(0, 1),
            HexCoord::from_axial(0, 2),
            HexCoord::from_axial(0, 3),
        ];
        let handoff = HexCoord::from_axial(0, 4);
        let exit = HexCoord::from_axial(0, 5);
        let destination = HexCoord::from_axial(0, 6);
        let middle = [entry, reused_choke, dominated_state]
            .into_iter()
            .chain(detour)
            .chain(tail)
            .collect::<BTreeSet<_>>();
        let masks = BTreeMap::from([
            (PatchId(10), BTreeSet::from([start])),
            (PatchId(20), middle),
            (PatchId(30), BTreeSet::from([handoff, exit])),
            (PatchId(40), BTreeSet::from([destination])),
        ]);
        let allowed = masks
            .values()
            .flat_map(|mask| mask.iter().copied())
            .collect::<BTreeSet<_>>();
        let mut bounds = BTreeMap::from([
            (start, (0, 0)),
            (entry, (0, 0)),
            (reused_choke, (0, 2)),
            (dominated_state, (1, 1)),
            (tail[0], (0, 3)),
            (tail[1], (0, 4)),
            (tail[2], (0, 5)),
            (handoff, (0, 6)),
            (exit, (7, 7)),
            (destination, (8, 8)),
        ]);
        bounds.extend(detour.into_iter().map(|coord| (coord, (0, 0))));

        let route = segmented_portal_route_ranked(
            start,
            &masks,
            &allowed,
            &[10, 20, 30, 40],
            &BTreeMap::new(),
            Some(0),
            Some(&bounds),
        )
        .expect("the higher exact handoff remains available after lower continuations fail");

        assert!(route.contains(&TilePos::new(handoff, 6)));
        assert_eq!(route.last(), Some(&TilePos::new(destination, 8)));
    }

    #[test]
    fn portal_route_never_borrows_unowned_allowed_coordinates() {
        let start = HexCoord::ORIGIN;
        let bridge = HexCoord::from_axial(1, 0);
        let destination = HexCoord::from_axial(2, 0);
        let masks = BTreeMap::from([
            (PatchId(10), BTreeSet::from([start])),
            (PatchId(20), BTreeSet::from([destination])),
        ]);
        let allowed = BTreeSet::from([start, bridge, destination]);
        let bounds = BTreeMap::from([(start, (0, 0)), (bridge, (0, 0)), (destination, (0, 0))]);

        let error = segmented_portal_route_ranked(
            start,
            &masks,
            &allowed,
            &[10, 20],
            &BTreeMap::new(),
            Some(0),
            Some(&bounds),
        )
        .expect_err("a coordinate outside both authored cells cannot bridge their contract");

        assert!(error.contains("no exact admitted boundary handoff"));
    }

    #[test]
    fn recovery_budget_exhaustion_is_not_reported_as_proven_disconnection() {
        let mut search = ExactRecoverySearch::new(0);
        assert!(!search.expand_neighbor());
        let ExactRecoveryOutcome::SearchExhausted(stats) = search.no_route_outcome() else {
            panic!("a bounded miss must remain distinguishable from a disconnected contract");
        };
        assert!(stats.budget_exhausted);
        assert_eq!(stats.work_units, 0);
    }

    #[test]
    fn budgeted_portal_route_propagates_a_typed_incomplete_outcome() {
        let start = HexCoord::ORIGIN;
        let middle = HexCoord::from_axial(1, 0);
        let destination = HexCoord::from_axial(2, 0);
        let masks = BTreeMap::from([
            (PatchId(1), BTreeSet::from([start, middle])),
            (PatchId(2), BTreeSet::from([destination])),
        ]);
        let allowed = masks
            .values()
            .flat_map(|mask| mask.iter().copied())
            .collect::<BTreeSet<_>>();
        let bounds = BTreeMap::from([(start, (0, 0)), (middle, (0, 0)), (destination, (2, 2))]);
        let mut recovery_work_remaining = 0;
        let mut search_incomplete = false;

        let error = segmented_portal_route_ranked_with_transit_budgeted(
            start,
            &masks,
            &allowed,
            &[1, 2],
            &BTreeMap::new(),
            Some(0),
            Some(&bounds),
            None,
            &mut recovery_work_remaining,
            &mut search_incomplete,
        )
        .expect_err("zero recovery budget cannot exhaustively classify the blocked handoff");

        assert!(search_incomplete);
        assert!(error.contains("search-exhausted"));
    }

    #[test]
    fn every_bounded_recovery_prune_is_reported_as_search_exhaustion() {
        for stats in [
            ExactRecoveryStats {
                entry_states_pruned: 1,
                ..ExactRecoveryStats::default()
            },
            ExactRecoveryStats {
                beam_states_pruned: 1,
                ..ExactRecoveryStats::default()
            },
            ExactRecoveryStats {
                runway_candidates_pruned: 1,
                ..ExactRecoveryStats::default()
            },
            ExactRecoveryStats {
                stage_enumerations_truncated: 1,
                ..ExactRecoveryStats::default()
            },
        ] {
            let search = ExactRecoverySearch {
                remaining_work: 1,
                stats,
            };
            assert!(matches!(
                search.no_route_outcome(),
                ExactRecoveryOutcome::SearchExhausted(_)
            ));
        }

        let exhaustive = ExactRecoverySearch::new(1);
        assert!(matches!(
            exhaustive.no_route_outcome(),
            ExactRecoveryOutcome::ProvenDisconnected(_)
        ));
    }

    #[test]
    fn runway_enumeration_preserves_work_for_an_already_discovered_handoff() {
        let start = HexCoord::ORIGIN;
        let branch = HexCoord::from_axial(1, 0);
        let destination = HexCoord::from_axial(-1, 0);
        let allowed = BTreeSet::from([start, branch]);
        let bounds = BTreeMap::from([(start, (0, 0)), (branch, (0, 0)), (destination, (0, 0))]);
        let mut search = ExactRecoverySearch::new(4);

        let runways = exact_simple_stage_runways(
            TilePos::new(start, 0),
            &allowed,
            &[BoundaryPortal {
                from: start,
                to: destination,
            }],
            &bounds,
            &BTreeSet::new(),
            &BTreeMap::new(),
            1,
            &mut search,
        );

        assert_eq!(
            runways,
            vec![vec![TilePos::new(start, 0), TilePos::new(destination, 0)]]
        );
        assert_eq!(search.stats.stage_enumerations_truncated, 1);
        assert_eq!(search.remaining_work, 3);
        assert!(search.attempt_continuation());
    }

    #[test]
    fn handoffs_and_recursive_continuations_share_the_recovery_budget() {
        let mut search = ExactRecoverySearch::new(2);
        assert!(search.consider_handoff());
        assert!(search.attempt_continuation());
        assert!(!search.expand_neighbor());
        assert_eq!(search.stats.work_units, 2);
        assert_eq!(search.stats.handoffs_considered, 1);
        assert_eq!(search.stats.continuations_attempted, 1);
        assert_eq!(search.stats.neighbor_expansions, 0);
        assert!(search.stats.budget_exhausted);
    }

    #[test]
    fn recovery_budget_reserves_work_for_every_earlier_stage() {
        let first = recovery_stage_work_budget(100_000, 3);
        assert_eq!(first, 83_616);
        let second = recovery_stage_work_budget(100_000 - first, 2);
        assert_eq!(second, EXACT_RECOVERY_EARLIER_STAGE_RESERVE);
        let third = recovery_stage_work_budget(100_000 - first - second, 1);
        assert_eq!(third, EXACT_RECOVERY_EARLIER_STAGE_RESERVE);

        assert_eq!(recovery_stage_work_budget(10, 3), 3);
        assert_eq!(recovery_stage_work_budget(7, 2), 3);
        assert_eq!(recovery_stage_work_budget(4, 1), 4);

        assert_eq!(recovery_runway_work_budget(100_000, 3), 75_424);
        assert_eq!(recovery_runway_work_budget(10, 3), 5);
        assert_eq!(recovery_runway_work_budget(4, 1), 2);
    }

    #[test]
    fn portal_route_visits_the_complete_required_inner_chain_sequence() {
        let sequence = INNER_CHAIN_ROUTE;
        let masks = sequence
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let start_q = i32::try_from(index).unwrap_or(i32::MAX).saturating_mul(2);
                (
                    PatchId(u32::from(*id)),
                    BTreeSet::from([
                        HexCoord::from_axial(start_q, 0),
                        HexCoord::from_axial(start_q.saturating_add(1), 0),
                    ]),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let allowed = masks
            .values()
            .flat_map(|mask| mask.iter().copied())
            .collect::<BTreeSet<_>>();

        let path = segmented_portal_path(HexCoord::ORIGIN, &masks, &allowed, &sequence)
            .expect("the complete authored sequence resolves by adjacent portals");
        let mut owners = Vec::new();
        for coord in &path {
            let owner = sequence
                .iter()
                .copied()
                .find(|id| {
                    masks
                        .get(&PatchId(u32::from(*id)))
                        .is_some_and(|mask| mask.contains(coord))
                })
                .expect("every route coordinate has one authored owner");
            if owners.last().copied() != Some(owner) {
                owners.push(owner);
            }
        }
        assert_eq!(owners, sequence);
        assert!(path.windows(2).all(|pair| {
            let [first, second] = pair else {
                return false;
            };
            first.distance(*second) == 1
        }));
    }

    #[test]
    fn taper_bounds_keep_a_mutable_authority_edge_regradeable() {
        let start = HexCoord::ORIGIN;
        let mutable_edge = HexCoord::from_axial(1, 0);
        let immutable = HexCoord::from_axial(2, 0);
        let footprint = BTreeSet::from([start, mutable_edge, immutable]);
        let mut volume = VolumePlan::new(footprint.clone());
        for (coord, level) in [(start, 0), (mutable_edge, 10), (immutable, 10)] {
            let surface = TilePos::new(coord, level);
            volume
                .columns
                .insert(coord, land_column(level, SolidMaterialRole::Stone));
            volume.surfaces.insert(
                surface,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }

        let mutable_allowed = BTreeSet::from([start, mutable_edge]);
        let tapered = shoulder_taper_safe_route_bounds(
            BTreeMap::from([(start, (0, 0)), (mutable_edge, (0, 20))]),
            &mutable_allowed,
            &volume,
        );

        assert_eq!(tapered.get(&start), Some(&(0, 0)));
        assert_eq!(tapered.get(&mutable_edge), Some(&(1, 19)));

        let masks = BTreeMap::from([
            (PatchId(1), BTreeSet::from([start])),
            (PatchId(2), BTreeSet::from([mutable_edge])),
        ]);
        let route = segmented_portal_route_ranked(
            start,
            &masks,
            &mutable_allowed,
            &[1, 2],
            &BTreeMap::new(),
            Some(0),
            Some(&tapered),
        )
        .expect("the mutable edge can taper to immutable terrain without being pinned to level 10");

        assert_eq!(
            route,
            vec![TilePos::new(start, 0), TilePos::new(mutable_edge, 1)]
        );
    }

    #[test]
    fn ledge_grading_is_upper_one_step_and_preserves_its_exact_junction() {
        let corridor = (0..=20)
            .map(|q| HexCoord::from_axial(q, 0))
            .collect::<BTreeSet<_>>();
        let surrounding = corridor
            .iter()
            .flat_map(|coord| coord.within_radius(1))
            .collect::<BTreeSet<_>>();
        let mut volume = VolumePlan::new(surrounding.clone());
        for coord in &surrounding {
            let shoulder_raise = coord.x().clamp(0, 16);
            let surface = TilePos::new(*coord, 150_i32.saturating_add(shoulder_raise));
            volume
                .columns
                .insert(*coord, land_column(surface.level, SolidMaterialRole::Stone));
            volume.surfaces.insert(
                surface,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }
        let junction = TilePos::new(HexCoord::ORIGIN, 150);

        let mutable = surrounding
            .difference(&BTreeSet::from([junction.coord]))
            .copied()
            .collect::<BTreeSet<_>>();
        let exact_centerline = (0..=20)
            .map(|q| {
                let coord = HexCoord::from_axial(q, 0);
                volume
                    .top_surface_at_coord(coord)
                    .map(|(surface, _)| surface)
                    .expect("test corridor retains one exact surface")
            })
            .collect::<Vec<_>>();
        let graded = grade_authored_inner_peak_ledge(
            junction,
            &exact_centerline,
            &mutable,
            None,
            &BTreeMap::new(),
            &volume,
        )
        .expect("a broad lower shoulder admits a one-step ledge");
        let levels = graded.route_levels;

        assert_eq!(levels.get(&junction.coord).copied(), Some(junction.level));
        assert!(exact_centerline
            .iter()
            .all(|position| levels.get(&position.coord).copied() == Some(position.level)));
        assert!(levels
            .values()
            .copied()
            .max()
            .is_some_and(|level| { level > junction.level && level <= 166 }));
        assert!(levels
            .values()
            .all(|level| OrdinaryRegionBand::Upper.accepts_new(*level)));
        assert!(corridor.iter().all(|coord| {
            coord.neighbors().into_iter().all(|neighbor| {
                !corridor.contains(&neighbor)
                    || levels
                        .get(coord)
                        .zip(levels.get(&neighbor))
                        .is_some_and(|(first, second)| first.abs_diff(*second) <= 1)
            })
        }));
    }
}
