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
    let surface = |at: TilePos| -> Result<(), String> {
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
        let above = TilePos::new(at.coord, at.level + 1);
        if view
            .voxels
            .get(&above)
            .is_some_and(|id| substances.is_solid(*id))
            || objects.get(&above.coord).is_some_and(|spans| {
                spans
                    .iter()
                    .any(|span| (span.bottom.level..=span.top_level).contains(&above.level))
            })
            || liquids.get(&above.coord).is_some_and(|spans| {
                spans
                    .iter()
                    .any(|span| (span.bottom.level..=span.top_level).contains(&above.level))
            })
        {
            return Err(format!("buried or wet expedition surface {at:?}"));
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
        if site
            .rally_entry
            .as_ref()
            .is_some_and(|key| !sites.route_nodes.contains_key(key))
        {
            return Err(format!("expedition site {id} has a dangling rally entry"));
        }
        for at in &site.deployment.surfaces {
            surface(*at)?;
        }
    }
    for (id, node) in &sites.route_nodes {
        if id.is_empty() {
            return Err("empty expedition junction id".into());
        }
        surface(*node)?;
    }
    let mut endpoints = BTreeSet::new();
    for (id, route) in &sites.routes {
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
            if a.coord.distance(b.coord) != 1 || a.level.abs_diff(b.level) > 1 {
                return Err(format!("expedition route {id} has a non-walkable step"));
            }
        }
        for at in &route.ribbon {
            surface(*at)?;
        }
        // Every shoulder must connect to the centerline through ordinary steps.
        let mut reached: BTreeSet<_> = route.supports.iter().copied().collect();
        let mut pending: Vec<_> = reached.iter().copied().collect();
        while let Some(at) = pending.pop() {
            for coord in at.coord.neighbors() {
                for level in (at.level - 1)..=(at.level + 1) {
                    let candidate = TilePos::new(coord, level);
                    if route.ribbon.contains(&candidate) && reached.insert(candidate) {
                        pending.push(candidate);
                    }
                }
            }
        }
        if reached != route.ribbon {
            return Err(format!("expedition route {id} has disconnected shoulders"));
        }
    }
    let mut claimed_water = BTreeSet::new();
    for (id, fountain) in &sites.fountains {
        if id.is_empty() || fountain.cells.is_empty() {
            return Err(format!("expedition fountain {id} has no liquid volume"));
        }
        for at in &fountain.cells {
            if !claimed_water.insert(*at) {
                return Err(format!("expedition fountain {id} overlaps another pool"));
            }
            if !liquids.get(&at.coord).is_some_and(|spans| {
                spans
                    .iter()
                    .any(|span| (span.bottom.level..=span.top_level).contains(&at.level))
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
            substance: SubstanceId::AIR,
        });
        validate(&view, geometry, &substances).expect("world liquid volume");
        view.expedition
            .as_mut()
            .expect("sites")
            .fountains
            .insert("duplicate".into(), pool);
        assert!(validate(&view, geometry, &substances).is_err());
    }
}
