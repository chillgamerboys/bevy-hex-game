//! Exact authored river links; physical liquid intervals remain unchanged.
use super::*;

/// Shared absolute longitudinal chart for revision-02's south-flowing river.
/// Every authored directed edge must strictly increase this projection.
pub const RIVER_PHASE_DIRECTION: [f32; 2] = [0.03, 0.9995499];

fn r02_progress(p: WorldHex) -> f64 {
    let [x, z] = world_xz(p);
    let [dx, dz] = RIVER_PHASE_DIRECTION;
    x * f64::from(dx) + z * f64::from(dz)
}

// The retained pre-r02 fixtures have their original southwest chart.
fn progress(p: WorldHex) -> f64 {
    let [x, z] = world_xz(p);
    x * -0.8 + z * 0.6
}

fn in_channel(p: WorldHex) -> bool {
    river_channel(p)
}

fn is_receiver(liquid: &LiquidColumn) -> bool {
    river_receiver(liquid.column, liquid.top)
}

impl GrandCompiler {
    pub(super) fn direct_river(&self, liquid: &mut LiquidColumn) {
        if self.geography.document.is_some() {
            self.r02_direct_river(liquid);
            return;
        }
        if !in_channel(liquid.column) || liquid.body_id != "grand/river" || liquid.top <= SEA_TOP {
            return;
        }
        // Targets are real neighboring intervals, not slope guessed by a renderer.
        // Strict chart progress is also a global acyclicity proof on flat reaches.
        let next = DIRS
            .into_iter()
            .map(|(q, r)| WorldHex::new(liquid.column.q + q, liquid.column.r + r))
            .filter(|p| progress(*p) > progress(liquid.column))
            .filter_map(|p| self.column(p).1)
            .filter(|next| next.top <= liquid.top)
            .filter(|next| in_channel(next.column) || is_receiver(next))
            .min_by(|a, b| {
                a.top
                    .cmp(&b.top)
                    .then_with(|| progress(b.column).total_cmp(&progress(a.column)))
            });
        if let Some(next) = next {
            liquid.kind = if liquid.top - next.top >= 8 {
                LiquidKind::Waterfall
            } else {
                LiquidKind::Directed
            };
            liquid.downstream = vec![VoxelPosition {
                column: next.column,
                level: next.top - 1,
            }];
        }
    }
}

impl GrandCompiler {
    fn r02_reach(&self, p: WorldHex) -> Option<usize> {
        let d = self.geography.document.as_ref()?;
        let point = self.geography.model_xz(p);
        [&d.fountain_rill, &d.falls, &d.river]
            .into_iter()
            .position(|c| geography::route_distance(point, &c.points).0 < c.width * 0.5 + 0.01)
    }
    fn r02_receiver(&self, reach: usize, liquid: &LiquidColumn) -> bool {
        let Some(d) = &self.geography.document else {
            return false;
        };
        if reach == 2 {
            return liquid.top == SEA_TOP;
        }
        let lake = if reach == 0 {
            &d.upper_lake
        } else {
            &d.lower_lake
        };
        liquid.top == self.geography.top_level(lake.level)
            && geography::irregular(
                self.geography.model_xz(liquid.column),
                lake.center,
                lake.radii,
                lake.phase,
            ) < 1.
    }
    /// Establish the complete drainage graph before publishing any chunks.
    /// Greedily choosing the lowest neighbor can strand a branch at a channel
    /// bend. Processing the monotone chart backwards admits only edges already
    /// known to reach the intended lake or sea; no phantom Standing sinks remain.
    pub(super) fn compile_r02_flow(
        &self,
    ) -> Result<BTreeMap<WorldHex, VoxelPosition>, ContractError> {
        let Some(d) = &self.geography.document else {
            return Ok(BTreeMap::new());
        };
        let mut out = BTreeMap::new();
        for (reach, channel) in [&d.fountain_rill, &d.falls, &d.river]
            .into_iter()
            .enumerate()
        {
            let mut cells = std::collections::BTreeSet::new();
            for pair in channel.points.windows(2) {
                let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
                    continue;
                };
                let pad = channel.width * 0.5 + 2.;
                let corners = [
                    [a[0].min(b[0]) - pad, a[2].min(b[2]) - pad],
                    [a[0].min(b[0]) - pad, a[2].max(b[2]) + pad],
                    [a[0].max(b[0]) + pad, a[2].min(b[2]) - pad],
                    [a[0].max(b[0]) + pad, a[2].max(b[2]) + pad],
                ]
                .map(|point| self.geography.world_hex(point));
                let q0 = corners.iter().map(|p| p.q).min().unwrap_or(0) - 2;
                let q1 = corners.iter().map(|p| p.q).max().unwrap_or(0) + 2;
                let r0 = corners.iter().map(|p| p.r).min().unwrap_or(0) - 2;
                let r1 = corners.iter().map(|p| p.r).max().unwrap_or(0) + 2;
                for q in q0..=q1 {
                    for r in r0..=r1 {
                        let p = WorldHex::new(q, r);
                        if self.r02_reach(p) == Some(reach) {
                            cells.insert(p);
                        }
                    }
                }
                if cells.len() > 100_000 {
                    return Err(ContractError::new(
                        "grand.flow",
                        "authored channel budget exceeded",
                    ));
                }
            }
            let mut water: BTreeMap<_, _> = cells
                .into_iter()
                .filter_map(|p| self.column(p).1.map(|liquid| (p, liquid)))
                .collect();
            // Include actual one-ring receivers even beyond the channel mask.
            let fringe: std::collections::BTreeSet<_> = water
                .keys()
                .flat_map(|p| {
                    DIRS.into_iter()
                        .map(|(q, r)| WorldHex::new(p.q + q, p.r + r))
                })
                .collect();
            for p in fringe {
                if !water.contains_key(&p) {
                    if let Some(liquid) = self.column(p).1.filter(|l| self.r02_receiver(reach, l)) {
                        water.insert(p, liquid);
                    }
                }
            }
            let mut ordered: Vec<_> = water.keys().copied().collect();
            ordered.sort_by(|a, b| r02_progress(*b).total_cmp(&r02_progress(*a)));
            let mut connected = std::collections::BTreeSet::new();
            for p in ordered {
                let Some(liquid) = water.get(&p) else {
                    continue;
                };
                if self.r02_receiver(reach, liquid) {
                    connected.insert(p);
                    continue;
                }
                let next = DIRS
                    .into_iter()
                    .map(|(q, r)| WorldHex::new(p.q + q, p.r + r))
                    .filter(|n| connected.contains(n) && r02_progress(*n) > r02_progress(p))
                    .filter_map(|n| water.get(&n))
                    .filter(|n| n.top <= liquid.top)
                    .min_by(|a, b| {
                        a.top
                            .cmp(&b.top)
                            .then_with(|| r02_progress(b.column).total_cmp(&r02_progress(a.column)))
                    });
                if let Some(next) = next {
                    out.insert(
                        p,
                        VoxelPosition {
                            column: next.column,
                            level: next.top - 1,
                        },
                    );
                    connected.insert(p);
                }
            }
            if let Some(liquid) = water.values().find(|l| !connected.contains(&l.column)) {
                return Err(ContractError::new(
                    "grand.flow",
                    format!(
                        "reach {reach} cannot drain from {:?}, model {:?}, surface {} to its receiver",
                        liquid.column,
                        self.geography.model_xz(liquid.column),
                        liquid.top
                    ),
                ));
            }
        }
        Ok(out)
    }
    fn r02_direct_river(&self, liquid: &mut LiquidColumn) {
        if liquid.body_id != "grand/river" {
            return;
        }
        if let Some(&next) = self.r02_flow.get(&liquid.column) {
            liquid.kind = if liquid.top - (next.level + 1) >= 8 {
                LiquidKind::Waterfall
            } else {
                LiquidKind::Directed
            };
            liquid.downstream = vec![next];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel_positions() -> impl Iterator<Item = WorldHex> {
        (-310..=320).flat_map(|r| {
            (-600..=600)
                .map(move |q| WorldHex::new(q, r))
                .filter(|p| in_channel(*p))
        })
    }

    #[test]
    #[expect(
        clippy::panic_in_result_fn,
        reason = "Assertions are test oracles; Result propagates only fixture and authoring errors."
    )]
    fn compiled_channel_chunks_publish_closed_downhill_paths_to_receiving_water()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut spec: GrandSpec = ron::from_str(include_str!(
            "../../../../../assets/config/v4/grand-v4/world.ron"
        ))?;
        spec.full_dressing = false;
        spec.geography = None; // Explicit legacy river fixture.
        let compiler = GrandCompiler::new(spec)?;
        let mut pending: std::collections::BTreeSet<_> =
            channel_positions().map(WorldHex::chunk).collect();
        let mut chunks = BTreeMap::new();
        while let Some(id) = pending.pop_first() {
            if chunks.contains_key(&id) {
                continue;
            }
            let chunk = compiler.chunk(id)?.ok_or("missing channel chunk")?;
            pending.extend(
                chunk
                    .semantics
                    .liquids
                    .iter()
                    .flat_map(|liquid| &liquid.downstream)
                    .map(|p| p.column.chunk()),
            );
            chunks.insert(id, chunk);
        }
        // Preserve the actual production chunks. A subset cannot claim the
        // manifest's entire mainland coverage, so check its liquid closure directly.
        let graph: BTreeMap<_, _> = chunks
            .values()
            .flat_map(|chunk| &chunk.semantics.liquids)
            .map(|liquid| (liquid.column, liquid))
            .collect();
        let mut complete = std::collections::BTreeSet::new();
        let mut directed = 0;
        for start in graph
            .values()
            .filter(|liquid| in_channel(liquid.column) && liquid.top > SEA_TOP)
        {
            assert_eq!(
                start.downstream.len(),
                1,
                "every channel cell must publish one outflow"
            );
            directed += 1;
            let mut position = start.column;
            let mut visited = std::collections::BTreeSet::new();
            loop {
                if complete.contains(&position) {
                    break;
                }
                assert!(
                    visited.insert(position),
                    "compiled liquid cycle at {position:?}"
                );
                let liquid = graph.get(&position).ok_or("missing emitted liquid")?;
                let Some(next) = liquid.downstream.first() else {
                    assert!(
                        is_receiver(liquid),
                        "compiled path ends at unintended receiver: {liquid:?}"
                    );
                    break;
                };
                let target = graph
                    .get(&next.column)
                    .ok_or("missing exact downstream target chunk")?;
                assert_eq!(target.top - 1, next.level);
                assert_eq!(position.checked_distance(next.column)?, 1);
                assert!(target.top <= liquid.top && progress(next.column) > progress(position));
                let target_column = chunks
                    .get(&next.column.chunk())
                    .and_then(|chunk| {
                        chunk
                            .columns
                            .iter()
                            .find(|column| column.position == next.column)
                    })
                    .ok_or("missing downstream geometry")?;
                assert_eq!(target_column.material_at(next.level), Some("water"));
                position = next.column;
            }
            complete.extend(visited);
        }
        assert!(directed > 5_000);
        assert!(chunks.len() < 1_000, "diagnostic closure stays bounded");
        Ok(())
    }

    #[test]
    #[expect(
        clippy::panic_in_result_fn,
        reason = "Assertions are test oracles; Result propagates only fixture and authoring errors."
    )]
    fn every_authored_channel_path_reaches_receiving_lake_or_ocean()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut spec: GrandSpec = ron::from_str(include_str!(
            "../../../../../assets/config/v4/grand-v4/world.ron"
        ))?;
        spec.full_dressing = false;
        spec.geography = None; // Explicit legacy river fixture.
        let compiler = GrandCompiler::new(spec)?;
        let mut graph = BTreeMap::new();
        for p in channel_positions() {
            let Some(mut liquid) = compiler.column(p).1 else {
                continue;
            };
            if liquid.top <= SEA_TOP {
                continue;
            }
            compiler.direct_river(&mut liquid);
            graph.insert(p, liquid);
        }
        assert!(
            graph.len() > 5_000,
            "the complete channel must be exercised"
        );
        let mut complete = std::collections::BTreeSet::new();
        for &start in graph.keys() {
            let mut position = start;
            let mut visited = std::collections::BTreeSet::new();
            while let Some(liquid) = graph.get(&position) {
                if complete.contains(&position) {
                    break;
                }
                assert!(
                    visited.insert(position),
                    "cycle from {start:?} at {position:?}"
                );
                position = liquid
                    .downstream
                    .first()
                    .ok_or_else(|| {
                        format!("channel path from {start:?} has a standing sink at {position:?}")
                    })?
                    .column;
            }
            if !complete.contains(&position) {
                let receiving = compiler.column(position).1.ok_or("dry receiver")?;
                let [x, z] = world_xz(position);
                assert!(
                    receiving.top == SEA_TOP
                        || (receiving.top == terrain::VALLEY_TOP
                            && ((x - 405.) / 90.).hypot((z + 115.) / 80.) < 1.2),
                    "path from {start:?} terminates outside receiving lake/ocean: {receiving:?}"
                );
            }
            complete.extend(visited);
        }
        assert_eq!(complete.len(), graph.len());
        Ok(())
    }
    #[test]
    #[expect(
        clippy::panic_in_result_fn,
        reason = "Assertions are test oracles; Result propagates only fixture and authoring errors."
    )]
    fn authored_channel_links_are_exact_downhill_and_increase_the_wave_chart()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut spec: GrandSpec = ron::from_str(include_str!(
            "../../../../../assets/config/v4/grand-v4/world.ron"
        ))?;
        spec.full_dressing = false;
        spec.geography = None; // Explicit legacy river fixture.
        let compiler = GrandCompiler::new(spec)?;
        let mut directed = 0;
        let mut falls = 0;
        for z in -449..465 {
            let z = f64::from(z);
            let x = if z < -140. {
                headwater_center(z)
            } else {
                river_center(z)
            };
            let p = nearest_hex(x, z);
            let Some(mut liquid) = compiler.column(p).1 else {
                continue;
            };
            let bounds = (liquid.bottom, liquid.top);
            compiler.direct_river(&mut liquid);
            assert_eq!(bounds, (liquid.bottom, liquid.top));
            for next in &liquid.downstream {
                let target = compiler
                    .column(next.column)
                    .1
                    .ok_or("missing downstream water")?;
                assert_eq!(next.level, target.top - 1);
                assert!(target.top <= liquid.top && progress(next.column) > progress(p));
                assert_eq!(p.checked_distance(next.column)?, 1);
                directed += 1;
                falls += usize::from(liquid.kind == LiquidKind::Waterfall);
            }
        }
        assert!(directed > 200 && falls >= 2);
        let mut lake = compiler.column(nearest_hex(330., -470.)).1.ok_or("lake")?;
        compiler.direct_river(&mut lake);
        assert_eq!(lake.kind, LiquidKind::Standing);
        Ok(())
    }
}
