use super::OceanBathymetry;

impl OceanBathymetry {
    /// Caches a bounded two-pass shore-distance approximation. Nearby coast and
    /// narrow bays reduce swells even where the local bed is deep. Recompute on
    /// package replacement, never each frame or after carving (no new flooding).
    ///
    /// # Errors
    /// Returns an error for invalid bathymetry, a nonfinite sea level or a
    /// nonpositive/nonfinite shelter range.
    pub fn with_shore_shelter(mut self, sea: f32, range: f32) -> Result<Self, &'static str> {
        if !self.is_valid() || !sea.is_finite() || !range.is_finite() || range <= 0.0 {
            return Err("Shelter requires valid bathymetry, sea level and positive range");
        }
        let width = usize::try_from(self.width).map_err(|_| "Invalid shelter width")?;
        let height = usize::try_from(self.height).map_err(|_| "Invalid shelter height")?;
        let mut distances: Vec<f32> = self
            .bed_heights
            .iter()
            .map(|bed| if *bed >= sea { 0.0 } else { range })
            .collect();
        let diagonal = self.spacing * std::f32::consts::SQRT_2;
        for z in 0..height {
            for x in 0..width {
                let at = z * width + x;
                if x > 0 {
                    distances[at] = distances[at].min(distances[at - 1] + self.spacing);
                }
                if z > 0 {
                    distances[at] = distances[at].min(distances[at - width] + self.spacing);
                    if x > 0 {
                        distances[at] = distances[at].min(distances[at - width - 1] + diagonal);
                    }
                    if x + 1 < width {
                        distances[at] = distances[at].min(distances[at - width + 1] + diagonal);
                    }
                }
            }
        }
        for z in (0..height).rev() {
            for x in (0..width).rev() {
                let at = z * width + x;
                if x + 1 < width {
                    distances[at] = distances[at].min(distances[at + 1] + self.spacing);
                }
                if z + 1 < height {
                    distances[at] = distances[at].min(distances[at + width] + self.spacing);
                    if x > 0 {
                        distances[at] = distances[at].min(distances[at + width - 1] + diagonal);
                    }
                    if x + 1 < width {
                        distances[at] = distances[at].min(distances[at + width + 1] + diagonal);
                    }
                }
            }
        }
        self.shore_shelter = distances
            .into_iter()
            .map(|distance| {
                let t = (distance / range).clamp(0.0, 1.0);
                0.25 + 0.75 * t * t * (3.0 - 2.0 * t)
            })
            .collect();
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deep_water_near_land_is_sheltered_without_changing_bathymetry() {
        let mut bed = OceanBathymetry {
            width: 7,
            height: 3,
            spacing: 10.0,
            bed_heights: vec![-80.0; 21],
            ..Default::default()
        };
        for z in 0..3 {
            bed.bed_heights[z * 7] = 5.0;
        }
        let original = bed.bed_heights.clone();
        let bed = bed.with_shore_shelter(0.0, 40.0).unwrap();
        assert_eq!(bed.bed_heights, original);
        assert!(bed.shore_shelter[8] < 0.4);
        assert!(bed.shore_shelter[9] < bed.shore_shelter[10]);
        assert!((bed.shore_shelter[11] - 1.0).abs() < 0.00001);
        assert!(bed.is_valid());
    }
}
