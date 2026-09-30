//! Immutable, source-derived terrain presentation, independent of residency.
//!
//! Profiles retain every solid interval. A macro surface replaces only explicitly
//! admitted exterior caps; protected columns keep their floors, roofs and voids.
//! The enclosing manifest binds this entire payload by its canonical fingerprint.
mod build;
mod certificate;

use hex_world_contracts::{ChunkId, ContractError, MaterialSpec, VoxelRun, WorldHex, CHUNK_SIZE};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Canonical manifest capability key for this presentation payload.
pub const TERRAIN_SURFACE_KEY: &str = "grand-terrain-surface-v1";

/// Missing finite-domain column, distinct from an actual empty solid profile.
pub const OUTSIDE_PROFILE: u16 = u16::MAX;
/// Source protection: any liquid occupies this column.
pub const WET: u8 = 1;
/// Source protection: disconnected solid intervals share this column.
pub const STACKED: u8 = 2;
/// Source protection: an exterior discontinuity cannot share an approximate cap.
pub const CLIFF: u8 = 4;
/// An authored object's actual support cap remains exact.
pub const OBJECT_CONTACT: u8 = 8;
/// Decorative ground cover retains its original support cap.
pub const GROUND_COVER: u8 = 16;
const KNOWN_PROTECTION: u8 = WET | STACKED | CLIFF | OBJECT_CONTACT | GROUND_COVER;
/// Representation limits, checked before generating presentation assets.
pub const MAX_SURFACE_CHUNKS: usize = 4096;
/// One complete fixed-size source profile index per represented column.
pub const MAX_SURFACE_COLUMNS: usize = MAX_SURFACE_CHUNKS * 256;
/// Profile IDs reserve one value for coordinates outside the finite world.
pub const MAX_SOLID_PROFILES: usize = 65_535;
/// Complete material runs across the interned dictionary, not expanded columns.
pub const MAX_PROFILE_RUNS: usize = 1_000_000;
/// A chunk cannot need more than every native cap corner and center.
pub const MAX_PATCH_VERTICES: usize = 2048;
/// Planar triangle bound for the bounded native lattice point set.
pub const MAX_PATCH_TRIANGLES: usize = 4096;

/// Exact solid-only profile, interned by complete equality rather than top height.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolidProfile {
    /// Ordered, disjoint, maximally coalesced source intervals.
    pub runs: Vec<SolidRun>,
}

/// Compact material interval; exclusive top retains the ColumnData convention.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolidRun {
    /// Inclusive solid bottom level.
    pub bottom: i16,
    /// Exclusive solid top level.
    pub top: i16,
    /// Index into the admitted overview/manifest material registry.
    pub material: u16,
}

/// A source-space point on the native lattice, never a moved collision vertex.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceVertex {
    /// Axial coordinates multiplied by three, relative to the owning chunk.
    /// Source centers are multiples of three; cap corners are the six exact
    /// integer offsets (1,1),(2,-1),(1,-2),(-1,-1),(-2,1),(-1,2).
    pub lattice: [i16; 2],
    /// Physical Y is this integer times one half of the declared level height.
    pub half_level: i16,
}

/// Verified topological patch; exact protected faces are derived from profiles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacroSurface {
    /// Deterministically ordered source-space positions.
    pub vertices: Vec<SurfaceVertex>,
    /// Upward-facing cap triangles, indexing only these vertices.
    pub triangles: Vec<[u16; 3]>,
    /// Maximum affine triangle/source-cap overlay error, in physical world units.
    /// It is a measured certificate, not permission to skip checking geometry.
    pub maximum_error: f64,
}

/// One immutable source-profile block, optionally owning a rendered macro patch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceChunk {
    /// Original storage address; it grants no source residency.
    pub coordinate: ChunkId,
    /// q changes fastest, then r. Exactly 256 IDs, including explicit outside IDs.
    pub profiles: Vec<u16>,
    /// Source protection bits for the same coordinates; unknown bits are refused.
    pub protection: Vec<u8>,
    /// None denotes a profile-only halo, which never draws or requests residency.
    pub surface: Option<MacroSurface>,
}

/// Canonical bounded input to disposable Grand far terrain presentation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerrainSurfaceOverview {
    /// Immutable presentation schema version, currently one.
    pub version: u32,
    /// Source error ceiling; currently at most two physical world units.
    pub tolerance: f64,
    /// Globally interned complete profiles in deterministic producer order.
    pub profiles: Vec<SolidProfile>,
    /// Unique, ordered source chunks; includes the exact one-column query halo.
    pub chunks: Vec<SurfaceChunk>,
}

/// Validated immutable source index reused by an entire compiler/admission batch.
/// It performs bounded global validation once; per-patch work never rebuilds the
/// dictionary lookup or treats missing/malformed profile storage as empty ground.
pub struct SurfaceBuilder<'a> {
    source: build::Source<'a>,
    level_height: f64,
}
impl<'a> SurfaceBuilder<'a> {
    /// Admit complete profiles, finite-domain addresses and the material registry.
    pub fn new(
        overview: &'a TerrainSurfaceOverview,
        radius: u32,
        levels: [i32; 2],
        materials: &[MaterialSpec],
        level_height: f64,
    ) -> Result<Self, ContractError> {
        if !level_height.is_finite() || level_height <= 0.0 {
            return Err(invalid("invalid cap level scale"));
        }
        if levels
            .into_iter()
            .any(|level| !(f64::from(level) * level_height).is_finite())
        {
            return Err(invalid("nonfinite physical profile extent"));
        }
        overview.validate_profiles(radius, levels, materials)?;
        Ok(Self {
            source: build::Source::new(overview),
            level_height,
        })
    }
    /// Construct and certify one source-constrained exterior cap patch.
    pub fn build_patch(&self, coordinate: ChunkId) -> Result<MacroSurface, ContractError> {
        build::build_patch(&self.source, coordinate, self.level_height)
    }
    /// Independently verify exported topology, coverage, canonical seams and error.
    pub fn certify_patch(
        &self,
        coordinate: ChunkId,
        surface: &MacroSurface,
    ) -> Result<f64, ContractError> {
        certificate::certify_patch(&self.source, coordinate, surface, self.level_height)
    }
}

fn invalid(message: &str) -> ContractError {
    ContractError::new("terrain_surface", message)
}

impl TerrainSurfaceOverview {
    /// Hash all canonical source profiles, topology and policy fields. The caller
    /// stores the result in the sealed manifest, outside this mutable payload.
    pub fn fingerprint(&self) -> Result<u64, ContractError> {
        hex_world_contracts::hash_serializable(self)
    }

    /// Validate compact source facts before any expansion into renderer assets.
    /// The triangulation module additionally certifies the full geometric overlay.
    pub fn validate_profiles(
        &self,
        radius: u32,
        levels: [i32; 2],
        materials: &[MaterialSpec],
    ) -> Result<(), ContractError> {
        if levels[0] < i32::from(i16::MIN) / 2
            || levels[1] >= i32::from(i16::MAX) / 2
            || levels[0] > levels[1]
            || self.version != 1
            || !self.tolerance.is_finite()
            || !(0.0..=2.0).contains(&self.tolerance)
            || self.chunks.len() > MAX_SURFACE_CHUNKS
            || self.profiles.len() > MAX_SOLID_PROFILES
        {
            return Err(invalid("invalid version, tolerance or source budget"));
        }
        let mut unique_profiles = BTreeSet::new();
        let mut run_count = 0usize;
        let mut stacked_profiles = Vec::with_capacity(self.profiles.len());
        for profile in &self.profiles {
            run_count = run_count
                .checked_add(profile.runs.len())
                .ok_or_else(|| invalid("profile run count overflow"))?;
            if run_count > MAX_PROFILE_RUNS || !unique_profiles.insert(profile) {
                return Err(invalid("profile run budget or duplicate profile"));
            }
            for run in &profile.runs {
                let Some(material) = materials.get(usize::from(run.material)) else {
                    return Err(invalid("profile material is outside the registry"));
                };
                if !material.solid
                    || run.bottom >= run.top
                    || i32::from(run.bottom) < levels[0]
                    || i32::from(run.top) > levels[1].saturating_add(1)
                {
                    return Err(invalid("invalid solid interval or physical level bound"));
                }
            }
            if profile.runs.windows(2).any(|pair| {
                let [a, b] = pair else { return true };
                a.top > b.bottom || (a.top == b.bottom && a.material == b.material)
            }) {
                return Err(invalid("profile is overlapping, unordered or uncoalesced"));
            }
            stacked_profiles.push(
                profile
                    .runs
                    .windows(2)
                    .any(|pair| matches!(pair,[a,b] if a.top<b.bottom)),
            );
        }
        let mut previous = None;
        for chunk in &self.chunks {
            if previous.is_some_and(|value| value >= chunk.coordinate)
                || chunk.profiles.len() != 256
                || chunk.protection.len() != 256
                || chunk
                    .protection
                    .iter()
                    .any(|flags| flags & !KNOWN_PROTECTION != 0)
            {
                return Err(invalid("invalid chunk order, column count or protection"));
            }
            previous = Some(chunk.coordinate);
            let origin = chunk.coordinate.origin()?;
            for (index, (&profile, &protection)) in
                chunk.profiles.iter().zip(&chunk.protection).enumerate()
            {
                let index = i64::try_from(index)
                    .map_err(|error| invalid(&format!("column index overflow: {error}")))?;
                let column =
                    WorldHex::new(origin.q + index % CHUNK_SIZE, origin.r + index / CHUNK_SIZE);
                let inside = column.checked_distance(WorldHex::new(0, 0))? <= u64::from(radius);
                if (profile == OUTSIDE_PROFILE) == inside
                    || (profile == OUTSIDE_PROFILE && protection != 0)
                    || (profile != OUTSIDE_PROFILE
                        && self.profiles.get(usize::from(profile)).is_none())
                {
                    return Err(invalid("source profile differs from the finite domain"));
                }
            }
            for (&profile, &protection) in chunk.profiles.iter().zip(&chunk.protection) {
                if stacked_profiles
                    .get(usize::from(profile))
                    .copied()
                    .unwrap_or(false)
                    && protection & STACKED == 0
                {
                    return Err(invalid(
                        "disconnected solid profile requires exact stacked protection",
                    ));
                }
            }
            if let Some(surface) = &chunk.surface {
                if surface.vertices.len() > MAX_PATCH_VERTICES
                    || surface.triangles.len() > MAX_PATCH_TRIANGLES
                    || !surface.maximum_error.is_finite()
                    || !(0.0..=self.tolerance).contains(&surface.maximum_error)
                    || surface.vertices.iter().any(|vertex| {
                        let [q, r] = vertex.lattice;
                        !(-2..=47).contains(&q)
                            || !(-2..=47).contains(&r)
                            || q.rem_euclid(3) != r.rem_euclid(3)
                            || i32::from(vertex.half_level) < 2 * levels[0]
                            || i32::from(vertex.half_level) > 2 * levels[1].saturating_add(1)
                    })
                    || surface.triangles.iter().any(|triangle| {
                        let [Some(a), Some(b), Some(c)] = triangle.map(|index| {
                            surface
                                .vertices
                                .get(usize::from(index))
                                .map(|vertex| vertex.lattice)
                        }) else {
                            return true;
                        };
                        let [aq, ar] = a.map(i32::from);
                        let [bq, br] = b.map(i32::from);
                        let [cq, cr] = c.map(i32::from);
                        // Clockwise in X/Z is the original upward-facing cap.
                        (bq - aq) * (cr - ar) - (br - ar) * (cq - aq) >= 0
                    })
                {
                    return Err(invalid("invalid compact patch geometry or certificate"));
                }
            }
        }
        Ok(())
    }

    /// Reconstruct exact admitted runs for disposable face exposure. This never
    /// supplies a gameplay terrain query or causes a source chunk to be resident.
    pub fn expand_profile(
        &self,
        id: u16,
        materials: &[MaterialSpec],
    ) -> Result<Vec<VoxelRun>, ContractError> {
        let profile = self
            .profiles
            .get(usize::from(id))
            .ok_or_else(|| invalid("missing solid profile"))?;
        profile
            .runs
            .iter()
            .map(|run| {
                let material = materials
                    .get(usize::from(run.material))
                    .ok_or_else(|| invalid("profile material is outside the registry"))?;
                Ok(VoxelRun {
                    bottom: i32::from(run.bottom),
                    top: i32::from(run.top),
                    material: material.id.clone(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (TerrainSurfaceOverview, Vec<MaterialSpec>) {
        let materials = vec![MaterialSpec {
            id: "stone".into(),
            solid: true,
            diggable: true,
            color: [100, 100, 100, 255],
        }];
        let surface = TerrainSurfaceOverview {
            version: 1,
            tolerance: 2.0,
            profiles: vec![SolidProfile {
                runs: vec![
                    SolidRun {
                        bottom: 0,
                        top: 5,
                        material: 0,
                    },
                    SolidRun {
                        bottom: 15,
                        top: 20,
                        material: 0,
                    },
                ],
            }],
            chunks: vec![SurfaceChunk {
                coordinate: ChunkId { q: 0, r: 0 },
                profiles: vec![0; 256],
                protection: vec![STACKED; 256],
                surface: None,
            }],
        };
        (surface, materials)
    }

    #[test]
    fn complete_profile_round_trip_preserves_cave_floor_ceiling_and_roof() {
        let (surface, materials) = fixture();
        surface
            .validate_profiles(64, [0, 100], &materials)
            .expect("valid profile block");
        let wire = ron::to_string(&surface).expect("surface serialization");
        let restored: TerrainSurfaceOverview = ron::from_str(&wire).expect("surface round trip");
        assert_eq!(restored, surface);
        let exact = restored
            .expand_profile(0, &materials)
            .expect("exact reconstruction");
        assert_eq!(
            exact,
            vec![
                VoxelRun {
                    bottom: 0,
                    top: 5,
                    material: "stone".into()
                },
                VoxelRun {
                    bottom: 15,
                    top: 20,
                    material: "stone".into()
                },
            ]
        );
        let before = surface.fingerprint().expect("canonical hash");
        let mut changed = surface;
        changed
            .profiles
            .first_mut()
            .expect("profile")
            .runs
            .first_mut()
            .expect("floor")
            .top = 6;
        assert_ne!(changed.fingerprint().expect("changed hash"), before);
    }

    #[test]
    fn malformed_profile_blocks_fail_before_mesh_expansion() {
        let (surface, materials) = fixture();
        let mut bad = surface.clone();
        bad.chunks.first_mut().expect("block").profiles.pop();
        assert!(bad.validate_profiles(64, [0, 100], &materials).is_err());
        let mut bad = surface.clone();
        bad.profiles
            .first_mut()
            .expect("profile")
            .runs
            .first_mut()
            .expect("floor")
            .top = 16;
        assert!(bad.validate_profiles(64, [0, 100], &materials).is_err());
        let mut bad = surface;
        bad.chunks.first_mut().expect("block").protection.fill(128);
        assert!(bad.validate_profiles(64, [0, 100], &materials).is_err());
    }
}

#[cfg(test)]
mod build_tests;
