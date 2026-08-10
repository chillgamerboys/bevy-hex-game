//! Shared terrain fitting for authored V3 river topology.
//!
//! A [`LiquidPlan`] owns flow, while a [`VolumePlan`] owns the ground which contains
//! it. Keeping the fitting pass here prevents recipes from independently accepting
//! a water run whose dry neighbour is lower than the water surface. Such a run is
//! valid occupancy, but renders as an elevated water ramp instead of an incised
//! channel.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use hex_core::{HexCoord, Level, TilePos};

use super::liquid::{LiquidBodyId, LiquidFlowState, LiquidNode, LiquidPlan};
use super::volume::{
    FillMaterialRole, LevelInterval, NonSolidFill, SolidMass, SolidMaterialRole, SurfaceAccess,
    SurfaceMetadata, VolumeElement, VolumeIssue, VolumePlan,
};

/// Exact changes made while fitting solid terrain around one liquid graph.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct RiverTerrainFit {
    /// Bed surfaces normalized directly below moving liquid runs.
    pub(crate) carved_beds: BTreeSet<TilePos>,
    /// Dry ground surfaces raised to contain a longitudinal flow lane.
    pub(crate) raised_banks: BTreeMap<TilePos, TilePos>,
}

/// Why an authored river could not be fitted without breaking another contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RiverTerrainIssue {
    InvalidInputVolume(Vec<VolumeIssue>),
    MissingLiquidFill {
        position: TilePos,
    },
    UnsupportedBedGeometry {
        position: TilePos,
    },
    ProtectedBed {
        position: TilePos,
    },
    MissingBankSurface {
        water: TilePos,
        coord: HexCoord,
    },
    ProtectedBankTooLow {
        water: TilePos,
        bank: TilePos,
        required_level: Level,
    },
    ExcessiveBankRaise {
        water: TilePos,
        bank: TilePos,
        required_level: Level,
    },
    UnsupportedBankGeometry {
        water: TilePos,
        bank: TilePos,
        required_level: Level,
    },
    LowBank {
        water: TilePos,
        bank: TilePos,
        required_level: Level,
    },
    SteepBankApron {
        bank: TilePos,
        outward: TilePos,
    },
}

impl fmt::Display for RiverTerrainIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInputVolume(issues) => {
                write!(
                    formatter,
                    "river terrain received an invalid volume: {issues:?}"
                )
            }
            Self::MissingLiquidFill { position } => {
                write!(formatter, "river node {position:?} has no exact water fill")
            }
            Self::UnsupportedBedGeometry { position } => write!(
                formatter,
                "river node {position:?} has no simple solid foundation which can be carved"
            ),
            Self::ProtectedBed { position } => write!(
                formatter,
                "river bed {position:?} is protected by an exact walker contract"
            ),
            Self::MissingBankSurface { water, coord } => write!(
                formatter,
                "river node {water:?} has no dry ground surface at lateral bank {coord:?}"
            ),
            Self::ProtectedBankTooLow {
                water,
                bank,
                required_level,
            } => write!(
                formatter,
                "river node {water:?} needs bank {bank:?} at level {required_level}, but that exact surface is protected"
            ),
            Self::ExcessiveBankRaise {
                water,
                bank,
                required_level,
            } => write!(
                formatter,
                "river node {water:?} would raise bank {bank:?} by more than one level to {required_level}; the water profile must be lowered"
            ),
            Self::UnsupportedBankGeometry {
                water,
                bank,
                required_level,
            } => write!(
                formatter,
                "river node {water:?} cannot raise bank {bank:?} to level {required_level} without changing stacked geometry"
            ),
            Self::LowBank {
                water,
                bank,
                required_level,
            } => write!(
                formatter,
                "river node {water:?} exposes water above bank {bank:?}; dry bank level must be at least {required_level}"
            ),
            Self::SteepBankApron { bank, outward } => write!(
                formatter,
                "raised river bank {bank:?} drops by more than one level to outward terrain {outward:?}; the water profile needs a wider fitted approach"
            ),
        }
    }
}

impl std::error::Error for RiverTerrainIssue {}

/// Atomically carves beds and raises dry banks around moving water lanes.
///
/// `Current`, `Rapid`, `Fall`, and their graph terminals form a river. Every in-mask
/// dry neighbour is a bank and must expose ground above the water. Only a `Fall`
/// node's exact downstream water neighbour is exempt as its vertical falling face;
/// lateral and upstream dry sides remain contained. Vertically overlapping authored
/// water in a neighbouring column is a channel cell, which handles wide lanes and
/// confluences without collapsing stacked liquid runs. Unconnected `Still` water is
/// not reclassified as a river. World and patch boundaries are not invented as banks.
///
/// Protected exact surfaces are never moved. Solid runs above a water fill, including
/// bridge decks, are also retained byte-for-byte. A bank may rise by at most one
/// level, and that rise must retain a one-level outward apron; larger corrections
/// reject so the caller lowers or reroutes the water instead of moving the visual
/// ramp into a levee. A failure leaves `volume` unchanged.
pub(crate) fn fit_river_terrain(
    volume: &mut VolumePlan,
    liquids: &LiquidPlan,
    protected_surfaces: &BTreeSet<TilePos>,
    bed_material: SolidMaterialRole,
) -> Result<RiverTerrainFit, Vec<RiverTerrainIssue>> {
    if let Err(issues) = volume.validate() {
        return Err(vec![RiverTerrainIssue::InvalidInputVolume(issues)]);
    }

    let river = river_water_nodes(liquids);
    let water = all_water_nodes(liquids);
    let requirements = bank_requirements(volume, &river, &water);
    let mut fitted = volume.clone();
    let mut report = RiverTerrainFit::default();
    let mut issues = Vec::new();

    for position in river.iter().filter_map(|(position, (_, node))| {
        (node.state != LiquidFlowState::Fall).then_some(*position)
    }) {
        match carve_bed(&mut fitted, position, protected_surfaces, bed_material) {
            Ok(bed) => {
                report.carved_beds.insert(bed);
            }
            Err(issue) => issues.push(issue),
        }
    }

    for (coord, requirement) in &requirements {
        match raise_bank(&mut fitted, *coord, *requirement, protected_surfaces) {
            Ok(Some((before, after))) => {
                report.raised_banks.insert(before, after);
            }
            Ok(None) => {}
            Err(issue) => issues.push(issue),
        }
    }

    if issues.is_empty() {
        issues.extend(validate_river_terrain(&fitted, liquids));
    }
    if let Err(volume_issues) = fitted.validate() {
        issues.push(RiverTerrainIssue::InvalidInputVolume(volume_issues));
    }
    if issues.is_empty() {
        *volume = fitted;
        Ok(report)
    } else {
        Err(issues)
    }
}

/// Checks non-fall river beds, all lateral banks, and containment-height bank aprons.
#[must_use]
pub(crate) fn validate_river_terrain(
    volume: &VolumePlan,
    liquids: &LiquidPlan,
) -> Vec<RiverTerrainIssue> {
    let river = river_water_nodes(liquids);
    let water = all_water_nodes(liquids);
    let requirements = bank_requirements(volume, &river, &water);
    let mut issues = Vec::new();

    for position in river.iter().filter_map(|(position, (_, node))| {
        (node.state != LiquidFlowState::Fall).then_some(*position)
    }) {
        match exact_fill_and_bed(volume, position) {
            Ok((_fill_index, _fill_bottom, bed)) => {
                let bed_is_solid = volume.columns.get(&bed.coord).is_some_and(|column| {
                    column.elements.iter().any(|element| {
                        matches!(
                            element,
                            VolumeElement::Solid(mass)
                                if mass.levels.bottom <= bed.level
                                    && bed.level < mass.levels.top
                        )
                    })
                });
                if !bed_is_solid {
                    issues.push(RiverTerrainIssue::UnsupportedBedGeometry { position });
                }
            }
            Err(issue) => issues.push(issue),
        }
    }

    for (coord, requirement) in &requirements {
        let Some(bank) = ground_surface(volume, *coord) else {
            issues.push(RiverTerrainIssue::MissingBankSurface {
                water: requirement.water,
                coord: *coord,
            });
            continue;
        };
        if bank.level < requirement.level {
            issues.push(RiverTerrainIssue::LowBank {
                water: requirement.water,
                bank,
                required_level: requirement.level,
            });
        }
    }
    issues.extend(validate_bank_aprons(volume, &requirements, &water));
    issues
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BankRequirement {
    water: TilePos,
    level: Level,
}

type WaterNodeIndex = BTreeMap<TilePos, (LiquidBodyId, LiquidNode)>;

fn river_water_nodes(liquids: &LiquidPlan) -> WaterNodeIndex {
    let mut river = BTreeMap::new();
    for (body_id, body) in liquids
        .bodies
        .iter()
        .filter(|(_, body)| body.material == FillMaterialRole::Water)
    {
        let downstream_targets = body
            .nodes
            .values()
            .filter_map(|node| node.downstream)
            .collect::<BTreeSet<_>>();
        river.extend(body.nodes.iter().filter_map(|(position, node)| {
            (node.downstream.is_some() || downstream_targets.contains(position))
                .then_some((*position, (*body_id, *node)))
        }));
    }
    river
}

fn all_water_nodes(liquids: &LiquidPlan) -> WaterNodeIndex {
    liquids
        .bodies
        .iter()
        .filter(|(_, body)| body.material == FillMaterialRole::Water)
        .flat_map(|(body_id, body)| {
            body.nodes
                .iter()
                .map(|(position, node)| (*position, (*body_id, *node)))
        })
        .collect()
}

fn bank_requirements(
    volume: &VolumePlan,
    river: &WaterNodeIndex,
    water: &WaterNodeIndex,
) -> BTreeMap<HexCoord, BankRequirement> {
    let fills = volume.fill_runs_by_top();
    let mut requirements = BTreeMap::<HexCoord, BankRequirement>::new();
    for (position, (body_id, node)) in river {
        let required_level = position.level.saturating_add(1);
        for neighbor in position.coord.neighbors() {
            if !volume.mask.contains(&neighbor)
                || is_channel_neighbor(*position, *body_id, *node, neighbor, water, &fills)
            {
                continue;
            }
            let candidate = BankRequirement {
                water: *position,
                level: required_level,
            };
            requirements
                .entry(neighbor)
                .and_modify(|existing| {
                    if (candidate.level, candidate.water) > (existing.level, existing.water) {
                        *existing = candidate;
                    }
                })
                .or_insert(candidate);
        }
    }
    requirements
}

fn is_channel_neighbor(
    position: TilePos,
    body_id: LiquidBodyId,
    node: LiquidNode,
    neighbor: HexCoord,
    water: &WaterNodeIndex,
    fills: &BTreeMap<TilePos, NonSolidFill>,
) -> bool {
    if node
        .downstream
        .is_some_and(|downstream| downstream.coord == neighbor)
        || water.iter().any(|(candidate, (_, candidate_node))| {
            candidate.coord == neighbor && candidate_node.downstream == Some(position)
        })
    {
        return true;
    }
    let Some(source_fill) = fills.get(&position) else {
        return false;
    };
    water.iter().any(|(candidate, (candidate_body, _))| {
        candidate.coord == neighbor
            && (*candidate_body == body_id
                || fills.get(candidate).is_some_and(|candidate_fill| {
                    source_fill.levels.bottom < candidate_fill.levels.top
                        && candidate_fill.levels.bottom < source_fill.levels.top
                }))
    })
}

fn validate_bank_aprons(
    volume: &VolumePlan,
    requirements: &BTreeMap<HexCoord, BankRequirement>,
    water: &WaterNodeIndex,
) -> Vec<RiverTerrainIssue> {
    let fills = volume.fill_runs_by_top();
    let mut issues = Vec::new();
    for (bank_coord, requirement) in requirements {
        let Some(bank) = ground_surface(volume, *bank_coord) else {
            continue;
        };
        if bank.level < requirement.level {
            continue;
        }
        // A naturally tall hillside or cliff is already a containing bank. Its
        // downslope side may legitimately fall away by several levels and is not
        // the one-cell levee this guard is intended to catch. Only a bank sitting
        // at the exact minimum containment height can be a fitted water ramp whose
        // apparent rise was merely displaced one column out from the river.
        if bank.level != requirement.level {
            continue;
        }
        let Some((body_id, node)) = water.get(&requirement.water).copied() else {
            continue;
        };
        for neighbor in bank.coord.neighbors() {
            if !volume.mask.contains(&neighbor)
                || is_channel_neighbor(requirement.water, body_id, node, neighbor, water, &fills)
            {
                continue;
            }
            let Some(outward) = ground_surface(volume, neighbor) else {
                continue;
            };
            if bank.level.saturating_sub(outward.level) > 1 {
                issues.push(RiverTerrainIssue::SteepBankApron { bank, outward });
            }
        }
    }
    issues
}

fn carve_bed(
    volume: &mut VolumePlan,
    position: TilePos,
    protected_surfaces: &BTreeSet<TilePos>,
    bed_material: SolidMaterialRole,
) -> Result<TilePos, RiverTerrainIssue> {
    let (fill_index, fill_bottom, _bed) = exact_fill_and_bed(volume, position)?;
    let Some(column) = volume.columns.get(&position.coord) else {
        return Err(RiverTerrainIssue::MissingLiquidFill { position });
    };
    let mut elements = column.elements.clone();
    let Some(support_index) = fill_index.checked_sub(1) else {
        return Err(RiverTerrainIssue::UnsupportedBedGeometry { position });
    };
    let Some(VolumeElement::Solid(mut support)) = elements.get(support_index).copied() else {
        return Err(RiverTerrainIssue::UnsupportedBedGeometry { position });
    };
    if support.cutaway_for.is_some()
        || support.levels.bottom >= fill_bottom
        || support.levels.top > fill_bottom
        || !simple_solid_foundation(&elements, support_index)
    {
        return Err(RiverTerrainIssue::UnsupportedBedGeometry { position });
    }
    let old_support_surface = TilePos::new(position.coord, support.levels.top.saturating_sub(1));
    if protected_surfaces.contains(&old_support_surface) {
        return Err(RiverTerrainIssue::ProtectedBed {
            position: old_support_surface,
        });
    }

    let bed_bottom = fill_bottom.saturating_sub(1);
    if bed_bottom < 1 || bed_bottom < support.levels.bottom {
        return Err(RiverTerrainIssue::UnsupportedBedGeometry { position });
    }
    let support_height = support.levels.top.saturating_sub(support.levels.bottom);
    if support.material == bed_material {
        support.levels.top = fill_bottom;
        if let Some(element) = elements.get_mut(support_index) {
            *element = VolumeElement::Solid(support);
        }
    } else if support_height == 1 && support.levels.top == fill_bottom {
        let merged_into_previous = support_index
            .checked_sub(1)
            .and_then(|previous_index| {
                elements
                    .get(previous_index)
                    .copied()
                    .map(|element| (previous_index, element))
            })
            .and_then(|(previous_index, element)| {
                let VolumeElement::Solid(previous) = element else {
                    return None;
                };
                (previous.material == bed_material
                    && previous.cutaway_for == support.cutaway_for
                    && previous.levels.top == support.levels.bottom)
                    .then_some((previous_index, previous))
            });
        if let Some((previous_index, mut previous)) = merged_into_previous {
            previous.levels.top = fill_bottom;
            if let Some(element) = elements.get_mut(previous_index) {
                *element = VolumeElement::Solid(previous);
            }
            elements.remove(support_index);
        } else {
            support.material = bed_material;
            if let Some(element) = elements.get_mut(support_index) {
                *element = VolumeElement::Solid(support);
            }
        }
    } else {
        support.levels.top = bed_bottom;
        if support.levels.bottom >= support.levels.top {
            return Err(RiverTerrainIssue::UnsupportedBedGeometry { position });
        }
        if let Some(element) = elements.get_mut(support_index) {
            *element = VolumeElement::Solid(support);
        }
        elements.insert(
            fill_index,
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(bed_bottom, fill_bottom),
                material: bed_material,
                cutaway_for: None,
            }),
        );
    }

    let bed = TilePos::new(position.coord, bed_bottom);
    let old_metadata = volume
        .surfaces
        .remove(&old_support_surface)
        .unwrap_or(SurfaceMetadata {
            access: SurfaceAccess::NonStandable,
            interior: None,
        });
    let _replaced = volume.surfaces.insert(
        bed,
        SurfaceMetadata {
            access: SurfaceAccess::NonStandable,
            interior: old_metadata.interior,
        },
    );
    if let Some(column) = volume.columns.get_mut(&position.coord) {
        column.elements = elements;
    }
    Ok(bed)
}

fn exact_fill_and_bed(
    volume: &VolumePlan,
    position: TilePos,
) -> Result<(usize, Level, TilePos), RiverTerrainIssue> {
    let Some(column) = volume.columns.get(&position.coord) else {
        return Err(RiverTerrainIssue::MissingLiquidFill { position });
    };
    let Some((fill_index, fill)) =
        column
            .elements
            .iter()
            .enumerate()
            .find_map(|(index, element)| {
                let VolumeElement::Fill(fill) = *element else {
                    return None;
                };
                (fill.material == FillMaterialRole::Water
                    && fill.levels.top.saturating_sub(1) == position.level)
                    .then_some((index, fill))
            })
    else {
        return Err(RiverTerrainIssue::MissingLiquidFill { position });
    };
    let bed_level = fill.levels.bottom.saturating_sub(1);
    Ok((
        fill_index,
        fill.levels.bottom,
        TilePos::new(position.coord, bed_level),
    ))
}

fn simple_solid_foundation(elements: &[VolumeElement], through: usize) -> bool {
    let mut expected_bottom = 0;
    for element in elements.iter().take(through.saturating_add(1)) {
        let VolumeElement::Solid(mass) = *element else {
            return false;
        };
        if mass.levels.bottom != expected_bottom {
            return false;
        }
        expected_bottom = mass.levels.top;
    }
    true
}

fn raise_bank(
    volume: &mut VolumePlan,
    coord: HexCoord,
    requirement: BankRequirement,
    protected_surfaces: &BTreeSet<TilePos>,
) -> Result<Option<(TilePos, TilePos)>, RiverTerrainIssue> {
    let Some(bank) = ground_surface(volume, coord) else {
        return Err(RiverTerrainIssue::MissingBankSurface {
            water: requirement.water,
            coord,
        });
    };
    if bank.level >= requirement.level {
        return Ok(None);
    }
    if protected_surfaces
        .iter()
        .any(|protected| protected.coord == coord)
    {
        return Err(RiverTerrainIssue::ProtectedBankTooLow {
            water: requirement.water,
            bank,
            required_level: requirement.level,
        });
    }
    if requirement.level.saturating_sub(bank.level) > 1 {
        return Err(RiverTerrainIssue::ExcessiveBankRaise {
            water: requirement.water,
            bank,
            required_level: requirement.level,
        });
    }
    let Some(column) = volume.columns.get(&coord) else {
        return Err(RiverTerrainIssue::MissingBankSurface {
            water: requirement.water,
            coord,
        });
    };
    let mut elements = column.elements.clone();
    if !simple_dry_ground(&elements) {
        return Err(RiverTerrainIssue::UnsupportedBankGeometry {
            water: requirement.water,
            bank,
            required_level: requirement.level,
        });
    }
    let Some(last_index) = elements.len().checked_sub(1) else {
        return Err(RiverTerrainIssue::MissingBankSurface {
            water: requirement.water,
            coord,
        });
    };
    let Some(VolumeElement::Solid(mut cap)) = elements.get(last_index).copied() else {
        return Err(RiverTerrainIssue::UnsupportedBankGeometry {
            water: requirement.water,
            bank,
            required_level: requirement.level,
        });
    };
    if cap.cutaway_for.is_some() || cap.material == SolidMaterialRole::Bedrock {
        return Err(RiverTerrainIssue::UnsupportedBankGeometry {
            water: requirement.water,
            bank,
            required_level: requirement.level,
        });
    }
    let new_top = requirement.level.saturating_add(1);
    if cap.levels.top.saturating_sub(cap.levels.bottom) == 1 && last_index > 0 {
        let Some(VolumeElement::Solid(mut core)) =
            elements.get(last_index.saturating_sub(1)).copied()
        else {
            return Err(RiverTerrainIssue::UnsupportedBankGeometry {
                water: requirement.water,
                bank,
                required_level: requirement.level,
            });
        };
        core.levels.top = requirement.level;
        cap.levels = LevelInterval::new(requirement.level, new_top);
        if let Some(element) = elements.get_mut(last_index.saturating_sub(1)) {
            *element = VolumeElement::Solid(core);
        }
    } else {
        cap.levels.top = new_top;
    }
    if let Some(element) = elements.get_mut(last_index) {
        *element = VolumeElement::Solid(cap);
    }

    let Some(metadata) = volume.surfaces.remove(&bank) else {
        return Err(RiverTerrainIssue::MissingBankSurface {
            water: requirement.water,
            coord,
        });
    };
    let raised = TilePos::new(coord, requirement.level);
    let _replaced = volume.surfaces.insert(raised, metadata);
    if let Some(column) = volume.columns.get_mut(&coord) {
        column.elements = elements;
    }
    Ok(Some((bank, raised)))
}

fn simple_dry_ground(elements: &[VolumeElement]) -> bool {
    if elements.is_empty() {
        return false;
    }
    let mut expected_bottom = 0;
    for element in elements {
        let VolumeElement::Solid(mass) = *element else {
            return false;
        };
        if mass.levels.bottom != expected_bottom {
            return false;
        }
        expected_bottom = mass.levels.top;
    }
    true
}

fn ground_surface(volume: &VolumePlan, coord: HexCoord) -> Option<TilePos> {
    let column = volume.columns.get(&coord)?;
    let mut expected_bottom = 0;
    let mut top = None;
    for element in &column.elements {
        let VolumeElement::Solid(mass) = *element else {
            break;
        };
        if mass.levels.bottom != expected_bottom {
            break;
        }
        expected_bottom = mass.levels.top;
        top = Some(mass.levels.top);
    }
    let surface = TilePos::new(coord, top?.saturating_sub(1));
    volume.surfaces.contains_key(&surface).then_some(surface)
}

#[cfg(test)]
mod tests {
    use super::super::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidNode};
    use super::super::volume::VolumeColumn;
    use super::*;

    fn land_column(surface: Level) -> VolumeColumn {
        VolumeColumn {
            elements: vec![
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(0, 1),
                    material: SolidMaterialRole::Bedrock,
                    cutaway_for: None,
                }),
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(1, surface),
                    material: SolidMaterialRole::Stone,
                    cutaway_for: None,
                }),
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(surface, surface.saturating_add(1)),
                    material: SolidMaterialRole::Grass,
                    cutaway_for: None,
                }),
            ],
        }
    }

    fn water_column(position: TilePos, fill_bottom: Level) -> (VolumeColumn, TilePos) {
        let bed = TilePos::new(position.coord, fill_bottom.saturating_sub(1));
        (
            VolumeColumn {
                elements: vec![
                    VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(0, 1),
                        material: SolidMaterialRole::Bedrock,
                        cutaway_for: None,
                    }),
                    VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(1, fill_bottom),
                        material: SolidMaterialRole::Stone,
                        cutaway_for: None,
                    }),
                    VolumeElement::Fill(NonSolidFill {
                        levels: LevelInterval::new(fill_bottom, position.level.saturating_add(1)),
                        material: FillMaterialRole::Water,
                    }),
                ],
            },
            bed,
        )
    }

    fn fixture(
        nodes: BTreeMap<TilePos, LiquidNode>,
        bridge: Option<TilePos>,
    ) -> (VolumePlan, LiquidPlan) {
        let mask = HexCoord::ORIGIN
            .within_radius(2)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let mut volume = VolumePlan::new(mask.clone());
        for coord in mask {
            volume.columns.insert(coord, land_column(5));
            volume.surfaces.insert(
                TilePos::new(coord, 5),
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }
        for position in nodes.keys().copied() {
            let fill_bottom = if nodes
                .get(&position)
                .is_some_and(|node| node.state == LiquidFlowState::Fall)
            {
                4
            } else {
                position.level.saturating_sub(1)
            };
            let (mut column, bed) = water_column(position, fill_bottom);
            volume.surfaces.remove(&TilePos::new(position.coord, 5));
            volume.surfaces.insert(
                bed,
                SurfaceMetadata {
                    access: SurfaceAccess::NonStandable,
                    interior: None,
                },
            );
            if bridge.is_some_and(|deck| deck.coord == position.coord) {
                let deck = bridge.expect("matching bridge");
                column.elements.push(VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(deck.level, deck.level.saturating_add(1)),
                    material: SolidMaterialRole::Metal,
                    cutaway_for: None,
                }));
                volume.surfaces.insert(
                    deck,
                    SurfaceMetadata {
                        access: SurfaceAccess::Ordinary,
                        interior: None,
                    },
                );
            }
            volume.columns.insert(position.coord, column);
        }
        (
            volume,
            LiquidPlan {
                bodies: BTreeMap::from([(
                    LiquidBodyId(7),
                    LiquidBodyPlan {
                        material: FillMaterialRole::Water,
                        nodes,
                    },
                )]),
            },
        )
    }

    fn straight_nodes() -> BTreeMap<TilePos, LiquidNode> {
        let source = TilePos::new(HexCoord::from_axial(0, -1), 5);
        let middle = TilePos::new(HexCoord::ORIGIN, 5);
        let outlet = TilePos::new(HexCoord::from_axial(0, 1), 5);
        BTreeMap::from([
            (
                source,
                LiquidNode {
                    state: LiquidFlowState::Current,
                    downstream: Some(middle),
                },
            ),
            (
                middle,
                LiquidNode {
                    state: LiquidFlowState::Rapid,
                    downstream: Some(outlet),
                },
            ),
            (
                outlet,
                LiquidNode {
                    state: LiquidFlowState::Still,
                    downstream: None,
                },
            ),
        ])
    }

    fn install_stacked_water(volume: &mut VolumePlan, position: TilePos) {
        volume.columns.insert(
            position.coord,
            VolumeColumn {
                elements: vec![
                    VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(0, 1),
                        material: SolidMaterialRole::Bedrock,
                        cutaway_for: None,
                    }),
                    VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(1, position.level.saturating_sub(1)),
                        material: SolidMaterialRole::Stone,
                        cutaway_for: None,
                    }),
                    VolumeElement::Fill(NonSolidFill {
                        levels: LevelInterval::new(
                            position.level.saturating_sub(1),
                            position.level.saturating_add(1),
                        ),
                        material: FillMaterialRole::Water,
                    }),
                ],
            },
        );
        volume
            .surfaces
            .retain(|surface, _| surface.coord != position.coord);
        volume.surfaces.insert(
            TilePos::new(position.coord, position.level.saturating_sub(2)),
            SurfaceMetadata {
                access: SurfaceAccess::NonStandable,
                interior: None,
            },
        );
    }

    #[test]
    fn moving_water_is_incised_and_the_mutated_low_bank_is_detected() {
        let (mut volume, liquids) = fixture(straight_nodes(), None);
        assert!(validate_river_terrain(&volume, &liquids)
            .iter()
            .any(|issue| matches!(issue, RiverTerrainIssue::LowBank { .. })));

        let fit = fit_river_terrain(
            &mut volume,
            &liquids,
            &BTreeSet::new(),
            SolidMaterialRole::Gravel,
        )
        .expect("fit river terrain");
        assert!(!fit.raised_banks.is_empty());
        assert_eq!(fit.carved_beds.len(), 3);
        assert!(validate_river_terrain(&volume, &liquids).is_empty());

        let (raised, original) = fit
            .raised_banks
            .iter()
            .next()
            .map(|(before, after)| (*after, *before))
            .expect("one raised bank");
        let metadata = volume
            .surfaces
            .remove(&raised)
            .expect("raised bank metadata");
        volume.surfaces.insert(original, metadata);
        volume
            .columns
            .insert(original.coord, land_column(original.level));
        assert!(validate_river_terrain(&volume, &liquids).iter().any(
            |issue| matches!(issue, RiverTerrainIssue::LowBank { bank, .. } if *bank == original)
        ));
    }

    #[test]
    fn floating_non_fall_fill_is_carved_down_to_an_exact_bed() {
        let (mut volume, liquids) = fixture(straight_nodes(), None);
        let source = TilePos::new(HexCoord::from_axial(0, -1), 5);
        let source_column = volume
            .columns
            .get_mut(&source.coord)
            .expect("source column");
        let Some(VolumeElement::Solid(stone)) = source_column.elements.get_mut(1) else {
            panic!("source stone foundation");
        };
        stone.levels.top = 2;
        let mut metadata = volume
            .surfaces
            .remove(&TilePos::new(source.coord, 3))
            .expect("old source bed metadata");
        metadata.access = SurfaceAccess::Ordinary;
        volume
            .surfaces
            .insert(TilePos::new(source.coord, 1), metadata);
        volume
            .validate()
            .expect("floating fill remains valid occupancy");

        let fit = fit_river_terrain(
            &mut volume,
            &liquids,
            &BTreeSet::new(),
            SolidMaterialRole::Gravel,
        )
        .expect("carve exact river bed");
        let bed = TilePos::new(source.coord, 3);
        assert!(fit.carved_beds.contains(&bed));
        assert!(volume.surfaces.contains_key(&bed));
        assert!(!volume.surfaces.contains_key(&TilePos::new(source.coord, 1)));
        assert!(volume.columns.get(&source.coord).is_some_and(|column| {
            column.elements.iter().any(|element| {
                matches!(
                    element,
                    VolumeElement::Solid(mass)
                        if mass.material == SolidMaterialRole::Gravel
                            && mass.levels == LevelInterval::new(3, 4)
                )
            })
        }));
        assert!(validate_river_terrain(&volume, &liquids).is_empty());
    }

    #[test]
    fn one_voxel_grass_cap_becomes_a_merged_dirt_bed() {
        let (mut volume, liquids) = fixture(straight_nodes(), None);
        let source = TilePos::new(HexCoord::from_axial(0, -1), 5);
        let fill = volume
            .columns
            .get(&source.coord)
            .and_then(|column| column.elements.last())
            .copied()
            .expect("source water fill");
        volume.columns.insert(
            source.coord,
            VolumeColumn {
                elements: vec![
                    VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(0, 1),
                        material: SolidMaterialRole::Bedrock,
                        cutaway_for: None,
                    }),
                    VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(1, 2),
                        material: SolidMaterialRole::Stone,
                        cutaway_for: None,
                    }),
                    VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(2, 3),
                        material: SolidMaterialRole::Dirt,
                        cutaway_for: None,
                    }),
                    VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(3, 4),
                        material: SolidMaterialRole::Grass,
                        cutaway_for: None,
                    }),
                    fill,
                ],
            },
        );
        volume.validate().expect("grass-capped water column");

        fit_river_terrain(
            &mut volume,
            &liquids,
            &BTreeSet::new(),
            SolidMaterialRole::Dirt,
        )
        .expect("replace grass cap with dirt bed");
        let column = volume.columns.get(&source.coord).expect("fitted source");
        assert!(column.elements.iter().any(|element| matches!(
            element,
            VolumeElement::Solid(mass)
                if mass.material == SolidMaterialRole::Dirt
                    && mass.levels == LevelInterval::new(2, 4)
        )));
        assert!(!column.elements.iter().any(|element| matches!(
            element,
            VolumeElement::Solid(mass) if mass.material == SolidMaterialRole::Grass
        )));
        assert!(validate_river_terrain(&volume, &liquids).is_empty());
    }

    #[test]
    fn only_the_exact_downstream_fall_face_is_exempt_from_banks() {
        let fall = TilePos::new(HexCoord::ORIGIN, 7);
        let landing = TilePos::new(HexCoord::from_axial(0, 1), 4);
        let nodes = BTreeMap::from([
            (
                fall,
                LiquidNode {
                    state: LiquidFlowState::Fall,
                    downstream: Some(landing),
                },
            ),
            (
                landing,
                LiquidNode {
                    state: LiquidFlowState::Still,
                    downstream: None,
                },
            ),
        ]);
        let deck = TilePos::new(HexCoord::ORIGIN, 9);
        let (mut volume, liquids) = fixture(nodes, Some(deck));
        let wet = liquids
            .bodies
            .values()
            .flat_map(|body| body.nodes.keys().map(|position| position.coord))
            .collect::<BTreeSet<_>>();
        let dry = volume.mask.difference(&wet).copied().collect::<Vec<_>>();
        for coord in dry {
            let metadata = volume
                .surfaces
                .remove(&TilePos::new(coord, 5))
                .expect("dry surface metadata");
            volume.columns.insert(coord, land_column(7));
            volume.surfaces.insert(TilePos::new(coord, 7), metadata);
        }
        let before_column = volume
            .columns
            .get(&deck.coord)
            .cloned()
            .expect("bridge column");
        let fit = fit_river_terrain(
            &mut volume,
            &liquids,
            &BTreeSet::from([deck]),
            SolidMaterialRole::Gravel,
        )
        .expect("fall face and landing basin fit");

        assert!(!fit.raised_banks.is_empty());
        assert_eq!(
            fit.carved_beds,
            BTreeSet::from([TilePos::new(landing.coord, 2)])
        );
        assert_eq!(volume.columns.get(&deck.coord), Some(&before_column));
        assert!(volume.surfaces.contains_key(&deck));
        for neighbor in fall.coord.neighbors() {
            if neighbor == landing.coord || !volume.mask.contains(&neighbor) {
                continue;
            }
            let bank = ground_surface(&volume, neighbor).expect("fall lateral bank");
            assert!(bank.level > fall.level, "low fall bank {bank:?}");
        }
        assert_eq!(
            volume
                .fill_runs_by_top()
                .get(&landing)
                .map(|fill| fill.levels),
            Some(LevelInterval::new(3, 5))
        );
    }

    #[test]
    fn confluence_and_mask_boundary_fit_deterministically() {
        let first = TilePos::new(HexCoord::from_axial(-1, 1), 5);
        let second = TilePos::new(HexCoord::from_axial(1, 0), 5);
        let merge = TilePos::new(HexCoord::from_axial(0, 1), 5);
        let outlet = TilePos::new(HexCoord::from_axial(0, 2), 5);
        let nodes = BTreeMap::from([
            (
                first,
                LiquidNode {
                    state: LiquidFlowState::Current,
                    downstream: Some(merge),
                },
            ),
            (
                second,
                LiquidNode {
                    state: LiquidFlowState::Current,
                    downstream: Some(merge),
                },
            ),
            (
                merge,
                LiquidNode {
                    state: LiquidFlowState::Current,
                    downstream: Some(outlet),
                },
            ),
            (
                outlet,
                LiquidNode {
                    state: LiquidFlowState::Still,
                    downstream: None,
                },
            ),
        ]);
        let (volume, liquids) = fixture(nodes, None);
        let mut first_fit = volume.clone();
        let mut second_fit = volume;
        let first_report = fit_river_terrain(
            &mut first_fit,
            &liquids,
            &BTreeSet::new(),
            SolidMaterialRole::Gravel,
        )
        .expect("first fit");
        let second_report = fit_river_terrain(
            &mut second_fit,
            &liquids,
            &BTreeSet::new(),
            SolidMaterialRole::Gravel,
        )
        .expect("second fit");

        assert_eq!(first_report, second_report);
        assert_eq!(first_fit, second_fit);
        assert!(validate_river_terrain(&first_fit, &liquids).is_empty());
    }

    #[test]
    fn protected_low_bank_rejects_atomically() {
        let (mut volume, liquids) = fixture(straight_nodes(), None);
        let original = volume.clone();
        let protected = TilePos::new(HexCoord::from_axial(1, -1), 5);
        let issues = fit_river_terrain(
            &mut volume,
            &liquids,
            &BTreeSet::from([protected]),
            SolidMaterialRole::Gravel,
        )
        .expect_err("protected low bank must reject");

        assert!(issues.iter().any(|issue| matches!(
            issue,
            RiverTerrainIssue::ProtectedBankTooLow { bank, .. } if *bank == protected
        )));
        assert_eq!(volume, original);
    }

    #[test]
    fn fitting_refuses_a_tall_levee_instead_of_moving_the_ramp_outward() {
        let (mut volume, liquids) = fixture(straight_nodes(), None);
        let bank = HexCoord::from_axial(1, -1);
        let metadata = volume
            .surfaces
            .remove(&TilePos::new(bank, 5))
            .expect("bank metadata");
        volume.columns.insert(bank, land_column(4));
        volume.surfaces.insert(TilePos::new(bank, 4), metadata);
        let original = volume.clone();

        let issues = fit_river_terrain(
            &mut volume,
            &liquids,
            &BTreeSet::new(),
            SolidMaterialRole::Gravel,
        )
        .expect_err("a two-level bank raise must reprofile water instead");
        assert!(issues.iter().any(|issue| matches!(
            issue,
            RiverTerrainIssue::ExcessiveBankRaise { bank, .. }
                if *bank == TilePos::new(HexCoord::from_axial(1, -1), 4)
        )));
        assert_eq!(volume, original);
    }

    #[test]
    fn fitting_refuses_a_one_cell_levee_with_a_steep_outward_face() {
        let (mut volume, liquids) = fixture(straight_nodes(), None);
        let outward = HexCoord::from_axial(2, -1);
        let metadata = volume
            .surfaces
            .remove(&TilePos::new(outward, 5))
            .expect("outward metadata");
        volume.columns.insert(outward, land_column(4));
        volume.surfaces.insert(TilePos::new(outward, 4), metadata);
        let original = volume.clone();

        let issues = fit_river_terrain(
            &mut volume,
            &liquids,
            &BTreeSet::new(),
            SolidMaterialRole::Gravel,
        )
        .expect_err("steep outward bank apron must reprofile water instead");
        assert!(issues.iter().any(|issue| matches!(
            issue,
            RiverTerrainIssue::SteepBankApron { outward: actual, .. }
                if *actual == TilePos::new(outward, 4)
        )));
        assert_eq!(volume, original);
    }

    #[test]
    fn vertically_separate_water_does_not_hide_a_dry_bank() {
        let (mut volume, mut liquids) = fixture(straight_nodes(), None);
        let source = TilePos::new(HexCoord::from_axial(0, -1), 5);
        let stacked_coord = HexCoord::from_axial(1, -1);
        let stacked = TilePos::new(stacked_coord, 10);
        install_stacked_water(&mut volume, stacked);
        liquids.bodies.insert(
            LiquidBodyId(9),
            LiquidBodyPlan {
                material: FillMaterialRole::Water,
                nodes: BTreeMap::from([(
                    stacked,
                    LiquidNode {
                        state: LiquidFlowState::Still,
                        downstream: None,
                    },
                )]),
            },
        );
        let water = all_water_nodes(&liquids);
        let fills = volume.fill_runs_by_top();
        let (source_body, source_node) = water.get(&source).copied().expect("source node");

        assert!(!is_channel_neighbor(
            source,
            source_body,
            source_node,
            stacked_coord,
            &water,
            &fills
        ));
    }

    #[test]
    fn same_body_adjacent_water_is_channel_across_a_nonoverlapping_step() {
        let (mut volume, mut liquids) = fixture(straight_nodes(), None);
        let source = TilePos::new(HexCoord::from_axial(0, -1), 5);
        let stepped = TilePos::new(HexCoord::from_axial(1, -1), 10);
        install_stacked_water(&mut volume, stepped);
        liquids
            .bodies
            .get_mut(&LiquidBodyId(7))
            .expect("river body")
            .nodes
            .insert(
                stepped,
                LiquidNode {
                    state: LiquidFlowState::Still,
                    downstream: None,
                },
            );
        let water = all_water_nodes(&liquids);
        let fills = volume.fill_runs_by_top();
        let (source_body, source_node) = water.get(&source).copied().expect("source node");

        assert!(is_channel_neighbor(
            source,
            source_body,
            source_node,
            stepped.coord,
            &water,
            &fills
        ));
    }
}
