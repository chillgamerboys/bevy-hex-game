//! Bounded, recipe-independent operators over canonical exact interval columns.
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};

use super::{geometry, model::*, volume};
use hex_world_contracts::*;

type OpResult<T> = Result<T, String>;

#[derive(Clone, Debug, Default)]
pub(super) struct RegionBuild {
    pub columns: BTreeMap<WorldHex, Vec<VoxelRun>>,
    pub liquids: BTreeMap<WorldHex, LiquidColumn>,
    pub semantics: ChunkSemantics,
    pub features: Vec<FeatureSummary>,
    pub reserved: BTreeSet<WorldHex>,
    pub routes: BTreeMap<String, BTreeMap<WorldHex, i32>>,
}

fn run(bottom: i32, top: i32, material: &str) -> VoxelRun {
    VoxelRun {
        bottom,
        top,
        material: material.into(),
    }
}

pub(super) fn stack(level: i32, cap: &str, strata: &StrataSpec) -> OpResult<Vec<VoxelRun>> {
    if !(1..=65_535).contains(&level) {
        return Err(format!("terrain level {level} outside 1..=65535"));
    }
    let soil_bottom = i32::try_from((i64::from(level) - i64::from(strata.soil_depth)).max(1))
        .map_err(|error| error.to_string())?;
    let mut runs = vec![run(0, 1, &strata.bedrock)];
    if soil_bottom > 1 {
        runs.push(run(1, soil_bottom, &strata.rock));
    }
    if level > soil_bottom {
        runs.push(run(soil_bottom, level, &strata.soil));
    }
    runs.push(run(level, level + 1, cap));
    volume::canonicalize(runs)
}

pub(super) fn terrain(build: &RegionBuild, p: WorldHex) -> OpResult<(i32, String)> {
    let runs = build
        .columns
        .get(&p)
        .ok_or_else(|| format!("column {p:?} is outside region"))?;
    let top = runs
        .last()
        .ok_or_else(|| format!("empty terrain at {p:?}"))?;
    Ok((top.top - 1, top.material.clone()))
}

fn set_terrain(
    build: &mut RegionBuild,
    p: WorldHex,
    level: i32,
    cap: &str,
    recipe: &RegionRecipe,
) -> OpResult<()> {
    if !build.columns.contains_key(&p) {
        return Err(format!("operator leaves region at {p:?}"));
    }
    build.columns.insert(p, stack(level, cap, &recipe.strata)?);
    Ok(())
}

fn hash(seed: u64, id: &str, p: WorldHex) -> OpResult<u64> {
    hash_serializable(&(seed, id, p)).map_err(|error| error.to_string())
}

fn smooth_noise(seed: u64, id: &str, p: WorldHex, amplitude: u32) -> OpResult<i32> {
    // Bilinear integer lattice noise: coordinate-local, coherent, traversal independent.
    let scale = 12_i64;
    let q = p.q.div_euclid(scale);
    let r = p.r.div_euclid(scale);
    let x = p.q.rem_euclid(scale);
    let y = p.r.rem_euclid(scale);
    let mut sum = 0_i64;
    for (dq, wq) in [(0, scale - x), (1, x)] {
        for (dr, wr) in [(0, scale - y), (1, y)] {
            let raw = hash(seed, id, WorldHex::new(q + dq, r + dr))?;
            let value = (raw % (2 * u64::from(amplitude) + 1)) as i64 - i64::from(amplitude);
            sum += value * wq * wr;
        }
    }
    i32::try_from(sum / (scale * scale)).map_err(|error| error.to_string())
}

pub(super) fn base(region: &RegionSpec, recipe: &RegionRecipe, seed: u64) -> OpResult<RegionBuild> {
    let mut build = RegionBuild::default();
    for p in geometry::disk(WorldHex::new(0, 0), region.radius)? {
        let mut level = i64::from(recipe.base_level);
        for field in &recipe.landforms {
            let d = field
                .centers
                .iter()
                .map(|center| geometry::distance(p, *center))
                .collect::<OpResult<Vec<_>>>()?
                .into_iter()
                .min()
                .ok_or_else(|| format!("landform {} has no centers", field.id))?;
            if d >= u64::from(field.radius) {
                continue;
            }
            let width = i64::from(field.radius - field.plateau_radius);
            let support = (i64::from(field.radius) - d as i64).min(width);
            let noise = smooth_noise(seed, &field.id, p, field.relief)?;
            level += (i64::from(field.rise) + i64::from(noise)) * support / width;
        }
        let mut cap = recipe.strata.surface.as_str();
        let mut winner: Option<(i32, &str)> = None;
        for biome in &recipe.biomes {
            if geometry::distance(p, biome.mask.center)? <= u64::from(biome.mask.radius) {
                let key = (biome.priority, biome.id.as_str());
                if winner.is_none_or(|old| key > old) {
                    cap = &biome.material;
                    winner = Some(key);
                }
            }
        }
        let height = i32::try_from(level).map_err(|error| error.to_string())?;
        build.columns.insert(p, stack(height, cap, &recipe.strata)?);
    }
    for patch in &recipe.overrides {
        for p in geometry::disk(patch.mask.center, patch.mask.radius)? {
            let (old, cap) = terrain(&build, p)?;
            set_terrain(
                &mut build,
                p,
                patch.surface_level.unwrap_or(old),
                patch.material.as_deref().unwrap_or(&cap),
                recipe,
            )?;
        }
        check_constraints(&build, recipe, &patch.id)?;
    }
    Ok(build)
}

pub(super) fn check_constraints(
    build: &RegionBuild,
    recipe: &RegionRecipe,
    invalidator: &str,
) -> OpResult<()> {
    for (route_id, levels) in &build.routes {
        for (p, level) in levels {
            let runs = build
                .columns
                .get(p)
                .ok_or_else(|| format!("route {route_id} leaves footprint"))?;
            let wet = build
                .liquids
                .get(p)
                .is_some_and(|liquid| liquid.bottom <= *level && *level < liquid.top);
            if wet
                || volume::material_at(runs, *level).is_none()
                || volume::clear_above(runs, *level).is_some_and(|clear| clear < 2)
            {
                return Err(format!("operator {invalidator} violates protected route {route_id} at {p:?}, required surface level {level} and two-level headroom"));
            }
        }
    }
    for patch in &recipe.overrides {
        // During initial application, later overrides have not established their
        // contract yet. Other stages must preserve every authored hard override.
        if recipe
            .overrides
            .iter()
            .any(|candidate| candidate.id == invalidator)
            && patch.id.as_str() > invalidator
        {
            continue;
        }
        for p in geometry::disk(patch.mask.center, patch.mask.radius)? {
            let (level, material) = terrain(build, p)?;
            if patch
                .surface_level
                .is_some_and(|expected| expected != level)
                || patch
                    .material
                    .as_ref()
                    .is_some_and(|expected| expected != &material)
            {
                return Err(format!("operator {invalidator} violates hard override {} at {p:?}: actual level {level}, material {material}", patch.id));
            }
        }
    }
    Ok(())
}

fn band(
    mask: &BTreeSet<WorldHex>,
    footprint: &BTreeMap<WorldHex, Vec<VoxelRun>>,
    width: u32,
) -> BTreeMap<WorldHex, (u32, WorldHex)> {
    let mut found = BTreeMap::new();
    let mut queue = VecDeque::new();
    for p in mask {
        found.insert(*p, (0, *p));
        queue.push_back(*p);
    }
    while let Some(p) = queue.pop_front() {
        let Some((d, root)) = found.get(&p).copied() else {
            continue;
        };
        if d >= width {
            continue;
        }
        for n in geometry::neighbors(p) {
            if footprint.contains_key(&n) && !found.contains_key(&n) {
                found.insert(n, (d + 1, root));
                queue.push_back(n);
            }
        }
    }
    found
}

fn shoulders(
    build: &mut RegionBuild,
    levels: &BTreeMap<WorldHex, i32>,
    width: u32,
    recipe: &RegionRecipe,
) -> OpResult<()> {
    if width == 0 {
        return Ok(());
    }
    let mask = levels.keys().copied().collect();
    for (p, (d, root)) in band(&mask, &build.columns, width) {
        if d == 0 || build.reserved.contains(&p) || build.liquids.contains_key(&p) {
            continue;
        }
        if recipe.overrides.iter().any(|patch| {
            geometry::distance(p, patch.mask.center)
                .is_ok_and(|distance| distance <= u64::from(patch.mask.radius))
        }) {
            continue;
        }
        let Some(target) = levels.get(&root).copied() else {
            continue;
        };
        let (old, cap) = terrain(build, p)?;
        let level = i64::from(target)
            + (i64::from(old) - i64::from(target)) * i64::from(d) / i64::from(width);
        set_terrain(
            build,
            p,
            i32::try_from(level).map_err(|error| error.to_string())?,
            &cap,
            recipe,
        )?;
    }
    Ok(())
}

pub(super) fn seam_terrain(
    build: &mut RegionBuild,
    recipe: &RegionRecipe,
    levels: &BTreeMap<WorldHex, i32>,
    width: u32,
) -> OpResult<()> {
    shoulders(build, levels, width, recipe)?;
    for (p, level) in levels {
        set_terrain(build, *p, *level, &recipe.strata.surface, recipe)?;
        build.reserved.insert(*p);
    }
    Ok(())
}

fn water_column(
    build: &mut RegionBuild,
    recipe: &RegionRecipe,
    p: WorldHex,
    bed: i32,
    level: i32,
    material: &str,
    bed_material: &str,
    body: &str,
) -> OpResult<()> {
    if let Some(old) = build.liquids.get(&p) {
        if old.top != level + 1 {
            return Err(format!("incompatible water surfaces at {p:?}"));
        }
    }
    set_terrain(build, p, bed, bed_material, recipe)?;
    let runs = build
        .columns
        .get_mut(&p)
        .ok_or_else(|| "missing water column".to_string())?;
    volume::insert(runs, run(bed + 1, level + 1, material))?;
    build.liquids.insert(
        p,
        LiquidColumn {
            column: p,
            bottom: bed + 1,
            top: level + 1,
            kind: LiquidKind::Standing,
            body_id: body.into(),
            downstream: vec![],
        },
    );
    build.reserved.insert(p);
    Ok(())
}

pub(super) fn basin(
    build: &mut RegionBuild,
    recipe: &RegionRecipe,
    basin: &BasinSpec,
    body: &str,
) -> OpResult<()> {
    let bed = basin.water_level - basin.depth as i32;
    let levels = geometry::disk(basin.mask.center, basin.mask.radius)?
        .into_iter()
        .map(|p| (p, bed))
        .collect();
    shoulders(build, &levels, basin.bank_width, recipe)?;
    for p in geometry::disk(basin.mask.center, basin.mask.radius)? {
        water_column(
            build,
            recipe,
            p,
            bed,
            basin.water_level,
            &basin.material,
            &basin.bed_material,
            body,
        )?;
    }
    Ok(())
}

pub(super) fn seam_water(
    build: &mut RegionBuild,
    recipe: &RegionRecipe,
    points: &BTreeSet<WorldHex>,
    water: &BoundaryWaterSpec,
    body: &str,
) -> OpResult<()> {
    for p in points {
        water_column(
            build,
            recipe,
            *p,
            water.level - water.depth as i32,
            water.level,
            &water.material,
            &water.bed_material,
            body,
        )?;
    }
    Ok(())
}

fn controlled_line(points: &[GradePoint], falls: &[usize]) -> OpResult<Vec<GradePoint>> {
    let mut output = Vec::new();
    for (segment, pair) in points.windows(2).enumerate() {
        let [a, b] = pair else {
            continue;
        };
        let line = geometry::line(a.column, b.column)?;
        let steps = line.len().saturating_sub(1);
        if steps == 0 {
            return Err("duplicate route control coordinate".into());
        }
        for (i, p) in line.into_iter().enumerate() {
            if segment > 0 && i == 0 {
                continue;
            }
            let level = if falls.contains(&segment) && i < steps {
                a.level
            } else {
                i32::try_from(
                    i64::from(a.level)
                        + (i64::from(b.level) - i64::from(a.level)) * i as i64 / steps as i64,
                )
                .map_err(|error| error.to_string())?
            };
            output.push(GradePoint { column: p, level });
        }
    }
    Ok(output)
}

pub(super) fn channel(
    build: &mut RegionBuild,
    recipe: &RegionRecipe,
    channel: &ChannelSpec,
    body: &str,
) -> OpResult<()> {
    let center = controlled_line(&channel.points, &channel.falls_after)?;
    let mut levels: BTreeMap<WorldHex, (u64, usize, i32)> = BTreeMap::new();
    for (index, point) in center.iter().enumerate() {
        for p in geometry::disk(point.column, channel.half_width)? {
            let rank = (geometry::distance(p, point.column)?, index, point.level);
            if levels.get(&p).is_none_or(|old| rank < *old) {
                levels.insert(p, rank);
            }
        }
    }
    let bed_levels = levels
        .iter()
        .map(|(p, (_, _, level))| (*p, *level - channel.depth as i32))
        .collect();
    shoulders(build, &bed_levels, channel.bank_width, recipe)?;
    let sink = center.last().ok_or_else(|| "empty channel".to_string())?;
    // Reverse monotone reachability proves every ribbon cell drains to the sink.
    let mut drainage = BTreeMap::new();
    let mut queue = VecDeque::new();
    drainage.insert(sink.column, 0_u32);
    queue.push_back(sink.column);
    while let Some(p) = queue.pop_front() {
        let Some((_, _, level)) = levels.get(&p) else {
            continue;
        };
        let d = drainage
            .get(&p)
            .copied()
            .ok_or_else(|| "missing drainage rank".to_string())?;
        for n in geometry::neighbors(p) {
            if let Some((_, _, nl)) = levels.get(&n) {
                if nl >= level && !drainage.contains_key(&n) {
                    drainage.insert(n, d + 1);
                    queue.push_back(n);
                }
            }
        }
    }
    if drainage.len() != levels.len() {
        return Err(format!("channel {} has undrained ribbon cells", channel.id));
    }
    for (p, (_, _, level)) in &levels {
        let rank = drainage
            .get(p)
            .copied()
            .ok_or_else(|| "undrained channel".to_string())?;
        let next = geometry::neighbors(*p)
            .filter_map(|n| {
                let d = *drainage.get(&n)?;
                let (_, _, next_level) = *levels.get(&n)?;
                (d < rank && next_level <= *level).then_some((d, n, next_level))
            })
            .min();
        let bed = (*level - channel.depth as i32).min(next.map_or(*level, |(_, _, height)| height));
        water_column(
            build,
            recipe,
            *p,
            bed,
            *level,
            &channel.material,
            &channel.bed_material,
            body,
        )?;
        if let (Some((_, n, next_level)), Some(liquid)) = (next, build.liquids.get_mut(p)) {
            liquid.kind = if level - next_level > 1 {
                LiquidKind::Waterfall
            } else {
                LiquidKind::Directed
            };
            liquid.downstream.push(VoxelPosition {
                column: n,
                level: next_level,
            });
        }
    }
    Ok(())
}

pub(super) fn route(
    build: &mut RegionBuild,
    recipe: &RegionRecipe,
    route: &RouteSpec,
) -> OpResult<()> {
    let mut levels = BTreeMap::new();
    for pair in route.points.windows(2) {
        let [a, b] = pair else {
            continue;
        };
        let path = geometry::line(a.column, b.column)?;
        let mask = geometry::ribbon(&path, route.half_width)?;
        let grade = geometry::grade(
            &mask,
            VoxelPosition {
                column: a.column,
                level: a.level,
            },
            VoxelPosition {
                column: b.column,
                level: b.level,
            },
        )?;
        for (p, height) in grade {
            if build.liquids.contains_key(&p) {
                return Err(format!(
                    "route {} crosses water at {p:?}; declare a bridge",
                    route.id
                ));
            }
            if let Some(old) = levels.insert(p, height) {
                if old != height {
                    return Err(format!(
                        "route {} has incompatible overlapping grades",
                        route.id
                    ));
                }
            }
        }
    }
    shoulders(build, &levels, route.shoulder_width, recipe)?;
    build.routes.insert(route.id.clone(), levels.clone());
    for (p, height) in levels {
        set_terrain(build, p, height, &route.material, recipe)?;
        build.reserved.insert(p);
    }
    Ok(())
}

pub(super) fn auto_route(
    build: &mut RegionBuild,
    recipe: &RegionRecipe,
    endpoint: GradePoint,
    id: &str,
) -> OpResult<()> {
    // A* only evaluates this declared region. Water is impassable; no world scan.
    let start = recipe.hub.column;
    let end = endpoint.column;
    let mut frontier = BinaryHeap::new();
    let mut costs = BTreeMap::new();
    let mut previous = BTreeMap::new();
    costs.insert(start, 0_u64);
    frontier.push(Reverse((geometry::distance(start, end)?, 0_u64, start)));
    while let Some(Reverse((_, cost, p))) = frontier.pop() {
        if p == end {
            break;
        }
        if costs.get(&p).is_some_and(|known| *known < cost) {
            continue;
        }
        for n in geometry::neighbors(p) {
            if !build.columns.contains_key(&n) || build.liquids.contains_key(&n) {
                continue;
            }
            if n != start
                && n != end
                && recipe.overrides.iter().any(|patch| {
                    geometry::distance(n, patch.mask.center)
                        .is_ok_and(|distance| distance <= u64::from(patch.mask.radius))
                })
            {
                continue;
            }
            let next = cost + 1;
            if costs.get(&n).is_none_or(|known| next < *known) {
                costs.insert(n, next);
                previous.insert(n, p);
                frontier.push(Reverse((next + geometry::distance(n, end)?, next, n)));
            }
        }
    }
    if !costs.contains_key(&end) {
        return Err(format!("boundary {id} cannot reach hub"));
    }
    let mut path = vec![end];
    let mut p = end;
    while p != start {
        p = *previous
            .get(&p)
            .ok_or_else(|| "incomplete boundary approach".to_string())?;
        path.push(p);
    }
    path.reverse();
    let mask: BTreeSet<_> = path.into_iter().collect();
    let levels = geometry::grade(
        &mask,
        VoxelPosition {
            column: start,
            level: recipe.hub.level,
        },
        VoxelPosition {
            column: end,
            level: endpoint.level,
        },
    )?;
    shoulders(build, &levels, 3, recipe)?;
    for (p, height) in levels {
        set_terrain(build, p, height, &recipe.strata.surface, recipe)?;
        build.reserved.insert(p);
    }
    Ok(())
}

pub(super) fn bridge(build: &mut RegionBuild, bridge: &BridgeSpec) -> OpResult<()> {
    let center = controlled_line(&bridge.points, &[])?;
    let mut deck = BTreeMap::new();
    for point in center {
        for p in geometry::disk(point.column, bridge.half_width)? {
            if let Some(old) = deck.insert(p, point.level) {
                if old != point.level {
                    return Err(format!(
                        "bridge {} needs constant level across overlapping deck disks",
                        bridge.id
                    ));
                }
            }
        }
    }
    for (p, level) in deck {
        if build
            .liquids
            .get(&p)
            .is_some_and(|liquid| level - bridge.thickness as i32 + 1 < liquid.top)
        {
            return Err(format!(
                "bridge {} intersects liquid at {p:?}; raise the deck",
                bridge.id
            ));
        }
        let columns = build
            .columns
            .get_mut(&p)
            .ok_or_else(|| format!("bridge {} leaves region", bridge.id))?;
        let bottom = level - bridge.thickness as i32 + 1;
        if columns.last().is_some_and(|run| run.top > level + 1) {
            return Err(format!(
                "bridge {} is buried beneath terrain at {p:?}",
                bridge.id
            ));
        }
        // Endpoints can be rooted in solid abutments; crossing intervals must retain air/water below.
        volume::replace(columns, bottom, level + 1, Some(&bridge.material))?;
        build.reserved.insert(p);
    }
    Ok(())
}

pub(super) fn cave(
    build: &mut RegionBuild,
    recipe: &RegionRecipe,
    region_id: &str,
    cave: &CaveSpec,
) -> OpResult<()> {
    let path = geometry::polyline(&cave.path)?;
    let mut mask = geometry::ribbon(&path, cave.half_width)?;
    for room in &cave.rooms {
        mask.extend(geometry::disk(room.center, room.radius)?);
    }
    let roof_bottom = cave.floor_level + cave.clearance as i32 + 1;
    let roof_top = roof_bottom + cave.roof_thickness as i32;
    let id = format!("{region_id}/{}", cave.id);
    let domain = format!("{id}/light");
    for p in mask {
        if build.liquids.contains_key(&p) {
            return Err(format!("cave {} intersects liquid at {p:?}", cave.id));
        }
        let (old, _) = terrain(build, p)?;
        if old < roof_top - 1 {
            let columns = build
                .columns
                .get_mut(&p)
                .ok_or_else(|| "missing cave column".to_string())?;
            // A vault adds only above the old exterior, preserving existing
            // lower interiors instead of replacing the entire column stack.
            volume::insert(columns, run(old + 1, roof_top, &recipe.strata.rock))?;
        }
        let columns = build
            .columns
            .get_mut(&p)
            .ok_or_else(|| "missing cave column".to_string())?;
        volume::replace(
            columns,
            cave.floor_level,
            cave.floor_level + 1,
            Some(&cave.material),
        )?;
        if cave.entrances.contains(&p) {
            let top = columns.last().map_or(roof_top, |run| run.top);
            volume::replace(
                columns,
                cave.floor_level + 1,
                top.max(cave.floor_level + 2),
                None,
            )?;
        } else {
            volume::replace(columns, cave.floor_level + 1, roof_bottom, None)?;
            volume::replace(columns, roof_bottom, roof_top, Some(&cave.material))?;
            build.semantics.interiors.push(InteriorSpan {
                id: id.clone(),
                column: p,
                floor_level: cave.floor_level,
                roof_bottom,
                roof_top,
                light_domain: domain.clone(),
            });
        }
        build.reserved.insert(p);
    }
    for (i, p) in cave.entrances.iter().enumerate() {
        build.semantics.anchors.push(WorldAnchor {
            id: format!("{id}/entrance-{i}"),
            region_id: region_id.into(),
            position: VoxelPosition {
                column: *p,
                level: cave.floor_level,
            },
            role: AnchorRole::Gameplay,
        });
    }
    if cave.light_spacing > 0 {
        for (i, p) in path.iter().enumerate().step_by(cave.light_spacing as usize) {
            if cave.entrances.contains(p) {
                continue;
            }
            build.semantics.lights.push(WorldLight {
                id: format!("{id}/lamp-{i}"),
                position: VoxelPosition {
                    column: *p,
                    level: cave.floor_level + 2,
                },
                domain: Some(domain.clone()),
                bright_radius: 4,
                dim_radius: 9,
            });
        }
    }
    Ok(())
}

pub(super) fn decorate(
    build: &mut RegionBuild,
    recipe: &RegionRecipe,
    region_id: &str,
    seed: u64,
) -> OpResult<()> {
    for rule in &recipe.features {
        let mut roots: BTreeSet<_> = rule.roots.iter().copied().collect();
        for p in geometry::disk(rule.mask.center, rule.mask.radius)? {
            if hash(seed, &format!("{region_id}/{}", rule.id), p)? % 10_000
                < u64::from(rule.density)
            {
                roots.insert(p);
            }
        }
        for root in roots {
            let explicit = rule.roots.contains(&root);
            if build.reserved.contains(&root) || !build.columns.contains_key(&root) {
                if explicit {
                    return Err(format!(
                        "explicit feature {} root is reserved/outside at {root:?}",
                        rule.id
                    ));
                }
                continue;
            }
            let (ground, _) = terrain(build, root)?;
            let turn = (hash(seed, &rule.id, root)? % 6) as u8;
            let mut occupancy: BTreeMap<WorldHex, Vec<VoxelRun>> = BTreeMap::new();
            let mut rejected = false;
            for voxel in &rule.voxels {
                let p = root
                    .checked_add(
                        voxel
                            .offset
                            .rotate_60(turn)
                            .map_err(|error| error.to_string())?,
                    )
                    .map_err(|error| error.to_string())?;
                let candidate = run(
                    ground + 1 + voxel.bottom,
                    ground + 1 + voxel.top,
                    &voxel.material,
                );
                let Some(existing) = build.columns.get(&p) else {
                    rejected = true;
                    break;
                };
                if build.reserved.contains(&p)
                    || existing
                        .iter()
                        .any(|old| old.bottom < candidate.top && candidate.bottom < old.top)
                {
                    rejected = true;
                    break;
                }
                occupancy.entry(p).or_default().push(candidate);
            }
            if rejected {
                if explicit {
                    return Err(format!(
                        "explicit feature {} has blocked occupancy at {root:?}",
                        rule.id
                    ));
                }
                continue;
            }
            let id = format!("{region_id}/{}/q{}-r{}", rule.id, root.q, root.r);
            let mut object_columns = Vec::new();
            for (p, runs) in occupancy {
                let canonical = volume::canonicalize(runs)?;
                object_columns.push(ColumnData {
                    position: p,
                    runs: canonical,
                });
                build.reserved.insert(p);
            }
            build.features.push(FeatureSummary {
                id: id.clone(),
                region_id: region_id.into(),
                kind: rule.kind.clone(),
                anchor: VoxelPosition {
                    column: root,
                    level: ground,
                },
                asset: Some(rule.asset.clone()),
            });
            build.semantics.objects.push(ObjectInstance {
                id,
                region_id: region_id.into(),
                asset: rule.asset.clone(),
                origin: VoxelPosition {
                    column: root,
                    level: ground + 1,
                },
                rotation: turn,
                occupancy: object_columns,
            });
        }
    }
    Ok(())
}

pub(super) fn validate_access(
    build: &RegionBuild,
    recipe: &RegionRecipe,
    materials: &[MaterialSpec],
) -> OpResult<()> {
    let solids: BTreeSet<_> = materials
        .iter()
        .filter(|material| material.solid)
        .map(|material| material.id.as_str())
        .collect();
    let mut projected: BTreeMap<WorldHex, Vec<VoxelRun>> = BTreeMap::new();
    for object in &build.semantics.objects {
        for column in &object.occupancy {
            projected
                .entry(column.position)
                .or_default()
                .extend(column.runs.clone());
        }
    }
    let mut surfaces = BTreeMap::new();
    for (p, terrain) in &build.columns {
        let mut runs = terrain.clone();
        if let Some(occupancy) = projected.get(p) {
            runs.extend(occupancy.clone());
        }
        let runs = volume::canonicalize(runs)?;
        let available: Vec<_> = runs
            .iter()
            .filter(|run| {
                solids.contains(run.material.as_str())
                    && volume::clear_above(&runs, run.top - 1).is_none_or(|clear| clear >= 2)
            })
            .map(|run| run.top - 1)
            .collect();
        surfaces.insert(*p, available);
    }
    for route in &recipe.routes {
        for pin in &route.points {
            if !surfaces
                .get(&pin.column)
                .is_some_and(|levels| levels.contains(&pin.level))
            {
                return Err(format!(
                    "route {} lost required endpoint support/headroom at {:?} level {}",
                    route.id, pin.column, pin.level
                ));
            }
        }
    }
    for bridge in &recipe.bridges {
        for pin in &bridge.points {
            let runs = build
                .columns
                .get(&pin.column)
                .ok_or_else(|| "bridge endpoint outside region".to_string())?;
            if volume::material_at(runs, pin.level) != Some(bridge.material.as_str()) {
                return Err(format!(
                    "bridge {} lost exact deck endpoint at {:?}",
                    bridge.id, pin.column
                ));
            }
        }
    }
    let start = VoxelPosition {
        column: recipe.hub.column,
        level: recipe.hub.level,
    };
    if !surfaces
        .get(&start.column)
        .is_some_and(|levels| levels.contains(&start.level))
    {
        return Err("hub lacks exact support or two-level clearance".into());
    }
    let mut reached = BTreeSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(p) = queue.pop_front() {
        for column in geometry::neighbors(p.column) {
            if let Some(levels) = surfaces.get(&column) {
                for level in levels {
                    let neighbor = VoxelPosition {
                        column,
                        level: *level,
                    };
                    if p.level.abs_diff(*level) <= 1 && reached.insert(neighbor) {
                        queue.push_back(neighbor);
                    }
                }
            }
        }
    }
    for anchor in &build.semantics.anchors {
        if anchor.role != AnchorRole::Observation && !reached.contains(&anchor.position) {
            return Err(format!(
                "anchor {} has no ordinary two-level walking route from hub",
                anchor.id
            ));
        }
    }
    for bridge in &recipe.bridges {
        for pin in &bridge.points {
            if !reached.contains(&VoxelPosition {
                column: pin.column,
                level: pin.level,
            }) {
                return Err(format!(
                    "bridge {} endpoint {:?} is disconnected from hub",
                    bridge.id, pin.column
                ));
            }
        }
    }
    Ok(())
}
