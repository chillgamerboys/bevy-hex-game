//! Deterministic local coordinates for recipes with authored axial geometry.
//!
//! Most V3 recipes operate directly on arbitrary world-space masks. Waterfall and
//! Forest retain reviewed geometry expressed around axial origin, so they use this
//! narrow frame to preserve Single output while translating composite patches.

use std::collections::{BTreeMap, BTreeSet};

use hex_assets::HexObjectRotation;
use hex_core::{BiomeRegionId, HexCoord, MapViewHint, TilePos};

use super::composition::GeneratedPatchPlan;
use super::layout::{
    HexSide, LayoutKind, PatchId, ResolvedEdgeReference, ResolvedLayoutPlan, ResolvedPatch,
};
use super::liquid::LiquidPlan;
use super::volume::VolumePlan;
use super::world::{
    FeaturePlan, InteriorPlan, LightId, PlannedGameplayLight, PlannedLightPresentation,
    StructurePlan,
};
use super::world::{GeneratedWorldPlan, ProtectedFeatureRoute};

/// Identity-preserving local frame for one resolved recipe patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LocalPatchFrame {
    center: HexCoord,
    scale: u32,
    rotation: u8,
    compose_presentation_rotation: bool,
}

#[derive(Debug)]
struct AxisDistanceIndex {
    values: Vec<i64>,
    prefix_sums: Vec<i128>,
}

impl AxisDistanceIndex {
    fn new(values: impl IntoIterator<Item = i32>) -> Self {
        let mut values = values.into_iter().map(i64::from).collect::<Vec<_>>();
        values.sort_unstable();
        let mut prefix_sums = Vec::<i128>::with_capacity(values.len().saturating_add(1));
        prefix_sums.push(0);
        for value in &values {
            let next = prefix_sums.last().copied().unwrap_or_default() + i128::from(*value);
            prefix_sums.push(next);
        }
        Self {
            values,
            prefix_sums,
        }
    }

    fn maximum_difference(&self, value: i32) -> u32 {
        let value = i64::from(value);
        let Some(first) = self.values.first().copied() else {
            return 0;
        };
        let last = self.values.last().copied().unwrap_or(first);
        u32::try_from((value - first).abs().max((last - value).abs())).unwrap_or(u32::MAX)
    }

    fn total_difference(&self, value: i32) -> i128 {
        let value = i64::from(value);
        let split = self.values.partition_point(|candidate| *candidate <= value);
        let split_i128 = split as i128;
        let len_i128 = self.values.len() as i128;
        let value_i128 = i128::from(value);
        let left_sum = self.prefix_sums.get(split).copied().unwrap_or_default();
        let total_sum = self.prefix_sums.last().copied().unwrap_or_default();
        let left = value_i128 * split_i128 - left_sum;
        let right = total_sum - left_sum - value_i128 * (len_i128 - split_i128);
        left + right
    }
}

fn exact_mask_medoid(mask: &BTreeSet<HexCoord>) -> Option<HexCoord> {
    let axes: [AxisDistanceIndex; 3] = std::array::from_fn(|axis| {
        AxisDistanceIndex::new(mask.iter().map(|coord| {
            coord
                .to_cubic_array()
                .get(axis)
                .copied()
                .unwrap_or_default()
        }))
    });
    mask.iter().copied().min_by_key(|candidate| {
        let cube = candidate.to_cubic_array();
        let max_distance = axes
            .iter()
            .zip(cube)
            .map(|(axis, value)| axis.maximum_difference(value))
            .max()
            .unwrap_or_default();
        // Cube coordinates satisfy dx + dy + dz = 0, so their L1 distance is
        // exactly twice the hex distance. Three sorted axis-prefix indexes make
        // the exact total-distance tie-breaker O(log n) per candidate instead of
        // scanning the complete patch mask again.
        let doubled_total = axes
            .iter()
            .zip(cube)
            .map(|(axis, value)| axis.total_difference(value))
            .sum::<i128>();
        (max_distance, doubled_total, *candidate)
    })
}

impl LocalPatchFrame {
    pub(crate) const fn from_resolved_ring19(center: HexCoord, scale: u32, rotation: u8) -> Self {
        Self {
            center,
            scale,
            rotation: rotation % 6,
            compose_presentation_rotation: true,
        }
    }

    /// Resolves a frame whose local recipe axes are rotated into world space.
    ///
    /// Single layouts deliberately keep axial origin and the configured grid radius
    /// so their established semantic fingerprints do not change. Legacy composite
    /// patches use the mask medoid and cap reviewed local geometry at radius twelve.
    pub(crate) fn resolve_rotated(
        mask: &BTreeSet<HexCoord>,
        kind: LayoutKind,
        grid_radius: u32,
        rotation: u8,
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
                rotation: rotation % 6,
                compose_presentation_rotation: false,
            });
        }

        let center = exact_mask_medoid(mask)
            .ok_or_else(|| "cannot frame an empty V3 patch mask".to_owned())?;
        let max_distance = mask
            .iter()
            .map(|coord| center.distance(*coord))
            .max()
            .unwrap_or_default();
        Ok(Self {
            center,
            scale: max_distance.min(12),
            rotation: rotation % 6,
            compose_presentation_rotation: matches!(kind, LayoutKind::Ring19 | LayoutKind::Macro),
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
            .map(|coord| rotate(coord, (6_u8.saturating_sub(self.rotation)) % 6))
    }

    /// Converts one local coordinate to exact world-space ownership.
    pub(crate) fn to_world(self, coord: HexCoord) -> Result<HexCoord, String> {
        checked_coord_sum(rotate(coord, self.rotation), self.center)
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
        self.view_hint_rotated_to_world(hint, 0)
    }

    /// Moves and additionally rotates a recipe-selected camera frame into world space.
    #[must_use]
    pub(crate) fn view_hint_rotated_to_world(
        self,
        hint: MapViewHint,
        additional_rotation: u8,
    ) -> MapViewHint {
        let offset = self.center().to_world(0.0);
        let rotation = self.rotation.saturating_add(additional_rotation) % 6;
        let eye = rotate_world_point(hint.eye, rotation);
        let focus = rotate_world_point(hint.focus, rotation);
        MapViewHint::new(
            (eye.0 + offset.x, eye.1, eye.2 + offset.z),
            (focus.0 + offset.x, focus.1, focus.2 + offset.z),
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
    /// Composite layouts own world coordinates, patch identities, and biome
    /// identities. Recipe validators deliberately reason in the same radius-limited
    /// local frame as their approved Single output, so this projection normalizes all
    /// three before invoking those validators.
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
                    rotation_turns: 0,
                    mask,
                    edges,
                },
            )]),
            shared_edges: BTreeMap::new(),
            boundary_liquid_outlets: BTreeMap::new(),
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
            if self.compose_presentation_rotation {
                feature.rotation = compose_object_rotation(feature.rotation, self, direction)?;
            }
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
            if self.compose_presentation_rotation {
                if let Some(presentation) = &mut light.presentation {
                    match presentation {
                        PlannedLightPresentation::CaveCrystal(crystal) => {
                            crystal.rotation =
                                compose_rotation_steps(crystal.rotation, self.rotation, direction);
                        }
                        PlannedLightPresentation::CrystalAscent(crystal) => {
                            crystal.rotation =
                                compose_rotation_steps(crystal.rotation, self.rotation, direction);
                        }
                    }
                }
            }
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
                let translated = MapViewHint::new(
                    (hint.eye.0 - offset.x, hint.eye.1, hint.eye.2 - offset.z),
                    (
                        hint.focus.0 - offset.x,
                        hint.focus.1,
                        hint.focus.2 - offset.z,
                    ),
                );
                let inverse = (6_u8.saturating_sub(self.rotation)) % 6;
                MapViewHint::new(
                    rotate_world_point(translated.eye, inverse),
                    rotate_world_point(translated.focus, inverse),
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

fn compose_object_rotation(
    rotation: HexObjectRotation,
    frame: LocalPatchFrame,
    direction: FrameDirection,
) -> Result<HexObjectRotation, String> {
    HexObjectRotation::new(compose_rotation_steps(
        rotation.steps(),
        frame.rotation,
        direction,
    ))
    .map_err(|error| format!("translated feature rotation is invalid: {error}"))
}

const fn compose_rotation_steps(current: u8, frame: u8, direction: FrameDirection) -> u8 {
    let delta = match direction {
        FrameDirection::ToLocal => (6_u8 - (frame % 6)) % 6,
        FrameDirection::ToWorld => frame % 6,
    };
    (current + delta) % 6
}

fn rotate(coord: HexCoord, turns: u8) -> HexCoord {
    let [mut x, mut y, mut z] = coord.to_cubic_array();
    for _ in 0..turns % 6 {
        (x, y, z) = (-z, -x, -y);
    }
    HexCoord::new_cubic(x, y, z)
}

fn rotate_world_point(point: (f32, f32, f32), turns: u8) -> (f32, f32, f32) {
    let turns = turns % 6;
    if turns == 0 {
        return point;
    }
    if turns == 3 {
        return (-point.0, point.1, -point.2);
    }
    let angle = -f32::from(turns) * std::f32::consts::FRAC_PI_3;
    let (sin, cos) = angle.sin_cos();
    (
        point.0.mul_add(cos, -point.2 * sin),
        point.1,
        point.0.mul_add(sin, point.2 * cos),
    )
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
    use hex_assets::ObjectAssetId;
    use hex_core::IlluminationLevel;

    use super::super::world::{
        CaveCrystalKind, CaveCrystalPresentation, CaveCrystalSiteKind, FeatureId, FeatureKind,
        PlannedFeature,
    };
    use super::*;

    fn quadratic_mask_medoid(mask: &BTreeSet<HexCoord>) -> Option<HexCoord> {
        mask.iter().copied().min_by_key(|candidate| {
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
    }

    fn presentation_fixture() -> GeneratedPatchPlan {
        let feature_root = TilePos::new(HexCoord::from_axial(1, 0), 3);
        GeneratedPatchPlan {
            patch_id: PatchId(0),
            volume: VolumePlan::new(BTreeSet::from([HexCoord::ORIGIN, feature_root.coord])),
            liquids: LiquidPlan::default(),
            features: FeaturePlan {
                by_id: BTreeMap::from([(
                    FeatureId(0),
                    PlannedFeature {
                        root: feature_root,
                        kind: FeatureKind::Tree,
                        object_id: ObjectAssetId::new("plant/small-broadleaf")
                            .expect("fixture object id"),
                        rotation: HexObjectRotation::new(2).expect("fixture rotation"),
                        blocker_footprint: BTreeSet::from([feature_root]),
                    },
                )]),
                ..FeaturePlan::default()
            },
            structures: StructurePlan::default(),
            blockers: BTreeSet::from([feature_root]),
            lights: BTreeMap::from([(
                LightId(0),
                PlannedGameplayLight {
                    origin: TilePos::new(HexCoord::ORIGIN, 3),
                    level: IlluminationLevel::Bright,
                    radius: 4,
                    presentation: Some(PlannedLightPresentation::CaveCrystal(
                        CaveCrystalPresentation {
                            kind: CaveCrystalKind::Branched,
                            site: CaveCrystalSiteKind::InteriorAlcove,
                            rotation: 1,
                        },
                    )),
                },
            )]),
            biome_regions: BTreeMap::new(),
            interiors: InteriorPlan::default(),
            anchors: BTreeMap::new(),
            view_hint: MapViewHint::new((4.0, 8.0, 2.0), (0.0, 3.0, 0.0)),
        }
    }

    #[test]
    fn single_frame_is_an_exact_identity_at_every_supported_radius() {
        for radius in [12, 20, 40] {
            let mask = HexCoord::ORIGIN.within_radius(radius).into_iter().collect();
            let frame = LocalPatchFrame::resolve_rotated(&mask, LayoutKind::Single, radius, 0)
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
        let frame = LocalPatchFrame::resolve_rotated(&world, LayoutKind::Ring7, 33, 0)
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
    fn rotated_composite_frame_round_trips_geometry_and_view() {
        let translation = HexCoord::from_axial(21, 0);
        let local: BTreeSet<_> = HexCoord::ORIGIN.within_radius(12).into_iter().collect();
        let world: BTreeSet<_> = local
            .iter()
            .copied()
            .map(|coord| {
                checked_coord_sum(rotate(coord, 3), translation).expect("small translation")
            })
            .collect();
        let frame = LocalPatchFrame::resolve_rotated(&world, LayoutKind::Ring7, 33, 3)
            .expect("rotated composite mask should frame");
        let hint = MapViewHint::new((12.0, 20.0, -5.0), (3.0, 6.0, 2.0));

        assert_eq!(frame.local_mask(&world), Ok(local));
        for coord in world {
            assert_eq!(
                frame
                    .to_local(coord)
                    .and_then(|local| frame.to_world(local)),
                Ok(coord)
            );
        }
        let translated = frame.view_hint_to_world(hint);
        assert_eq!(
            frame.translate_view_hint(translated, FrameDirection::ToLocal),
            hint
        );
    }

    #[test]
    fn composite_medoid_and_camera_translation_are_deterministic() {
        let mask = BTreeSet::from([
            HexCoord::from_axial(4, -1),
            HexCoord::from_axial(5, -1),
            HexCoord::from_axial(4, 0),
            HexCoord::from_axial(5, 0),
        ]);
        let first = LocalPatchFrame::resolve_rotated(&mask, LayoutKind::Ring7, 33, 0)
            .expect("small connected mask");
        let second = LocalPatchFrame::resolve_rotated(&mask, LayoutKind::Ring7, 33, 0)
            .expect("same connected mask");
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

    #[test]
    fn indexed_composite_medoid_exactly_matches_the_quadratic_contract() {
        let mut masks = (0..=8)
            .map(|radius| {
                HexCoord::ORIGIN
                    .within_radius(radius)
                    .into_iter()
                    .filter(|coord| {
                        let [x, y, _] = coord.to_cubic_array();
                        radius < 3 || (x + 2 * y).rem_euclid(5) != 0
                    })
                    .collect::<BTreeSet<_>>()
            })
            .collect::<Vec<_>>();
        let irregular = BTreeSet::from([
            HexCoord::from_axial(-9, 4),
            HexCoord::from_axial(-8, 4),
            HexCoord::from_axial(-4, 1),
            HexCoord::from_axial(0, 0),
            HexCoord::from_axial(1, -1),
            HexCoord::from_axial(7, -3),
        ]);
        masks.push(irregular);
        let translation = HexCoord::from_axial(1_000_000, -900_000);
        masks.push(
            HexCoord::ORIGIN
                .within_radius(12)
                .into_iter()
                .filter(|coord| {
                    let [x, _, _] = coord.to_cubic_array();
                    x <= 7
                })
                .map(|coord| checked_coord_sum(coord, translation).expect("bounded translation"))
                .collect(),
        );

        for mask in masks {
            assert_eq!(exact_mask_medoid(&mask), quadratic_mask_medoid(&mask));
        }
    }

    #[test]
    fn recipe_camera_rotation_composes_with_the_patch_frame() {
        let translation = HexCoord::from_axial(21, 0);
        let local: BTreeSet<_> = HexCoord::ORIGIN.within_radius(12).into_iter().collect();
        let world = local
            .iter()
            .copied()
            .map(|coord| checked_coord_sum(coord, translation).expect("small translation"))
            .collect();
        let frame = LocalPatchFrame::resolve_rotated(&world, LayoutKind::Ring7, 33, 0)
            .expect("translated composite mask should frame");
        let hint = MapViewHint::new((12.0, 20.0, -5.0), (3.0, 6.0, 2.0));
        let locally_rotated = MapViewHint::new(
            rotate_world_point(hint.eye, 2),
            rotate_world_point(hint.focus, 2),
        );

        assert_eq!(
            frame.view_hint_rotated_to_world(hint, 2),
            frame.view_hint_to_world(locally_rotated)
        );
    }

    #[test]
    fn ring19_frame_round_trips_feature_and_crystal_presentation_rotations() {
        let frame = LocalPatchFrame::from_resolved_ring19(HexCoord::from_axial(22, 0), 12, 4);
        let mut patch = presentation_fixture();
        frame
            .patch_to_world(&mut patch)
            .expect("Ring19 presentation should project");

        assert_eq!(
            patch
                .features
                .by_id
                .get(&FeatureId(0))
                .expect("fixture feature")
                .rotation
                .steps(),
            0
        );
        let Some(PlannedLightPresentation::CaveCrystal(crystal)) = patch
            .lights
            .get(&LightId(0))
            .expect("fixture light")
            .presentation
        else {
            panic!("fixture crystal presentation");
        };
        assert_eq!(crystal.rotation, 5);

        let local = frame
            .canonical_local_world(&patch)
            .expect("Ring19 presentation should return to recipe-local coordinates");
        assert_eq!(
            local
                .features
                .by_id
                .get(&FeatureId(0))
                .expect("round-tripped feature")
                .rotation
                .steps(),
            2
        );
        let Some(PlannedLightPresentation::CaveCrystal(crystal)) = local
            .lights
            .get(&LightId(0))
            .expect("round-tripped light")
            .presentation
        else {
            panic!("round-tripped crystal presentation");
        };
        assert_eq!(crystal.rotation, 1);
    }

    #[test]
    fn legacy_ring7_frame_does_not_compose_presentation_rotations() {
        let translation = HexCoord::from_axial(21, 0);
        let world_mask = BTreeSet::from([
            translation,
            checked_coord_sum(HexCoord::from_axial(1, 0), translation)
                .expect("fixture translation"),
        ]);
        let frame = LocalPatchFrame::resolve_rotated(&world_mask, LayoutKind::Ring7, 33, 4)
            .expect("legacy frame");
        let mut patch = presentation_fixture();
        frame
            .patch_to_world(&mut patch)
            .expect("legacy presentation should project");

        assert_eq!(
            patch
                .features
                .by_id
                .get(&FeatureId(0))
                .expect("fixture feature")
                .rotation
                .steps(),
            2
        );
        let Some(PlannedLightPresentation::CaveCrystal(crystal)) = patch
            .lights
            .get(&LightId(0))
            .expect("fixture light")
            .presentation
        else {
            panic!("fixture crystal presentation");
        };
        assert_eq!(crystal.rotation, 1);
    }

    #[test]
    fn macro_frame_composes_and_round_trips_presentation_rotations() {
        let translation = HexCoord::from_axial(21, 0);
        let world_mask = BTreeSet::from([
            translation,
            checked_coord_sum(HexCoord::from_axial(1, 0), translation)
                .expect("fixture translation"),
        ]);
        let frame = LocalPatchFrame::resolve_rotated(&world_mask, LayoutKind::Macro, 33, 4)
            .expect("Macro frame");
        let mut patch = presentation_fixture();
        frame
            .patch_to_world(&mut patch)
            .expect("Macro presentation should project");

        assert_eq!(
            patch
                .features
                .by_id
                .get(&FeatureId(0))
                .expect("fixture feature")
                .rotation
                .steps(),
            0
        );
        let Some(PlannedLightPresentation::CaveCrystal(crystal)) = patch
            .lights
            .get(&LightId(0))
            .expect("fixture light")
            .presentation
        else {
            panic!("fixture crystal presentation");
        };
        assert_eq!(crystal.rotation, 5);

        let local = frame
            .canonical_local_world(&patch)
            .expect("Macro presentation should return to recipe-local coordinates");
        assert_eq!(local.features, presentation_fixture().features);
        assert_eq!(local.lights, presentation_fixture().lights);
    }
}
