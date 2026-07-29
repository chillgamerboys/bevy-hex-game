//! Deterministic local coordinates for recipes with authored axial geometry.
//!
//! Most V3 recipes operate directly on arbitrary world-space masks. Waterfall and
//! Forest retain reviewed geometry expressed around axial origin, so they use this
//! narrow frame to preserve Single output while translating Ring7 patches.

use std::collections::{BTreeMap, BTreeSet};

use hex_core::{BiomeRegionId, HexCoord, MapViewHint, TilePos};

use super::composition::GeneratedPatchPlan;
use super::layout::{
    HexSide, LayoutKind, PatchId, ResolvedEdgeReference, ResolvedLayoutPlan, ResolvedPatch,
};
use super::liquid::LiquidPlan;
use super::volume::VolumePlan;
use super::world::{FeaturePlan, InteriorPlan, LightId, PlannedGameplayLight, StructurePlan};
use super::world::{GeneratedWorldPlan, ProtectedFeatureRoute};

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

    /// Converts one exact world-space voxel position to local recipe coordinates.
    pub(crate) fn position_to_local(self, position: TilePos) -> Result<TilePos, String> {
        Ok(TilePos::new(self.to_local(position.coord)?, position.level))
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

    /// Converts a complete local surface-level plan to exact world coordinates.
    pub(crate) fn levels_to_world(
        self,
        levels: BTreeMap<HexCoord, i32>,
    ) -> Result<BTreeMap<HexCoord, i32>, String> {
        self.translate_coord_map(levels, FrameDirection::ToWorld)
    }

    /// Converts a complete world-space surface-level plan to local coordinates.
    pub(crate) fn levels_to_local(
        self,
        levels: BTreeMap<HexCoord, i32>,
    ) -> Result<BTreeMap<HexCoord, i32>, String> {
        self.translate_coord_map(levels, FrameDirection::ToLocal)
    }

    /// Moves a locally authored camera frame over the resolved patch.
    #[must_use]
    pub(crate) fn view_hint_to_world(self, hint: MapViewHint) -> MapViewHint {
        let offset = self.center().to_world(0.0);
        MapViewHint::new(
            (hint.eye.0 + offset.x, hint.eye.1, hint.eye.2 + offset.z),
            (
                hint.focus.0 + offset.x,
                hint.focus.1,
                hint.focus.2 + offset.z,
            ),
        )
    }

    /// Converts every exact semantic position in one local patch to world space.
    pub(crate) fn patch_to_world(self, plan: &mut GeneratedPatchPlan) -> Result<(), String> {
        self.translate_semantics(
            FrameDirection::ToWorld,
            &mut plan.volume,
            &mut plan.liquids,
            &mut plan.features,
            &mut plan.structures,
            &mut plan.blockers,
            &mut plan.lights,
            &mut plan.biome_regions,
            &mut plan.interiors,
            &mut plan.anchors,
            &mut plan.view_hint,
        )
    }

    /// Creates a canonical Single-layout copy for recipe-specific validation.
    ///
    /// Ring7 owns world coordinates, patch identities, and biome identities. Recipe
    /// validators deliberately reason in the same radius-limited local frame as
    /// their approved Single output, so this projection normalizes all three before
    /// invoking those validators.
    pub(crate) fn canonical_local_world(
        self,
        plan: &GeneratedPatchPlan,
    ) -> Result<GeneratedWorldPlan, String> {
        let mut volume = plan.volume.clone();
        let mut liquids = plan.liquids.clone();
        let mut features = plan.features.clone();
        let mut structures = plan.structures.clone();
        let mut blockers = plan.blockers.clone();
        let mut lights = plan.lights.clone();
        let mut biome_regions = plan.biome_regions.clone();
        let mut interiors = plan.interiors.clone();
        let mut anchors = plan.anchors.clone();
        let mut view_hint = plan.view_hint;
        self.translate_semantics(
            FrameDirection::ToLocal,
            &mut volume,
            &mut liquids,
            &mut features,
            &mut structures,
            &mut blockers,
            &mut lights,
            &mut biome_regions,
            &mut interiors,
            &mut anchors,
            &mut view_hint,
        )?;
        for region in biome_regions.values_mut() {
            *region = BiomeRegionId(0);
        }
        let mask = volume.mask.clone();
        let edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect();
        let layout = ResolvedLayoutPlan {
            kind: LayoutKind::Single,
            grid_radius: self.scale,
            footprint: mask.clone(),
            patches: BTreeMap::from([(
                PatchId(0),
                ResolvedPatch {
                    biome_region: BiomeRegionId(0),
                    mask,
                    edges,
                },
            )]),
            shared_edges: BTreeMap::new(),
        };
        Ok(GeneratedWorldPlan {
            layout,
            volume,
            liquids,
            features,
            structures,
            blockers,
            lights,
            biome_regions,
            interiors,
            anchors,
            view_hint,
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "translation must cover every exact semantic position atomically"
    )]
    fn translate_semantics(
        self,
        direction: FrameDirection,
        volume: &mut VolumePlan,
        liquids: &mut LiquidPlan,
        features: &mut FeaturePlan,
        structures: &mut StructurePlan,
        blockers: &mut BTreeSet<TilePos>,
        lights: &mut BTreeMap<LightId, PlannedGameplayLight>,
        biome_regions: &mut BTreeMap<TilePos, hex_core::BiomeRegionId>,
        interiors: &mut InteriorPlan,
        anchors: &mut BTreeMap<String, TilePos>,
        view_hint: &mut MapViewHint,
    ) -> Result<(), String> {
        volume.mask = self.translate_coords(&volume.mask, direction)?;
        volume.columns =
            self.translate_coord_map(std::mem::take(&mut volume.columns), direction)?;
        volume.surfaces =
            self.translate_position_map(std::mem::take(&mut volume.surfaces), direction)?;

        for body in liquids.bodies.values_mut() {
            let mut nodes = BTreeMap::new();
            for (position, mut node) in std::mem::take(&mut body.nodes) {
                if let Some(downstream) = node.downstream {
                    node.downstream = Some(self.translate_position(downstream, direction)?);
                }
                nodes.insert(self.translate_position(position, direction)?, node);
            }
            body.nodes = nodes;
        }
        for feature in features.by_id.values_mut() {
            feature.root = self.translate_position(feature.root, direction)?;
            feature.blocker_footprint =
                self.translate_positions(&feature.blocker_footprint, direction)?;
        }
        for route in features.protected_routes.values_mut() {
            *route = ProtectedFeatureRoute {
                centerline: route
                    .centerline
                    .iter()
                    .copied()
                    .map(|position| self.translate_position(position, direction))
                    .collect::<Result<_, _>>()?,
                surfaces: self.translate_positions(&route.surfaces, direction)?,
            };
        }
        for clearing in features.clearings.values_mut() {
            clearing.surfaces = self.translate_positions(&clearing.surfaces, direction)?;
        }
        for structure in structures.by_id.values_mut() {
            structure.voxels = self.translate_positions(&structure.voxels, direction)?;
        }
        *blockers = self.translate_positions(blockers, direction)?;
        for light in lights.values_mut() {
            light.origin = self.translate_position(light.origin, direction)?;
        }
        *biome_regions = self.translate_position_map(std::mem::take(biome_regions), direction)?;
        for interior in interiors.by_id.values_mut() {
            interior.floors = self.translate_positions(&interior.floors, direction)?;
            interior.entrances = self.translate_positions(&interior.entrances, direction)?;
            interior.roof_voxels = self.translate_positions(&interior.roof_voxels, direction)?;
        }
        for position in anchors.values_mut() {
            *position = self.translate_position(*position, direction)?;
        }
        *view_hint = self.translate_view_hint(*view_hint, direction);
        Ok(())
    }

    fn translate_coords(
        self,
        coords: &BTreeSet<HexCoord>,
        direction: FrameDirection,
    ) -> Result<BTreeSet<HexCoord>, String> {
        coords
            .iter()
            .copied()
            .map(|coord| self.translate_coord(coord, direction))
            .collect()
    }

    fn translate_positions(
        self,
        positions: &BTreeSet<TilePos>,
        direction: FrameDirection,
    ) -> Result<BTreeSet<TilePos>, String> {
        positions
            .iter()
            .copied()
            .map(|position| self.translate_position(position, direction))
            .collect()
    }

    fn translate_coord_map<T>(
        self,
        values: BTreeMap<HexCoord, T>,
        direction: FrameDirection,
    ) -> Result<BTreeMap<HexCoord, T>, String> {
        values
            .into_iter()
            .map(|(coord, value)| Ok((self.translate_coord(coord, direction)?, value)))
            .collect()
    }

    fn translate_position_map<T>(
        self,
        values: BTreeMap<TilePos, T>,
        direction: FrameDirection,
    ) -> Result<BTreeMap<TilePos, T>, String> {
        values
            .into_iter()
            .map(|(position, value)| Ok((self.translate_position(position, direction)?, value)))
            .collect()
    }

    fn translate_coord(
        self,
        coord: HexCoord,
        direction: FrameDirection,
    ) -> Result<HexCoord, String> {
        match direction {
            FrameDirection::ToLocal => self.to_local(coord),
            FrameDirection::ToWorld => self.to_world(coord),
        }
    }

    fn translate_position(
        self,
        position: TilePos,
        direction: FrameDirection,
    ) -> Result<TilePos, String> {
        match direction {
            FrameDirection::ToLocal => self.position_to_local(position),
            FrameDirection::ToWorld => self.position_to_world(position),
        }
    }

    fn translate_view_hint(self, hint: MapViewHint, direction: FrameDirection) -> MapViewHint {
        match direction {
            FrameDirection::ToWorld => self.view_hint_to_world(hint),
            FrameDirection::ToLocal => {
                let offset = self.center.to_world(0.0);
                MapViewHint::new(
                    (hint.eye.0 - offset.x, hint.eye.1, hint.eye.2 - offset.z),
                    (
                        hint.focus.0 - offset.x,
                        hint.focus.1,
                        hint.focus.2 - offset.z,
                    ),
                )
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum FrameDirection {
    ToLocal,
    ToWorld,
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
