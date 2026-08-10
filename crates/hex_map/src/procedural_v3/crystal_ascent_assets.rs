//! Authored object contract for Crystal Ascent fixtures.
//!
//! The small landing crystals deliberately reuse the accepted cave dependency
//! set. The cathedral heart is stricter: every occupied voxel is structural and
//! its exact radius-four footprint is gameplay-authoritative.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use hex_assets::{
    ConnectivityPolicy, HexObjectRotation, LocalAxialCoord, LocalVoxelCoord, ObjectAssetId,
    ObjectBlueprint, ObjectCategory, ObjectPart, PropPart, RuntimeArtCatalog, VoxelStyleId,
};
use hex_core::{
    AuthoredObjectVoxelRun, AuthoredObjectVoxelRuns, HexCoord, TilePos, TraversalProfile,
};

use super::{CaveCrystalAssetError, CaveCrystalKind, CaveCrystalObjectSet};

pub(crate) const CRYSTAL_CATHEDRAL_HEART_ID: &str = "prop/crystal-cathedral-heart";
const CRYSTAL_BODY_STYLE_ID: &str = "crystal/cyan-body";
const CRYSTAL_GLOW_STYLE_ID: &str = "crystal/cyan-glow";
const HEART_HEIGHT: u8 = 30;

/// Fully preflighted object ids used by the landmark renderer and occupancy adapter.
#[derive(Debug, Clone)]
pub(crate) struct CrystalAscentObjectSet {
    landing: CaveCrystalObjectSet,
    heart: ObjectAssetId,
    heart_origin: LocalVoxelCoord,
    heart_runs: Vec<(LocalAxialCoord, i32, i32)>,
}

impl CrystalAscentObjectSet {
    pub(crate) fn resolve(catalog: &RuntimeArtCatalog) -> Result<Self, CrystalAscentAssetError> {
        let landing = CaveCrystalObjectSet::resolve(catalog)
            .map_err(CrystalAscentAssetError::LandingAssets)?;
        let heart_id = ObjectAssetId::new(CRYSTAL_CATHEDRAL_HEART_ID).map_err(|error| {
            CrystalAscentAssetError::new(format!("Crystal Ascent heart id is invalid: {error}"))
        })?;
        let heart = catalog.object(&heart_id).ok_or_else(|| {
            CrystalAscentAssetError::new(format!(
                "Crystal Ascent requires authored object '{CRYSTAL_CATHEDRAL_HEART_ID}'"
            ))
        })?;
        validate_heart(heart)?;
        let heart_runs = compact_heart_runs(heart);
        Ok(Self {
            landing,
            heart: heart_id,
            heart_origin: heart.origin,
            heart_runs,
        })
    }

    #[must_use]
    pub(crate) fn landing_id(&self, kind: CaveCrystalKind) -> &ObjectAssetId {
        self.landing.object_id(kind)
    }

    #[must_use]
    pub(crate) const fn heart_id(&self) -> &ObjectAssetId {
        &self.heart
    }

    #[must_use]
    pub(crate) const fn glow_color(&self) -> hex_assets::SrgbColor {
        self.landing.glow_color()
    }

    /// Projects the exact rotated structural volume at the visual object's origin.
    pub(crate) fn project_heart_runs(
        &self,
        visual_origin: TilePos,
        rotation: HexObjectRotation,
    ) -> Option<AuthoredObjectVoxelRuns> {
        let mut runs = Vec::with_capacity(self.heart_runs.len());
        for (local, bottom, top) in &self.heart_runs {
            let rotated = rotation.rotate_axial(*local, self.heart_origin.axial())?;
            let q = visual_origin
                .coord
                .x()
                .checked_add(rotated.q.checked_sub(self.heart_origin.q)?)?;
            let r = visual_origin
                .coord
                .y()
                .checked_add(rotated.r.checked_sub(self.heart_origin.r)?)?;
            let bottom = visual_origin
                .level
                .checked_add(bottom.checked_sub(self.heart_origin.level)?)?;
            let top = visual_origin
                .level
                .checked_add(top.checked_sub(self.heart_origin.level)?)?;
            runs.push(AuthoredObjectVoxelRun::new(
                TilePos::new(HexCoord::from_axial(q, r), top),
                bottom,
            ));
        }
        Some(AuthoredObjectVoxelRuns::new(runs))
    }

    /// Derives exact blocked footing from the projected structural volume.
    ///
    /// The authored object's visual origin is one voxel above its terrain support.
    /// A surface blocks the canonical walker only when the two occupied body voxels
    /// above that surface overlap a projected structural run in the same column.
    pub(crate) fn project_heart_traversal_blockers(
        &self,
        supports: impl IntoIterator<Item = TilePos>,
        visual_origin: TilePos,
        rotation: HexObjectRotation,
    ) -> Option<BTreeSet<TilePos>> {
        let projected = self.project_heart_runs(visual_origin, rotation)?;
        let mut runs_by_coord = BTreeMap::<HexCoord, Vec<(i32, i32)>>::new();
        for run in projected.iter() {
            runs_by_coord
                .entry(run.top.coord)
                .or_default()
                .push((run.bottom, run.top.level));
        }

        let mut blockers = BTreeSet::new();
        for support in supports {
            let body_bottom = support.level.checked_add(1)?;
            let body_top = support
                .level
                .checked_add(TraversalProfile::WALKER.levels_tall)?;
            if runs_by_coord.get(&support.coord).is_some_and(|runs| {
                runs.iter()
                    .any(|(run_bottom, run_top)| *run_bottom <= body_top && *run_top >= body_bottom)
            }) {
                blockers.insert(support);
            }
        }
        Some(blockers)
    }
}

/// Failure to resolve or preflight Crystal Ascent's complete authored object set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CrystalAscentAssetError {
    LandingAssets(CaveCrystalAssetError),
    Heart(String),
}

impl CrystalAscentAssetError {
    fn new(detail: impl Into<String>) -> Self {
        Self::Heart(detail.into())
    }

    pub(crate) fn missing_catalog() -> Self {
        Self::new("Crystal Ascent requires the accepted runtime art catalog")
    }
}

impl fmt::Display for CrystalAscentAssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LandingAssets(error) => {
                write!(formatter, "landing crystal preflight failed: {error}")
            }
            Self::Heart(detail) => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for CrystalAscentAssetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::LandingAssets(error) => Some(error),
            Self::Heart(_) => None,
        }
    }
}

fn validate_heart(blueprint: &ObjectBlueprint) -> Result<(), CrystalAscentAssetError> {
    let body_style = VoxelStyleId::new(CRYSTAL_BODY_STYLE_ID)
        .map_err(|error| CrystalAscentAssetError::new(error.to_string()))?;
    let glow_style = VoxelStyleId::new(CRYSTAL_GLOW_STYLE_ID)
        .map_err(|error| CrystalAscentAssetError::new(error.to_string()))?;

    if blueprint.category != ObjectCategory::Prop
        || blueprint.connectivity != ConnectivityPolicy::Free
        || blueprint.bounds.radius != 4
        || blueprint.bounds.min_level != 0
        || blueprint.bounds.height != HEART_HEIGHT
        || blueprint.origin.q != 0
        || blueprint.origin.r != 0
        || blueprint.origin.level != 0
    {
        return Err(CrystalAscentAssetError::new(format!(
            "Crystal Ascent heart '{}' must be a free radius-4 Prop with origin (0, 0, 0) and levels 0..=29",
            blueprint.id
        )));
    }
    if !blueprint.canopy_occluders.is_empty() {
        return Err(CrystalAscentAssetError::new(format!(
            "Crystal Ascent heart '{}' cannot define canopy occluders",
            blueprint.id
        )));
    }

    let expected_footprint = (-4_i32..=4)
        .flat_map(|q| (-4_i32..=4).map(move |r| (q, r)))
        .filter(|(q, r)| (*q).abs().max((*r).abs()).max((-*q - *r).abs()) <= 4)
        .collect::<BTreeSet<_>>();
    let actual_footprint = blueprint
        .blocker_footprint
        .iter()
        .map(|coord| (coord.q, coord.r))
        .collect::<BTreeSet<_>>();
    if actual_footprint != expected_footprint {
        return Err(CrystalAscentAssetError::new(format!(
            "Crystal Ascent heart '{}' must block the exact radius-4 footprint",
            blueprint.id
        )));
    }

    let mut has_body = false;
    let mut has_glow = false;
    let mut levels = BTreeSet::new();
    let mut layers = BTreeMap::<i32, BTreeSet<(i32, i32)>>::new();
    let mut glow_voxels = BTreeSet::<(i32, i32, i32)>::new();
    for placement in &blueprint.placements {
        if placement.part != ObjectPart::Prop(PropPart::Structure) {
            return Err(CrystalAscentAssetError::new(format!(
                "Crystal Ascent heart '{}' contains a non-structural voxel",
                blueprint.id
            )));
        }
        if placement.style == body_style {
            has_body = true;
        } else if placement.style == glow_style {
            has_glow = true;
            glow_voxels.insert((
                placement.position.q,
                placement.position.r,
                placement.position.level,
            ));
        } else {
            return Err(CrystalAscentAssetError::new(format!(
                "Crystal Ascent heart '{}' uses unsupported style '{}'",
                blueprint.id, placement.style
            )));
        }
        levels.insert(placement.position.level);
        layers
            .entry(placement.position.level)
            .or_default()
            .insert((placement.position.q, placement.position.r));
    }
    if !has_body || !has_glow || levels != (0..i32::from(HEART_HEIGHT)).collect::<BTreeSet<_>>() {
        return Err(CrystalAscentAssetError::new(format!(
            "Crystal Ascent heart '{}' must use body and glow voxels across every level 0..=29",
            blueprint.id
        )));
    }
    let expected_layers = expected_heart_layers();
    if layers != expected_layers {
        return Err(CrystalAscentAssetError::new(format!(
            "Crystal Ascent heart '{}' must retain its exact 30-level irregular-prism silhouette",
            blueprint.id
        )));
    }
    if glow_voxels != expected_heart_glow_voxels(&expected_layers) {
        return Err(CrystalAscentAssetError::new(format!(
            "Crystal Ascent heart '{}' must retain its exact irregular ridge-glow accents",
            blueprint.id
        )));
    }
    Ok(())
}

fn expected_heart_layers() -> BTreeMap<i32, BTreeSet<(i32, i32)>> {
    fn disk(radius: i32) -> BTreeSet<(i32, i32)> {
        (-radius..=radius)
            .flat_map(|q| (-radius..=radius).map(move |r| (q, r)))
            .filter(|(q, r)| q.abs().max(r.abs()).max((-q - r).abs()) <= radius)
            .collect()
    }

    let mut layers = BTreeMap::new();
    layers.insert(0, disk(4));
    layers.insert(
        1,
        disk(3)
            .difference(&BTreeSet::from([(-3, 0), (0, 3)]))
            .copied()
            .collect(),
    );
    layers.insert(
        2,
        disk(2)
            .difference(&BTreeSet::from([(-2, 0), (0, 2), (2, 0)]))
            .copied()
            .collect(),
    );
    let prism = BTreeSet::from([
        (-2, 1),
        (-1, 0),
        (-1, 1),
        (-1, 2),
        (0, -1),
        (0, 0),
        (0, 1),
        (1, -2),
        (1, -1),
        (1, 0),
        (2, -2),
        (2, -1),
    ]);
    for level in 3..=24 {
        layers.insert(level, prism.clone());
    }
    let mut crown = disk(1);
    crown.extend([(-1, 2), (2, -1)]);
    for level in 25..=27 {
        layers.insert(level, crown.clone());
    }
    layers.insert(28, BTreeSet::from([(0, -1), (0, 0), (1, -1)]));
    layers.insert(29, BTreeSet::from([(1, -1)]));
    layers
}

fn expected_heart_glow_voxels(
    layers: &BTreeMap<i32, BTreeSet<(i32, i32)>>,
) -> BTreeSet<(i32, i32, i32)> {
    layers
        .iter()
        .flat_map(|(level, coords)| {
            coords.iter().filter_map(move |(q, r)| {
                let glow = if *level == 29 {
                    true
                } else if *level >= 25 {
                    (*q == -1 && *r == 2) || (*q == 2 && *r == -1 && *level % 2 == 1)
                } else if *level >= 3 {
                    (*q == -2 && *r == 1 && *level % 3 != 1)
                        || (*q == 2 && *r == -1 && *level % 4 == 1)
                        || (*q == 1 && *r == -2 && *level % 7 == 0)
                } else {
                    (q.saturating_mul(17)
                        .saturating_add(r.saturating_mul(11))
                        .saturating_add(level.saturating_mul(5)))
                    .rem_euclid(19)
                        == 0
                };
                glow.then_some((*q, *r, *level))
            })
        })
        .collect()
}

fn compact_heart_runs(blueprint: &ObjectBlueprint) -> Vec<(LocalAxialCoord, i32, i32)> {
    let mut levels_by_coord = std::collections::BTreeMap::<LocalAxialCoord, Vec<i32>>::new();
    for placement in &blueprint.placements {
        levels_by_coord
            .entry(placement.position.axial())
            .or_default()
            .push(placement.position.level);
    }
    let mut runs = Vec::new();
    for (coord, mut levels) in levels_by_coord {
        levels.sort_unstable();
        let Some(mut bottom) = levels.first().copied() else {
            continue;
        };
        let mut top = bottom;
        for level in levels.into_iter().skip(1) {
            if level == top.saturating_add(1) {
                top = level;
            } else {
                runs.push((coord, bottom, top));
                bottom = level;
                top = level;
            }
        }
        runs.push((coord, bottom, top));
    }
    runs
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::OnceLock;

    use super::*;
    use hex_assets::{ArtPalette, ObjectBlueprint, ObjectCatalogFile, VoxelStyleCatalog};

    pub(crate) fn runtime_art_catalog() -> &'static RuntimeArtCatalog {
        static CATALOG: OnceLock<RuntimeArtCatalog> = OnceLock::new();
        CATALOG.get_or_init(|| {
            let palette: ArtPalette = ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/art/palette.ron"
            )))
            .expect("tracked art palette should parse");
            let styles: VoxelStyleCatalog = ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/art/voxel_styles.ron"
            )))
            .expect("tracked voxel styles should parse");
            let mut objects = BTreeMap::new();
            for source in [
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/plant/small-broadleaf.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/plant/tall-narrow.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/plant/old-growth.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/crystal-low-cluster.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/crystal-branched.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/crystal-spire.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/crystal-cathedral-heart.ron"
                )),
            ] {
                let blueprint: ObjectBlueprint =
                    ron::from_str(source).expect("tracked object blueprint should parse");
                objects.insert(blueprint.id.clone(), blueprint);
            }
            let manifest = ObjectCatalogFile::new(objects.keys().cloned())
                .expect("Crystal Ascent fixture ids should form a valid manifest");
            RuntimeArtCatalog::from_sources(&palette, &styles, &manifest, objects)
                .expect("Crystal Ascent runtime art graph should resolve")
        })
    }

    #[test]
    fn shipped_heart_has_exact_structural_contract() {
        let blueprint: ObjectBlueprint = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/art/objects/prop/crystal-cathedral-heart.ron"
        )))
        .expect("tracked cathedral heart should parse");
        validate_heart(&blueprint).expect("tracked cathedral heart should satisfy the contract");
        assert_eq!(blueprint.placements.len(), 407);
        let layer_counts = blueprint.placements.iter().fold(
            BTreeMap::<i32, usize>::new(),
            |mut counts, placement| {
                *counts.entry(placement.position.level).or_default() += 1;
                counts
            },
        );
        assert_eq!(layer_counts.get(&0), Some(&61));
        assert_eq!(layer_counts.get(&1), Some(&35));
        assert_eq!(layer_counts.get(&2), Some(&16));
        assert!((3..=24).all(|level| layer_counts.get(&level) == Some(&12)));
        assert!((25..=27).all(|level| layer_counts.get(&level) == Some(&9)));
        assert_eq!(layer_counts.get(&28), Some(&3));
        assert_eq!(layer_counts.get(&29), Some(&1));
    }

    #[test]
    fn projected_heart_runs_drive_walker_blockers_for_every_rotation() {
        let objects = CrystalAscentObjectSet::resolve(runtime_art_catalog())
            .expect("tracked Crystal Ascent objects should resolve");
        let support_level = 6;
        let supports = HexCoord::ORIGIN
            .within_radius(6)
            .into_iter()
            .map(|coord| TilePos::new(coord, support_level))
            .collect::<BTreeSet<_>>();
        let expected = HexCoord::ORIGIN
            .within_radius(4)
            .into_iter()
            .map(|coord| TilePos::new(coord, support_level))
            .collect::<BTreeSet<_>>();
        let visual_origin = TilePos::new(HexCoord::ORIGIN, support_level + 1);

        let expected_tip_offsets = [(1, -1), (1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1)];
        for (steps, (tip_q, tip_r)) in expected_tip_offsets.into_iter().enumerate() {
            let steps = u8::try_from(steps).expect("six rotations fit u8");
            let rotation = HexObjectRotation::new(steps).expect("test rotation is canonical");
            let projected = objects
                .project_heart_runs(visual_origin, rotation)
                .expect("tracked heart projection should remain in range");
            assert_eq!(projected.runs.len(), 61);
            let tips = projected
                .iter()
                .filter(|run| run.top.level == visual_origin.level + 29)
                .collect::<Vec<_>>();
            assert_eq!(
                tips.len(),
                1,
                "rotation {steps} must keep one fractured tip"
            );
            let tip = tips
                .first()
                .expect("one exact fractured tip was asserted above");
            assert_eq!(
                tip.top.coord,
                HexCoord::from_axial(
                    visual_origin.coord.x() + tip_q,
                    visual_origin.coord.y() + tip_r,
                ),
                "rotation {steps} must rotate the asymmetric upper prism exactly"
            );
            let blockers = objects
                .project_heart_traversal_blockers(supports.iter().copied(), visual_origin, rotation)
                .expect("tracked heart projection should remain in range");
            assert_eq!(blockers, expected, "rotation {steps} changed footing");
        }
    }

    #[test]
    fn heart_projection_fails_closed_at_extreme_coordinates() {
        let objects = CrystalAscentObjectSet::resolve(runtime_art_catalog())
            .expect("tracked Crystal Ascent objects should resolve");
        let visual_origin = TilePos::new(HexCoord::from_axial(i32::MAX, 0), 7);
        assert!(objects
            .project_heart_runs(
                visual_origin,
                HexObjectRotation::new(0).expect("zero rotation is valid")
            )
            .is_none());
    }
}
