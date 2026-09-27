//! Layered production solids and clear routes from the shared geography document.
use super::*;
#[derive(Clone, Copy)]
pub(super) struct ClearLayer {
    pub top: i32,
    pub ceiling: i32,
    pub layer: SupportLayer,
    pub open: bool,
}
pub(super) struct AuthoredRoute {
    pub id: String,
    pub from: String,
    pub to: String,
    pub supports: Vec<VoxelPosition>,
    pub ribbon: Vec<VoxelPosition>,
}
#[derive(Default)]
pub(super) struct Layered {
    pub columns: BTreeMap<WorldHex, Vec<ClearLayer>>,
    pub cover: BTreeMap<WorldHex, i32>,
    pub reserved: BTreeMap<WorldHex, Vec<[i32; 2]>>,
    pub routes: Vec<AuthoredRoute>,
}
fn hex_line(a: WorldHex, b: WorldHex) -> Vec<WorldHex> {
    let n =
        a.q.abs_diff(b.q)
            .max(a.r.abs_diff(b.r))
            .max((a.q + a.r).abs_diff(b.q + b.r));
    if n == 0 {
        return vec![a];
    }
    let ax = world_xz(a);
    let bx = world_xz(b);
    (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            nearest_hex(ax[0] + (bx[0] - ax[0]) * t, ax[1] + (bx[1] - ax[1]) * t)
        })
        .collect()
}
fn centerline(
    g: &GrandGeography,
    points: &[[f64; 3]],
) -> Result<Vec<VoxelPosition>, ContractError> {
    let mut out = Vec::new();
    for pair in points.windows(2) {
        let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        let cells = hex_line(g.world_hex([a[0], a[2]]), g.world_hex([b[0], b[2]]));
        let count = cells.len().saturating_sub(1);
        let start = g.top_level(a[1]);
        let end = g.top_level(b[1]);
        if (end - start).unsigned_abs() as usize > count {
            return Err(ContractError::new(
                "grand.route",
                format!(
                    "route segment rises{} levels over{count} hex steps: {a:?}->{b:?}",
                    end - start
                ),
            ));
        }
        for (i, p) in cells.into_iter().enumerate() {
            let top = if count == 0 {
                start
            } else {
                start + ((end - start) as f64 * i as f64 / count as f64).round() as i32
            };
            let support = VoxelPosition {
                column: p,
                level: top - 1,
            };
            if out.last() != Some(&support) {
                out.push(support);
            }
        }
    }
    Ok(out)
}
impl Layered {
    fn room(
        &mut self,
        g: &GrandGeography,
        center: [f64; 2],
        half: [f64; 2],
        floor: f64,
        height: f64,
        layer: SupportLayer,
    ) {
        let root = g.world_hex(center);
        let radius = (g.length(half[0].hypot(half[1])) / 1.5).ceil() as i64 + 2;
        for q in -radius..=radius {
            for r in -radius..=radius {
                let p = WorldHex::new(root.q + q, root.r + r);
                let xz = g.model_xz(p);
                if (xz[0] - center[0]).abs() <= half[0] && (xz[1] - center[1]).abs() <= half[1] {
                    self.columns.entry(p).or_default().push(ClearLayer {
                        top: g.top_level(floor),
                        ceiling: g.top_level(floor + height),
                        layer,
                        open: false,
                    });
                }
            }
        }
    }
    fn route(
        &mut self,
        g: &GrandGeography,
        id: &str,
        from: &str,
        to: &str,
        points: &[[f64; 3]],
        width: f64,
        layer: SupportLayer,
        open: bool,
        open_ends: [bool; 2],
    ) -> Result<(), ContractError> {
        let supports = centerline(g, points)?;
        let radius = (g.length(width) * 0.5 / 1.5).ceil() as i64 + 1;
        let max_distance = g.length(width) * 0.5;
        let mut candidates: BTreeMap<WorldHex, Vec<(i32, f64)>> = BTreeMap::new();
        for s in &supports {
            let c = world_xz(s.column);
            for q in -radius..=radius {
                for r in -radius..=radius {
                    let p = WorldHex::new(s.column.q + q, s.column.r + r);
                    let x = world_xz(p);
                    let distance = (x[0] - c[0]).hypot(x[1] - c[1]);
                    if distance <= max_distance {
                        candidates
                            .entry(p)
                            .or_default()
                            .push((s.level + 1, distance));
                    }
                }
            }
        }
        let mut ribbon = Vec::new();
        for (p, mut choices) in candidates {
            // A flat start cap meets the final Crystal tread without a
            // rounded platform overhanging several earlier stair treads.
            if id == "frozen_shore" {
                if let (Some(first), Some(document)) = (supports.first(), g.document.as_ref()) {
                    let origin = g.model_xz(first.column);
                    let x = g.model_xz(p);
                    let end = -std::f64::consts::FRAC_PI_2
                        + document.ascent.turns * std::f64::consts::TAU;
                    let previous = end - std::f64::consts::TAU / 6.;
                    let incoming = [end.cos() - previous.cos(), end.sin() - previous.sin()];
                    if (x[0] - origin[0]).hypot(x[1] - origin[1]) < width + 2.
                        && (x[0] - origin[0]) * incoming[0] + (x[1] - origin[1]) * incoming[1]
                            < -0.01
                    {
                        continue;
                    }
                }
            }
            choices.sort_by_key(|(level, _)| *level);
            let mut clusters: Vec<Vec<(i32, f64)>> = Vec::new();
            for c in choices {
                if let Some(last) = clusters.last_mut() {
                    if last.last().is_some_and(|old| c.0 - old.0 <= 5) {
                        last.push(c);
                        continue;
                    }
                }
                clusters.push(vec![c]);
            }
            for cluster in clusters {
                let Some(&(top, _)) = cluster.iter().min_by(|a, b| a.1.total_cmp(&b.1)) else {
                    continue;
                };
                let xz = g.model_xz(p);
                let natural_top = g.document.as_ref().map_or(top, |d| {
                    g.top_level(oracle::mainland(d, xz).max(oracle::volcano(d, xz)))
                });
                // A passage can emerge through its intended terminal when the
                // natural cover ends. It must never extrude a ridge to hide it.
                let terminal_radius = width * 4.;
                let near_terminal = points.first().is_some_and(|a| {
                    open_ends[0] && (xz[0] - a[0]).hypot(xz[1] - a[2]) < terminal_radius
                }) || points.last().is_some_and(|a| {
                    open_ends[1] && (xz[0] - a[0]).hypot(xz[1] - a[2]) < terminal_radius
                });
                let terminal_cover = if near_terminal {
                    let mut minimum = natural_top;
                    if let Some(d) = &g.document {
                        for q in -3_i64..=3 {
                            for r in -3_i64..=3 {
                                if q.abs().max(r.abs()).max((q + r).abs()) <= 3 {
                                    let point = g.model_xz(WorldHex::new(p.q + q, p.r + r));
                                    minimum = minimum.min(g.top_level(
                                        oracle::mainland(d, point).max(oracle::volcano(d, point)),
                                    ));
                                }
                            }
                        }
                    }
                    minimum
                } else {
                    natural_top
                };
                let portal = near_terminal && terminal_cover < top + 16;
                self.columns.entry(p).or_default().push(ClearLayer {
                    top,
                    ceiling: top + 36,
                    layer,
                    open: open || portal,
                });
                self.reserved.entry(p).or_default().push([top, top + 8]);
                ribbon.push(VoxelPosition {
                    column: p,
                    level: top - 1,
                });
            }
        }
        self.routes.push(AuthoredRoute {
            id: id.into(),
            from: from.into(),
            to: to.into(),
            supports,
            ribbon,
        });
        Ok(())
    }
    pub fn compile(g: &GrandGeography) -> Result<Self, ContractError> {
        let Some(d) = &g.document else {
            return Ok(Self::default());
        };
        let mut out = Self::default();
        let a = &d.ascent;
        let mut spiral = Vec::new();
        for i in 0..=9 {
            let angle = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::TAU / 6.;
            spiral.push([
                a.center[0] + angle.cos() * a.stair_radius,
                a.base + (a.top - a.base) * i as f64 / 9.,
                a.center[1] + angle.sin() * a.stair_radius,
            ]);
        }
        out.route(
            g,
            "crystal_ascent",
            "crystal_ascent",
            "frozen_woods",
            &spiral,
            a.width,
            SupportLayer::CrystalFloor,
            true,
            [false, false],
        )?;
        out.route(
            g,
            "frozen_shore",
            "frozen_woods",
            "mountain_lake",
            &d.frozen_route.points,
            d.frozen_route.width,
            SupportLayer::Exterior,
            true,
            [false, false],
        )?;
        out.route(
            g,
            "volcano_ascent",
            "volcano_landing",
            "shrine_fire",
            &oracle::volcano_points(d),
            d.volcano_route.width,
            SupportLayer::Exterior,
            true,
            [false, false],
        )?;
        out.route(
            g,
            "shadow_tunnel",
            "shadow_entrance",
            "crystal_ascent",
            &d.shadow_route,
            16.,
            SupportLayer::Shadow,
            false,
            [true, true],
        )?;
        for room in &d.rooms {
            let frame = d
                .frames
                .get(&room.frame)
                .ok_or_else(|| ContractError::new("grand.room", "missing frame"))?;
            let floor = frame.floor.ok_or_else(|| {
                ContractError::new("grand.room", "interior room needs an explicit floor")
            })?;
            out.room(
                g,
                frame.origin,
                room.half_extents,
                floor,
                room.height,
                frame.layer,
            );
        }
        for (id, route) in &d.layer_routes {
            out.route(
                g,
                id,
                &route.from,
                &route.to,
                &route.points,
                route.width,
                route.layer,
                false,
                route.open_ends,
            )?;
        }
        // Fit cave ceilings beneath the actual mountain. Rooms under the World
        // Tree are enclosed by its reserved trunk volume, not an inflated pad.
        let mut natural = BTreeMap::new();
        for (&p, layers) in &mut out.columns {
            let mut neighborhood = Vec::new();
            for q in -3_i64..=3 {
                for r in -3_i64..=3 {
                    if q.abs().max(r.abs()).max((q + r).abs()) <= 3 {
                        let n = WorldHex::new(p.q + q, p.r + r);
                        let top = *natural.entry(n).or_insert_with(|| {
                            let point = g.model_xz(n);
                            g.top_level(oracle::mainland(d, point).max(oracle::volcano(d, point)))
                        });
                        neighborhood.push((n, top));
                    }
                }
            }
            let ceiling = neighborhood.iter().map(|(_, top)| *top - 8).min();
            for l in layers {
                if !l.open && l.layer != SupportLayer::RootTemple {
                    if let Some(limit) = ceiling.filter(|limit| *limit >= l.top + 8) {
                        l.ceiling = l.ceiling.min(limit);
                    }
                    for &(n, _) in &neighborhood {
                        out.cover
                            .entry(n)
                            .and_modify(|h| *h = (*h).max(l.ceiling + 8))
                            .or_insert(l.ceiling + 8);
                    }
                }
            }
        }
        Ok(out)
    }
}
impl GrandCompiler {
    pub(super) fn r02_surface(&self, p: WorldHex) -> GrandSurface {
        let Some(d) = &self.geography.document else {
            return terrain::surface(self, p);
        };
        let point = self.geography.model_xz(p);
        let mainland = oracle::mainland(d, point);
        let volcano = oracle::volcano(d, point);
        let mut h = mainland.max(volcano);
        let mut water = oracle::water(d, point, h);
        let fountain = &d.fountain_basin;
        if geography::irregular(point, fountain.center, fountain.radii, fountain.phase) < 1. {
            h = fountain.level - 2.;
            water = Some(fountain.level);
        }
        let (distance, y, _) = geography::route_distance(point, &d.fountain_rill.points);
        if distance < d.fountain_rill.width * 0.5 {
            h = h.min(y - 1.);
            water = Some(y);
        }
        // A vertical plunge is one connected liquid volume, not separated
        // three-unit pools following a steep staircase of surface samples.
        for pair in d.falls.points.windows(2) {
            let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            let length = (b[0] - a[0]).hypot(b[2] - a[2]);
            let (dist, t) = geography::segment(point, [a[0], a[2]], [b[0], b[2]]);
            if a[1] - b[1] > 10.
                && length < (a[1] - b[1]) * 0.5
                && dist < d.falls.width * 0.5
                && t > 0.
                && t < 1.
            {
                h = h.min(b[1] - 3.);
                water = Some(a[1] + t * (b[1] - a[1]));
            }
        }
        // The approved natural landform is authoritative. Cave authoring must
        // fit beneath it; cover requirements never change the exterior skyline.
        let top = self.geography.top_level(h).max(2);
        let mut material = if h < 9. {
            "sand"
        } else if volcano > mainland && h > 45. {
            "basalt"
        } else if h > 340. + 17. * (point[0] * 0.012 - point[1] * 0.014).sin() {
            "snow"
        } else if h > 210. + 17. * (point[0] * 0.014 + point[1] * 0.009).sin() {
            "stone"
        } else {
            "moss"
        };
        if self.crystal.contains(&p)
            && oracle::crystal_distance(d, point) >= d.ascent.well_apothem
            && material != "snow"
        {
            material = "slate";
        }
        if self.geography.frozen_planting_weight(p) > 0. && h > 180. && h < 255. {
            material = "snow";
        }
        GrandSurface {
            level: top - 1,
            material,
            water: water.map(|y| self.geography.top_level(y)),
        }
    }
    pub(super) fn r02_column(&self, p: WorldHex) -> (ColumnData, Option<LiquidColumn>) {
        let s = self.r02_surface(p);
        let top = s.level + 1;
        let mut runs = vec![
            run(0, 1, "bedrock"),
            run(1, (top - 4).max(1), "stone"),
            run(
                (top - 4).max(1),
                top - 1,
                if s.material == "moss" {
                    "soil"
                } else {
                    "slate"
                },
            ),
            run(top - 1, top, s.material),
        ];
        runs.retain(|r| r.bottom < r.top);
        if let Some(layers) = self.layered.columns.get(&p) {
            // The Crystal annulus is supporting stone strata, with clear
            // lower-turn passages cut through it. Only its upper exit bridges
            // the open well; normal exterior stairs are grounded into terrain.
            let document = self.geography.document.as_ref().expect("r02 document");
            let point = self.geography.model_xz(p);
            let a = &document.ascent;
            let dx = point[0] - a.center[0];
            let dz = point[1] - a.center[1];
            let in_well = dx
                .abs()
                .max((0.5 * dx + 0.866025403784 * dz).abs())
                .max((0.5 * dx - 0.866025403784 * dz).abs())
                < a.well_apothem;
            for l in layers {
                let base = if l.layer == SupportLayer::CrystalFloor {
                    self.geography.top_level(a.base)
                } else if l.open && !in_well {
                    top.min(l.top - 2)
                } else {
                    l.top - 2
                };
                if base < l.top {
                    cut(&mut runs, base, l.top);
                    runs.push(run(base, l.top, "stone"));
                }
            }
            for l in layers {
                cut(
                    &mut runs,
                    l.top,
                    if l.open && l.layer != SupportLayer::CrystalFloor {
                        MAX_LEVEL + 1
                    } else {
                        l.ceiling
                    },
                );
            }
            for l in layers {
                cut(&mut runs, l.top - 2, l.top);
                runs.push(run(
                    l.top - 2,
                    l.top,
                    if matches!(
                        l.layer,
                        SupportLayer::LibraryLower | SupportLayer::LibraryUpper
                    ) {
                        "limestone"
                    } else {
                        "stone"
                    },
                ));
            }
            runs.sort_by_key(|r| r.bottom);
        }
        let liquid = super::super::fill_sea_column(
            p,
            &mut runs,
            s.water.unwrap_or(SEA_TOP) - 1,
            "water",
            if s.water.is_some() {
                "grand/river"
            } else {
                "grand/ocean"
            },
        )
        .expect("finite admitted geography levels");
        (ColumnData { position: p, runs }, liquid)
    }
}

#[cfg(test)]
mod cave_cover_tests {
    use super::*;
    #[test]
    fn closed_cave_cover_never_inflates_approved_landforms() {
        let g = tests::compiler(false);
        let d = g.geography.document.as_ref().expect("canonical geography");
        let failures: Vec<_> = g
            .layered
            .cover
            .iter()
            .filter_map(|(&p, &required)| {
                let point = g.geography.model_xz(p);
                let natural = g
                    .geography
                    .top_level(oracle::mainland(d, point).max(oracle::volcano(d, point)));
                (required > natural).then_some((p, required, natural))
            })
            .collect();
        assert!(
            failures.is_empty(),
            "closed cave breaks natural cover: {} columns, first {:?}",
            failures.len(),
            failures.iter().take(24).map(|(p,r,n)| (g.geography.model_xz(*p), r,n)).collect::<Vec<_>>()
        );
    }
}
