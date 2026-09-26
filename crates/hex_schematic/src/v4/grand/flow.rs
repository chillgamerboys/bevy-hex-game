//! Exact authored river links; physical liquid intervals remain unchanged.
use super::*;

/// Shared absolute longitudinal chart for Grand's southwest-flowing river.
/// Every authored directed edge must strictly increase this projection.
pub const RIVER_PHASE_DIRECTION: [f32; 2] = [-0.8, 0.6];

fn progress(p: WorldHex) -> f64 {
    let [x, z] = world_xz(p);
    let [dx, dz] = RIVER_PHASE_DIRECTION;
    x * f64::from(dx) + z * f64::from(dz)
}

impl GrandCompiler {
    pub(super) fn direct_river(&self, liquid: &mut LiquidColumn) {
        let [x, z] = world_xz(liquid.column);
        let channel = if (-450. ..=-140.).contains(&z) {
            (x - headwater_center(z)).abs() < 13. + 2.5 * ((z + 450.) / 37.).sin()
        } else if (-65. ..=465.).contains(&z) {
            let t = ((z + 60.) / 525.).clamp(0., 1.);
            (x - river_center(z)).abs() < 10. + 12. * t + 3. * (t * std::f64::consts::PI * 5.).sin()
        } else {
            false
        };
        if !channel || liquid.body_id != "grand/river" || liquid.top <= SEA_TOP {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authored_channel_links_are_exact_downhill_and_increase_the_wave_chart(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut spec: GrandSpec = ron::from_str(include_str!(
            "../../../../../assets/config/v4/grand-v4/world.ron"
        ))?;
        spec.full_dressing = false;
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
