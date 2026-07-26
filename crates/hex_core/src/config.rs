//! Constants describing the geometry of `hex.glb`.
//!
//! These are **not** settings. They are measurements of the tile mesh, and every
//! coordinate conversion in [`crate::hex`] depends on them matching the asset.
//! Changing them without editing the mesh does not make tiles bigger — it makes them
//! overlap or leaves gaps between them, with nothing reported at runtime.
//!
//! That is why they stayed here when everything else moved into RON: a value someone
//! can change should be one they can change *safely*. Tunable settings live in
//! `hex_assets` and `hex_map`, loaded from `assets/config/`.
//!
//! # The mesh is a unit hex
//!
//! Read directly out of `assets/meshes/hex.glb`, whose vertex positions span:
//!
//! | Axis | Extent | Meaning |
//! |---|---|---|
//! | X | ±0.866_025_4 | flat-to-flat width ÷ 2 — the inner radius, √3/2 |
//! | Z | ±1.0 | corner-to-corner ÷ 2 — the circumradius, exactly 1 |
//! | Y | ±0.5 | height of exactly 1, centred on the origin |
//!
//! The Y extent is why tile spawning scales by the span height directly: a mesh one
//! unit tall scaled by *n* is *n* units tall.
//!
//! These are written as exact values rather than measurements. `0.866_025_4` is √3/2,
//! not an approximation of something rounder.

/// Centre-to-corner distance of the tile mesh, in world units.
///
/// Exactly 1: the mesh spans ±1.0 along Z. This is the spacing constant for
/// hex-to-world conversion, so an error here shows up as a uniform gap or overlap
/// between every pair of tiles.
pub const HEX_CIRCUMRADIUS: f32 = 1.0;

/// Centre-to-edge distance of the tile mesh, in world units.
///
/// √3/2, the inner radius of a hexagon whose circumradius is 1.
const HEX_INNER_RADIUS: f32 = 0.866_025_4;

/// Edge-to-edge width of a tile — the distance between adjacent tile centres.
///
/// Used to work out how long crossing one hex should take at a given speed.
pub const HEX_SMALL_DIAMETER: f32 = 2.0 * HEX_INNER_RADIUS;

#[cfg(test)]
mod tests {
    use super::*;

    /// The two radii have to describe the same hexagon.
    ///
    /// A **consistency** check, not a correctness one — it would have passed with the
    /// old values too, because the circumradius was derived from the inner radius and
    /// the pair was self-consistent while matching nothing. It catches someone editing
    /// one constant and forgetting the other. Whether they match the *mesh* is checked
    /// in `hex_assets`, against the file itself.
    #[test]
    fn the_radii_describe_one_hexagon() {
        // inner = circumradius * cos(30°) = circumradius * √3/2
        let expected_inner = HEX_CIRCUMRADIUS * (3.0f32).sqrt() / 2.0;
        assert!(
            (HEX_INNER_RADIUS - expected_inner).abs() < 1e-6,
            "inner radius {HEX_INNER_RADIUS} does not match circumradius {HEX_CIRCUMRADIUS}"
        );
    }

    /// Adjacent tile centres are exactly one flat-to-flat width apart.
    ///
    /// Also a consistency check: `to_world` and these constants have to agree about
    /// what a hex is. It cannot tell you whether either agrees with the mesh.
    #[test]
    fn adjacent_centres_are_one_tile_apart() {
        use crate::HexCoord;

        let origin = HexCoord::ORIGIN.to_world(0.0);
        for neighbour in HexCoord::ORIGIN.neighbors() {
            let distance = origin.distance(neighbour.to_world(0.0));
            assert!(
                (distance - HEX_SMALL_DIAMETER).abs() < 1e-5,
                "neighbour sits {distance} away but tiles are {HEX_SMALL_DIAMETER} wide"
            );
        }
    }
}
