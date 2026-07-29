//! Serializable party-formation content and session runtime state.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bevy_ecs::{prelude::Resource, reflect::ReflectResource};
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};

use crate::{HexCoord, Sextant, TilePos, UnitId};

/// Minimum and maximum authored slots in a formation preset.
pub const MIN_FORMATION_SLOTS: usize = 1;
/// Maximum party size and authored slot count.
pub const MAX_FORMATION_SLOTS: usize = 6;

/// One authored axial slot relative to a formation anchor.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormationSlot {
    /// Offset from the moving anchor before facing rotation.
    pub offset: HexCoord,
    /// Whether this is the preset's one anchor slot.
    pub anchor: bool,
}

/// A stable named formation of one to six connected, unique slots.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FormationPreset {
    /// Stable content name.
    pub name: String,
    /// Slots in authored preference order.
    pub slots: Vec<FormationSlot>,
}

/// Why formation content is invalid.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum FormationError {
    /// The stable name was empty.
    EmptyName,
    /// Slot count was outside the supported range.
    SlotCount {
        /// Authored slot count.
        actual: usize,
    },
    /// Two authored slots used the same offset.
    DuplicateSlot {
        /// Repeated offset.
        offset: HexCoord,
    },
    /// The preset did not contain exactly one anchor.
    AnchorCount {
        /// Number of authored anchor slots.
        actual: usize,
    },
    /// Not every slot connects to the anchor through adjacent authored slots.
    Disconnected,
}

impl FormationPreset {
    /// Validates the complete formation content contract.
    pub fn validate(&self) -> Result<(), FormationError> {
        if self.name.trim().is_empty() {
            return Err(FormationError::EmptyName);
        }
        if !(MIN_FORMATION_SLOTS..=MAX_FORMATION_SLOTS).contains(&self.slots.len()) {
            return Err(FormationError::SlotCount {
                actual: self.slots.len(),
            });
        }

        let mut offsets = BTreeSet::new();
        let mut anchors = Vec::new();
        for slot in &self.slots {
            if !offsets.insert(slot.offset) {
                return Err(FormationError::DuplicateSlot {
                    offset: slot.offset,
                });
            }
            if slot.anchor {
                anchors.push(slot.offset);
            }
        }
        if anchors.len() != 1 {
            return Err(FormationError::AnchorCount {
                actual: anchors.len(),
            });
        }

        let Some(&anchor) = anchors.first() else {
            return Err(FormationError::AnchorCount { actual: 0 });
        };
        let mut reached = BTreeSet::new();
        let mut frontier = VecDeque::from([anchor]);
        while let Some(offset) = frontier.pop_front() {
            if !reached.insert(offset) {
                continue;
            }
            for neighbor in offset.neighbors() {
                if offsets.contains(&neighbor) && !reached.contains(&neighbor) {
                    frontier.push_back(neighbor);
                }
            }
        }
        if reached.len() != offsets.len() {
            return Err(FormationError::Disconnected);
        }
        Ok(())
    }

    /// Returns the authored anchor offset after successful validation.
    #[must_use]
    pub fn anchor(&self) -> Option<HexCoord> {
        self.slots
            .iter()
            .find(|slot| slot.anchor)
            .map(|slot| slot.offset)
    }
}

/// Whether exploration clicks move the formation or only the selected member.
#[derive(Reflect, Serialize, Deserialize, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PartyMovementMode {
    /// Move every party member atomically.
    #[default]
    Group,
    /// Move the selected member independently.
    Solo,
}

/// Session-scoped runtime formation state.
#[derive(Resource, Reflect, Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
#[reflect(Resource)]
pub struct PartyFormation {
    /// Stable preset content name.
    pub preset: String,
    /// Exact member-to-slot assignment.
    pub assignments: BTreeMap<UnitId, HexCoord>,
    /// Current travel-facing sextant.
    pub facing: Sextant,
    /// Exploration movement mode.
    pub mode: PartyMovementMode,
}

/// One member's exact path inside an atomic party move.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PartyPath {
    /// Stable party member identity.
    pub member: UnitId,
    /// Exact surfaces, beginning at the member's current position.
    pub path: Vec<TilePos>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_connected_unique_slots_and_one_anchor() {
        let preset = FormationPreset {
            name: "Compact".to_owned(),
            slots: vec![
                FormationSlot {
                    offset: HexCoord::ORIGIN,
                    anchor: true,
                },
                FormationSlot {
                    offset: HexCoord::from_axial(1, 0),
                    anchor: false,
                },
                FormationSlot {
                    offset: HexCoord::from_axial(0, 1),
                    anchor: false,
                },
            ],
        };
        assert_eq!(preset.validate(), Ok(()));
        assert_eq!(preset.anchor(), Some(HexCoord::ORIGIN));
    }

    #[test]
    fn rejects_a_disconnected_slot() {
        let preset = FormationPreset {
            name: "Broken".to_owned(),
            slots: vec![
                FormationSlot {
                    offset: HexCoord::ORIGIN,
                    anchor: true,
                },
                FormationSlot {
                    offset: HexCoord::from_axial(3, 0),
                    anchor: false,
                },
            ],
        };
        assert_eq!(preset.validate(), Err(FormationError::Disconnected));
    }
}
