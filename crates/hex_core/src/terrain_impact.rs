//! Announcing elemental damage to the world, and hearing exactly what it did.
//!
//! Gameplay owns the exact affected volume. The world owns substance toughness,
//! element/material admission, remaining voxel health, and the resulting mutation.
//! Conjuration remains the separate [`TerrainEdit`](crate::TerrainEdit) path because
//! it names the material the spell creates rather than asking an existing material
//! how it responds.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;

use crate::elements::ElementId;
use crate::voxel::{SubstanceId, TilePos};

/// Highest authored maximum health in the initial four-level terrain scale.
pub const MAX_TERRAIN_TOUGHNESS: u8 = 8;

/// Whether a maximum health value belongs to the initial authored toughness scale.
#[must_use]
pub const fn is_terrain_toughness(value: u8) -> bool {
    matches!(value, 1 | 2 | 4 | MAX_TERRAIN_TOUGHNESS)
}

/// Identifies one announcement so its outcome can be matched to it.
///
/// Session-local, like every other runtime handle here. A durable log stores its own
/// key and converts elements and substances back to stable names.
#[derive(Reflect, Debug, Default, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TerrainBatchId(pub u64);

/// Why the world rejected a complete terrain-impact batch without mutating anything.
#[derive(Reflect, Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum TerrainImpactRejection {
    /// A spell announced no exact voxels.
    EmptyVolume,
    /// The volume was not strictly sorted and deduplicated.
    NonCanonicalVolume,
    /// Terrain damage must remove at least one health point when admitted.
    ZeroPower,
    /// The runtime element handle does not belong to the active element catalog.
    UnknownElement,
    /// This session-local batch id was already consumed.
    ReusedBatch,
    /// No complete active terrain projection was available to answer the request.
    TerrainUnavailable,
}

/// An energetic effect announced over an exact set of voxels.
///
/// The world decides what each material does about it, including resistance. A fully
/// resisted applied announcement is still a completed cast rather than an error.
#[derive(Message, Reflect, Debug, Clone, PartialEq, Eq)]
pub struct TerrainImpact {
    /// Dealt by gameplay; echoed back by [`TerrainImpactOutcome`].
    pub batch: TerrainBatchId,
    /// Every exact voxel the effect reaches, sorted and deduplicated.
    pub volume: Vec<TilePos>,
    /// Which element arrived. Authored response content uses its stable name.
    pub element: ElementId,
    /// Health points removed from every admitted voxel, capped at remaining health.
    pub power: u8,
}

impl TerrainImpact {
    /// Whether `volume` is strictly sorted and contains no repeats.
    #[must_use]
    pub fn is_canonical(&self) -> bool {
        self.volume.windows(2).all(|pair| match pair {
            [a, b] => a < b,
            _ => true,
        })
    }

    /// The first structural contract failure, before catalog and session checks.
    #[must_use]
    pub fn structural_rejection(&self) -> Option<TerrainImpactRejection> {
        if self.volume.is_empty() {
            Some(TerrainImpactRejection::EmptyVolume)
        } else if !self.is_canonical() {
            Some(TerrainImpactRejection::NonCanonicalVolume)
        } else if self.power == 0 {
            Some(TerrainImpactRejection::ZeroPower)
        } else {
            None
        }
    }
}

/// Current health of one extant destructible voxel.
#[derive(Reflect, Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct TerrainVoxelHealth {
    /// Health points still present. Zero is never represented: that voxel is gone.
    pub remaining: u8,
    /// Authored maximum health for the current substance.
    pub maximum: u8,
}

impl TerrainVoxelHealth {
    /// Creates valid health for an extant voxel in the initial toughness range.
    #[must_use]
    pub const fn new(remaining: u8, maximum: u8) -> Option<Self> {
        if remaining == 0 || remaining > maximum || !is_terrain_toughness(maximum) {
            None
        } else {
            Some(Self { remaining, maximum })
        }
    }

    /// Whether this health belongs to an extant voxel in the authored range.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.remaining > 0 && self.remaining <= self.maximum && is_terrain_toughness(self.maximum)
    }

    /// Whether the voxel has lost health but has not been destroyed.
    #[must_use]
    pub const fn is_damaged(self) -> bool {
        self.is_valid() && self.remaining < self.maximum
    }
}

/// What the world decided for one voxel an applied impact reached.
#[derive(Reflect, Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum TerrainImpactDisposition {
    /// There was nothing there to affect.
    NoMaterial,
    /// Material was present but the impact was not admitted.
    Resisted,
    /// Material survived with fewer health points.
    Damaged,
    /// Material reached zero health and was removed.
    Destroyed,
}

/// One exact voxel's authoritative result.
#[derive(Reflect, Debug, Copy, Clone, PartialEq, Eq)]
pub struct TerrainVoxelOutcome {
    /// Which voxel was resolved.
    pub pos: TilePos,
    /// What the world decided.
    pub disposition: TerrainImpactDisposition,
    /// Substance before resolution, if material existed. Air is represented by `None`.
    pub before: Option<SubstanceId>,
    /// Substance after resolution, if material remains. Air is represented by `None`.
    pub after: Option<SubstanceId>,
    /// Health before resolution for a destructible substance.
    pub health_before: Option<TerrainVoxelHealth>,
    /// Health after resolution for a surviving destructible substance.
    pub health_after: Option<TerrainVoxelHealth>,
}

impl TerrainVoxelOutcome {
    /// Whether material and health agree with the disposition and impact power.
    #[must_use]
    pub fn is_consistent_with_power(&self, power: u8) -> bool {
        let had_material = self.before.is_some_and(|substance| !substance.is_air());
        match self.disposition {
            TerrainImpactDisposition::NoMaterial => {
                self.before.is_none()
                    && self.after.is_none()
                    && self.health_before.is_none()
                    && self.health_after.is_none()
            }
            TerrainImpactDisposition::Resisted => {
                had_material
                    && self.before == self.after
                    && self.health_before == self.health_after
                    && self.health_before.is_none_or(TerrainVoxelHealth::is_valid)
            }
            TerrainImpactDisposition::Damaged => {
                let (Some(before), Some(after)) = (self.health_before, self.health_after) else {
                    return false;
                };
                power > 0
                    && had_material
                    && self.before == self.after
                    && before.is_valid()
                    && after.is_valid()
                    && before.maximum == after.maximum
                    && before.remaining > power
                    && after.remaining == before.remaining - power
            }
            TerrainImpactDisposition::Destroyed => {
                let Some(before) = self.health_before else {
                    return false;
                };
                had_material
                    && self.after.is_none()
                    && before.is_valid()
                    && self.health_after.is_none()
                    && power >= before.remaining
            }
        }
    }
}

/// Applied per-voxel results or one whole-batch rejection.
#[derive(Reflect, Debug, Clone, PartialEq, Eq)]
pub enum TerrainImpactResult {
    /// Every announced position resolved in the same canonical order.
    Applied(Vec<TerrainVoxelOutcome>),
    /// Nothing mutated and there are no partial voxel results.
    Rejected(TerrainImpactRejection),
}

/// What one announced impact became.
///
/// This is authoritative simulation truth, not permission to reveal its payload.
/// Faction-facing logs and presentation must filter applied entries through current
/// observation.
#[derive(Message, Reflect, Debug, Clone, PartialEq, Eq)]
pub struct TerrainImpactOutcome {
    /// Correlates with the announcement that caused it.
    pub batch: TerrainBatchId,
    /// The complete applied or rejected result.
    pub result: TerrainImpactResult,
}

impl TerrainImpactOutcome {
    /// Whether this answer is structurally compatible with its originating impact.
    #[must_use]
    pub fn is_consistent_with(&self, impact: &TerrainImpact) -> bool {
        if self.batch != impact.batch {
            return false;
        }
        match &self.result {
            TerrainImpactResult::Applied(voxels) => {
                impact.structural_rejection().is_none()
                    && voxels.len() == impact.volume.len()
                    && voxels
                        .iter()
                        .zip(&impact.volume)
                        .all(|(outcome, position)| {
                            outcome.pos == *position
                                && outcome.is_consistent_with_power(impact.power)
                        })
            }
            TerrainImpactResult::Rejected(reason) => match reason {
                TerrainImpactRejection::EmptyVolume => impact.volume.is_empty(),
                TerrainImpactRejection::NonCanonicalVolume => {
                    !impact.volume.is_empty() && !impact.is_canonical()
                }
                TerrainImpactRejection::ZeroPower => {
                    !impact.volume.is_empty() && impact.is_canonical() && impact.power == 0
                }
                TerrainImpactRejection::ReusedBatch => true,
                TerrainImpactRejection::UnknownElement
                | TerrainImpactRejection::TerrainUnavailable => {
                    impact.structural_rejection().is_none()
                }
            },
        }
    }
}

/// Exact current partial-health projection published by the map.
///
/// Entries exist only while `0 < remaining < maximum`. This resource is world truth,
/// not a disclosure grant: presentation must separately require current faction
/// observation. `hex_map` is the sole runtime writer.
#[derive(Resource, Reflect, Debug, Default, Clone, PartialEq, Eq)]
#[reflect(Resource)]
pub struct DamagedVoxels {
    by_position: BTreeMap<TilePos, TerrainVoxelHealth>,
}

impl DamagedVoxels {
    /// Creates an empty projection.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            by_position: BTreeMap::new(),
        }
    }

    /// Returns partial health at an exact position.
    #[must_use]
    pub fn get(&self, pos: TilePos) -> Option<TerrainVoxelHealth> {
        self.by_position.get(&pos).copied()
    }

    /// Iterates partial health in exact position order.
    pub fn iter(&self) -> impl Iterator<Item = (TilePos, TerrainVoxelHealth)> + '_ {
        self.by_position
            .iter()
            .map(|(position, health)| (*position, *health))
    }

    /// Number of partially damaged voxels.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_position.len()
    }

    /// Whether no voxel is partially damaged.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_position.is_empty()
    }

    /// Publishes valid partial health, removing full or malformed values.
    ///
    /// Runtime ownership belongs to `hex_map`; this is public only because the map is
    /// downstream of the shared contract crate.
    pub fn publish(&mut self, pos: TilePos, health: TerrainVoxelHealth) {
        if health.is_damaged() {
            self.by_position.insert(pos, health);
        } else {
            self.by_position.remove(&pos);
        }
    }

    /// Removes one position after healing, replacement, or destruction.
    pub fn remove(&mut self, pos: TilePos) {
        self.by_position.remove(&pos);
    }

    /// Clears the projection during world teardown.
    pub fn clear(&mut self) {
        self.by_position.clear();
    }
}

/// Shared cross-owner phases for terrain mutation and gameplay reconciliation.
///
/// [`ApplyWorld`](Self::ApplyWorld) is live. The remaining variants reserve the
/// accepted integration protocol without claiming that gameplay participants exist.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerrainSystems {
    /// The map consumes edits/impacts and publishes rebuilt terrain plus outcomes.
    ApplyWorld,
    /// Reserved for gameplay to refresh occupancy and reconcile movement.
    RefreshProjections,
    /// Reserved for gameplay to settle actors against the rebuilt terrain.
    ReconcileActors,
    /// Reserved for gameplay to validate answers and release matching pending work.
    ConsumeOutcomes,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HexCoord;

    fn at(q: i32, r: i32, level: crate::Level) -> TilePos {
        TilePos::new(HexCoord::from_axial(q, r), level)
    }

    fn impact(volume: Vec<TilePos>, power: u8) -> TerrainImpact {
        TerrainImpact {
            batch: TerrainBatchId(7),
            volume,
            element: ElementId(2),
            power,
        }
    }

    fn health(remaining: u8, maximum: u8) -> TerrainVoxelHealth {
        TerrainVoxelHealth::new(remaining, maximum).expect("fixture health is valid")
    }

    fn voxel_outcome(
        pos: TilePos,
        disposition: TerrainImpactDisposition,
        before: Option<SubstanceId>,
        after: Option<SubstanceId>,
        health_before: Option<TerrainVoxelHealth>,
        health_after: Option<TerrainVoxelHealth>,
    ) -> TerrainVoxelOutcome {
        TerrainVoxelOutcome {
            pos,
            disposition,
            before,
            after,
            health_before,
            health_after,
        }
    }

    fn no_material_outcome(pos: TilePos) -> TerrainVoxelOutcome {
        voxel_outcome(
            pos,
            TerrainImpactDisposition::NoMaterial,
            None,
            None,
            None,
            None,
        )
    }

    fn valid_health_cases() -> Vec<TerrainVoxelHealth> {
        let mut cases = Vec::new();
        for maximum in [1, 2, 4, 8] {
            for remaining in 1..=maximum {
                cases.push(TerrainVoxelHealth { remaining, maximum });
            }
        }
        cases
    }

    fn health_semantic_cases() -> Vec<Option<TerrainVoxelHealth>> {
        let mut cases = vec![None];
        cases.extend(valid_health_cases().into_iter().map(Some));
        cases.extend([
            Some(TerrainVoxelHealth {
                remaining: 0,
                maximum: 1,
            }),
            Some(TerrainVoxelHealth {
                remaining: 2,
                maximum: 1,
            }),
            Some(TerrainVoxelHealth {
                remaining: 1,
                maximum: 0,
            }),
            Some(TerrainVoxelHealth {
                remaining: 1,
                maximum: 3,
            }),
            Some(TerrainVoxelHealth {
                remaining: 9,
                maximum: 8,
            }),
            Some(TerrainVoxelHealth {
                remaining: u8::MAX,
                maximum: u8::MAX,
            }),
        ]);
        cases
    }

    fn accepted_voxel_outcomes(pos: TilePos, power: u8) -> Vec<TerrainVoxelOutcome> {
        let mut accepted = vec![no_material_outcome(pos)];
        let health_cases = valid_health_cases();

        for material in [SubstanceId(3), SubstanceId(9)] {
            accepted.push(voxel_outcome(
                pos,
                TerrainImpactDisposition::Resisted,
                Some(material),
                Some(material),
                None,
                None,
            ));
            for health in &health_cases {
                accepted.push(voxel_outcome(
                    pos,
                    TerrainImpactDisposition::Resisted,
                    Some(material),
                    Some(material),
                    Some(*health),
                    Some(*health),
                ));
                if power > 0 && health.remaining > power {
                    accepted.push(voxel_outcome(
                        pos,
                        TerrainImpactDisposition::Damaged,
                        Some(material),
                        Some(material),
                        Some(*health),
                        Some(TerrainVoxelHealth {
                            remaining: health.remaining - power,
                            maximum: health.maximum,
                        }),
                    ));
                }
                if power >= health.remaining {
                    accepted.push(voxel_outcome(
                        pos,
                        TerrainImpactDisposition::Destroyed,
                        Some(material),
                        None,
                        Some(*health),
                        None,
                    ));
                }
            }
        }

        accepted
    }

    #[test]
    fn structural_rejections_are_exact() {
        assert_eq!(
            impact(Vec::new(), 1).structural_rejection(),
            Some(TerrainImpactRejection::EmptyVolume)
        );
        assert_eq!(
            impact(vec![at(1, 0, 1), at(0, 0, 1)], 1).structural_rejection(),
            Some(TerrainImpactRejection::NonCanonicalVolume)
        );
        assert_eq!(
            impact(vec![at(0, 0, 1)], 0).structural_rejection(),
            Some(TerrainImpactRejection::ZeroPower)
        );
        assert_eq!(impact(vec![at(0, 0, 1)], 1).structural_rejection(), None);
    }

    #[test]
    fn health_excludes_zero_and_values_outside_the_authored_range() {
        for maximum in [1, 2, 4, 8] {
            assert_eq!(
                TerrainVoxelHealth::new(maximum, maximum),
                Some(TerrainVoxelHealth {
                    remaining: maximum,
                    maximum,
                })
            );
        }
        assert_eq!(TerrainVoxelHealth::new(0, 1), None);
        assert_eq!(TerrainVoxelHealth::new(2, 1), None);
        assert_eq!(TerrainVoxelHealth::new(1, 3), None);
        assert_eq!(TerrainVoxelHealth::new(1, 9), None);
        assert!(health(4, 8).is_damaged());
        assert!(!health(8, 8).is_damaged());
    }

    #[test]
    fn dispositions_pin_material_and_health_transitions() {
        let pos = at(0, 0, 1);
        let stone = Some(SubstanceId(3));
        let no_material = TerrainVoxelOutcome {
            pos,
            disposition: TerrainImpactDisposition::NoMaterial,
            before: None,
            after: None,
            health_before: None,
            health_after: None,
        };
        let resisted = TerrainVoxelOutcome {
            pos,
            disposition: TerrainImpactDisposition::Resisted,
            before: stone,
            after: stone,
            health_before: Some(health(3, 4)),
            health_after: Some(health(3, 4)),
        };
        let damaged = TerrainVoxelOutcome {
            pos,
            disposition: TerrainImpactDisposition::Damaged,
            before: stone,
            after: stone,
            health_before: Some(health(4, 4)),
            health_after: Some(health(2, 4)),
        };
        let destroyed = TerrainVoxelOutcome {
            pos,
            disposition: TerrainImpactDisposition::Destroyed,
            before: stone,
            after: None,
            health_before: Some(health(2, 4)),
            health_after: None,
        };

        assert!(no_material.is_consistent_with_power(2));
        assert!(resisted.is_consistent_with_power(2));
        assert!(damaged.is_consistent_with_power(2));
        assert!(destroyed.is_consistent_with_power(2));

        let malformed = TerrainVoxelOutcome {
            health_after: Some(health(1, 4)),
            ..destroyed
        };
        assert!(!malformed.is_consistent_with_power(2));
    }

    #[test]
    fn applied_outcome_requires_the_exact_announced_order() {
        let first = at(0, 0, 1);
        let second = at(1, 0, 1);
        let impact = impact(vec![first, second], 2);
        let outcome = |pos| TerrainVoxelOutcome {
            pos,
            disposition: TerrainImpactDisposition::NoMaterial,
            before: None,
            after: None,
            health_before: None,
            health_after: None,
        };
        let exact = TerrainImpactOutcome {
            batch: impact.batch,
            result: TerrainImpactResult::Applied(vec![outcome(first), outcome(second)]),
        };
        assert!(exact.is_consistent_with(&impact));

        let reversed = TerrainImpactOutcome {
            batch: impact.batch,
            result: TerrainImpactResult::Applied(vec![outcome(second), outcome(first)]),
        };
        assert!(!reversed.is_consistent_with(&impact));
    }

    #[test]
    fn applied_outcome_rejects_every_volume_and_batch_mismatch() {
        let first = at(0, 0, 1);
        let second = at(1, 0, 1);
        let third = at(2, 0, 1);
        let substitute = at(3, 0, 1);
        let announced = impact(vec![first, second, third], 2);
        let exact = vec![
            no_material_outcome(first),
            no_material_outcome(second),
            no_material_outcome(third),
        ];

        let exact_answer = TerrainImpactOutcome {
            batch: announced.batch,
            result: TerrainImpactResult::Applied(exact.clone()),
        };
        assert!(exact_answer.is_consistent_with(&announced));

        let wrong_batch = TerrainImpactOutcome {
            batch: TerrainBatchId(99),
            result: TerrainImpactResult::Applied(exact.clone()),
        };
        assert!(!wrong_batch.is_consistent_with(&announced));

        let assert_mismatch = |case: &str, voxels: Vec<TerrainVoxelOutcome>| {
            let answer = TerrainImpactOutcome {
                batch: announced.batch,
                result: TerrainImpactResult::Applied(voxels),
            };
            assert!(
                !answer.is_consistent_with(&announced),
                "{case} positions were accepted"
            );
        };

        for omitted in 0..exact.len() {
            let voxels = exact
                .iter()
                .copied()
                .enumerate()
                .filter_map(|(index, outcome)| (index != omitted).then_some(outcome))
                .collect();
            assert_mismatch("missing", voxels);
        }

        for insertion in 0..=announced.volume.len() {
            let mut voxels = Vec::with_capacity(announced.volume.len() + 1);
            for (index, pos) in announced.volume.iter().copied().enumerate() {
                if index == insertion {
                    voxels.push(no_material_outcome(substitute));
                }
                voxels.push(no_material_outcome(pos));
            }
            if insertion == announced.volume.len() {
                voxels.push(no_material_outcome(substitute));
            }
            assert_mismatch("extra", voxels);
        }

        for order in [
            [first, third, second],
            [second, first, third],
            [second, third, first],
            [third, first, second],
            [third, second, first],
        ] {
            assert_mismatch(
                "reordered",
                order.into_iter().map(no_material_outcome).collect(),
            );
        }

        for (replaced, original) in announced.volume.iter().copied().enumerate() {
            for replacement in announced.volume.iter().copied() {
                if replacement == original {
                    continue;
                }
                let voxels = announced
                    .volume
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(index, pos)| {
                        no_material_outcome(if index == replaced { replacement } else { pos })
                    })
                    .collect();
                assert_mismatch("duplicated", voxels);
            }

            let voxels = announced
                .volume
                .iter()
                .copied()
                .enumerate()
                .map(|(index, pos)| {
                    no_material_outcome(if index == replaced { substitute } else { pos })
                })
                .collect();
            assert_mismatch("substituted", voxels);
        }
    }

    #[test]
    fn structurally_rejected_impacts_never_accept_applied_answers() {
        let first = at(0, 0, 1);
        let second = at(1, 0, 1);
        for (case, announced, voxels) in [
            ("empty", impact(Vec::new(), 1), Vec::new()),
            (
                "noncanonical order",
                impact(vec![second, first], 1),
                vec![no_material_outcome(second), no_material_outcome(first)],
            ),
            (
                "duplicate position",
                impact(vec![first, first], 1),
                vec![no_material_outcome(first), no_material_outcome(first)],
            ),
            (
                "zero power",
                impact(vec![first], 0),
                vec![no_material_outcome(first)],
            ),
        ] {
            let answer = TerrainImpactOutcome {
                batch: announced.batch,
                result: TerrainImpactResult::Applied(voxels),
            };
            assert!(
                !answer.is_consistent_with(&announced),
                "{case} impact accepted an applied answer"
            );
        }
    }

    #[test]
    fn voxel_outcome_validation_exhausts_material_and_health_semantics() {
        let pos = at(0, 0, 1);
        let materials = [
            None,
            Some(SubstanceId::AIR),
            Some(SubstanceId(3)),
            Some(SubstanceId(9)),
        ];
        let health_cases = health_semantic_cases();
        let dispositions = [
            TerrainImpactDisposition::NoMaterial,
            TerrainImpactDisposition::Resisted,
            TerrainImpactDisposition::Damaged,
            TerrainImpactDisposition::Destroyed,
        ];

        // Powers 0..=9 cover every threshold for the 1/2/4/8 health scale; MAX
        // proves values above the scale stay in the same semantic class as 9.
        for power in (0..=9).chain(std::iter::once(u8::MAX)) {
            let accepted = accepted_voxel_outcomes(pos, power);
            let announced = impact(vec![pos], power);
            for disposition in dispositions {
                for before in materials {
                    for after in materials {
                        for health_before in &health_cases {
                            for health_after in &health_cases {
                                let candidate = voxel_outcome(
                                    pos,
                                    disposition,
                                    before,
                                    after,
                                    *health_before,
                                    *health_after,
                                );
                                let expected = accepted.contains(&candidate);
                                assert_eq!(
                                    candidate.is_consistent_with_power(power),
                                    expected,
                                    "power={power}, candidate={candidate:?}"
                                );

                                let answer = TerrainImpactOutcome {
                                    batch: announced.batch,
                                    result: TerrainImpactResult::Applied(vec![candidate]),
                                };
                                assert_eq!(
                                    answer.is_consistent_with(&announced),
                                    power > 0 && expected,
                                    "integrated power={power}, candidate={candidate:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rejected_outcomes_acknowledge_structural_failures() {
        let empty_impact = impact(Vec::new(), 1);
        let outcome = TerrainImpactOutcome {
            batch: empty_impact.batch,
            result: TerrainImpactResult::Rejected(TerrainImpactRejection::EmptyVolume),
        };
        assert!(outcome.is_consistent_with(&empty_impact));

        let wrong = TerrainImpactOutcome {
            result: TerrainImpactResult::Rejected(TerrainImpactRejection::ZeroPower),
            ..outcome
        };
        assert!(!wrong.is_consistent_with(&empty_impact));

        let structurally_valid = impact(vec![at(0, 0, 1)], 1);
        let unknown_element = TerrainImpactOutcome {
            batch: structurally_valid.batch,
            result: TerrainImpactResult::Rejected(TerrainImpactRejection::UnknownElement),
        };
        assert!(unknown_element.is_consistent_with(&structurally_valid));
        assert!(!unknown_element.is_consistent_with(&empty_impact));

        let terrain_unavailable = TerrainImpactOutcome {
            batch: structurally_valid.batch,
            result: TerrainImpactResult::Rejected(TerrainImpactRejection::TerrainUnavailable),
        };
        assert!(terrain_unavailable.is_consistent_with(&structurally_valid));

        let reused = TerrainImpactOutcome {
            batch: empty_impact.batch,
            result: TerrainImpactResult::Rejected(TerrainImpactRejection::ReusedBatch),
        };
        assert!(reused.is_consistent_with(&empty_impact));
    }

    #[test]
    fn rejection_compatibility_is_exact_for_every_reason() {
        let first = at(0, 0, 1);
        let second = at(1, 0, 1);
        let reasons = [
            TerrainImpactRejection::EmptyVolume,
            TerrainImpactRejection::NonCanonicalVolume,
            TerrainImpactRejection::ZeroPower,
            TerrainImpactRejection::UnknownElement,
            TerrainImpactRejection::ReusedBatch,
            TerrainImpactRejection::TerrainUnavailable,
        ];
        let cases = [
            (
                "empty",
                impact(Vec::new(), 1),
                vec![
                    TerrainImpactRejection::EmptyVolume,
                    TerrainImpactRejection::ReusedBatch,
                ],
            ),
            (
                "empty precedes zero power",
                impact(Vec::new(), 0),
                vec![
                    TerrainImpactRejection::EmptyVolume,
                    TerrainImpactRejection::ReusedBatch,
                ],
            ),
            (
                "noncanonical",
                impact(vec![second, first], 1),
                vec![
                    TerrainImpactRejection::NonCanonicalVolume,
                    TerrainImpactRejection::ReusedBatch,
                ],
            ),
            (
                "noncanonical precedes zero power",
                impact(vec![second, first], 0),
                vec![
                    TerrainImpactRejection::NonCanonicalVolume,
                    TerrainImpactRejection::ReusedBatch,
                ],
            ),
            (
                "zero power",
                impact(vec![first], 0),
                vec![
                    TerrainImpactRejection::ZeroPower,
                    TerrainImpactRejection::ReusedBatch,
                ],
            ),
            (
                "structurally valid",
                impact(vec![first], 1),
                vec![
                    TerrainImpactRejection::UnknownElement,
                    TerrainImpactRejection::ReusedBatch,
                    TerrainImpactRejection::TerrainUnavailable,
                ],
            ),
        ];

        for (case, announced, compatible) in cases {
            for reason in reasons {
                let answer = TerrainImpactOutcome {
                    batch: announced.batch,
                    result: TerrainImpactResult::Rejected(reason),
                };
                assert_eq!(
                    answer.is_consistent_with(&announced),
                    compatible.contains(&reason),
                    "case={case}, reason={reason:?}"
                );

                let wrong_batch = TerrainImpactOutcome {
                    batch: TerrainBatchId(99),
                    result: TerrainImpactResult::Rejected(reason),
                };
                assert!(
                    !wrong_batch.is_consistent_with(&announced),
                    "case={case}, reason={reason:?} accepted a mismatched batch"
                );
            }
        }

        let first_use = impact(vec![first], 1);
        let structurally_different_reuse = impact(Vec::new(), 1);
        let reused = TerrainImpactOutcome {
            batch: first_use.batch,
            result: TerrainImpactResult::Rejected(TerrainImpactRejection::ReusedBatch),
        };
        assert!(reused.is_consistent_with(&first_use));
        assert!(reused.is_consistent_with(&structurally_different_reuse));
    }

    #[test]
    fn damaged_projection_keeps_only_partial_exact_positions() {
        let lower = at(0, 0, 1);
        let upper = at(0, 0, 5);
        let mut damaged = DamagedVoxels::new();
        damaged.publish(lower, health(2, 4));
        damaged.publish(upper, health(7, 8));
        assert_eq!(damaged.len(), 2);
        assert_eq!(damaged.get(lower), Some(health(2, 4)));
        assert_eq!(damaged.get(upper), Some(health(7, 8)));

        damaged.publish(lower, health(4, 4));
        assert_eq!(damaged.get(lower), None);
        assert_eq!(
            damaged.iter().collect::<Vec<_>>(),
            vec![(upper, health(7, 8))]
        );
    }
}
