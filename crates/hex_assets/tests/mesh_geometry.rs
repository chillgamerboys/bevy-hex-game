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

use bevy::camera::PerspectiveProjection;
use hex_assets::{CameraSettings, PlayerSettings};
use hex_core::config::{HEX_CIRCUMRADIUS, HEX_SMALL_DIAMETER};

/// The tile mesh, relative to the workspace root.
fn hex_mesh_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/meshes/hex.glb")
}

/// The selected-unit mesh, relative to the workspace root.
fn pieces_mesh_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/meshes/pieces.glb")
}

/// Minimum and maximum vertex position of the first primitive of one mesh.
///
/// glTF stores those bounds in the accessor metadata, so the JSON chunk is enough —
/// no need to decode the binary vertex buffer.
///
/// Returns `None` for any malformed input rather than panicking, so the failure
/// surfaces at the assertion with a message about the mesh rather than as a panic in
/// a helper.
fn mesh_bounds(glb: &[u8], mesh_index: usize) -> Option<([f64; 3], [f64; 3])> {
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
        .get(mesh_index)?
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
    let (min, max) = mesh_bounds(&glb, 0).expect("hex.glb should be a readable glTF binary");

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
    let (min, max) = mesh_bounds(&glb, 0).expect("hex.glb should be a readable glTF binary");

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

/// The shipped king is assembled from the first two primitives in `pieces.glb`.
/// `hex_units::spawn_unit` applies one shared scale and the authored
/// `(-scale, -scale, -10 * scale)` origin correction to both children. The camera's
/// hide threshold must enclose that real transformed asset plus the production
/// projection's default near plane; otherwise a fully controlled near-first-person
/// view can still be covered by the selected unit before proximity occlusion starts.
#[test]
fn character_self_hide_radius_encloses_the_shipped_selected_mesh() {
    let glb = fs::read(pieces_mesh_path()).expect("assets/meshes/pieces.glb should exist");
    let (first_min, first_max) =
        mesh_bounds(&glb, 0).expect("the king body should expose glTF accessor bounds");
    let (second_min, second_max) =
        mesh_bounds(&glb, 1).expect("the king cross should expose glTF accessor bounds");
    let [first_min_x, first_min_y, first_min_z] = first_min;
    let [first_max_x, first_max_y, first_max_z] = first_max;
    let [second_min_x, second_min_y, second_min_z] = second_min;
    let [second_max_x, second_max_y, second_max_z] = second_max;
    let [min_x, min_y, min_z] = [
        first_min_x.min(second_min_x),
        first_min_y.min(second_min_y),
        first_min_z.min(second_min_z),
    ];
    let [max_x, max_y, max_z] = [
        first_max_x.max(second_max_x),
        first_max_y.max(second_max_y),
        first_max_z.max(second_max_z),
    ];
    let player: PlayerSettings = ron::from_str(include_str!("../../../assets/config/player.ron"))
        .expect("the shipped player settings should parse");
    let camera: CameraSettings = ron::from_str(include_str!("../../../assets/config/camera.ron"))
        .expect("the shipped camera settings should parse");
    let scale = f64::from(player.scale);
    let transformed_min = [
        min_x * scale - scale,
        min_y * scale - scale,
        min_z * scale - 10.0 * scale,
    ];
    let transformed_max = [
        max_x * scale - scale,
        max_y * scale - scale,
        max_z * scale - 10.0 * scale,
    ];
    let [transformed_min_x, transformed_min_y, transformed_min_z] = transformed_min;
    let [transformed_max_x, transformed_max_y, transformed_max_z] = transformed_max;
    let focus_y = f64::from(camera.character_focus_height);
    let farthest_axis = [
        transformed_min_x.abs().max(transformed_max_x.abs()),
        (transformed_min_y - focus_y)
            .abs()
            .max((transformed_max_y - focus_y).abs()),
        transformed_min_z.abs().max(transformed_max_z.abs()),
    ];
    let enclosing_radius = farthest_axis
        .into_iter()
        .map(|component| component * component)
        .sum::<f64>()
        .sqrt();
    let near_plane = f64::from(PerspectiveProjection::default().near);

    assert!(
        f64::from(camera.character_self_hide_radius) >= enclosing_radius + near_plane,
        "self-hide radius {} does not enclose the transformed selected mesh radius \
         {enclosing_radius} plus the {near_plane}-unit near plane",
        camera.character_self_hide_radius
    );
}
