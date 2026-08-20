//! Multiplayer authority contracts owned by unit movement.

use std::collections::BTreeMap;

use hex_core::{FormationPreset, FormationSlot, HexCoord, PartyFormation, UnitId};
use hex_units::formation_subset_anchor;

#[test]
fn each_owned_party_subset_has_one_stable_authored_anchor() {
    let preset = FormationPreset {
        name: "Six seats".to_owned(),
        slots: vec![
            FormationSlot {
                offset: HexCoord::ORIGIN,
                anchor: true,
            },
            FormationSlot {
                offset: HexCoord::from_axial(-1, 0),
                anchor: false,
            },
            FormationSlot {
                offset: HexCoord::from_axial(1, 0),
                anchor: false,
            },
        ],
    };
    let formation = PartyFormation {
        preset: preset.name.clone(),
        assignments: BTreeMap::from([
            (UnitId(1), HexCoord::ORIGIN),
            (UnitId(2), HexCoord::from_axial(-1, 0)),
            (UnitId(3), HexCoord::from_axial(1, 0)),
        ]),
        ..PartyFormation::default()
    };

    assert_eq!(
        formation_subset_anchor(&preset, &formation, &[UnitId(1), UnitId(3)]),
        Some(UnitId(1)),
        "the authored party anchor wins when its owner includes it"
    );
    assert_eq!(
        formation_subset_anchor(&preset, &formation, &[UnitId(2), UnitId(3)]),
        Some(UnitId(2)),
        "another seat deterministically receives its first authored occupied slot"
    );
    assert_eq!(
        formation_subset_anchor(&preset, &formation, &[UnitId(99)]),
        None,
        "an unassigned unit cannot invent a group-movement anchor"
    );
}
