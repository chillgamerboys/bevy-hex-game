//! Round-trip coverage for the serde derives on the unit-side domain types.
//!
//! [`Body`] only round-trips because `hex_core::TraversalProfile` also derives
//! serde; this locks in that the whole `Body` → profile chain persists.

use hex_core::TraversalProfile;
use hex_units::{Body, Faction};

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
fn faction_round_trips() {
    assert_round_trips!(Faction::Player);
    assert_round_trips!(Faction::Hostile);
}

#[test]
fn body_round_trips() {
    assert_round_trips!(Body::new(TraversalProfile::WALKER));
}
