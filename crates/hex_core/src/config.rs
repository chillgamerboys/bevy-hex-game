//! Constants describing the geometry of `hex.glb`.
//!
//! These are **not** settings. They are measurements of the tile mesh, and every
//! coordinate conversion in [`crate::hex`] depends on them matching the asset.
//! Changing `HEX_INNER_RADIUS` without editing the mesh does not make tiles bigger —
//! it makes them overlap or leaves gaps between them, with nothing reported at
//! runtime.
//!
//! That is why they stayed here when everything else moved into RON: a value someone
//! can change should be one they can change *safely*. Tunable settings live in
//! `hex_assets` and `hex_map`, loaded from `assets/config/`.

/// Centre-to-edge distance of the tile mesh, in world units.
///
/// Measured from `assets/meshes/hex.glb`. Everything else here derives from it.
const HEX_INNER_RADIUS: f32 = 0.88;

/// Centre-to-corner distance, `inner_radius * sqrt(4/3)`.
///
/// The spacing constant for hex-to-world conversion.
pub const HEX_CIRCUMRADIUS: f32 = HEX_INNER_RADIUS * 1.154_700_5;

/// Edge-to-edge width of a tile — the distance between adjacent tile centres.
///
/// Used to work out how long crossing one hex should take at a given speed.
pub const HEX_SMALL_DIAMETER: f32 = 2.0 * HEX_INNER_RADIUS;
