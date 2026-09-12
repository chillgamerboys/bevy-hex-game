//! Shared deterministic coastline planning for the V3 island recipes.
//!
//! The planner owns only horizontal topology and dry-surface levels. Sandy
//! Islets and Wooded Island retain their separate material, feature, anchor, and
//! validation contracts.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::{HexCoord, Level, TilePos};

use super::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidFlowState, LiquidNode, LiquidPlan};
use super::seed::SeedStream;
use super::volume::{
    FillMaterialRole, LevelInterval, NonSolidFill, SolidMass, SolidMaterialRole, SurfaceAccess,
    SurfaceMetadata, VolumeColumn, VolumeElement, VolumePlan,
};

pub(super) const REQUIRED_SEA_LEVEL: Level = 8;
pub(super) const SAND_FRINGE_WIDTH: u32 = 2;

#[derive(Debug, Clone, Copy)]
pub(super) struct CoastalPlannerSettings {
    pub(super) sea_level: Level,
    pub(super) land_coverage_percent: u8,
    pub(super) component_count: u8,
    pub(super) max_relief: Level,
}

#[derive(Debug, Clone)]
pub(super) struct CoastalIslandPlan {
    pub(super) land: BTreeSet<HexCoord>,
    pub(super) water: BTreeSet<HexCoord>,
    pub(super) components: Vec<BTreeSet<HexCoord>>,
    pub(super) primary_index: usize,
    pub(super) levels: BTreeMap<HexCoord, Level>,
    pub(super) sand_fringe: BTreeSet<HexCoord>,
    pub(super) grass_interior: BTreeSet<HexCoord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CoastalSurfacePalette {
    AllSand,
    SandAndGrass,
}

#[derive(Debug)]
pub(super) struct CoastalSemanticPlan {
    pub(super) volume: VolumePlan,
    pub(super) liquids: LiquidPlan,
    pub(super) dry_surfaces: BTreeMap<HexCoord, TilePos>,
}

impl CoastalIslandPlan {
    pub(super) fn primary(&self) -> &BTreeSet<HexCoord> {
        self.components
            .get(self.primary_index)
            .unwrap_or(&self.land)
    }
}

/// Plans an exact requested number of mutually nonadjacent dry components.
///
/// `required_primary` contains walker seam approaches. Those exact cells are
/// always dry and are connected into component zero before the organic growth
/// pass. Every other boundary column remains ocean.
pub(super) fn plan_coast(
    mask: &BTreeSet<HexCoord>,
    settings: CoastalPlannerSettings,
    required_primary: &BTreeSet<HexCoord>,
    stream: Option<SeedStream<'_>>,
) -> Result<CoastalIslandPlan, String> {
    validate_inputs(mask, settings, required_primary)?;
    let target = coverage_target(mask.len(), settings.land_coverage_percent);
    let component_count = usize::from(settings.component_count);
    if target < component_count {
        return Err(format!(
            "coastal land target {target} cannot contain {component_count} components"
        ));
    }

    let boundary = boundary_coords(mask);
    let candidates = mask
        .iter()
        .copied()
        .filter(|coord| !boundary.contains(coord) || required_primary.contains(coord))
        .collect::<BTreeSet<_>>();
    if candidates.len() < target {
        return Err(format!(
            "coastal interior capacity {} is smaller than requested land target {target}",
            candidates.len()
        ));
    }

    let mut primary = connect_required_primary(mask, required_primary)?;
    if primary.is_empty() {
        let seed = select_first_seed(&candidates, &boundary, stream)
            .ok_or_else(|| "coastal planner could not select a primary centre".to_owned())?;
        primary.insert(seed);
    }
    if !primary.is_subset(&candidates) {
        return Err("coastal required route enters a reserved boundary-ocean column".to_owned());
    }

    let mut components = vec![primary];
    while components.len() < component_count {
        let occupied = components
            .iter()
            .flat_map(BTreeSet::iter)
            .copied()
            .collect::<BTreeSet<_>>();
        let seed =
            select_farthest_seed(&candidates, &occupied, &components, stream).ok_or_else(|| {
                format!(
                    "coastal footprint cannot separate {} requested components",
                    component_count
                )
            })?;
        components.push(BTreeSet::from([seed]));
    }

    let relief_radius =
        usize::try_from(settings.max_relief.saturating_sub(1)).unwrap_or(usize::MAX);
    let relief_capacity = 1_usize.saturating_add(
        3_usize
            .saturating_mul(relief_radius)
            .saturating_mul(relief_radius.saturating_add(1)),
    );
    let primary_minimum = components
        .first()
        .map_or(relief_capacity, BTreeSet::len)
        .max(relief_capacity);
    let quotas = component_quotas(target, component_count, primary_minimum);
    grow_components(&candidates, &quotas, stream, &mut components)?;
    components.sort_by_key(|component| {
        let is_primary = !required_primary.is_empty()
            && required_primary
                .iter()
                .all(|coord| component.contains(coord));
        (
            !is_primary,
            std::cmp::Reverse(component.len()),
            component.first().copied(),
        )
    });
    let primary_index = if required_primary.is_empty() {
        0
    } else {
        components
            .iter()
            .position(|component| required_primary.is_subset(component))
            .ok_or_else(|| "coastal growth lost a required primary route".to_owned())?
    };
    let land = components
        .iter()
        .flat_map(BTreeSet::iter)
        .copied()
        .collect::<BTreeSet<_>>();
    if land.len() != target {
        return Err(format!(
            "coastal planner produced {} land columns; expected {target}",
            land.len()
        ));
    }
    let water = mask.difference(&land).copied().collect::<BTreeSet<_>>();
    let shore_distances = inward_distances(&land);
    let sand_fringe = shore_distances
        .iter()
        .filter_map(|(coord, distance)| (*distance <= SAND_FRINGE_WIDTH).then_some(*coord))
        .collect::<BTreeSet<_>>();
    let grass_interior = land
        .difference(&sand_fringe)
        .copied()
        .collect::<BTreeSet<_>>();
    let levels = shore_distances
        .iter()
        .map(|(coord, distance)| {
            let rise = i32::try_from(*distance)
                .unwrap_or(i32::MAX)
                .clamp(1, settings.max_relief);
            (*coord, settings.sea_level.saturating_add(rise))
        })
        .collect::<BTreeMap<_, _>>();
    let authored_relief = levels
        .values()
        .copied()
        .max()
        .unwrap_or(settings.sea_level)
        .saturating_sub(settings.sea_level);
    if authored_relief != settings.max_relief {
        return Err(format!(
            "coastal footprint reaches relief {authored_relief}; expected {}",
            settings.max_relief
        ));
    }

    Ok(CoastalIslandPlan {
        land,
        water,
        components,
        primary_index,
        levels,
        sand_fringe,
        grass_interior,
    })
}

pub(super) fn coverage_target(columns: usize, percentage: u8) -> usize {
    columns
        .saturating_mul(usize::from(percentage))
        .saturating_add(50)
        / 100
}

pub(super) fn build_semantics(
    mask: &BTreeSet<HexCoord>,
    coast: &CoastalIslandPlan,
    sea_level: Level,
    palette: CoastalSurfacePalette,
    access_by_coord: impl Fn(TilePos, SurfaceAccess) -> SurfaceAccess,
) -> Result<CoastalSemanticPlan, String> {
    if sea_level != REQUIRED_SEA_LEVEL
        || coast
            .land
            .union(&coast.water)
            .copied()
            .collect::<BTreeSet<_>>()
            != *mask
        || !coast.land.is_disjoint(&coast.water)
    {
        return Err("coastal semantic projection received inconsistent topology".to_owned());
    }
    let bed_level = sea_level.saturating_sub(2);
    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    let mut dry_surfaces = BTreeMap::new();
    for coord in mask {
        if coast.water.contains(coord) {
            let position = TilePos::new(*coord, bed_level);
            columns.insert(*coord, submerged_sand_column(bed_level, sea_level));
            surfaces.insert(
                position,
                SurfaceMetadata {
                    access: access_by_coord(position, SurfaceAccess::NonStandable),
                    interior: None,
                },
            );
            continue;
        }
        let level = coast
            .levels
            .get(coord)
            .copied()
            .ok_or_else(|| format!("coastal dry coordinate {coord:?} has no authored level"))?;
        let position = TilePos::new(*coord, level);
        let material = match palette {
            CoastalSurfacePalette::AllSand => SolidMaterialRole::Sand,
            CoastalSurfacePalette::SandAndGrass if coast.sand_fringe.contains(coord) => {
                SolidMaterialRole::Sand
            }
            CoastalSurfacePalette::SandAndGrass => SolidMaterialRole::Grass,
        };
        columns.insert(*coord, dry_column(level, material));
        surfaces.insert(
            position,
            SurfaceMetadata {
                access: access_by_coord(position, SurfaceAccess::Ordinary),
                interior: None,
            },
        );
        dry_surfaces.insert(*coord, position);
    }
    let liquid_bodies = connected_components(&coast.water)
        .into_iter()
        .enumerate()
        .map(|(index, component)| {
            let nodes = component
                .into_iter()
                .map(|coord| {
                    (
                        TilePos::new(coord, sea_level),
                        LiquidNode {
                            state: LiquidFlowState::Still,
                            downstream: None,
                        },
                    )
                })
                .collect();
            (
                LiquidBodyId(u32::try_from(index).unwrap_or(u32::MAX)),
                LiquidBodyPlan {
                    material: FillMaterialRole::Water,
                    nodes,
                },
            )
        })
        .collect();
    Ok(CoastalSemanticPlan {
        volume: VolumePlan {
            mask: mask.clone(),
            columns,
            surfaces,
        },
        liquids: LiquidPlan {
            bodies: liquid_bodies,
        },
        dry_surfaces,
    })
}

pub(super) fn surface_material(
    volume: &VolumePlan,
    position: TilePos,
) -> Option<SolidMaterialRole> {
    volume
        .columns
        .get(&position.coord)?
        .elements
        .iter()
        .find_map(|element| match element {
            VolumeElement::Solid(mass) if mass.levels.top == position.level.saturating_add(1) => {
                Some(mass.material)
            }
            VolumeElement::Solid(_) | VolumeElement::Fill(_) => None,
        })
}

fn dry_column(surface: Level, cap: SolidMaterialRole) -> VolumeColumn {
    VolumeColumn {
        elements: vec![
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(0, 1),
                material: SolidMaterialRole::Bedrock,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(1, surface.saturating_sub(3)),
                material: SolidMaterialRole::Stone,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(surface.saturating_sub(3), surface),
                material: SolidMaterialRole::Dirt,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(surface, surface.saturating_add(1)),
                material: cap,
                cutaway_for: None,
            }),
        ],
    }
}

fn submerged_sand_column(bed_level: Level, sea_level: Level) -> VolumeColumn {
    VolumeColumn {
        elements: vec![
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(0, 1),
                material: SolidMaterialRole::Bedrock,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(1, bed_level),
                material: SolidMaterialRole::Stone,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(bed_level, bed_level.saturating_add(1)),
                material: SolidMaterialRole::Sand,
                cutaway_for: None,
            }),
            VolumeElement::Fill(NonSolidFill {
                levels: LevelInterval::new(
                    bed_level.saturating_add(1),
                    sea_level.saturating_add(1),
                ),
                material: FillMaterialRole::Water,
            }),
        ],
    }
}

pub(super) fn connected_components(coords: &BTreeSet<HexCoord>) -> Vec<BTreeSet<HexCoord>> {
    let mut remaining = coords.clone();
    let mut components = Vec::new();
    while let Some(start) = remaining.first().copied() {
        remaining.remove(&start);
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
    components.sort_by_key(|component| {
        (
            std::cmp::Reverse(component.len()),
            component.first().copied(),
        )
    });
    components
}

/// Proves the water around an island is one sea and reaches every unreserved edge.
pub(super) fn validate_ocean_topology(
    mask: &BTreeSet<HexCoord>,
    water: &BTreeSet<HexCoord>,
    allowed_dry_boundary: &BTreeSet<HexCoord>,
) -> Result<(), String> {
    if connected_components(water).len() != 1 {
        return Err("coastal water must form exactly one connected sea".to_owned());
    }
    let boundary = boundary_coords(mask);
    let unexpected_dry = boundary
        .difference(water)
        .filter(|coord| !allowed_dry_boundary.contains(coord))
        .copied()
        .collect::<BTreeSet<_>>();
    if !unexpected_dry.is_empty() {
        return Err(format!(
            "coastal sea must occupy every unreserved boundary column; dry boundary: {unexpected_dry:?}"
        ));
    }
    Ok(())
}

pub(super) fn inward_distances(land: &BTreeSet<HexCoord>) -> BTreeMap<HexCoord, u32> {
    let shoreline = land
        .iter()
        .copied()
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .any(|neighbor| !land.contains(&neighbor))
        })
        .collect::<BTreeSet<_>>();
    let mut distances = shoreline
        .iter()
        .copied()
        .map(|coord| (coord, 1_u32))
        .collect::<BTreeMap<_, _>>();
    let mut frontier = VecDeque::from_iter(shoreline);
    while let Some(coord) = frontier.pop_front() {
        let Some(distance) = distances.get(&coord).copied() else {
            continue;
        };
        for neighbor in coord.neighbors() {
            if land.contains(&neighbor) && !distances.contains_key(&neighbor) {
                distances.insert(neighbor, distance.saturating_add(1));
                frontier.push_back(neighbor);
            }
        }
    }
    distances
}

fn validate_inputs(
    mask: &BTreeSet<HexCoord>,
    settings: CoastalPlannerSettings,
    required_primary: &BTreeSet<HexCoord>,
) -> Result<(), String> {
    if mask.is_empty() || connected_components(mask).len() != 1 {
        return Err("coastal planner requires one nonempty connected mask".to_owned());
    }
    if settings.sea_level != REQUIRED_SEA_LEVEL
        || !(1..=9).contains(&settings.component_count)
        || !(1..=100).contains(&settings.land_coverage_percent)
        || settings.max_relief <= 0
    {
        return Err("coastal planner settings are outside their exact contract".to_owned());
    }
    if !required_primary.is_subset(mask) {
        return Err("coastal required primary cells leave the owned mask".to_owned());
    }
    Ok(())
}

fn boundary_coords(mask: &BTreeSet<HexCoord>) -> BTreeSet<HexCoord> {
    mask.iter()
        .copied()
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .any(|neighbor| !mask.contains(&neighbor))
        })
        .collect()
}

fn connect_required_primary(
    mask: &BTreeSet<HexCoord>,
    required: &BTreeSet<HexCoord>,
) -> Result<BTreeSet<HexCoord>, String> {
    let Some(first) = required.first().copied() else {
        return Ok(BTreeSet::new());
    };
    let mut connected = BTreeSet::from([first]);
    for terminal in required.iter().copied().skip(1) {
        let path = shortest_path_to_set(mask, terminal, &connected)
            .ok_or_else(|| "coastal walker approaches cannot be connected".to_owned())?;
        connected.extend(path);
    }
    Ok(connected)
}

fn shortest_path_to_set(
    mask: &BTreeSet<HexCoord>,
    start: HexCoord,
    destinations: &BTreeSet<HexCoord>,
) -> Option<Vec<HexCoord>> {
    if destinations.contains(&start) {
        return Some(vec![start]);
    }
    let mut parent = BTreeMap::from([(start, None)]);
    let mut frontier = VecDeque::from([start]);
    let mut reached = None;
    while let Some(coord) = frontier.pop_front() {
        let mut neighbors = coord.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if !mask.contains(&neighbor) || parent.contains_key(&neighbor) {
                continue;
            }
            parent.insert(neighbor, Some(coord));
            if destinations.contains(&neighbor) {
                reached = Some(neighbor);
                frontier.clear();
                break;
            }
            frontier.push_back(neighbor);
        }
    }
    let mut cursor = reached?;
    let mut path = vec![cursor];
    while cursor != start {
        cursor = parent.get(&cursor).copied().flatten()?;
        path.push(cursor);
    }
    Some(path)
}

fn select_first_seed(
    candidates: &BTreeSet<HexCoord>,
    boundary: &BTreeSet<HexCoord>,
    stream: Option<SeedStream<'_>>,
) -> Option<HexCoord> {
    candidates.iter().copied().max_by_key(|coord| {
        (
            distance_to_set(*coord, boundary),
            priority(stream, *coord, 1),
            *coord,
        )
    })
}

fn select_farthest_seed(
    candidates: &BTreeSet<HexCoord>,
    occupied: &BTreeSet<HexCoord>,
    components: &[BTreeSet<HexCoord>],
    stream: Option<SeedStream<'_>>,
) -> Option<HexCoord> {
    candidates
        .iter()
        .copied()
        .filter(|coord| {
            !occupied.contains(coord)
                && coord
                    .neighbors()
                    .into_iter()
                    .all(|neighbor| !occupied.contains(&neighbor))
        })
        .max_by_key(|coord| {
            let separation = components
                .iter()
                .flat_map(BTreeSet::iter)
                .map(|other| coord.distance(*other))
                .min()
                .unwrap_or_default();
            (separation, priority(stream, *coord, 7), *coord)
        })
}

fn distance_to_set(coord: HexCoord, others: &BTreeSet<HexCoord>) -> u32 {
    others
        .iter()
        .map(|other| coord.distance(*other))
        .min()
        .unwrap_or_default()
}

fn component_quotas(target: usize, count: usize, primary_minimum: usize) -> Vec<usize> {
    if count == 1 {
        return vec![target.max(primary_minimum)];
    }
    let mut quotas = vec![0; count];
    let primary = target
        .saturating_mul(2)
        .checked_div(count.saturating_add(1))
        .unwrap_or_default()
        .max(primary_minimum)
        .min(target.saturating_sub(count.saturating_sub(1)));
    if let Some(quota) = quotas.first_mut() {
        *quota = primary;
    }
    let remaining = target.saturating_sub(primary);
    for index in 1..count {
        let slots = count.saturating_sub(1);
        let quota = remaining / slots + usize::from(index <= remaining % slots);
        if let Some(destination) = quotas.get_mut(index) {
            *destination = quota;
        }
    }
    quotas
}

fn grow_components(
    candidates: &BTreeSet<HexCoord>,
    quotas: &[usize],
    stream: Option<SeedStream<'_>>,
    components: &mut [BTreeSet<HexCoord>],
) -> Result<(), String> {
    let mut labels = BTreeMap::new();
    let mut centres = Vec::new();
    let mut frontiers = Vec::new();
    for (index, component) in components.iter().enumerate() {
        for coord in component {
            labels.insert(*coord, index);
        }
        let centre = component
            .first()
            .copied()
            .ok_or_else(|| "coastal growth received an empty component seed".to_owned())?;
        centres.push(centre);
        let mut frontier = BTreeSet::new();
        for coord in component {
            add_frontier_neighbors(
                *coord,
                index,
                centre,
                candidates,
                stream,
                &labels,
                &mut frontier,
            );
        }
        frontiers.push(frontier);
    }

    loop {
        let mut changed = false;
        let mut complete = true;
        for index in 0..components.len() {
            let quota = quotas.get(index).copied().unwrap_or_default();
            if components
                .get(index)
                .is_some_and(|component| component.len() >= quota)
            {
                continue;
            }
            complete = false;
            let Some(frontier) = frontiers.get_mut(index) else {
                continue;
            };
            let mut accepted = None;
            while let Some(entry) = frontier.pop_first() {
                let coord = entry.2;
                if labels.contains_key(&coord)
                    || coord
                        .neighbors()
                        .into_iter()
                        .any(|neighbor| labels.get(&neighbor).is_some_and(|owner| *owner != index))
                {
                    continue;
                }
                accepted = Some(coord);
                break;
            }
            let Some(coord) = accepted else {
                continue;
            };
            labels.insert(coord, index);
            let Some(component) = components.get_mut(index) else {
                continue;
            };
            component.insert(coord);
            let Some(centre) = centres.get(index).copied() else {
                continue;
            };
            add_frontier_neighbors(coord, index, centre, candidates, stream, &labels, frontier);
            changed = true;
        }
        if complete {
            return Ok(());
        }
        if !changed {
            let actual = components.iter().map(BTreeSet::len).sum::<usize>();
            let target = quotas.iter().sum::<usize>();
            return Err(format!(
                "coastal separated-component growth stopped at {actual}/{target} land columns"
            ));
        }
    }
}

fn add_frontier_neighbors(
    coord: HexCoord,
    component: usize,
    centre: HexCoord,
    candidates: &BTreeSet<HexCoord>,
    stream: Option<SeedStream<'_>>,
    labels: &BTreeMap<HexCoord, usize>,
    frontier: &mut BTreeSet<(u32, u64, HexCoord)>,
) {
    for neighbor in coord.neighbors() {
        if candidates.contains(&neighbor) && !labels.contains_key(&neighbor) {
            frontier.insert((
                centre.distance(neighbor),
                priority(
                    stream,
                    neighbor,
                    u64::try_from(component).unwrap_or(u64::MAX),
                ),
                neighbor,
            ));
        }
    }
}

fn priority(stream: Option<SeedStream<'_>>, coord: HexCoord, salt: u64) -> u64 {
    stream.map_or_else(
        || {
            let x = u64::from(coord.x().unsigned_abs());
            let y = u64::from(coord.y().unsigned_abs());
            x.rotate_left(17) ^ y.rotate_left(41) ^ salt.rotate_left(7)
        },
        |stream| stream.sample_coord(coord, salt),
    )
}

#[cfg(test)]
mod tests {
    use super::super::seed::SeedStreams;
    use super::*;

    #[test]
    fn radius_twenty_four_supports_five_exact_separated_components() {
        let mask = HexCoord::ORIGIN.within_radius(24).into_iter().collect();
        let plan = plan_coast(
            &mask,
            CoastalPlannerSettings {
                sea_level: 8,
                land_coverage_percent: 30,
                component_count: 5,
                max_relief: 4,
            },
            &BTreeSet::new(),
            Some(SeedStreams::new(73, 0, 0).stage("test.coast")),
        )
        .expect("focused islet footprint should fit");

        assert_eq!(plan.components.len(), 5);
        assert_eq!(plan.land.len(), coverage_target(mask.len(), 30));
        assert!(plan.components.first().is_some_and(|primary| {
            plan.components
                .iter()
                .skip(1)
                .all(|other| primary.len() > other.len())
        }));
        for (index, component) in plan.components.iter().enumerate() {
            assert_eq!(connected_components(component).len(), 1);
            for other in plan.components.iter().skip(index.saturating_add(1)) {
                assert!(component.iter().all(|coord| {
                    coord
                        .neighbors()
                        .into_iter()
                        .all(|neighbor| !other.contains(&neighbor))
                }));
            }
        }
    }

    #[test]
    fn arbitrary_connected_mask_and_boundary_walker_route_are_preserved() {
        let first = HexCoord::ORIGIN
            .within_radius(12)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let second_origin = HexCoord::from_axial(20, 0);
        let mut mask = first;
        mask.extend(second_origin.within_radius(12));
        let boundary_terminal = mask
            .iter()
            .copied()
            .min_by_key(|coord| (coord.x(), coord.y()))
            .expect("mask has a boundary");
        let required = BTreeSet::from([boundary_terminal, HexCoord::ORIGIN]);
        let plan = plan_coast(
            &mask,
            CoastalPlannerSettings {
                sea_level: 8,
                land_coverage_percent: 55,
                component_count: 1,
                max_relief: 6,
            },
            &required,
            None,
        )
        .expect("arbitrary connected mask should be supported");

        assert!(required.is_subset(plan.primary()));
        assert_eq!(connected_components(&plan.land).len(), 1);
        assert_eq!(
            plan.land
                .union(&plan.water)
                .copied()
                .collect::<BTreeSet<_>>(),
            mask
        );
    }

    #[test]
    fn fringe_is_exactly_two_inward_columns() {
        let land = HexCoord::ORIGIN
            .within_radius(5)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let distances = inward_distances(&land);
        let fringe = distances
            .iter()
            .filter_map(|(coord, distance)| (*distance <= 2).then_some(*coord))
            .collect::<BTreeSet<_>>();

        assert!(fringe
            .iter()
            .all(|coord| distances.get(coord).is_some_and(|distance| *distance <= 2)));
        assert!(land
            .difference(&fringe)
            .all(|coord| distances.get(coord).is_some_and(|distance| *distance >= 3)));
    }

    #[test]
    fn ocean_topology_requires_one_sea_and_every_unreserved_boundary_column() {
        let mask = HexCoord::ORIGIN
            .within_radius(3)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let centre_land = BTreeSet::from([HexCoord::ORIGIN]);
        let water = mask
            .difference(&centre_land)
            .copied()
            .collect::<BTreeSet<_>>();
        validate_ocean_topology(&mask, &water, &BTreeSet::new())
            .expect("one boundary sea around central land should validate");

        let disconnected =
            BTreeSet::from([HexCoord::from_axial(-3, 0), HexCoord::from_axial(3, 0)]);
        assert!(validate_ocean_topology(&mask, &disconnected, &BTreeSet::new()).is_err());

        let dry_edge = HexCoord::from_axial(3, 0);
        let water_with_dry_edge = water
            .iter()
            .copied()
            .filter(|coord| *coord != dry_edge)
            .collect::<BTreeSet<_>>();
        assert!(validate_ocean_topology(&mask, &water_with_dry_edge, &BTreeSet::new()).is_err());
        validate_ocean_topology(&mask, &water_with_dry_edge, &BTreeSet::from([dry_edge]))
            .expect("an exact walker boundary aperture may remain dry");
    }
}
