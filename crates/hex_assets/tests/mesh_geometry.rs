//! Checks `hex_core`'s geometry constants against the mesh they claim to describe.
//!
//! # Why this exists
//!
//! `HEX_INNER_RADIUS` was `0.88` for about four years. The mesh's actual inner radius
//! is √3/2 ≈ 0.8660254, so every tile centre was placed **1.6% too far out** and the
//! whole grid carried a uniform hairline gap. Nothing reported it: the constants were
//! self-consistent, the game rendered, and the tests passed.
//!
//! That is the trap with constants that describe an external file — a test written
//! against the constants alone is a tautology. The only check with teeth reads the
//! asset.
//!
//! It also means this keeps working if someone **replaces the mesh**, which is the
//! more likely future cause.

use std::fs;
use std::path::PathBuf;

use hex_core::config::{HEX_CIRCUMRADIUS, HEX_SMALL_DIAMETER};

/// The tile mesh, relative to the workspace root.
fn hex_mesh_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/meshes/hex.glb")
}

/// Minimum and maximum vertex position of the first primitive of the first mesh.
///
/// glTF stores those bounds in the accessor metadata, so the JSON chunk is enough —
/// no need to decode the binary vertex buffer.
///
/// Returns `None` for any malformed input rather than panicking, so the failure
/// surfaces at the assertion with a message about the mesh rather than as a panic in
/// a helper.
fn mesh_bounds(glb: &[u8]) -> Option<([f64; 3], [f64; 3])> {
    // GLB layout: a 12-byte header, then chunks of (u32 length, u32 type, payload).
    // The first chunk is always JSON.
    const HEADER: usize = 12;
    let len_bytes: [u8; 4] = glb.get(HEADER..HEADER + 4)?.try_into().ok()?;
    let json_len = u32::from_le_bytes(len_bytes) as usize;

    let start = HEADER + 8;
    let json = glb.get(start..start.checked_add(json_len)?)?;
    let gltf: serde_json::Value = serde_json::from_slice(json).ok()?;

    // `serde_json::Value` indexing panics on a missing key, so every step goes
    // through `get`. A malformed file should fail the assertion, not the helper.
    let accessor_index = gltf
        .get("meshes")?
        .get(0)?
        .get("primitives")?
        .get(0)?
        .get("attributes")?
        .get("POSITION")?
        .as_u64()?;
    let accessor = gltf
        .get("accessors")?
        .get(usize::try_from(accessor_index).ok()?)?;

    let read = |key: &str| -> Option<[f64; 3]> {
        let values = accessor.get(key)?.as_array()?;
        let mut out = [0.0; 3];
        for (slot, value) in out.iter_mut().zip(values) {
            *slot = value.as_f64()?;
        }
        Some(out)
    };

    Some((read("min")?, read("max")?))
}

#[test]
fn the_geometry_constants_match_the_mesh() {
    let glb = fs::read(hex_mesh_path()).expect("assets/meshes/hex.glb should exist");
    let (min, max) = mesh_bounds(&glb).expect("hex.glb should be a readable glTF binary");

    // Z is corner-to-corner; half of it is the circumradius, and that is the constant
    // that sets tile spacing.
    let mesh_circumradius = (max[2] - min[2]) / 2.0;
    assert!(
        (mesh_circumradius - f64::from(HEX_CIRCUMRADIUS)).abs() < 1e-4,
        "mesh circumradius is {mesh_circumradius} but HEX_CIRCUMRADIUS is {HEX_CIRCUMRADIUS}"
    );

    // X is flat-to-flat, which is the distance between adjacent tile centres.
    let mesh_width = max[0] - min[0];
    assert!(
        (mesh_width - f64::from(HEX_SMALL_DIAMETER)).abs() < 1e-4,
        "mesh is {mesh_width} across but HEX_SMALL_DIAMETER is {HEX_SMALL_DIAMETER}"
    );
}

/// Tile spawning scales the mesh directly by a span's height, which is only correct
/// because the mesh is exactly one unit tall and centred on its origin. If either
/// changes, every column renders at the wrong height and the error scales with
/// terrain — tall columns look worse than short ones, which reads as a terrain bug.
#[test]
fn the_mesh_is_one_unit_tall_and_centred() {
    let glb = fs::read(hex_mesh_path()).expect("assets/meshes/hex.glb should exist");
    let (min, max) = mesh_bounds(&glb).expect("hex.glb should be a readable glTF binary");

    let height = max[1] - min[1];
    assert!(
        (height - 1.0).abs() < 1e-4,
        "mesh is {height} tall, expected 1.0"
    );

    let centre = (max[1] + min[1]) / 2.0;
    assert!(
        centre.abs() < 1e-4,
        "mesh centre is at y={centre}, expected 0.0"
    );
}
