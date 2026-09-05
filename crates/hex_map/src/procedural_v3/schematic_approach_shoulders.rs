//! Fill-only shoulders beside the lower and diagonal natural-pass approach.
//!
//! The old Peak/Crystal shoulder solvers retain their exact authority. This
//! additional visual pass never cuts a column or solves an unrelated cliff.

use super::*;

const SUPPORT_RADIUS: u32 = 18;
const APPROACH_NORTHERN_ROW: i32 = -110;

#[derive(Debug, Default, PartialEq, Eq)]
struct FillProjection {
    fills: BTreeMap<HexCoord, Level>,
    deferred_seeds: BTreeSet<HexCoord>,
}

fn edge_allowance(
    original: &BTreeMap<HexCoord, Level>,
    a: HexCoord,
    b: HexCoord,
) -> Result<Level, V3GenerationError> {
    let source = |coord| {
        original.get(&coord).copied().ok_or_else(|| {
            schematic_contract(format!(
                "approach edge lost its source surface at {coord:?}"
            ))
        })
    };
    let difference = Level::try_from(source(a)?.abs_diff(source(b)?)).map_err(|_| {
        schematic_contract("approach edge difference exceeds the bounded level domain")
    })?;
    Ok(difference.max(NATURAL_PASS_SHOULDER_MAXIMUM_STEP))
}

/// Every original edge already satisfies its own allowance. Both propagated
/// fields do too: the route field falls by nine, while boundary ceilings use
/// the original allowance. Min/max of fields with those same edge bounds keep
/// the bounds. Taking max(original, min(route, ceiling)) therefore only fills,
/// and cannot create a steep edge or increase an existing cliff's difference.
fn project_fill(
    original: &BTreeMap<HexCoord, Level>,
    route: &BTreeMap<HexCoord, Level>,
    eligible: &BTreeSet<HexCoord>,
) -> Result<FillProjection, V3GenerationError> {
    if route
        .iter()
        .any(|(coord, level)| original.get(coord) != Some(level))
        || !eligible.iter().all(|coord| original.contains_key(coord))
    {
        return Err(schematic_contract(
            "approach shoulder source does not retain every exact route and eligible surface",
        ));
    }
    let seeds = route
        .iter()
        .filter_map(|(coord, level)| {
            (coord.y() >= APPROACH_NORTHERN_ROW
                && coord.neighbors().into_iter().any(|neighbor| {
                    !route.contains_key(&neighbor)
                        && original.get(&neighbor).is_some_and(|neighbor_level| {
                            *level
                                > neighbor_level.saturating_add(NATURAL_PASS_SHOULDER_MAXIMUM_STEP)
                        })
                }))
            .then_some((*coord, *level))
        })
        .collect::<BTreeMap<_, _>>();
    let domain = seeds
        .keys()
        .flat_map(|coord| coord.within_radius(SUPPORT_RADIUS))
        .filter(|coord| {
            coord.y() >= APPROACH_NORTHERN_ROW
                && eligible.contains(coord)
                && !route.contains_key(coord)
        })
        .collect::<BTreeSet<_>>();

    // Fixed route, water, protected geometry and the radius/row boundary all
    // contribute ceilings. Existing high cliffs are not lower-bound sources.
    let mut upper = BTreeMap::<HexCoord, Level>::new();
    for coord in &domain {
        for outside in coord
            .neighbors()
            .into_iter()
            .filter(|n| !domain.contains(n))
        {
            let Some(level) = original.get(&outside) else {
                continue;
            };
            let ceiling = level.saturating_add(edge_allowance(original, *coord, outside)?);
            upper
                .entry(*coord)
                .and_modify(|current| *current = (*current).min(ceiling))
                .or_insert(ceiling);
        }
    }
    let mut upper_frontier = BTreeSet::from_iter(upper.iter().map(|(c, l)| (*l, *c)));
    while let Some((level, coord)) = upper_frontier.pop_first() {
        if upper.get(&coord) != Some(&level) {
            continue;
        }
        for neighbor in coord.neighbors().into_iter().filter(|n| domain.contains(n)) {
            let ceiling = level.saturating_add(edge_allowance(original, coord, neighbor)?);
            if upper
                .get(&neighbor)
                .is_none_or(|current| ceiling < *current)
            {
                upper.insert(neighbor, ceiling);
                upper_frontier.insert((ceiling, neighbor));
            }
        }
    }

    let mut desired = BTreeMap::<HexCoord, Level>::new();
    for (seed, source_level) in &seeds {
        let level = source_level.saturating_sub(NATURAL_PASS_SHOULDER_MAXIMUM_STEP);
        for neighbor in seed.neighbors().into_iter().filter(|n| domain.contains(n)) {
            desired
                .entry(neighbor)
                .and_modify(|current| *current = (*current).max(level))
                .or_insert(level);
        }
    }
    let mut lower_frontier = BinaryHeap::from_iter(desired.iter().map(|(c, l)| (*l, *c)));
    while let Some((level, coord)) = lower_frontier.pop() {
        if desired.get(&coord) != Some(&level) {
            continue;
        }
        for neighbor in coord.neighbors().into_iter().filter(|n| domain.contains(n)) {
            let candidate = level.saturating_sub(NATURAL_PASS_SHOULDER_MAXIMUM_STEP);
            if desired
                .get(&neighbor)
                .is_none_or(|current| candidate > *current)
            {
                desired.insert(neighbor, candidate);
                lower_frontier.push((candidate, neighbor));
            }
        }
    }

    let mut fills = BTreeMap::new();
    for (coord, wanted) in &desired {
        let current = original.get(coord).copied().ok_or_else(|| {
            schematic_contract("approach projection lost an eligible source surface")
        })?;
        // No reachable fixed boundary means no additional upper constraint.
        let level = (*wanted).min(upper.get(coord).copied().unwrap_or(Level::MAX));
        if level > current {
            fills.insert(*coord, level);
        }
    }
    // Check the concrete field before any geometry mutation; clipped shoulders
    // are valid partial extensions, but violating the original edge bound is not.
    for (coord, level) in &fills {
        for neighbor in coord.neighbors() {
            let Some(before) = original.get(&neighbor) else {
                continue;
            };
            let after = fills.get(&neighbor).copied().unwrap_or(*before);
            if level.abs_diff(after) > edge_allowance(original, *coord, neighbor)?.unsigned_abs() {
                return Err(schematic_contract(format!(
                    "approach fill worsens edge {coord:?}@{level} -> {neighbor:?}@{after}"
                )));
            }
        }
    }
    let deferred_seeds = seeds
        .into_iter()
        .filter_map(|(coord, route_level)| {
            coord
                .neighbors()
                .into_iter()
                .any(|neighbor| {
                    !route.contains_key(&neighbor)
                        && original.get(&neighbor).is_some_and(|before| {
                            let after = fills.get(&neighbor).copied().unwrap_or(*before);
                            route_level > after.saturating_add(NATURAL_PASS_SHOULDER_MAXIMUM_STEP)
                        })
                })
                .then_some(coord)
        })
        .collect();
    Ok(FillProjection {
        fills,
        deferred_seeds,
    })
}

/// Only a single exposed, contiguous, dry natural column can receive fill.
/// Stacked floors, liquids, roofs and authored object materials are immutable.
fn fillable_column(volume: &VolumePlan, coord: HexCoord) -> bool {
    let Some((surface, metadata)) = volume.top_surface_at_coord(coord) else {
        return false;
    };
    if metadata.interior.is_some()
        || metadata.access == SurfaceAccess::NonStandable
        || volume.surfaces_at_coord(coord).count() != 1
    {
        return false;
    }
    let Some(column) = volume.columns.get(&coord) else {
        return false;
    };
    let mut next = 0;
    for element in &column.elements {
        let VolumeElement::Solid(mass) = element else {
            return false;
        };
        if mass.levels.bottom != next
            || mass.levels.top <= mass.levels.bottom
            || mass.cutaway_for.is_some()
            || matches!(
                mass.material,
                SolidMaterialRole::Metal | SolidMaterialRole::WorkedStone
            )
        {
            return false;
        }
        next = mass.levels.top;
    }
    next == surface.level.saturating_add(1)
}

fn filled_column(column: &VolumeColumn, old: Level, new: Level) -> VolumeColumn {
    let cap = top_solid_material(column);
    let soil = column
        .elements
        .iter()
        .rev()
        .find_map(|element| match element {
            VolumeElement::Solid(mass)
                if mass.levels.top <= old
                    && matches!(
                        mass.material,
                        SolidMaterialRole::Dirt | SolidMaterialRole::Stone
                    ) =>
            {
                Some(mass.material)
            }
            _ => None,
        })
        .unwrap_or(cap);
    let mut filled = column.clone();
    if old.saturating_add(1) < new {
        push_canonical_solid(
            &mut filled.elements,
            SolidMass {
                levels: LevelInterval::new(old.saturating_add(1), new),
                material: soil,
                cutaway_for: None,
            },
        );
    }
    push_canonical_solid(
        &mut filled.elements,
        SolidMass {
            levels: LevelInterval::new(new, new.saturating_add(1)),
            material: cap,
            cutaway_for: None,
        },
    );
    filled
}

fn publish_fills(
    fills: &BTreeMap<HexCoord, Level>,
    fine_index: &FineWorldIndex,
    volume: &mut VolumePlan,
    biome_regions: &mut BTreeMap<TilePos, BiomeRegionId>,
) -> Result<(), V3GenerationError> {
    let updates = fills
        .iter()
        .map(|(coord, level)| {
            let (surface, _) = volume
                .top_surface_at_coord(*coord)
                .ok_or_else(|| schematic_contract("approach fill lost its preflight surface"))?;
            if !fillable_column(volume, *coord) || *level <= surface.level || *level > MAX_V3_LEVEL
            {
                return Err(schematic_contract(
                    "approach fill is not a bounded dry-ground addition",
                ));
            }
            let biome = fine_index
                .biome(*coord)
                .ok_or_else(|| schematic_contract("approach fill lost its exact biome owner"))?;
            let source_column = volume.columns.get(coord).ok_or_else(|| {
                schematic_contract("approach fill lost its preflight source column")
            })?;
            let column = filled_column(source_column, surface.level, *level);
            Ok((*coord, *level, column, biome))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for (coord, level, column, biome) in updates {
        replace_column_surface(
            volume,
            biome_regions,
            coord,
            column,
            TilePos::new(coord, level),
            SurfaceMetadata {
                access: SurfaceAccess::SpecialMovement(SCENIC_MOVEMENT_REGION),
                interior: None,
            },
            biome,
        );
    }
    Ok(())
}

pub(super) fn grade(
    pass_coords: &BTreeSet<HexCoord>,
    layout: &ResolvedLayoutPlan,
    fine_index: &FineWorldIndex,
    water_coords: &BTreeSet<HexCoord>,
    protected_coords: &BTreeSet<HexCoord>,
    shared_terrain_minimums: &BTreeMap<HexCoord, Level>,
    volume: &mut VolumePlan,
    biome_regions: &mut BTreeMap<TilePos, BiomeRegionId>,
) -> Result<BTreeSet<HexCoord>, V3GenerationError> {
    let route = pass_coords
        .iter()
        .map(|coord| {
            volume
                .top_surface_at_coord(*coord)
                .map(|(surface, _)| (*coord, surface.level))
                .ok_or_else(|| schematic_contract("approach shoulder lost an exact pass surface"))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    // Keep the immutable perimeter in the source map, including water and exact
    // shared authorities, so their ceilings constrain the extension itself.
    let footprint = route
        .keys()
        .filter(|coord| coord.y() >= APPROACH_NORTHERN_ROW)
        .flat_map(|coord| coord.within_radius(SUPPORT_RADIUS.saturating_add(1)))
        .collect::<BTreeSet<_>>();
    let mut original = footprint
        .iter()
        .filter_map(|coord| {
            volume
                .top_surface_at_coord(*coord)
                .map(|(surface, _)| (*coord, surface.level))
        })
        .collect::<BTreeMap<_, _>>();
    original.extend(route.iter().map(|(coord, level)| (*coord, *level)));
    let eligible = footprint
        .into_iter()
        .filter(|coord| {
            coord.y() >= APPROACH_NORTHERN_ROW
                && layout.footprint.contains(coord)
                && fine_index.by_coord.contains_key(coord)
                && !pass_coords.contains(coord)
                && !water_coords.contains(coord)
                && !protected_coords.contains(coord)
                && !shared_terrain_minimums.contains_key(coord)
                && fillable_column(volume, *coord)
        })
        .collect::<BTreeSet<_>>();
    let projection = project_fill(&original, &route, &eligible)?;
    publish_fills(&projection.fills, fine_index, volume, biome_regions)?;
    if std::env::var_os("HEX_GRAND_MASSIF_PROFILE").is_some() {
        eprintln!(
            "grand-v3 approach shoulders: filled={:?}, locally_deferred_seeds={:?}",
            projection.fills, projection.deferred_seeds,
        );
    }
    Ok(projection.fills.into_keys().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_ground() -> BTreeMap<HexCoord, Level> {
        HexCoord::ORIGIN
            .within_radius(24)
            .into_iter()
            .map(|coord| (coord, 40))
            .collect()
    }

    fn assert_edges_not_worsened(
        original: &BTreeMap<HexCoord, Level>,
        fills: &BTreeMap<HexCoord, Level>,
    ) {
        for (coord, old) in original {
            let new = fills.get(coord).copied().unwrap_or(*old);
            assert!(new >= *old);
            for neighbor in coord.neighbors() {
                if let Some(other_old) = original.get(&neighbor) {
                    let other_new = fills.get(&neighbor).copied().unwrap_or(*other_old);
                    // Independent literal assertion of the visual contract.
                    assert!(new.abs_diff(other_new) <= old.abs_diff(*other_old).max(9));
                }
            }
        }
    }

    #[test]
    fn raised_approach_receives_falling_shoulders_without_changing_exact_route() {
        let mut original = flat_ground();
        let route = BTreeMap::from_iter((-3..=3).map(|y| (HexCoord::from_axial(0, y), 100 + y)));
        original.extend(route.iter().map(|(coord, level)| (*coord, *level)));
        let eligible = original.keys().copied().collect();
        let before = original.clone();
        let projection =
            project_fill(&original, &route, &eligible).expect("fill projection resolves");
        assert_eq!(original, before, "projection is read-only");
        assert!(!projection.fills.is_empty());
        assert!(projection.deferred_seeds.is_empty());
        assert_eq!(projection.fills[&HexCoord::from_axial(1, 0)], 92);
        assert_eq!(projection.fills[&HexCoord::from_axial(2, 0)], 84);
        assert_eq!(projection.fills[&HexCoord::from_axial(6, 0)], 49);
        assert!(!projection.fills.contains_key(&HexCoord::from_axial(7, 0)));
        assert!(route
            .keys()
            .all(|coord| !projection.fills.contains_key(coord)));
        assert_edges_not_worsened(&original, &projection.fills);
        assert_eq!(
            projection,
            project_fill(&original, &route, &eligible).expect("deterministic")
        );
    }

    #[test]
    fn nearby_high_cliff_is_neither_cut_nor_a_fill_source() {
        let mut original = flat_ground();
        let route = BTreeMap::from([(HexCoord::ORIGIN, 100)]);
        original.extend(route.iter().map(|(coord, level)| (*coord, *level)));
        let cliff = HexCoord::from_axial(5, 0);
        original.insert(cliff, 240);
        let eligible = original.keys().copied().collect();
        let projection =
            project_fill(&original, &route, &eligible).expect("cliff is already legal");
        assert!(!projection.fills.contains_key(&cliff));
        assert_eq!(projection.fills[&HexCoord::from_axial(4, 0)], 64);
        assert!(!projection.fills.contains_key(&HexCoord::from_axial(7, 0)));
        assert_edges_not_worsened(&original, &projection.fills);
    }

    #[test]
    fn low_fixed_boundary_caps_fill_and_reports_only_unfinished_route_edges() {
        let mut original = flat_ground();
        let route = BTreeMap::from([(HexCoord::ORIGIN, 100)]);
        original.extend(route.iter().map(|(coord, level)| (*coord, *level)));
        let fixed = HexCoord::from_axial(1, 0);
        let eligible = original
            .keys()
            .copied()
            .filter(|coord| *coord != fixed)
            .collect();
        let projection =
            project_fill(&original, &route, &eligible).expect("boundary clips locally");
        assert!(!projection.fills.contains_key(&fixed));
        assert_eq!(
            projection.deferred_seeds,
            BTreeSet::from([HexCoord::ORIGIN])
        );
        assert_eq!(projection.fills[&HexCoord::from_axial(1, -1)], 49);
        assert!(projection.fills.values().any(|level| *level > 49));
        assert_edges_not_worsened(&original, &projection.fills);
    }

    #[test]
    fn publication_preserves_existing_strata_and_makes_only_new_caps_scenic() {
        let coord = HexCoord::ORIGIN;
        let route_coord = HexCoord::from_axial(0, 1);
        let mask = BTreeSet::from([coord, route_coord]);
        let mut volume = VolumePlan::new(mask.clone());
        let biome = BiomeRegionId(1);
        let mut biomes = BTreeMap::new();
        for current in &mask {
            replace_column_surface(
                &mut volume,
                &mut biomes,
                *current,
                land_column(40, SolidMaterialRole::Grass),
                TilePos::new(*current, 40),
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
                biome,
            );
        }
        let before = volume.clone();
        let fine_index = FineWorldIndex {
            by_coord: BTreeMap::from_iter(mask.iter().map(|c| {
                (
                    *c,
                    FineWorldOwner {
                        patch: PatchId(1),
                        biome,
                    },
                )
            })),
        };
        publish_fills(
            &BTreeMap::from([(coord, 55)]),
            &fine_index,
            &mut volume,
            &mut biomes,
        )
        .expect("dry fill publishes");
        assert_eq!(volume.columns[&route_coord], before.columns[&route_coord]);
        assert_eq!(
            volume.top_surface_at_coord(route_coord),
            before.top_surface_at_coord(route_coord)
        );
        for level in 0..=40 {
            let material_at = |column: &VolumeColumn| {
                column.elements.iter().find_map(|element| {
                    let VolumeElement::Solid(mass) = element else {
                        return None;
                    };
                    (mass.levels.bottom <= level && level < mass.levels.top)
                        .then_some(mass.material)
                })
            };
            assert_eq!(
                material_at(&volume.columns[&coord]),
                material_at(&before.columns[&coord])
            );
        }
        assert_eq!(
            top_solid_material(&volume.columns[&coord]),
            SolidMaterialRole::Grass
        );
        assert_eq!(
            volume.top_surface_at_coord(coord),
            Some((
                TilePos::new(coord, 55),
                SurfaceMetadata {
                    access: SurfaceAccess::SpecialMovement(SCENIC_MOVEMENT_REGION),
                    interior: None
                }
            ))
        );
        assert!(!biomes.contains_key(&TilePos::new(coord, 40)));
        assert_eq!(biomes[&TilePos::new(coord, 55)], biome);
    }

    #[test]
    fn water_stacked_and_northern_surfaces_remain_fixed() {
        let center = HexCoord::from_axial(0, -110);
        let mut original =
            BTreeMap::from_iter(center.within_radius(22).into_iter().map(|c| (c, 40)));
        let route = BTreeMap::from([(center, 100)]);
        original.insert(center, 100);
        let projection = project_fill(&original, &route, &original.keys().copied().collect())
            .expect("north cap clips locally");
        assert!(projection.fills.keys().all(|coord| coord.y() >= -110));
        assert!(projection.deferred_seeds.contains(&center));
        assert_edges_not_worsened(&original, &projection.fills);

        let mut volume = VolumePlan::new(BTreeSet::from([center]));
        volume
            .columns
            .insert(center, water_column(40, 42, SolidMaterialRole::Sand));
        volume.surfaces.insert(
            TilePos::new(center, 40),
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );
        assert!(
            !fillable_column(&volume, center),
            "fill occupancy excludes water even before access metadata"
        );
        volume
            .columns
            .insert(center, land_column(40, SolidMaterialRole::Grass));
        volume.surfaces.insert(
            TilePos::new(center, 20),
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: Some(InteriorRegionId(1)),
            },
        );
        assert!(
            !fillable_column(&volume, center),
            "a stacked column never enters the scalar solver"
        );
    }

    #[test]
    fn author_freezes_water_prior_shoulders_shared_authority_and_route() {
        let mask = HexCoord::ORIGIN
            .within_radius(20)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let biome = BiomeRegionId(1);
        let fine_index = FineWorldIndex {
            by_coord: BTreeMap::from_iter(mask.iter().map(|coord| {
                (
                    *coord,
                    FineWorldOwner {
                        patch: PatchId(1),
                        biome,
                    },
                )
            })),
        };
        let layout = ResolvedLayoutPlan {
            kind: LayoutKind::Single,
            grid_radius: 20,
            footprint: mask.clone(),
            patches: BTreeMap::new(),
            shared_edges: BTreeMap::new(),
            boundary_liquid_outlets: BTreeMap::new(),
        };
        let route = HexCoord::ORIGIN;
        let water = HexCoord::from_axial(1, 0);
        let prior_shoulder = HexCoord::from_axial(0, 1);
        let shared = HexCoord::from_axial(-1, 1);
        let mut volume = VolumePlan::new(mask.clone());
        let mut biomes = BTreeMap::new();
        for coord in &mask {
            let level = if *coord == route { 100 } else { 40 };
            replace_column_surface(
                &mut volume,
                &mut biomes,
                *coord,
                land_column(level, SolidMaterialRole::Grass),
                TilePos::new(*coord, level),
                SurfaceMetadata {
                    access: if *coord == prior_shoulder {
                        SurfaceAccess::SpecialMovement(SCENIC_MOVEMENT_REGION)
                    } else {
                        SurfaceAccess::Ordinary
                    },
                    interior: None,
                },
                biome,
            );
        }
        volume
            .columns
            .insert(water, water_column(40, 42, SolidMaterialRole::Sand));
        let before = volume.clone();
        let filled = grade(
            &BTreeSet::from([route]),
            &layout,
            &fine_index,
            &BTreeSet::from([water]),
            &BTreeSet::from([prior_shoulder]),
            &BTreeMap::from([(shared, 40)]),
            &mut volume,
            &mut biomes,
        )
        .expect("fixed constraints clip without route changes");
        assert!(!filled.is_empty());
        for coord in [route, water, prior_shoulder, shared] {
            assert!(!filled.contains(&coord));
            assert_eq!(volume.columns[&coord], before.columns[&coord]);
            assert_eq!(
                volume.top_surface_at_coord(coord),
                before.top_surface_at_coord(coord)
            );
        }
        let original = before
            .surfaces
            .iter()
            .map(|(p, _)| (p.coord, p.level))
            .collect();
        let new_levels = filled
            .iter()
            .map(|coord| {
                let (surface, metadata) = volume.top_surface_at_coord(*coord).expect("new surface");
                assert_eq!(
                    metadata.access,
                    SurfaceAccess::SpecialMovement(SCENIC_MOVEMENT_REGION)
                );
                (*coord, surface.level)
            })
            .collect();
        assert_edges_not_worsened(&original, &new_levels);
    }
}
