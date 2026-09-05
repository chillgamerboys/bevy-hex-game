//! Bounded voxel spring beside the existing Garden island.
//!
//! This construction record stays private to final generation. Its exact caps
//! are the only exception to the island's warm-grass material contract.

use super::*;

pub(super) struct GardenSpring {
    pub(super) caps: BTreeMap<TilePos, SolidMaterialRole>,
    water: BTreeMap<TilePos, LiquidNode>,
    body: LiquidBodyId,
    outlet: TilePos,
    outlet_column: VolumeColumn,
    source: TilePos,
}

impl GardenSpring {
    pub(super) fn validate(&self, world: &GeneratedWorldPlan) -> Result<(), V3GenerationError> {
        let body = world.liquids.bodies.get(&self.body).ok_or_else(|| {
            schematic_contract("Garden spring lost the existing upper-lake liquid body")
        })?;
        if self.water.len() != 5
            || self.source.level != self.outlet.level.saturating_add(3)
            || world.volume.columns.get(&self.outlet.coord) != Some(&self.outlet_column)
            || !body.nodes.contains_key(&self.outlet)
        {
            return Err(schematic_contract(
                "Garden spring changed its five-cell source, descent, or lake outlet",
            ));
        }
        let fills = world.volume.fill_runs_by_top();
        for (position, expected) in &self.water {
            if body.nodes.get(position) != Some(expected)
                || fills.get(position).is_none_or(|fill| {
                    fill.material != FillMaterialRole::Water
                        || fill.levels.bottom != position.level
                        || fill.levels.top != position.level.saturating_add(1)
                })
            {
                return Err(schematic_contract(format!(
                    "Garden spring water or directed ownership changed at {position:?}"
                )));
            }
            for neighbor in position.coord.neighbors() {
                if let Some(external) = fills
                    .keys()
                    .find(|fill| fill.coord == neighbor && !self.water.contains_key(fill))
                {
                    if expected.downstream != Some(self.outlet)
                        || external.level != self.outlet.level
                        || !body.nodes.contains_key(external)
                    {
                        return Err(schematic_contract(
                            "Garden spring has an external liquid contact outside its spillway mouth",
                        ));
                    }
                }
            }
            let mut cursor = *position;
            let mut seen = BTreeSet::new();
            while cursor != self.outlet {
                if !seen.insert(cursor) {
                    return Err(schematic_contract("Garden spring has a flow cycle"));
                }
                let next = self
                    .water
                    .get(&cursor)
                    .and_then(|node| node.downstream)
                    .ok_or_else(|| {
                        schematic_contract("Garden spring has a stagnant side pocket")
                    })?;
                if cursor.coord.distance(next.coord) != 1
                    || cursor.level < next.level
                    || cursor.level.saturating_sub(next.level) > 1
                {
                    return Err(schematic_contract(
                        "Garden spring no longer has a narrow descending spillway",
                    ));
                }
                cursor = next;
            }
        }
        for (surface, material) in &self.caps {
            if world
                .volume
                .top_surface_at_coord(surface.coord)
                .map(|(top, _)| top)
                != Some(*surface)
                || solid_material_at(&world.volume, *surface) != Some(*material)
                || surface.coord.distance(self.source.coord) > 3
            {
                return Err(schematic_contract(format!(
                    "Garden spring lost bounded ground support at {surface:?}"
                )));
            }
        }
        let wet = self
            .water
            .keys()
            .map(|water| water.coord)
            .collect::<BTreeSet<_>>();
        if let Some((water, bank, level)) = water_bank_violations(&world.volume)
            .into_iter()
            .find(|(water, _, _)| wet.contains(&water.coord))
        {
            return Err(schematic_contract(format!(
                "Garden spring overtops dry bank {bank:?}@{level} beside {water:?}"
            )));
        }
        Ok(())
    }
}

/// Author after the final natural-cap pass, before vegetation. The tiny basin
/// has three source cells, a two-cell spillway, and an unchanged lake outlet.
pub(super) fn author_spring(
    plan: &SchematicPlanV1,
    profile: V3GrandV3BasicTerrainProfile,
    hydrology: &HydrologyCompilation,
    world: &mut GeneratedWorldPlan,
) -> Result<GardenSpring, V3GenerationError> {
    let island = semantic_overlay_coords(plan, &world.layout, SchematicFeature::LakeIsland);
    let courtyard = schematic_ecology::garden_courtyard_reservation(plan);
    let cell = plan
        .cells
        .iter()
        .find(|cell| has_overlay(cell, SchematicFeature::LakeIsland))
        .ok_or_else(|| schematic_contract("Garden spring has no island"))?;
    let center = schematic_to_world(cell.coord, 22);
    let falls = hydrology
        .waterfall_centerline
        .get(waterfall_intake_start(hydrology.waterfall_lip_index)?)
        .ok_or_else(|| schematic_contract("Garden spring has no waterfall-facing shore"))?
        .coord;
    let mut protected = courtyard;
    protected.extend(
        world
            .structures
            .by_id
            .values()
            .flat_map(|structure| structure.voxels.iter().map(|voxel| voxel.coord)),
    );
    protected.extend(
        world
            .features
            .protected_routes
            .values()
            .flat_map(|route| route.surfaces.iter().map(|surface| surface.coord)),
    );
    protected.extend(world.blockers.iter().map(|blocker| blocker.coord));
    let water_levels = world
        .volume
        .fill_runs_by_top()
        .keys()
        .map(|position| (position.coord, *position))
        .collect::<BTreeMap<_, _>>();
    let mut outlets = world
        .liquids
        .bodies
        .iter()
        .flat_map(|(id, body)| {
            body.nodes
                .keys()
                .filter(|position| {
                    position.level == profile.mountain_lake_level
                        && !island.contains(&position.coord)
                        && position
                            .coord
                            .neighbors()
                            .iter()
                            .any(|coord| island.contains(coord))
                })
                .map(move |position| (*position, *id))
        })
        .collect::<Vec<_>>();
    outlets.sort_by_key(|(outlet, _)| {
        (
            outlet.coord.distance(falls),
            outlet.coord.distance(center),
            *outlet,
        )
    });

    let (outlet, body, channel, bank_levels) = outlets.into_iter().find_map(|(outlet, body)| {
        let outward = outlet.coord.line_between(center).into_iter().take(4).collect::<Vec<_>>();
        if outward.len() != 4 { return None; }
        let source = *outward.last()?;
        let dry = |coord: &HexCoord| {
            island.contains(coord) && !protected.contains(coord) && !water_levels.contains_key(coord)
                && world.volume.surfaces_at_coord(*coord).count() == 1
                && world.volume.top_surface_at_coord(*coord).is_some_and(|(_, metadata)| {
                    metadata.interior.is_none() && metadata.access != SurfaceAccess::Ordinary
                })
        };
        if !outward.iter().skip(1).all(dry) { return None; }
        let wings = source.neighbors().into_iter().filter(|coord| {
            dry(coord) && !outward.contains(coord) && coord.distance(outlet.coord) >= 3
        }).collect::<Vec<_>>();
        for (first_index, first) in wings.iter().enumerate() {
            for second in wings.iter().skip(first_index.saturating_add(1)) {
                // Opposed wings form a small bowl rather than a long side arm.
                if first.distance(*second) != 2 { continue; }
                let mut channel = outward.iter().skip(1).enumerate().map(|(index, coord)| {
                    Some(TilePos::new(*coord, outlet.level.saturating_add(i32::try_from(index).ok()?.saturating_add(1))))
                }).collect::<Option<Vec<_>>>()?;
                channel.extend([TilePos::new(*first, outlet.level.saturating_add(3)), TilePos::new(*second, outlet.level.saturating_add(3))]);
                let wet = channel.iter().map(|position| position.coord).collect::<BTreeSet<_>>();
                let mut banks = BTreeMap::<HexCoord, Level>::new();
                let mut valid = true;
                for water in &channel {
                    for neighbor in water.coord.neighbors() {
                        if wet.contains(&neighbor) { continue; }
                        if let Some(external) = water_levels.get(&neighbor) {
                            if channel.first() == Some(water) && external.level == outlet.level {
                                continue;
                            }
                            valid = false;
                            break;
                        }
                        if !dry(&neighbor) || source.distance(neighbor) > 3 { valid = false; break; }
                        let old = world.volume.top_surface_at_coord(neighbor)?.0.level;
                        let level = old.max(water.level.saturating_add(1));
                        if level > old.saturating_add(3) { valid = false; break; }
                        banks.entry(neighbor).and_modify(|height| *height = (*height).max(level)).or_insert(level);
                    }
                    if !valid { break; }
                }
                if valid { return Some((outlet, body, channel, banks)); }
            }
        }
        None
    }).ok_or_else(|| schematic_contract("Garden spring cannot fit a bounded bowl and spillway on the waterfall-facing shore"))?;

    let source = channel
        .get(2)
        .copied()
        .ok_or_else(|| schematic_contract("Garden spring lost its authored source cell"))?;
    if source.coord.distance(falls) >= center.distance(falls) {
        return Err(schematic_contract(
            "Garden spring is not on the shore facing the descending waterfall intake",
        ));
    }
    let outlet_column = world
        .volume
        .columns
        .get(&outlet.coord)
        .cloned()
        .ok_or_else(|| schematic_contract("Garden spring lake outlet has no volume"))?;
    let mut caps = BTreeMap::new();
    let mut water = BTreeMap::new();
    for (index, position) in channel.iter().copied().enumerate() {
        let downstream = match index {
            0 => outlet,
            1 | 2 => channel.get(index - 1).copied().ok_or_else(|| {
                schematic_contract("Garden spring lost its previous spillway cell")
            })?,
            _ => source,
        };
        let node = LiquidNode {
            state: if position.level == downstream.level {
                LiquidFlowState::Current
            } else {
                LiquidFlowState::Rapid
            },
            downstream: Some(downstream),
        };
        let old = world
            .volume
            .top_surface_at_coord(position.coord)
            .ok_or_else(|| schematic_contract("Garden spring lost its dry footing"))?
            .0;
        let biome = *world
            .biome_regions
            .get(&old)
            .ok_or_else(|| schematic_contract("Garden spring lost its island biome"))?;
        let bed = TilePos::new(position.coord, position.level.saturating_sub(1));
        replace_column_surface(
            &mut world.volume,
            &mut world.biome_regions,
            position.coord,
            water_column(bed.level, position.level, SolidMaterialRole::Stone),
            bed,
            SurfaceMetadata {
                access: SurfaceAccess::NonStandable,
                interior: None,
            },
            biome,
        );
        caps.insert(bed, SolidMaterialRole::Stone);
        water.insert(position, node);
    }
    for (coord, level) in bank_levels {
        let (old, metadata) = world
            .volume
            .top_surface_at_coord(coord)
            .ok_or_else(|| schematic_contract("Garden spring lost its bank footing"))?;
        let biome = *world
            .biome_regions
            .get(&old)
            .ok_or_else(|| schematic_contract("Garden spring bank lost its island biome"))?;
        let cap = if named_sample(plan.provenance.world_seed, "garden_spring_bank", coord)
            .is_multiple_of(3)
        {
            SolidMaterialRole::Dirt
        } else {
            SolidMaterialRole::Stone
        };
        let surface = TilePos::new(coord, level);
        replace_column_surface(
            &mut world.volume,
            &mut world.biome_regions,
            coord,
            land_column(level, cap),
            surface,
            metadata,
            biome,
        );
        caps.insert(surface, cap);
    }
    let lake = world
        .liquids
        .bodies
        .get_mut(&body)
        .ok_or_else(|| schematic_contract("Garden spring lost its existing lake ownership"))?;
    for (position, node) in &water {
        if lake.nodes.insert(*position, *node).is_some() {
            return Err(schematic_contract(
                "Garden spring replaced existing lake flow",
            ));
        }
    }
    // Existing observation anchors reserve three cells against complete tree
    // geometry. That exactly encloses this source, its spillway and all banks.
    world.observation_anchors.insert(
        "grand_v3.garden_spring".to_owned(),
        TilePos::new(source.coord, source.level.saturating_sub(1)),
    );
    let spring = GardenSpring {
        caps,
        water,
        body,
        outlet,
        outlet_column,
        source,
    };
    spring.validate(world)?;
    Ok(spring)
}
