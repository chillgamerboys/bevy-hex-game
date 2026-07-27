//! Round-trip coverage for the serde derives on the shared domain vocabulary.
//!
//! Every persisted domain type must survive serialize → deserialize unchanged.
//! `HexSpan` is deliberately absent: saves store [`TilePos`] only and re-derive
//! spans, so floats never enter a save.

use hex_core::{HexCoord, SubstanceId, TerrainEdit, TilePos, TraversalProfile, Turn};

/// Serializes a value to JSON and back, asserting it comes back unchanged.
macro_rules! assert_round_trips {
    ($value:expr) => {{
        let value = $value;
        let json = serde_json::to_string(&value).expect("serialize");
        let back = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(value, back, "round trip changed the value");
    }};
}

#[test]
fn tile_pos_round_trips() {
    assert_round_trips!(TilePos::new(HexCoord::new_cubic(2, -1, -1), 3));
}

#[test]
fn hex_coord_round_trips_as_its_axial_pair() {
    // Only the axial pair is stored, so the cube invariant holds by construction:
    // any deserialized value is a valid hex, and the serialized form reconstructs
    // exactly what `from_axial` would build.
    let coord = HexCoord::new_cubic(2, -1, -1);
    assert_round_trips!(coord);

    let json = serde_json::to_string(&coord).expect("serialize");
    let back: HexCoord = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, HexCoord::from_axial(2, -1));
}

/// The wire format is a contract, pinned by string rather than by round-trip.
///
/// A round-trip assertion is invariant under a symmetric field rename — both
/// sides move together and the test keeps passing while every existing save
/// silently breaks. Asserting the concrete text is the only guard that fails
/// when the serialized names drift.
#[test]
fn wire_formats_are_pinned() {
    let coord = HexCoord::from_axial(2, -1);
    let json = serde_json::to_string(&coord).expect("serialize");
    assert_eq!(json, r#"{"q":2,"r":-1}"#);

    let pos = TilePos::new(coord, 3);
    let json = serde_json::to_string(&pos).expect("serialize");
    assert_eq!(json, r#"{"coord":{"q":2,"r":-1},"level":3}"#);
}

#[test]
fn substance_id_round_trips() {
    assert_round_trips!(SubstanceId(7));
    assert_round_trips!(SubstanceId::AIR);
}

#[test]
fn terrain_edit_round_trips() {
    let pos = TilePos::new(HexCoord::ORIGIN, 1);
    assert_round_trips!(TerrainEdit::Set {
        pos,
        substance: SubstanceId(3),
    });
    assert_round_trips!(TerrainEdit::Clear { pos });
}

#[test]
fn traversal_profile_round_trips() {
    assert_round_trips!(TraversalProfile::WALKER);
}

#[test]
fn turn_round_trips() {
    assert_round_trips!(Turn {
        movement_left: 4,
        acted: true,
    });
}
