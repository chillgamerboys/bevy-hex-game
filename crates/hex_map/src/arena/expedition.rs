//! Admission of passive expedition facts before the world publishes a reset.

use super::*;

pub(super) fn validate(
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    substances: &SubstanceTable,
) -> Result<(), String> {
    let Some(sites) = &view.expedition else {
        return Ok(());
    };
    if geometry.min_level >= geometry.max_level {
        return Err("invalid expedition vertical bounds".into());
    }
    let mut objects = BTreeMap::<_, Vec<_>>::new();
    for span in &view.static_spans {
        if span.blocks_movement {
            objects.entry(span.bottom.coord).or_default().push(span);
        }
    }
    let mut liquids = BTreeMap::<_, Vec<_>>::new();
    for span in &view.liquids {
        liquids.entry(span.bottom.coord).or_default().push(span);
    }
    let clear_interval = |coord: HexCoord, low: i64, high: i64| -> bool {
        if low > high || low < i64::from(geometry.min_level) || high > i64::from(geometry.max_level)
        {
            return false;
        }
        let (Ok(low), Ok(high)) = (i32::try_from(low), i32::try_from(high)) else {
            return false;
        };
        view.voxels
            .range(TilePos::new(coord, low)..=TilePos::new(coord, high))
            .next()
            .is_none()
            && !objects.get(&coord).is_some_and(|spans| {
                spans
                    .iter()
                    .any(|span| span.bottom.level <= high && low <= span.top_level)
            })
            && !liquids.get(&coord).is_some_and(|spans| {
                spans
                    .iter()
                    .any(|span| span.bottom.level <= high && low <= span.top_level)
            })
    };
    let surface = |at: TilePos, clearance: u32| -> Result<(), String> {
        if at.coord.distance(HexCoord::ORIGIN) > geometry.radius
            || at.level < geometry.min_level
            || at.level >= geometry.max_level
            || !view
                .voxels
                .get(&at)
                .is_some_and(|id| substances.is_solid(*id))
        {
            return Err(format!("unsupported expedition surface {at:?}"));
        }
        if !clear_interval(
            at.coord,
            i64::from(at.level) + 1,
            i64::from(at.level) + i64::from(clearance),
        ) {
            return Err(format!("buried, wet or cramped expedition surface {at:?}"));
        }
        Ok(())
    };
    for (id, site) in &sites.encounters {
        if id.is_empty()
            || !site
                .deployment
                .surfaces
                .contains(&site.deployment.preferred)
        {
            return Err(format!(
                "expedition site {id} has no admitted preferred surface"
            ));
        }
        if let Some(key) = &site.rally_entry {
            let entry = sites
                .route_nodes
                .get(key)
                .ok_or_else(|| format!("expedition site {id} has a dangling rally entry"))?;
            if !site.deployment.surfaces.contains(entry) {
                return Err(format!(
                    "expedition site {id} rally entry lies outside its deployment"
                ));
            }
        }
        for at in &site.deployment.surfaces {
            surface(*at, 1)?;
        }
    }
    for (id, node) in &sites.route_nodes {
        if id.is_empty() {
            return Err("empty expedition junction id".into());
        }
        surface(*node, 1)?;
    }
    let mut endpoints = BTreeSet::new();
    for (id, route) in &sites.routes {
        if route.clearance_levels == 0
            || i64::from(route.clearance_levels)
                > i64::from(geometry.max_level) - i64::from(geometry.min_level)
        {
            return Err(format!("expedition route {id} has invalid clearance"));
        }
        let step = |a: TilePos, b: TilePos| -> bool {
            let lower_air = i64::from(a.level.max(b.level)) + 1;
            let upper_air = i64::from(a.level.max(b.level)) + i64::from(route.clearance_levels);
            a.coord.distance(b.coord) == 1
                && a.level.abs_diff(b.level) <= 1
                && clear_interval(a.coord, lower_air, upper_air)
                && clear_interval(b.coord, lower_air, upper_air)
        };
        let from = sites.route_nodes.get(&route.from);
        let to = sites.route_nodes.get(&route.to);
        if id.is_empty()
            || from.is_none()
            || to.is_none()
            || route.from == route.to
            || route.supports.len() < 2
            || route.supports.first() != from
            || route.supports.last() != to
        {
            return Err(format!("expedition route {id} has invalid endpoints"));
        }
        let pair = if route.from < route.to {
            (&route.from, &route.to)
        } else {
            (&route.to, &route.from)
        };
        if !endpoints.insert(pair) {
            return Err(format!(
                "expedition route {id} duplicates a bidirectional edge"
            ));
        }
        for at in &route.supports {
            if !route.ribbon.contains(at) {
                return Err(format!("expedition route {id} leaves its declared ribbon"));
            }
        }
        for pair in route.supports.windows(2) {
            let [a, b] = pair else { continue };
            if !step(*a, *b) {
                return Err(format!("expedition route {id} has a non-walkable step"));
            }
        }
        for at in &route.ribbon {
            surface(*at, route.clearance_levels)?;
        }
        // Every shoulder must connect to the centerline through ordinary steps.
        let mut reached: BTreeSet<_> = route.supports.iter().copied().collect();
        let mut pending: Vec<_> = reached.iter().copied().collect();
        while let Some(at) = pending.pop() {
            for coord in at.coord.neighbors() {
                for level in at.level.saturating_sub(1)..=at.level.saturating_add(1) {
                    let candidate = TilePos::new(coord, level);
                    if route.ribbon.contains(&candidate)
                        && step(at, candidate)
                        && reached.insert(candidate)
                    {
                        pending.push(candidate);
                    }
                }
            }
        }
        if reached != route.ribbon {
            return Err(format!("expedition route {id} has disconnected shoulders"));
        }
    }
    let water = substances
        .id("water")
        .filter(|id| !id.is_air() && substances.get(*id).is_some_and(|material| !material.solid));
    let mut claimed_water = BTreeSet::new();
    for (id, fountain) in &sites.fountains {
        if id.is_empty() || fountain.cells.is_empty() {
            return Err(format!("expedition fountain {id} has no liquid volume"));
        }
        for at in &fountain.cells {
            if at.coord.distance(HexCoord::ORIGIN) > geometry.radius
                || at.level < geometry.min_level
                || at.level > geometry.max_level
                || view.voxels.contains_key(at)
                || objects.get(&at.coord).is_some_and(|spans| {
                    spans
                        .iter()
                        .any(|span| (span.bottom.level..=span.top_level).contains(&at.level))
                })
            {
                return Err(format!(
                    "expedition fountain {id} has an invalid water cell"
                ));
            }
            if !claimed_water.insert(*at) {
                return Err(format!("expedition fountain {id} overlaps another pool"));
            }
            if !liquids.get(&at.coord).is_some_and(|spans| {
                spans.iter().any(|span| {
                    Some(span.substance) == water
                        && water.is_some()
                        && (span.bottom.level..=span.top_level).contains(&at.level)
                })
            }) {
                return Err(format!("expedition fountain {id} names a non-liquid voxel"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::arena::{
        ArenaDeploymentRegion, ArenaEncounterSite, ArenaExpeditionRoute, ArenaExpeditionSites,
        ArenaFountainVolume,
    };

    fn fixture() -> (ArenaTerrainView, ArenaVoxelGeometry, SubstanceTable) {
        let content = load_content().expect("accepted content");
        let surface = |q, r| TilePos::new(HexCoord::from_axial(q, r), 8);
        let path: Vec<_> = (-3..=3).map(|q| surface(q, 0)).collect();
        let mut sites = ArenaExpeditionSites::default();
        sites.route_nodes.insert("bridge".into(), surface(-3, 0));
        sites.route_nodes.insert("grove".into(), surface(3, 0));
        sites.routes.insert(
            "trail".into(),
            ArenaExpeditionRoute {
                from: "bridge".into(),
                to: "grove".into(),
                clearance_levels: 3,
                supports: path.clone(),
                ribbon: path
                    .into_iter()
                    .chain((-3..=3).map(|q| surface(q, 1)))
                    .collect(),
            },
        );
        sites.encounters.insert(
            "camp".into(),
            ArenaEncounterSite {
                deployment: ArenaDeploymentRegion {
                    preferred: surface(3, 0),
                    surfaces: [surface(3, 0), surface(3, 1)].into(),
                },
                rally_entry: Some("grove".into()),
            },
        );
        let view = ArenaTerrainView {
            voxels: HexCoord::ORIGIN
                .within_radius(8)
                .into_iter()
                .map(|coord| (TilePos::new(coord, 8), content.materials.stone))
                .collect(),
            expedition: Some(sites),
            ..Default::default()
        };
        (view, ArenaVoxelGeometry::default(), content.substances)
    }

    #[test]
    fn valid_sites_and_full_width_route_are_admitted() {
        let (view, geometry, substances) = fixture();
        validate(&view, geometry, &substances).expect("supported geometry");
    }

    #[test]
    fn intact_centerline_does_not_hide_a_missing_shoulder() {
        let (mut view, geometry, substances) = fixture();
        view.voxels
            .remove(&TilePos::new(HexCoord::from_axial(0, 1), 8));
        assert!(validate(&view, geometry, &substances).is_err());
    }

    #[test]
    fn dangling_rally_and_reversed_endpoint_metadata_are_rejected() {
        let (view, geometry, substances) = fixture();
        let mut broken = view.clone();
        broken
            .expedition
            .as_mut()
            .expect("sites")
            .encounters
            .get_mut("camp")
            .expect("camp")
            .rally_entry = Some("missing".into());
        assert!(validate(&broken, geometry, &substances).is_err());
        let mut broken = view;
        broken
            .expedition
            .as_mut()
            .expect("sites")
            .routes
            .get_mut("trail")
            .expect("trail")
            .supports
            .reverse();
        assert!(validate(&broken, geometry, &substances).is_err());
    }

    #[test]
    fn pool_volume_requires_real_water_and_unique_claims() {
        let (mut view, geometry, substances) = fixture();
        let cell = TilePos::new(HexCoord::from_axial(-5, 0), 9);
        let pool = ArenaFountainVolume {
            cells: [cell].into(),
        };
        view.expedition
            .as_mut()
            .expect("sites")
            .fountains
            .insert("spring".into(), pool.clone());
        assert!(validate(&view, geometry, &substances).is_err());
        view.liquids.push(ArenaSolidSpan {
            bottom: cell,
            top_level: 9,
            substance: substances.id("water").expect("known water"),
        });
        validate(&view, geometry, &substances).expect("world liquid volume");
        view.expedition
            .as_mut()
            .expect("sites")
            .fountains
            .insert("duplicate".into(), pool);
        assert!(validate(&view, geometry, &substances).is_err());
    }

    #[test]
    fn route_clearance_rejects_zero_overflow_and_world_ceiling() {
        for clearance in [0, u32::MAX] {
            let (mut view, geometry, substances) = fixture();
            view.expedition
                .as_mut()
                .expect("sites")
                .routes
                .get_mut("trail")
                .expect("route")
                .clearance_levels = clearance;
            assert!(validate(&view, geometry, &substances)
                .expect_err("invalid clearance")
                .contains("invalid clearance"));
        }
        let (view, mut geometry, substances) = fixture();
        geometry.max_level = 10;
        assert!(
            validate(&view, geometry, &substances).is_err(),
            "complete route clearance must remain inside published bounds"
        );
    }

    #[test]
    fn full_route_clearance_rejects_terrain_canopy_and_water_above_first_air_voxel() {
        let (view, geometry, substances) = fixture();
        let blocked = TilePos::new(HexCoord::from_axial(0, 1), 10);
        let mut terrain = view.clone();
        terrain
            .voxels
            .insert(blocked, substances.id("stone").expect("stone"));
        assert!(validate(&terrain, geometry, &substances).is_err());
        let mut canopy = view.clone();
        canopy.static_spans.push(hex_core::arena::ArenaStaticSpan {
            bottom: blocked,
            top_level: 12,
            blocks_movement: true,
            blocks_projectiles: true,
            blocks_sight: true,
        });
        assert!(validate(&canopy, geometry, &substances).is_err());
        let mut flooded = view;
        flooded.liquids.push(ArenaSolidSpan {
            bottom: blocked,
            top_level: 12,
            substance: substances.id("water").expect("water"),
        });
        assert!(validate(&flooded, geometry, &substances).is_err());
    }

    #[test]
    fn deployment_candidates_do_not_claim_actor_body_clearance() {
        let (mut view, geometry, substances) = fixture();
        let floor = TilePos::new(HexCoord::from_axial(-4, 0), 8);
        view.voxels.insert(
            TilePos::new(floor.coord, 10),
            substances.id("stone").expect("stone"),
        );
        view.expedition.as_mut().expect("sites").encounters.insert(
            "candidate-only".into(),
            ArenaEncounterSite {
                deployment: ArenaDeploymentRegion {
                    preferred: floor,
                    surfaces: [floor].into(),
                },
                rally_entry: None,
            },
        );
        validate(&view, geometry, &substances).expect("gameplay owns actual body admission");
    }

    fn short_route(view: &mut ArenaTerrainView, path: Vec<TilePos>, ribbon: BTreeSet<TilePos>) {
        let sites = view.expedition.as_mut().expect("sites");
        sites.encounters.clear();
        sites.route_nodes = BTreeMap::from([
            ("a".into(), *path.first().expect("start")),
            ("b".into(), *path.last().expect("end")),
        ]);
        sites.routes = BTreeMap::from([(
            "step".into(),
            ArenaExpeditionRoute {
                from: "a".into(),
                to: "b".into(),
                clearance_levels: 2,
                supports: path,
                ribbon,
            },
        )]);
    }

    #[test]
    fn one_level_stair_requires_shared_aperture_in_both_directions() {
        let (mut view, geometry, substances) = fixture();
        let low = TilePos::new(HexCoord::from_axial(-1, 0), 8);
        let high = TilePos::new(HexCoord::from_axial(0, 0), 9);
        let stone = substances.id("stone").expect("stone");
        view.voxels.insert(high, stone);
        short_route(&mut view, vec![low, high], [low, high].into());
        view.voxels.insert(TilePos::new(low.coord, 11), stone);
        assert!(validate(&view, geometry, &substances)
            .expect_err("separately clear surfaces lack shared aperture")
            .contains("non-walkable step"));
        view.voxels.remove(&TilePos::new(low.coord, 11));
        view.voxels.insert(TilePos::new(low.coord, 12), stone);
        validate(&view, geometry, &substances).expect("raised lintel permits stair");
        let route = view
            .expedition
            .as_mut()
            .expect("sites")
            .routes
            .get_mut("step")
            .expect("route");
        std::mem::swap(&mut route.from, &mut route.to);
        route.supports.reverse();
        validate(&view, geometry, &substances).expect("same aperture downhill");
    }

    #[test]
    fn shoulder_connectivity_uses_the_same_shared_aperture_as_centerline_steps() {
        let (mut view, geometry, substances) = fixture();
        let a = TilePos::new(HexCoord::from_axial(-1, 0), 8);
        let b = TilePos::new(HexCoord::from_axial(0, 0), 8);
        let shoulder = TilePos::new(HexCoord::from_axial(0, 1), 9);
        let stone = substances.id("stone").expect("stone");
        view.voxels.insert(shoulder, stone);
        for floor in [a, b] {
            view.voxels.insert(TilePos::new(floor.coord, 11), stone);
        }
        short_route(&mut view, vec![a, b], [a, b, shoulder].into());
        assert!(validate(&view, geometry, &substances)
            .expect_err("cramped shoulder join")
            .contains("disconnected shoulders"));
        for floor in [a, b] {
            view.voxels.remove(&TilePos::new(floor.coord, 11));
        }
        validate(&view, geometry, &substances).expect("open shoulder join");
    }

    #[test]
    fn named_rally_entry_must_physically_join_its_deployment_region() {
        let (mut view, geometry, substances) = fixture();
        view.expedition
            .as_mut()
            .expect("sites")
            .encounters
            .get_mut("camp")
            .expect("camp")
            .rally_entry = Some("bridge".into());
        assert!(validate(&view, geometry, &substances)
            .expect_err("existing but remote entry")
            .contains("outside its deployment"));
    }

    #[test]
    fn fountain_water_rejects_air_unknown_solid_and_out_of_bounds_publications() {
        let (view, geometry, substances) = fixture();
        let cell = TilePos::new(HexCoord::from_axial(-5, 0), 9);
        let water = substances.id("water").expect("water");
        let stone = substances.id("stone").expect("stone");
        for (at, material) in [
            (cell, SubstanceId::AIR),
            (cell, SubstanceId(u16::MAX)),
            (cell, stone),
            (TilePos::new(HexCoord::from_axial(13, 0), 9), water),
            (TilePos::new(cell.coord, geometry.min_level - 1), water),
            (TilePos::new(cell.coord, geometry.max_level + 1), water),
        ] {
            let mut invalid = view.clone();
            invalid
                .expedition
                .as_mut()
                .expect("sites")
                .fountains
                .insert("spring".into(), ArenaFountainVolume { cells: [at].into() });
            invalid.liquids.push(ArenaSolidSpan {
                bottom: at,
                top_level: at.level,
                substance: material,
            });
            assert!(
                validate(&invalid, geometry, &substances).is_err(),
                "invalid water {at:?} {material:?}"
            );
        }
        let mut occupied = view;
        occupied
            .expedition
            .as_mut()
            .expect("sites")
            .fountains
            .insert(
                "spring".into(),
                ArenaFountainVolume {
                    cells: [cell].into(),
                },
            );
        occupied.liquids.push(ArenaSolidSpan {
            bottom: cell,
            top_level: cell.level,
            substance: water,
        });
        occupied.voxels.insert(cell, stone);
        assert!(
            validate(&occupied, geometry, &substances).is_err(),
            "water cannot occupy solid terrain"
        );
    }
}
