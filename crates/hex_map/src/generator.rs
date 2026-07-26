//! Terrain height generation.
//!
//! Entirely internal to `hex_map`. Nothing outside this crate reads [`HeightMap`]
//! or the generators. The map publishes the complete tile component contract from
//! [`crate::grid`], so this file can be rewritten without exposing its representation
//! to another crate.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use xxhash_rust::xxh3::xxh3_64_with_seed;

use hex_core::{HexCoord, Level};

/// Hashes bytes with a seed, salted by `msg`.
///
/// The salt lets one seed produce independent noise fields — the x and y components
/// of a gradient hash the same coordinate but with different messages, so they do
/// not correlate.
pub fn seeded_hash(bytes: &[u8], seed: u64, msg: &str) -> u64 {
    let mut vec = bytes.to_vec();
    let msg_bytes = msg.as_bytes();
    let mut msg_vec = msg_bytes.to_vec();
    vec.append(&mut msg_vec);
    xxh3_64_with_seed(vec.as_slice(), seed)
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Wrapper Struct ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ //

#[derive(Resource)]
/// A generated height field: how tall the ground is at each coordinate.
///
/// Private to this crate. The rest of the game never sees it — terrain reaches
/// gameplay through the tile components spawned by [`crate::grid`], so the generator
/// can be replaced wholesale without exposing its implementation outside `hex_map`.
pub struct HeightMap {
    generator: Box<dyn HeightGenerator>,
    /// Heights for every coord in the spawned grid, precomputed once in [`Self::new`].
    /// Coords outside the grid fall through to the generator.
    cache: HashMap<HexCoord, u32>,
}

impl HeightMap {
    /// Height at a coordinate, in quantized units. Never below 1.
    pub fn get_height(&self, coord: HexCoord) -> u32 {
        if let Some(height) = self.cache.get(&coord) {
            return *height;
        }
        Self::generate(self.generator.as_ref(), coord)
    }

    /// The level of the topmost solid voxel at a coordinate.
    ///
    /// Always at least 1, so every column is bedrock plus something above it. A
    /// column of nothing but bedrock would be a hole in the world that nothing could
    /// dig through, since bedrock is deliberately not diggable.
    #[must_use]
    pub fn surface_level(&self, coord: HexCoord) -> Level {
        Level::try_from(self.get_height(coord).max(1)).unwrap_or(Level::MAX)
    }

    /// Builds a height map, precomputing every coordinate within `grid_radius`.
    ///
    /// Coordinates outside that radius still work; they fall through to the
    /// generator instead of being served from the cache.
    pub fn new(generator: impl HeightGenerator, grid_radius: u32) -> Self {
        let generator: Box<dyn HeightGenerator> = Box::new(generator);
        // The perlin generator is pure but not cheap, and callers hit it constantly:
        // once per tile at spawn, then again for every waypoint of every click-to-move
        // path. The playable grid is a fixed, known set of coords, so evaluate it once
        // up front and read from the map thereafter.
        let cache = HexCoord::ORIGIN
            .within_radius(grid_radius)
            .into_iter()
            .map(|coord| (coord, Self::generate(generator.as_ref(), coord)))
            .collect();
        Self { generator, cache }
    }

    /// The uncached height for a coord. Floors at 1 so no tile is scaled to zero.
    fn generate(generator: &dyn HeightGenerator, coord: HexCoord) -> u32 {
        std::cmp::max(generator.generate_height(coord), 1)
    }
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Inner Trait  ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ //

/// Produces a height for any coordinate.
///
/// Implement this to add a new kind of terrain. Implementations must be **pure** —
/// the same coordinate must always give the same height — because results are
/// cached, so an inconsistent generator produces terrain that changes depending on
/// what has been looked at.
pub trait HeightGenerator: Send + Sync + 'static {
    /// Height at `coord`, in quantized units.
    fn generate_height(&self, coord: HexCoord) -> u32;
}

/// Flat ground everywhere. Useful for tests and for isolating a change from
/// terrain noise.
pub struct FlatGenerator {
    height: u32,
}

impl FlatGenerator {
    /// A generator returning `height` everywhere.
    pub fn new(height: u32) -> Self {
        Self { height }
    }
}

impl HeightGenerator for FlatGenerator {
    fn generate_height(&self, _coord: HexCoord) -> u32 {
        self.height
    }
}

/// Uniform random heights — white noise, with no correlation between neighbours.
///
/// Produces jagged, unwalkable terrain; kept because it is the simplest way to prove
/// something works regardless of terrain shape.
pub struct RandGenerator {
    min: u32,
    max: u32,
    seed: u64,
}

impl RandGenerator {
    /// Heights uniformly distributed in `min..max`.
    ///
    /// `None` for the seed picks a new one each run, so the world is different every
    /// launch.
    pub fn new(min: u32, max: u32, seed: Option<u64>) -> Self {
        let seed = seed.unwrap_or(rand::random());
        Self { min, max, seed }
    }
}

impl HeightGenerator for RandGenerator {
    fn generate_height(&self, coord: HexCoord) -> u32 {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "discarding the high half of a hash is the intent; any bits do"
        )]
        let hash = seeded_hash(&coord.to_bytes(), self.seed, "Random Height Map") as u32;
        hash % (self.max - self.min) + self.min
    }
}

/// Fractal Perlin noise for the optional procedural terrain preset.
///
/// Sums several [`PerlinStep`] octaves. Neighbouring coordinates get similar
/// heights, which is what makes the result walkable rather than jagged.
pub struct PerlinGenerator {
    steps: Vec<PerlinStep>,
    seed: u64,
}

impl PerlinGenerator {
    /// A generator summing the given octaves.
    ///
    /// `None` for the seed picks a new one each run.
    pub fn new(steps: Vec<PerlinStep>, seed: Option<u64>) -> Self {
        let seed = seed.unwrap_or(rand::random());
        Self { steps, seed }
    }

    // ~~~~~~~~~~~~~~ Prefabs ~~~~~~~~~~~~~~ //

    /// Long, smooth ridges with fine ripples over them.
    pub fn dunes(seed: Option<u64>) -> Self {
        Self::new(
            vec![
                PerlinStep::new(0.05, 0.01, 30.),
                PerlinStep::new(0.5, 0.1, 1.),
            ],
            seed,
        )
    }

    /// Rolling hills.
    pub fn hills(seed: Option<u64>) -> Self {
        Self::new(vec![PerlinStep::new(0.05, 0.05, 30.)], seed)
    }

    /// Broad, gentle gradients across the whole map.
    pub fn slopes(seed: Option<u64>) -> Self {
        Self::new(vec![PerlinStep::new(0.01, 0.01, 50.)], seed)
    }

    /// Sharp, high-frequency peaks.
    pub fn crags(seed: Option<u64>) -> Self {
        Self::new(vec![PerlinStep::new(0.15, 0.15, 35.)], seed)
    }

    /// Shallow terrain with gentle variation. Useful for the optional Perlin preset.
    pub fn lowlands(seed: Option<u64>) -> Self {
        Self::new(vec![PerlinStep::new(0.035, 0.05, 3.)], seed)
    }

    // ~~~~~~~~~~~ Internal Funcs ~~~~~~~~~~~ //
    // These were created by following https://gpfault.net/posts/perlin-noise.txt.html

    fn gradient(&self, vec: Vec2) -> Vec2 {
        // Precision is irrelevant here: these two hashes are only a direction, and
        // are immediately normalized. Any spread of values does the job.
        #[expect(
            clippy::cast_precision_loss,
            reason = "hash bits used only as a direction"
        )]
        let x_dir = seeded_hash(vec.to_string().as_bytes(), self.seed, "Perlin X Dir") as f32;
        #[expect(
            clippy::cast_precision_loss,
            reason = "hash bits used only as a direction"
        )]
        let y_dir = seeded_hash(vec.to_string().as_bytes(), self.seed, "Perlin Y Dir") as f32;
        Vec2::new(x_dir, y_dir).normalize()
    }

    fn fade(p: f32) -> f32 {
        p * p * p * (p * (p * 6. - 15.) + 10.)
    }

    fn noise(&self, v: Vec2) -> f32 {
        let v0 = v.floor();
        let v1 = v0 + Vec2::new(1., 0.);
        let v2 = v0 + Vec2::new(0., 1.);
        let v3 = v0 + Vec2::new(1., 1.);

        let g0 = self.gradient(v0);
        let g1 = self.gradient(v1);
        let g2 = self.gradient(v2);
        let g3 = self.gradient(v3);

        let t0 = v.x - v0.x;
        let t1 = v.y - v0.y;

        let fade_t0 = Self::fade(t0);
        let fade_t1 = Self::fade(t1);

        let v0v1 = (1. - fade_t0) * g0.dot(v - v0) + fade_t0 * g1.dot(v - v1);
        let v2v3 = (1. - fade_t0) * g2.dot(v - v2) + fade_t0 * g3.dot(v - v3);

        (1. - fade_t1) * v0v1 + fade_t1 * v2v3
    }
}

impl HeightGenerator for PerlinGenerator {
    fn generate_height(&self, coord: HexCoord) -> u32 {
        let mut height = 0.;
        for step in self.steps.iter() {
            #[expect(
                clippy::cast_precision_loss,
                reason = "hex coordinates are small integers, exact in f32"
            )]
            let x = (coord.x() as f32) * step.x_freq;
            #[expect(
                clippy::cast_precision_loss,
                reason = "hex coordinates are small integers, exact in f32"
            )]
            let y = (coord.y() as f32) * step.y_freq;
            let noise = self.noise(Vec2::new(x, y));
            height += (noise * 2. + 0.7) * step.magnitude;
            height += noise * step.magnitude;
        }
        // Noise can go negative. Clamping first makes the floor explicit rather
        // than relying on `as u32` saturating, and `HeightMap::generate` then lifts
        // it to at least 1 so no tile is scaled to nothing.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped non-negative on the line above; the fractional part is \
                      what we mean to discard"
        )]
        let quantized = height.max(0.0) as u32;
        quantized
    }
}

// For adding a level in the perlin noise generation.
// Can add any number of these to the perlin noise generator
/// One octave of Perlin noise.
///
/// Higher frequencies give finer detail; magnitude is how much this octave
/// contributes to the total height. Stacking a low-frequency, high-magnitude step
/// with higher-frequency, lower-magnitude ones gives broad shapes with detail on
/// top.
pub struct PerlinStep {
    x_freq: f32,
    y_freq: f32,
    magnitude: f32,
}

impl PerlinStep {
    /// An octave with the given frequencies and magnitude.
    pub fn new(x_freq: f32, y_freq: f32, magnitude: f32) -> Self {
        Self {
            x_freq,
            y_freq,
            magnitude,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_RADIUS: u32 = 20;

    /// Sample coordinates spread across and beyond the playable grid, so the
    /// cached and uncached paths are both exercised.
    fn sample_coords() -> Vec<HexCoord> {
        vec![
            HexCoord::ORIGIN,
            HexCoord::new_cubic(1, -1, 0),
            HexCoord::new_cubic(7, -3, -4),
            HexCoord::new_cubic(-12, 5, 7),
            HexCoord::new_cubic(19, -19, 0),
            // Outside the test grid radius, so this misses the cache.
            HexCoord::new_cubic(64, -32, -32),
        ]
    }

    #[test]
    fn same_seed_produces_the_same_terrain() {
        let a = HeightMap::new(PerlinGenerator::lowlands(Some(20260725)), TEST_RADIUS);
        let b = HeightMap::new(PerlinGenerator::lowlands(Some(20260725)), TEST_RADIUS);
        for coord in sample_coords() {
            assert_eq!(
                a.get_height(coord),
                b.get_height(coord),
                "terrain diverged at {coord:?}"
            );
        }
    }

    #[test]
    fn different_seeds_produce_different_terrain() {
        let a = HeightMap::new(PerlinGenerator::lowlands(Some(1)), TEST_RADIUS);
        let b = HeightMap::new(PerlinGenerator::lowlands(Some(2)), TEST_RADIUS);
        let differs = HexCoord::ORIGIN
            .within_radius(20)
            .into_iter()
            .any(|coord| a.get_height(coord) != b.get_height(coord));
        assert!(differs, "two seeds produced an identical grid");
    }

    /// The cache is a memoization of the generator, so it must not be able to
    /// disagree with it. Guards the cached and uncached paths against drift.
    #[test]
    fn cached_and_uncached_paths_agree() {
        let generator = PerlinGenerator::lowlands(Some(7));
        let expected: Vec<u32> = sample_coords()
            .into_iter()
            .map(|coord| std::cmp::max(generator.generate_height(coord), 1))
            .collect();

        let map = HeightMap::new(PerlinGenerator::lowlands(Some(7)), TEST_RADIUS);
        for (coord, want) in sample_coords().into_iter().zip(expected) {
            assert_eq!(map.get_height(coord), want, "mismatch at {coord:?}");
        }
    }

    /// Tile meshes are scaled by height, so a zero would collapse a tile.
    #[test]
    fn height_never_falls_below_one() {
        let map = HeightMap::new(FlatGenerator::new(0), TEST_RADIUS);
        assert_eq!(map.get_height(HexCoord::ORIGIN), 1);
        assert_eq!(map.get_height(HexCoord::new_cubic(99, -99, 0)), 1);
    }

    #[test]
    fn the_surface_level_matches_the_generated_height() {
        let map = HeightMap::new(FlatGenerator::new(3), TEST_RADIUS);
        assert_eq!(map.surface_level(HexCoord::ORIGIN), 3);
    }

    /// A generator returning zero must still leave a level above bedrock. Bedrock is
    /// not diggable, so a column of nothing but bedrock is a permanent hole.
    #[test]
    fn the_surface_never_collapses_onto_bedrock() {
        let map = HeightMap::new(FlatGenerator::new(0), TEST_RADIUS);
        assert_eq!(map.surface_level(HexCoord::ORIGIN), 1);
    }
}
