//! Deterministic local coordinates for recipes with authored axial geometry.
//!
//! Most V3 recipes operate directly on arbitrary world-space masks. Waterfall and
//! Forest retain reviewed geometry expressed around axial origin, so they use this
//! narrow frame to preserve Single output while translating Ring7 patches.

use std::collections::BTreeSet;

use hex_core::{HexCoord, MapViewHint, TilePos};

use super::layout::LayoutKind;

/// Identity-preserving local frame for one resolved recipe patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LocalPatchFrame {
    center: HexCoord,
    scale: u32,
}

impl LocalPatchFrame {
    /// Resolves a stable frame from an exact connected patch mask.
    ///
    /// Single layouts deliberately keep axial origin and the configured grid radius
    /// so their established semantic fingerprints do not change. Composite patches
    /// use the mask medoid and cap reviewed local geometry at radius twelve.
    pub(crate) fn resolve(
        mask: &BTreeSet<HexCoord>,
        kind: LayoutKind,
        grid_radius: u32,
    ) -> Result<Self, String> {
        if mask.is_empty() {
            return Err("cannot frame an empty V3 patch mask".to_owned());
        }
        if kind == LayoutKind::Single {
            if !mask.contains(&HexCoord::ORIGIN) {
                return Err("Single V3 patch mask does not contain axial origin".to_owned());
            }
            return Ok(Self {
                center: HexCoord::ORIGIN,
                scale: grid_radius,
            });
        }

        let center = mask
            .iter()
            .copied()
            .min_by_key(|candidate| {
                let max_distance = mask
                    .iter()
                    .map(|coord| candidate.distance(*coord))
                    .max()
                    .unwrap_or_default();
                let total_distance = mask
                    .iter()
                    .map(|coord| u64::from(candidate.distance(*coord)))
                    .sum::<u64>();
                (max_distance, total_distance, *candidate)
            })
            .ok_or_else(|| "cannot frame an empty V3 patch mask".to_owned())?;
        let max_distance = mask
            .iter()
            .map(|coord| center.distance(*coord))
            .max()
            .unwrap_or_default();
        Ok(Self {
            center,
            scale: max_distance.min(12),
        })
    }

    /// Stable world-space center selected for this patch.
    #[must_use]
    pub(crate) const fn center(self) -> HexCoord {
        self.center
    }

    /// Effective local radius used by reviewed recipe geometry.
    #[must_use]
    pub(crate) const fn scale(self) -> u32 {
        self.scale
    }

    /// Converts one world-space coordinate to this recipe's local frame.
    pub(crate) fn to_local(self, coord: HexCoord) -> Result<HexCoord, String> {
        checked_coord_difference(coord, self.center)
    }

    /// Converts one local coordinate to exact world-space ownership.
    pub(crate) fn to_world(self, coord: HexCoord) -> Result<HexCoord, String> {
        checked_coord_sum(coord, self.center)
    }

    /// Converts one exact local voxel position to world space.
    pub(crate) fn position_to_world(self, position: TilePos) -> Result<TilePos, String> {
        Ok(TilePos::new(self.to_world(position.coord)?, position.level))
    }

    /// Converts the complete exact patch mask into local coordinates.
    pub(crate) fn local_mask(
        self,
        mask: &BTreeSet<HexCoord>,
    ) -> Result<BTreeSet<HexCoord>, String> {
        mask.iter()
            .copied()
            .map(|coord| self.to_local(coord))
            .collect()
    }

    /// Moves a locally authored camera frame over the resolved patch.
    #[must_use]
    pub(crate) fn view_hint_to_world(self, hint: MapViewHint) -> MapViewHint {
        let offset = self.center.to_world(0.0);
        MapViewHint::new(
            (hint.eye.0 + offset.x, hint.eye.1, hint.eye.2 + offset.z),
            (
                hint.focus.0 + offset.x,
                hint.focus.1,
                hint.focus.2 + offset.z,
            ),
        )
    }
}

fn checked_coord_sum(first: HexCoord, second: HexCoord) -> Result<HexCoord, String> {
    checked_coord_zip(first, second, i32::checked_add, "addition")
}

fn checked_coord_difference(first: HexCoord, second: HexCoord) -> Result<HexCoord, String> {
    checked_coord_zip(first, second, i32::checked_sub, "subtraction")
}

fn checked_coord_zip(
    first: HexCoord,
    second: HexCoord,
    operation: fn(i32, i32) -> Option<i32>,
    operation_name: &str,
) -> Result<HexCoord, String> {
    let [first_x, first_y, first_z] = first.to_cubic_array();
    let [second_x, second_y, second_z] = second.to_cubic_array();
    let x = operation(first_x, second_x)
        .ok_or_else(|| format!("V3 local-frame {operation_name} overflowed x"))?;
    let y = operation(first_y, second_y)
        .ok_or_else(|| format!("V3 local-frame {operation_name} overflowed y"))?;
    let z = operation(first_z, second_z)
        .ok_or_else(|| format!("V3 local-frame {operation_name} overflowed z"))?;
    HexCoord::try_new_cubic(x, y, z)
        .ok_or_else(|| format!("V3 local-frame {operation_name} broke cube coordinates"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_frame_is_an_exact_identity_at_every_supported_radius() {
        for radius in [12, 20, 40] {
            let mask = HexCoord::ORIGIN.within_radius(radius).into_iter().collect();
            let frame = LocalPatchFrame::resolve(&mask, LayoutKind::Single, radius)
                .expect("whole-world Single mask should frame");
            assert_eq!(frame.center(), HexCoord::ORIGIN);
            assert_eq!(frame.scale(), radius);
            assert_eq!(frame.local_mask(&mask), Ok(mask));
        }
    }

    #[test]
    fn translated_composite_mask_round_trips_exactly() {
        let translation = HexCoord::from_axial(21, 0);
        let local: BTreeSet<_> = HexCoord::ORIGIN.within_radius(12).into_iter().collect();
        let world: BTreeSet<_> = local
            .iter()
            .copied()
            .map(|coord| checked_coord_sum(coord, translation).expect("small translation"))
            .collect();
        let frame = LocalPatchFrame::resolve(&world, LayoutKind::Ring7, 33)
            .expect("translated composite mask should frame");

        assert_eq!(frame.center(), translation);
        assert_eq!(frame.scale(), 12);
        assert_eq!(frame.local_mask(&world), Ok(local));
        for coord in world {
            assert_eq!(
                frame
                    .to_local(coord)
                    .and_then(|local| frame.to_world(local)),
                Ok(coord)
            );
        }
    }

    #[test]
    fn composite_medoid_and_camera_translation_are_deterministic() {
        let mask = BTreeSet::from([
            HexCoord::from_axial(4, -1),
            HexCoord::from_axial(5, -1),
            HexCoord::from_axial(4, 0),
            HexCoord::from_axial(5, 0),
        ]);
        let first =
            LocalPatchFrame::resolve(&mask, LayoutKind::Ring7, 33).expect("small connected mask");
        let second =
            LocalPatchFrame::resolve(&mask, LayoutKind::Ring7, 33).expect("same connected mask");
        assert_eq!(first, second);

        let local = MapViewHint::new((3.0, 8.0, 2.0), (0.0, 4.0, 0.0));
        let offset = first.center().to_world(0.0);
        let translated = first.view_hint_to_world(local);
        assert_eq!(translated.focus, (offset.x, local.focus.1, offset.z),);
        assert_eq!(
            translated.eye,
            (local.eye.0 + offset.x, local.eye.1, local.eye.2 + offset.z),
        );
    }
}
