//! Layered production solids and clear routes from the shared geography document.
use super::*;
/// Each layer and presentation samples the same conservative physical distance
/// through the exact connected coast. One graph step spans at least 1.5 world units.
fn coast_distance(coast: &[u16], p: WorldHex) -> f64 {
    f64::from(grid_value(coast, p, 0)) * 1.5
}
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
        coast: &[u16],
        id: &str,
        from: &str,
        to: &str,
        points: &[[f64; 3]],
        width: f64,
        layer: SupportLayer,
        open: bool,
        open_ends: [bool; 2],
    ) -> Result<(), ContractError> {
        let natural_surface = |p, fallback| {
            let xz = g.model_xz(p);
            g.document.as_ref().map_or(fallback, |d| {
                g.top_level(
                    oracle::mainland(d, xz, coast_distance(coast, p)).max(oracle::volcano(d, xz)),
                )
            })
        };
        let mut supports = centerline(g, points)?;
        if id == "garden_ascent" {
            // The shared garden field already grades the entire approach and
            // its court shoulders. A second waypoint-to-hex quantization cuts
            // lower strips into that field and creates two-level cross-steps.
            for support in &mut supports {
                support.level = natural_surface(support.column, support.level + 1) - 1;
            }
        }
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
                    // Overlapping samples of one bend form a single surface.
                    // Two 2-level slabs need 8 clear levels between them to be
                    // distinct traversable stories; closer proposals coalesce.
                    if last.last().is_some_and(|old| {
                        c.0 - old.0
                            < if layer == SupportLayer::Exterior {
                                10
                            } else {
                                6
                            }
                    }) {
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
                // End the Crystal ribbon at its shared exit cross-section.
                // A rounded cap beyond this line belongs to Frozen's departing
                // surface and must not publish obsolete lower stair supports.
                if id == "crystal_ascent" {
                    if let Some(end) = points.last() {
                        let point = g.model_xz(p);
                        if top >= g.top_level(end[1]) - 8
                            && (point[0] - end[0]).hypot(point[1] - end[2]) < width
                            && point[1] > end[2] + 0.5
                            && !supports.iter().any(|s| s.column == p && s.level + 1 == top)
                        {
                            continue;
                        }
                    }
                }
                // Where a ribbon edge enters its room across an ordinary
                // one-level threshold, the room floor is the composed support.
                // Publish that floor rather than a buried pre-union stair cap.
                let mut top = self
                    .columns
                    .get(&p)
                    .into_iter()
                    .flatten()
                    .find(|room| {
                        room.layer == layer
                            && !room.open
                            && room.top >= top
                            && room.top.abs_diff(top) <= 1
                    })
                    .map_or(top, |room| room.top);
                let xz = g.model_xz(p);
                let natural_top = natural_surface(p, top);
                if id == "garden_ascent" {
                    top = natural_top;
                }
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
                                    minimum = minimum.min(
                                        g.top_level(
                                            oracle::mainland(
                                                d,
                                                point,
                                                coast_distance(
                                                    coast,
                                                    WorldHex::new(p.q + q, p.r + r),
                                                ),
                                            )
                                            .max(oracle::volcano(d, point)),
                                        ),
                                    );
                                }
                            }
                        }
                    }
                    minimum
                } else {
                    natural_top
                };
                let portal = near_terminal && terminal_cover < top + 16;
                // A physical-height corridor joins the room's existing roof.
                // Terrain scaling may leave that roof below the corridor's
                // usual ceiling; do not carve through it into the exterior.
                let ceiling = self
                    .columns
                    .get(&p)
                    .into_iter()
                    .flatten()
                    .filter(|room| {
                        room.layer == layer
                            && !room.open
                            && room.top == top
                            && room.ceiling >= top + 8
                    })
                    .map(|room| room.ceiling)
                    .min()
                    .map_or(top + 36, |ceiling| ceiling.min(top + 36));
                self.columns.entry(p).or_default().push(ClearLayer {
                    top,
                    ceiling,
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
    pub fn compile(g: &GrandGeography, coast: &[u16]) -> Result<Self, ContractError> {
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
            coast,
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
            coast,
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
            coast,
            "volcano_ascent",
            "volcano_landing",
            "shrine_fire",
            &oracle::volcano_points(d),
            d.volcano_route.width,
            SupportLayer::Exterior,
            true,
            [false, false],
        )?;
        if let Some(access) = &d.garden_access {
            out.route(
                g,
                coast,
                "garden_ascent",
                "garden_landing",
                "garden",
                &access.points,
                access.width,
                SupportLayer::Exterior,
                true,
                [false, false],
            )?;
        }
        out.route(
            g,
            coast,
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
                coast,
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
        // Fit every closed passage beneath the actual landform. The root
        // temple keeps its four-level natural roof, including the approach;
        // the tree's reserved volume must not excuse an exposed trench.
        let mut natural = BTreeMap::new();
        for (&p, layers) in &mut out.columns {
            let mut neighborhood = Vec::new();
            for q in -3_i64..=3 {
                for r in -3_i64..=3 {
                    if q.abs().max(r.abs()).max((q + r).abs()) <= 3 {
                        let n = WorldHex::new(p.q + q, p.r + r);
                        let top = *natural.entry(n).or_insert_with(|| {
                            let point = g.model_xz(n);
                            g.top_level(
                                oracle::mainland(d, point, coast_distance(coast, n))
                                    .max(oracle::volcano(d, point)),
                            )
                        });
                        neighborhood.push((n, top));
                    }
                }
            }
            for l in layers {
                if !l.open {
                    // The shallow root chamber already requires four levels
                    // of roof in its full-room/architecture contract. Mountain
                    // cave routes retain their deeper eight-level cover.
                    let cover = if l.layer == SupportLayer::RootTemple {
                        4
                    } else {
                        8
                    };
                    let ceiling = neighborhood.iter().map(|(_, top)| *top - cover).min();
                    if let Some(limit) = ceiling.filter(|limit| *limit >= l.top + 8) {
                        l.ceiling = l.ceiling.min(limit);
                    }
                    for &(n, _) in &neighborhood {
                        out.cover
                            .entry(n)
                            .and_modify(|h| *h = (*h).max(l.ceiling + cover))
                            .or_insert(l.ceiling + cover);
                    }
                }
            }
        }
        Ok(out)
    }
}
impl GrandCompiler {
    /// The basin's southern stair is terrain authority, so emitted water starts
    /// above each tread rather than overlapping an object placed in the pool.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "Validated entry dimensions bound the nonnegative number of one-voxel risers."
    )]
    fn fountain_entry_top(&self, p: WorldHex) -> Option<i32> {
        let d = self.geography.document.as_ref()?;
        let entry = d.fountain_entry.as_ref()?;
        let frame = self.geography.frame("fountain").ok()?;
        let [east, north] = frame.local(p);
        if east.abs() > entry.half_width
            || north < -entry.south_length
            || north > -entry.central_inset
        {
            return None;
        }
        // The flowing outlet keeps its original bed and water profile.
        let (distance, _, _) =
            geography::route_distance(self.geography.model_xz(p), &d.fountain_rill.points);
        if distance < d.fountain_rill.width * 0.5 {
            return None;
        }
        let bed = self.geography.top_level(d.fountain_basin.level - 2.);
        let water = self.geography.top_level(d.fountain_basin.level);
        let risers = ((-north - entry.central_inset) / entry.tread_depth).ceil() as i32;
        Some((bed + risers).min(water))
    }

    pub(super) fn r02_surface(&self, p: WorldHex) -> GrandSurface {
        let Some(d) = &self.geography.document else {
            return terrain::surface(self, p);
        };
        let point = self.geography.model_xz(p);
        let mainland = oracle::mainland(d, point, coast_distance(&self.coast, p));
        let volcano = oracle::volcano(d, point);
        let mut h = mainland.max(volcano);
        let mut water = oracle::water(d, point, h);
        let fountain = &d.fountain_basin;
        if geography::irregular(point, fountain.center, fountain.radii, fountain.phase) < 1. {
            h = fountain.level - 2.;
            water = Some(fountain.level);
        }
        let fountain_entry = self.fountain_entry_top(p);
        if let Some(top) = fountain_entry {
            h = f64::from(top - d.transform.sea_top) * LEVEL_HEIGHT / d.transform.vertical_scale;
            water = (top < self.geography.top_level(fountain.level)).then_some(fountain.level);
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
        let water_top = water.map(|y| self.geography.top_level(y));
        // An authored wet margin stays at least one physical voxel deep when
        // relief scaling rounds its surface and bed to the same level.
        let top = water_top
            .map_or_else(
                || self.geography.top_level(h),
                |water| self.geography.top_level(h).min(water - 1),
            )
            .max(2);
        let mut material = if h < 9. {
            "sand"
        } else if water.is_some() {
            // Inland rock beds have a deliberate submerged cap. Terrestrial
            // altitude/biome bands must not paint moss or snow through lakes.
            "stone"
        } else if volcano > mainland && h > 45. {
            "basalt"
        } else if h > 340. + 17. * (point[0] * 0.012 - point[1] * 0.014).sin() {
            "snow"
        } else if h > 210. + 17. * (point[0] * 0.014 + point[1] * 0.009).sin() {
            "stone"
        } else {
            "moss"
        };
        if water.is_none()
            && self.crystal.contains(&p)
            && oracle::crystal_distance(d, point) >= d.ascent.well_apothem
            && material != "snow"
        {
            material = "slate";
        }
        if water.is_none() && self.geography.frozen_planting_weight(p) > 0. && h > 180. && h < 255.
        {
            material = "snow";
        }
        if fountain_entry.is_some() {
            material = "sand";
        }
        GrandSurface {
            level: top - 1,
            material,
            water: water_top,
        }
    }
    #[expect(
        clippy::expect_used,
        reason = "Only the admitted r02 compiler calls this path; its finite levels and required document are constructor invariants."
    )]
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
mod fountain_tests;

#[cfg(test)]
mod cave_cover_tests {
    use super::*;
    use std::collections::BTreeSet;
    #[test]
    fn inland_lake_beds_do_not_inherit_terrestrial_snow_or_moss() {
        let compiler = tests::compiler(false);
        let g = &compiler.geography;
        let d = g.document.as_ref().expect("canonical geography");
        // These exact columns produced blue, green and pale panels in the
        // plain05 lake frame despite sharing one standing-water surface.
        for (q, r) in [
            (728, -250),
            (783, -294),
            (690, -262),
            (695, -314),
            (743, -333),
        ] {
            let p = WorldHex::new(q, r);
            let surface = compiler.surface(p);
            let (column, liquid) = compiler.column(p);
            let liquid = liquid.expect("reviewed lake sample remains submerged");
            assert_eq!(liquid.top, g.top_level(d.upper_lake.level));
            assert!(liquid.top > surface.level + 1);
            assert_eq!(column.material_at(surface.level), Some("stone"), "{p:?}");
        }
        let dry_frozen = compiler.surface(g.world_hex([-60., 776.]));
        assert!(dry_frozen.water.is_none(), "forest ground remains dry");
        assert_eq!(dry_frozen.material, "snow", "dry Frozen forest keeps snow");
        let ocean = compiler.surface(g.world_hex([0., -1200.]));
        assert!(ocean.level < SEA_TOP);
        assert_eq!(
            ocean.material, "sand",
            "ocean/coastal sediment is preserved"
        );
    }

    #[test]
    fn closed_cave_cover_never_inflates_approved_landforms() {
        let compiler = tests::compiler(false);
        let g = &compiler.geography;
        let layered = &compiler.layered;
        let coast = &compiler.coast;
        let d = g.document.as_ref().expect("canonical geography");
        let failures: Vec<_> = layered
            .cover
            .iter()
            .filter_map(|(&p, &required)| {
                let point = g.model_xz(p);
                let natural = g.top_level(
                    oracle::mainland(d, point, coast_distance(coast, p))
                        .max(oracle::volcano(d, point)),
                );
                (required > natural).then_some((p, required, natural))
            })
            .collect();
        let mut regions = BTreeMap::<(i32, i32), (usize, i32)>::new();
        for (p, required, natural) in &failures {
            let [x, z] = g.model_xz(*p);
            let value = regions
                .entry(((x / 100.).floor() as i32, (z / 100.).floor() as i32))
                .or_default();
            value.0 += 1;
            value.1 = value.1.max(required - natural);
        }
        assert!(
            failures.is_empty(),
            "closed cave breaks natural cover: {} columns; regions {regions:?}, first {:?}",
            failures.len(),
            failures
                .iter()
                .take(24)
                .map(|(p, r, n)| (g.model_xz(*p), r, n))
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn root_corridor_keeps_natural_ground_above_its_complete_closed_ribbon() {
        let compiler = tests::compiler(false);
        let g = &compiler.geography;
        let d = g.document.as_ref().expect("canonical geography");
        let route = compiler
            .layered
            .routes
            .iter()
            .find(|route| route.id == "root_temple")
            .expect("root approach route");
        let room = d
            .rooms
            .iter()
            .find(|room| room.frame == "root_temple")
            .expect("root chamber");
        let frame = d.frames.get("root_temple").expect("root chamber frame");
        let mut closed = 0;
        let mut outside_room = 0;
        for support in &route.ribbon {
            let layers = compiler
                .layered
                .columns
                .get(&support.column)
                .expect("every published ribbon has its physical layer");
            let layer = layers
                .iter()
                .find(|layer| {
                    layer.layer == SupportLayer::RootTemple && layer.top == support.level + 1
                })
                .expect("exact root support layer");
            // Only the explicitly open entrance is exempt from a natural roof.
            if layer.open {
                continue;
            }
            let natural = compiler.surface(support.column).level + 1;
            let (column, water) = compiler.column(support.column);
            let exterior = column
                .runs
                .iter()
                .filter(|run| run.material != "water")
                .map(|run| run.top)
                .max()
                .expect("natural solid ground");
            assert!(water.is_none(), "root corridor remains dry");
            assert_eq!(
                exterior, natural,
                "closed root passage removes usable ground at {:?}",
                support.column
            );
            assert!(layer.ceiling - layer.top >= 8, "physical body clearance");
            assert!(natural - layer.ceiling >= 4, "existing root roof contract");
            assert!(compiler.clear_support(*support, 8));
            for level in layer.ceiling..natural {
                assert!(
                    column.material_at(level).is_some_and(|m| m != "water"),
                    "root roof breached at {:?}, level {level}",
                    support.column
                );
            }
            let point = g.model_xz(support.column);
            outside_room += usize::from(
                (point[0] - frame.origin[0]).abs() > room.half_extents[0]
                    || (point[1] - frame.origin[1]).abs() > room.half_extents[1],
            );
            closed += 1;
        }
        assert!(
            closed > route.ribbon.len() / 2,
            "survey the complete approach"
        );
        assert!(
            outside_room > 100,
            "cover the corridor preceding the chamber"
        );
        eprintln!("ROOT_CORRIDOR closed_columns={closed} outside_room={outside_room}");
    }

    #[test]
    fn root_room_keeps_natural_roof_and_architecture_beneath_lake_shore() {
        let compiler = tests::compiler(true);
        let g = &compiler.geography;
        let d = g.document.as_ref().expect("canonical geography");
        let room = d
            .rooms
            .iter()
            .find(|room| room.frame == "root_temple")
            .expect("authored root room");
        let frame = d.frames.get(&room.frame).expect("root room frame");
        let floor = g.top_level(frame.floor.expect("interior floor"));
        let ceiling = g.top_level(frame.floor.expect("interior floor") + room.height);
        let mut checked = 0;
        let mut minimum_cover = i32::MAX;
        for (&p, layers) in &compiler.layered.columns {
            let point = g.model_xz(p);
            if (point[0] - frame.origin[0]).abs() > room.half_extents[0]
                || (point[1] - frame.origin[1]).abs() > room.half_extents[1]
                || !layers
                    .iter()
                    .any(|layer| layer.layer == SupportLayer::RootTemple)
            {
                continue;
            }
            let (column, water) = compiler.column(p);
            let natural_top = compiler.surface(p).level + 1;
            assert!(water.is_none(), "root room stays dry at {p:?}");
            assert!(
                natural_top - ceiling >= 4,
                "room roof lacks natural cover at {p:?}: ceiling {ceiling}, terrain {natural_top}"
            );
            for level in ceiling..natural_top {
                assert!(
                    column
                        .material_at(level)
                        .is_some_and(|material| material != "water"),
                    "room carve removed its exterior roof at {p:?}, level {level}"
                );
            }
            assert!(
                column.material_at(floor - 1).is_some(),
                "supported room floor"
            );
            minimum_cover = minimum_cover.min(natural_top - ceiling);
            checked += 1;
        }
        assert!(checked > 500, "review the entire room, not only its center");
        for id in ["grand/root-temple-ribs", "grand/root-temple-plant"] {
            let object = compiler
                .objects
                .values()
                .flatten()
                .find(|object| object.id == id)
                .expect("retained root architecture");
            assert!(!object.occupancy.is_empty());
            for column in &object.occupancy {
                for run in &column.runs {
                    assert!(
                        run.bottom >= floor && run.top < ceiling,
                        "{id} no longer fits room height at {:?}",
                        column.position
                    );
                }
            }
        }
        eprintln!("ROOT_ROOM columns={checked} minimum_natural_cover_levels={minimum_cover}");
    }

    #[test]
    fn library_air_stair_reaches_the_actual_summit_shrine() {
        let compiler = tests::compiler(false);
        let route = compiler
            .layered
            .routes
            .iter()
            .find(|route| route.id == "library_air")
            .expect("authored upper-library stair");
        let anchor = |id: &str| {
            compiler
                .anchors
                .iter()
                .find(|anchor| anchor.id == id)
                .expect("published route destination")
                .position
        };
        assert_eq!(
            route.supports.first().copied(),
            Some(anchor("grand/anchor/library_upper")),
            "the spiral starts on the actual upper-library floor"
        );
        let shrine = anchor("grand/anchor/shrine_air");
        assert_eq!(
            shrine.level,
            compiler.r02_surface(shrine.column).level,
            "the summit landing meets natural rock instead of an elevated end plug"
        );
        assert_eq!(
            route.supports.last().copied(),
            Some(shrine),
            "a clear buried stair is not an arrival at the summit shrine"
        );
        assert!(
            compiler
                .layered
                .columns
                .get(&shrine.column)
                .into_iter()
                .flatten()
                .any(|layer| layer.layer == SupportLayer::LibraryUpper
                    && layer.top == shrine.level + 1
                    && layer.open),
            "the terminal must emerge at the exterior summit"
        );
        for support in route.supports.iter().chain(&route.ribbon) {
            assert!(
                compiler.clear_support(*support, 8),
                "Air stair lacks physical body space at {support:?}"
            );
        }
        for pair in route.supports.windows(2) {
            let [a, b] = pair else { unreachable!() };
            assert!(
                a.column.checked_distance(b.column).expect("bounded stair") <= 1
                    && a.level.abs_diff(b.level) <= 1,
                "Air stair requires an impossible step {a:?} -> {b:?}"
            );
        }
        eprintln!(
            "AIR_SUMMIT_JOIN supports={} ribbon={} actual_shrine={shrine:?}",
            route.supports.len(),
            route.ribbon.len()
        );
    }

    #[test]
    fn all_authored_route_ribbons_keep_full_body_clearance() {
        let compiler = tests::compiler(false);
        let mut failures = Vec::new();
        for route in &compiler.layered.routes {
            let blocked: Vec<_> = route
                .ribbon
                .iter()
                .filter(|p| !compiler.clear_support(**p, 8))
                .collect();
            println!(
                "R02_RIBBON {} columns={} blocked={}",
                route.id,
                route.ribbon.len(),
                blocked.len()
            );
            if !blocked.is_empty() {
                failures.push(format!(
                    "{}: {} blocked; first {:?}",
                    route.id,
                    blocked.len(),
                    blocked
                        .iter()
                        .take(12)
                        .map(|p| (
                            p,
                            compiler.geography.model_xz(p.column),
                            compiler.column(p.column).0.runs
                        ))
                        .collect::<Vec<_>>()
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn library_stairs_do_not_dam_the_main_plunge_receiving_pool() {
        let compiler = tests::compiler(false);
        let g = &compiler.geography;
        let layered = &compiler.layered;
        let d = g.document.as_ref().expect("document");
        let center = g.world_hex([494., 442.]);
        for (q, r) in DIRS.into_iter().chain(std::iter::once((0, 0))) {
            let p = WorldHex::new(center.q + q, center.r + r);
            let point = g.model_xz(p);
            let bed = oracle::mainland(d, point, coast_distance(&compiler.coast, p));
            let water = oracle::water(d, point, bed).expect("receiving pool is wet");
            let bed = g.top_level(bed);
            let water = g.top_level(water);
            for layer in layered.columns.get(&p).into_iter().flatten() {
                assert!(
                    layer.top <= bed,
                    "library slab {:?}..{} dams receiving pool {p:?}:{bed}..{water}",
                    layer.top - 2,
                    layer.top
                );
            }
        }
    }
    #[test]
    fn garden_ascent_ribbon_does_not_add_steps_across_the_court() {
        let compiler = tests::compiler(false);
        let route = compiler
            .layered
            .routes
            .iter()
            .find(|route| route.id == "garden_ascent")
            .expect("authored garden approach");
        let actual_top = |p| {
            compiler
                .column(p)
                .0
                .runs
                .iter()
                .filter(|run| run.material != "water")
                .map(|run| run.top)
                .max()
                .expect("court terrain")
        };
        let failure_edge = [WorldHex::new(736, -286), WorldHex::new(736, -285)];
        assert!(
            actual_top(failure_edge[0]).abs_diff(actual_top(failure_edge[1])) <= 1,
            "the observed court-to-fountain crossing must not drop two levels"
        );
        let mut crossings = 0;
        for support in &route.ribbon {
            let surface = compiler.r02_surface(support.column);
            assert_eq!(
                support.level, surface.level,
                "the garden tread and broad court use the same graded surface"
            );
            assert_eq!(actual_top(support.column), surface.level + 1);
            for neighbor in support.column.neighbors().expect("bounded garden") {
                let adjacent = compiler.r02_surface(neighbor);
                if surface.water.is_none()
                    && adjacent.water.is_none()
                    && surface.level.abs_diff(adjacent.level) <= 1
                {
                    assert!(
                        actual_top(support.column).abs_diff(actual_top(neighbor)) <= 1,
                        "a graded court crossing gained an extra step at {:?}->{neighbor:?}",
                        support.column
                    );
                    crossings += 1;
                }
            }
        }
        assert!(crossings > route.ribbon.len());
        eprintln!(
            "GARDEN_COURT_CROSSINGS ribbon={} gentle_dry_edges={crossings}",
            route.ribbon.len()
        );
    }

    #[test]
    fn garden_landing_has_a_shallow_water_join_and_open_court_route() {
        let compiler = tests::compiler(true);
        let route = compiler
            .layered
            .routes
            .iter()
            .find(|r| r.id == "garden_ascent")
            .expect("garden access is part of the authored expedition");
        let first = route.supports.first().expect("landing support");
        let last = route.supports.last().expect("court support");
        for support in route.supports.iter().chain(&route.ribbon) {
            assert!(
                compiler.clear_support(*support, 8),
                "blocked garden support {support:?}"
            );
        }
        for pair in route.supports.windows(2) {
            let [a, b] = pair else { unreachable!() };
            assert!(a.level.abs_diff(b.level) <= 1, "garden riser {a:?}->{b:?}");
        }
        let neighbors = [[1_i64, 0], [0, 1], [-1, 1], [-1, 0], [0, -1], [1, -1]];
        let mut domain = BTreeMap::new();
        for q in -12_i64..=12 {
            for r in -12_i64..=12 {
                let p = WorldHex::new(first.column.q + q, first.column.r + r);
                let (column, liquid) = compiler.column(p);
                let top = column
                    .runs
                    .iter()
                    .filter(|run| run.material != "water")
                    .map(|run| run.top)
                    .max()
                    .expect("lake or island ground");
                domain.insert(p, (top, liquid));
            }
        }
        let mut reached = BTreeSet::from([first.column]);
        let mut queue = std::collections::VecDeque::from([first.column]);
        while let Some(p) = queue.pop_front() {
            let (top, _) = domain.get(&p).expect("bounded support");
            for [q, r] in neighbors {
                let n = WorldHex::new(p.q + q, p.r + r);
                if domain
                    .get(&n)
                    .is_some_and(|(other, _)| top.abs_diff(*other) <= 1)
                    && reached.insert(n)
                {
                    queue.push_back(n);
                }
            }
        }
        assert!(
            reached
                .iter()
                .any(|p| domain.get(p).is_some_and(|(top, liquid)| {
                    liquid
                        .as_ref()
                        .is_some_and(|water| (1..=2).contains(&(water.top - top)))
                })),
            "island landing is isolated from its shallow submerged margin"
        );
        let garden = compiler.geography.frame("garden").expect("court frame");
        let support = compiler.support_at(&garden, [0., 0.]).expect("court floor");
        assert_eq!(
            *last, support,
            "the route must join the original garden court"
        );
        for p in [WorldHex::new(739, -283), WorldHex::new(739, -284)] {
            let rim = compiler
                .influences
                .get(&p.chunk())
                .into_iter()
                .flatten()
                .find(|object| object.id == "grand/fountain-rim")
                .expect("the existing fountain rim remains authored in its source chunk");
            assert!(
                rim.occupancy
                    .iter()
                    .any(|column| column.position == p && !column.runs.is_empty()),
                "the garden approach must preserve the nearby fountain rim at {p:?}"
            );
        }
    }
}
