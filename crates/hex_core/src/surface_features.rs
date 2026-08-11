//! Reserved, renderer-independent vocabulary for semantic surface features.
//!
//! This module defines the cross-owner data contract only. No runtime system in this
//! foundation publishes [`SurfaceFeatures`], processes [`PlaceSurfaceFeature`], or
//! renders a [`SurfaceFeature`]. A later world adapter will own admission and the
//! complete projection; gameplay will own request correlation and payment.

use std::cmp::Ordering;
use std::fmt;

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;
use serde::{Deserialize, Deserializer, Serialize};

use crate::TilePos;

/// Correlates one surface-feature placement request with its authoritative answer.
///
/// Allocated monotonically by the requester and meaningful only in the current
/// session. It is neither an authored identity nor a durable save identity.
#[derive(
    Reflect, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
#[serde(transparent)]
pub struct SurfaceFeatureBatchId(pub u64);

/// Stable identity of one semantic feature within the current active map.
///
/// A future authoritative world producer allocates these monotonically. The value is
/// not a Bevy [`Entity`], a generator-private id, or a durable Campaign identity.
#[derive(
    Reflect, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
#[serde(transparent)]
pub struct SurfaceFeatureId(pub u64);

/// Closed semantic identity of an authoritative surface feature.
///
/// Kinds deliberately carry no object asset, mesh, style, blocker, or presentation
/// metadata. Those remain projections owned by later runtime adapters.
#[derive(
    Reflect, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
pub enum SurfaceFeatureKind {
    /// Tall-grass semantics rooted on one exact material support.
    TallGrass,
}

/// One exact semantic consequence published by the authoritative world producer.
#[derive(
    Reflect, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
#[serde(deny_unknown_fields)]
pub struct SurfaceFeature {
    /// Stable identity within the active map.
    pub id: SurfaceFeatureId,
    /// Renderer-independent semantic kind.
    pub kind: SurfaceFeatureKind,
    /// Exact material surface voxel supporting the feature.
    pub support: TilePos,
}

/// Requests one semantic feature at one exact support.
///
/// This message derives the runtime transport vocabulary without installing a sender,
/// receiver, or schedule in this foundation.
#[derive(Message, Reflect, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlaceSurfaceFeature {
    /// Session-local correlation echoed by the outcome.
    pub batch: SurfaceFeatureBatchId,
    /// Semantic feature requested.
    pub kind: SurfaceFeatureKind,
    /// Exact material surface voxel requested as support.
    pub support: TilePos,
}

/// Why a future authoritative producer rejected a processed request.
///
/// Derived ordering is the required rejection precedence. The first processed use of
/// a batch id will consume it whether placement is applied or rejected.
#[derive(
    Reflect, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
#[repr(u8)]
pub enum SurfaceFeaturePlacementRejection {
    /// The session-local batch id was already consumed.
    ReusedBatch = 0,
    /// No complete active terrain projection was available.
    TerrainUnavailable = 1,
    /// The active producer does not admit this semantic kind.
    UnsupportedKind = 2,
    /// The exact support is not valid for the requested kind.
    InvalidSupport = 3,
    /// Another authoritative feature conflicts with this placement.
    FeatureConflict = 4,
}

impl SurfaceFeaturePlacementRejection {
    /// Rejection reasons in their required authoritative precedence.
    pub const PRECEDENCE: [Self; 5] = [
        Self::ReusedBatch,
        Self::TerrainUnavailable,
        Self::UnsupportedKind,
        Self::InvalidSupport,
        Self::FeatureConflict,
    ];

    /// Zero-based authoritative precedence; lower values win.
    #[must_use]
    pub const fn precedence(self) -> u8 {
        self as u8
    }
}

/// The complete result of one processed placement request.
///
/// The enum makes an answer that is both applied and rejected unrepresentable. An
/// applied result contains the complete feature record; a rejected result contains no
/// feature record.
#[derive(Reflect, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
pub enum SurfaceFeaturePlacementResult {
    /// The complete record inserted into the next valid projection.
    Applied(SurfaceFeature),
    /// No feature was inserted.
    Rejected(SurfaceFeaturePlacementRejection),
}

/// Exactly one authoritative answer to one placement request.
#[derive(Message, Reflect, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SurfaceFeaturePlacementOutcome {
    /// Session-local correlation copied from the processed request.
    pub batch: SurfaceFeatureBatchId,
    /// Applied feature or one closed rejection.
    pub result: SurfaceFeaturePlacementResult,
}

impl SurfaceFeaturePlacementOutcome {
    /// Validates structural identity against the request this answer claims to settle.
    ///
    /// Producer-owned terrain, support, conflict, and kind-admission policy is
    /// deliberately outside this helper. Rejections therefore need only the exact
    /// batch; applied records must also preserve the requested kind and support.
    pub fn validate_for(
        &self,
        request: &PlaceSurfaceFeature,
    ) -> Result<(), SurfaceFeatureValidationError> {
        if self.batch != request.batch {
            return Err(SurfaceFeatureValidationError::MismatchedOutcomeBatch {
                expected: request.batch,
                actual: self.batch,
            });
        }

        if let SurfaceFeaturePlacementResult::Applied(feature) = self.result {
            if feature.kind != request.kind {
                return Err(SurfaceFeatureValidationError::AppliedKindMismatch {
                    expected: request.kind,
                    actual: feature.kind,
                });
            }
            if feature.support != request.support {
                return Err(SurfaceFeatureValidationError::AppliedSupportMismatch {
                    expected: request.support,
                    actual: feature.support,
                });
            }
        }

        Ok(())
    }

    /// Whether this outcome is structurally compatible with one request.
    #[must_use]
    pub fn is_consistent_with(&self, request: &PlaceSurfaceFeature) -> bool {
        self.validate_for(request).is_ok()
    }

    /// Validates this answer and the producer's next complete projection together.
    ///
    /// Applied answers must appear there as the exact same complete record. Rejected
    /// answers contain no feature and impose no projection membership requirement.
    pub fn validate_for_projection(
        &self,
        request: &PlaceSurfaceFeature,
        projection: &SurfaceFeatures,
    ) -> Result<(), SurfaceFeatureValidationError> {
        self.validate_for(request)?;
        projection.validate()?;
        let SurfaceFeaturePlacementResult::Applied(feature) = self.result else {
            return Ok(());
        };

        match projection.get(feature.id) {
            None => {
                Err(SurfaceFeatureValidationError::AppliedFeatureMissingFromProjection(feature.id))
            }
            Some(_) if !projection.contains_exact(feature) => {
                Err(SurfaceFeatureValidationError::AppliedFeatureProjectionMismatch(feature.id))
            }
            Some(_) => Ok(()),
        }
    }
}

/// Typed structural failures in the reserved surface-feature contract.
#[derive(Reflect, Debug, Copy, Clone, PartialEq, Eq)]
pub enum SurfaceFeatureValidationError {
    /// Two complete projection records use the same stable feature id.
    DuplicateFeatureId(SurfaceFeatureId),
    /// Serialized projection records were not strictly ordered by stable id.
    NonCanonicalFeatureOrder {
        /// Earlier id on the wire.
        previous: SurfaceFeatureId,
        /// Following id that did not sort after `previous`.
        next: SurfaceFeatureId,
    },
    /// A pending request had no first outcome.
    MissingOutcome(SurfaceFeatureBatchId),
    /// A pending request had more than one first outcome.
    DuplicateOutcome {
        /// Batch whose obligation received duplicate answers.
        batch: SurfaceFeatureBatchId,
        /// Number of answers presented for the obligation.
        count: usize,
    },
    /// An outcome echoed a different request correlation.
    MismatchedOutcomeBatch {
        /// Batch required by the pending request.
        expected: SurfaceFeatureBatchId,
        /// Batch carried by the outcome.
        actual: SurfaceFeatureBatchId,
    },
    /// An applied record changed the requested semantic kind.
    ///
    /// With V1's single closed `TallGrass` kind, an unequal typed pair cannot be
    /// constructed. The branch remains explicit so adding a future kind cannot make
    /// request/outcome correlation silently permissive.
    AppliedKindMismatch {
        /// Semantic kind required by the request.
        expected: SurfaceFeatureKind,
        /// Semantic kind carried by the applied record.
        actual: SurfaceFeatureKind,
    },
    /// An applied record changed the requested exact support.
    AppliedSupportMismatch {
        /// Exact support required by the request.
        expected: TilePos,
        /// Exact support carried by the applied record.
        actual: TilePos,
    },
    /// The next complete projection omitted an applied feature id.
    AppliedFeatureMissingFromProjection(SurfaceFeatureId),
    /// The next complete projection reused the id for a different record.
    AppliedFeatureProjectionMismatch(SurfaceFeatureId),
}

impl fmt::Display for SurfaceFeatureValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateFeatureId(id) => {
                write!(formatter, "duplicate surface feature id {id:?}")
            }
            Self::NonCanonicalFeatureOrder { previous, next } => write!(
                formatter,
                "surface feature ids are not in canonical order: {previous:?} then {next:?}"
            ),
            Self::MissingOutcome(batch) => {
                write!(formatter, "surface feature batch {batch:?} has no outcome")
            }
            Self::DuplicateOutcome { batch, count } => write!(
                formatter,
                "surface feature batch {batch:?} has {count} first outcomes"
            ),
            Self::MismatchedOutcomeBatch { expected, actual } => write!(
                formatter,
                "surface feature outcome batch {actual:?} does not match {expected:?}"
            ),
            Self::AppliedKindMismatch { expected, actual } => write!(
                formatter,
                "applied surface feature kind {actual:?} does not match {expected:?}"
            ),
            Self::AppliedSupportMismatch { expected, actual } => write!(
                formatter,
                "applied surface feature support {actual:?} does not match {expected:?}"
            ),
            Self::AppliedFeatureMissingFromProjection(id) => write!(
                formatter,
                "applied surface feature {id:?} is missing from the complete projection"
            ),
            Self::AppliedFeatureProjectionMismatch(id) => write!(
                formatter,
                "complete projection record {id:?} differs from the applied feature"
            ),
        }
    }
}

impl std::error::Error for SurfaceFeatureValidationError {}

/// Requires exactly one structurally matching first answer for one pending request.
///
/// This is a pure validation helper, not an outcome inbox. A future consumer remains
/// responsible for presenting only the answers attached to one pending obligation.
pub fn validate_surface_feature_outcomes<'a>(
    request: &PlaceSurfaceFeature,
    outcomes: &'a [SurfaceFeaturePlacementOutcome],
) -> Result<&'a SurfaceFeaturePlacementOutcome, SurfaceFeatureValidationError> {
    match outcomes {
        [] => Err(SurfaceFeatureValidationError::MissingOutcome(request.batch)),
        [outcome] => {
            outcome.validate_for(request)?;
            Ok(outcome)
        }
        _ => Err(SurfaceFeatureValidationError::DuplicateOutcome {
            batch: request.batch,
            count: outcomes.len(),
        }),
    }
}

/// Complete deterministic projection of current semantic surface features.
///
/// Records iterate in stable feature-id order. Exact-support lookup retains the
/// complete [`TilePos`], so stacked ground, bridge, and cave surfaces at one
/// horizontal coordinate remain independent. The generic projection permits several
/// records at one exact support; a future producer owns conflict policy.
///
/// A present empty value means an authoritative map is ready and has no semantic
/// features once a consumer requires this resource. Absence remains distinct and
/// means unavailable. This foundation installs neither value.
#[derive(Resource, Reflect, Serialize, Debug, Default, Clone, PartialEq, Eq)]
#[reflect(Resource)]
#[serde(transparent)]
pub struct SurfaceFeatures {
    features: Vec<SurfaceFeature>,
}

impl SurfaceFeatures {
    /// Creates a valid empty projection.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            features: Vec::new(),
        }
    }

    /// Builds a canonical projection from records in any insertion order.
    ///
    /// Duplicate stable ids are rejected rather than silently replacing one record.
    pub fn from_features(
        features: impl IntoIterator<Item = SurfaceFeature>,
    ) -> Result<Self, SurfaceFeatureValidationError> {
        let mut features: Vec<_> = features.into_iter().collect();
        features.sort_unstable_by_key(|feature| feature.id);
        Self::from_canonical_features(features)
    }

    /// Validates deterministic id order and uniqueness.
    pub fn validate(&self) -> Result<(), SurfaceFeatureValidationError> {
        validate_canonical_features(&self.features)
    }

    /// Finds one record by stable feature id.
    #[must_use]
    pub fn get(&self, id: SurfaceFeatureId) -> Option<SurfaceFeature> {
        let index = self
            .features
            .binary_search_by_key(&id, |feature| feature.id)
            .ok()?;
        self.features.get(index).copied()
    }

    /// Whether the projection contains this complete id/kind/support record.
    #[must_use]
    pub fn contains_exact(&self, feature: SurfaceFeature) -> bool {
        self.get(feature.id) == Some(feature)
    }

    /// Iterates complete records in stable feature-id order.
    pub fn iter(&self) -> impl Iterator<Item = SurfaceFeature> + '_ {
        self.features.iter().copied()
    }

    /// Iterates records rooted on one exact support in stable feature-id order.
    pub fn at_support(&self, support: TilePos) -> impl Iterator<Item = SurfaceFeature> + '_ {
        self.features
            .iter()
            .copied()
            .filter(move |feature| feature.support == support)
    }

    /// Number of published semantic records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.features.len()
    }

    /// Whether this valid projection currently contains no semantic records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.features.is_empty()
    }

    fn from_canonical_features(
        features: Vec<SurfaceFeature>,
    ) -> Result<Self, SurfaceFeatureValidationError> {
        validate_canonical_features(&features)?;
        Ok(Self { features })
    }
}

impl<'de> Deserialize<'de> for SurfaceFeatures {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let features = Vec::<SurfaceFeature>::deserialize(deserializer)?;
        Self::from_canonical_features(features).map_err(serde::de::Error::custom)
    }
}

fn validate_canonical_features(
    features: &[SurfaceFeature],
) -> Result<(), SurfaceFeatureValidationError> {
    for pair in features.windows(2) {
        let [previous, next] = pair else {
            continue;
        };
        match previous.id.cmp(&next.id) {
            Ordering::Less => {}
            Ordering::Equal => {
                return Err(SurfaceFeatureValidationError::DuplicateFeatureId(
                    previous.id,
                ));
            }
            Ordering::Greater => {
                return Err(SurfaceFeatureValidationError::NonCanonicalFeatureOrder {
                    previous: previous.id,
                    next: next.id,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HexCoord;
    use bevy_ecs::world::World;

    fn support(q: i32, r: i32, level: crate::Level) -> TilePos {
        TilePos::new(HexCoord::from_axial(q, r), level)
    }

    fn feature(id: u64, support: TilePos) -> SurfaceFeature {
        SurfaceFeature {
            id: SurfaceFeatureId(id),
            kind: SurfaceFeatureKind::TallGrass,
            support,
        }
    }

    fn request() -> PlaceSurfaceFeature {
        PlaceSurfaceFeature {
            batch: SurfaceFeatureBatchId(7),
            kind: SurfaceFeatureKind::TallGrass,
            support: support(2, -1, 3),
        }
    }

    fn applied(request: &PlaceSurfaceFeature, id: u64) -> SurfaceFeaturePlacementOutcome {
        SurfaceFeaturePlacementOutcome {
            batch: request.batch,
            result: SurfaceFeaturePlacementResult::Applied(SurfaceFeature {
                id: SurfaceFeatureId(id),
                kind: request.kind,
                support: request.support,
            }),
        }
    }

    fn round_trip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + fmt::Debug,
    {
        let encoded = serde_json::to_string(value).expect("contract value serializes");
        let decoded: T = serde_json::from_str(&encoded).expect("contract value deserializes");
        assert_eq!(&decoded, value);
    }

    #[test]
    fn every_reserved_value_round_trips_exactly() {
        let request = request();
        let record = feature(9, request.support);
        round_trip(&SurfaceFeatureBatchId(7));
        round_trip(&SurfaceFeatureId(9));
        round_trip(&SurfaceFeatureKind::TallGrass);
        round_trip(&record);
        round_trip(&request);
        round_trip(&SurfaceFeaturePlacementResult::Applied(record));
        for rejection in SurfaceFeaturePlacementRejection::PRECEDENCE {
            round_trip(&rejection);
            round_trip(&SurfaceFeaturePlacementOutcome {
                batch: request.batch,
                result: SurfaceFeaturePlacementResult::Rejected(rejection),
            });
        }
        round_trip(&SurfaceFeatures::from_features([record]).expect("unique projection"));
    }

    #[test]
    fn public_records_contain_only_the_reserved_semantic_fields() {
        let request = request();
        let record = feature(9, request.support);

        let SurfaceFeature { id, kind, support } = record;
        let _: SurfaceFeatureId = id;
        let _: SurfaceFeatureKind = kind;
        let _: TilePos = support;

        let PlaceSurfaceFeature {
            batch,
            kind,
            support,
        } = request;
        let _: SurfaceFeatureBatchId = batch;
        let _: SurfaceFeatureKind = kind;
        let _: TilePos = support;

        match kind {
            SurfaceFeatureKind::TallGrass => {}
        }
        match SurfaceFeaturePlacementResult::Applied(record) {
            SurfaceFeaturePlacementResult::Applied(applied) => assert_eq!(applied, record),
            SurfaceFeaturePlacementResult::Rejected(_) => {
                unreachable!("fixture result is applied")
            }
        }
    }

    #[test]
    fn wire_shapes_are_canonical_and_contain_only_semantic_facts() {
        let request = request();
        let record = feature(9, request.support);
        assert_eq!(
            serde_json::to_string(&record).expect("record serializes"),
            r#"{"id":9,"kind":"TallGrass","support":{"coord":{"q":2,"r":-1},"level":3}}"#
        );
        assert_eq!(
            serde_json::to_string(&request).expect("request serializes"),
            r#"{"batch":7,"kind":"TallGrass","support":{"coord":{"q":2,"r":-1},"level":3}}"#
        );
        assert_eq!(
            serde_json::to_string(&applied(&request, 9)).expect("outcome serializes"),
            r#"{"batch":7,"result":{"Applied":{"id":9,"kind":"TallGrass","support":{"coord":{"q":2,"r":-1},"level":3}}}}"#
        );
        assert_eq!(
            serde_json::to_string(&SurfaceFeatures::new()).expect("empty projection serializes"),
            "[]"
        );
    }

    #[test]
    fn invalid_and_noncanonical_wire_representations_fail_closed() {
        // V1 has exactly one closed kind, so a typed kind-mismatch fixture is
        // unrepresentable. Every non-TallGrass kind instead fails at deserialization;
        // `validate_for` already has the explicit unequal-kind branch for expansion.
        assert!(serde_json::from_str::<SurfaceFeatureKind>(r#""Shrub""#).is_err());
        assert!(serde_json::from_str::<SurfaceFeatureId>(r#""9""#).is_err());

        let request_with_asset = r#"{"batch":7,"kind":"TallGrass","support":{"coord":{"q":2,"r":-1},"level":3},"asset":"prop/grass"}"#;
        assert!(serde_json::from_str::<PlaceSurfaceFeature>(request_with_asset).is_err());

        let record = feature(9, support(2, -1, 3));
        let record_json = serde_json::to_string(&record).expect("record serializes");
        let both_results = format!(
            r#"{{"batch":7,"result":{{"Applied":{record_json},"Rejected":"FeatureConflict"}}}}"#
        );
        assert!(
            serde_json::from_str::<SurfaceFeaturePlacementOutcome>(&both_results).is_err(),
            "an outcome cannot be both applied and rejected"
        );

        let first = feature(1, support(0, 0, 1));
        let second = feature(2, support(1, 0, 1));
        let reversed = serde_json::to_string(&vec![second, first]).expect("fixture serializes");
        assert!(serde_json::from_str::<SurfaceFeatures>(&reversed).is_err());
        let duplicate = serde_json::to_string(&vec![first, first]).expect("fixture serializes");
        assert!(serde_json::from_str::<SurfaceFeatures>(&duplicate).is_err());
    }

    #[test]
    fn projection_order_is_deterministic_and_duplicate_ids_are_typed_errors() {
        let first = feature(1, support(0, 0, 1));
        let second = feature(2, support(1, 0, 1));
        let third = feature(3, support(2, 0, 1));
        let expected = SurfaceFeatures::from_features([first, second, third])
            .expect("unique records form a projection");

        for insertion_order in [
            [third, first, second],
            [second, third, first],
            [second, first, third],
        ] {
            let actual = SurfaceFeatures::from_features(insertion_order)
                .expect("constructor canonicalizes insertion order");
            assert_eq!(actual, expected);
            assert_eq!(
                actual.iter().map(|record| record.id).collect::<Vec<_>>(),
                vec![
                    SurfaceFeatureId(1),
                    SurfaceFeatureId(2),
                    SurfaceFeatureId(3)
                ]
            );
        }

        assert_eq!(
            SurfaceFeatures::from_features([first, feature(1, support(4, 0, 9))]),
            Err(SurfaceFeatureValidationError::DuplicateFeatureId(
                SurfaceFeatureId(1)
            ))
        );
    }

    #[test]
    fn exact_support_lookup_never_collapses_stacked_surfaces() {
        let coord = HexCoord::from_axial(4, -2);
        let cave = TilePos::new(coord, -3);
        let ground = TilePos::new(coord, 1);
        let bridge = TilePos::new(coord, 8);
        let records = SurfaceFeatures::from_features([
            feature(4, bridge),
            feature(2, cave),
            feature(3, ground),
            feature(1, ground),
        ])
        .expect("levels and same-support records remain independent");

        assert_eq!(
            records.at_support(cave).collect::<Vec<_>>(),
            vec![feature(2, cave)]
        );
        assert_eq!(
            records.at_support(ground).collect::<Vec<_>>(),
            vec![feature(1, ground), feature(3, ground)]
        );
        assert_eq!(
            records.at_support(bridge).collect::<Vec<_>>(),
            vec![feature(4, bridge)]
        );
        assert_eq!(records.get(SurfaceFeatureId(4)), Some(feature(4, bridge)));
        assert!(records.contains_exact(feature(4, bridge)));
        assert!(!records.contains_exact(feature(4, ground)));
        assert_eq!(records.get(SurfaceFeatureId(99)), None);
    }

    #[test]
    fn applied_answers_retain_request_identity_and_rejections_have_no_record() {
        let request = request();
        let exact = applied(&request, 11);
        assert!(exact.is_consistent_with(&request));

        let wrong_batch = SurfaceFeaturePlacementOutcome {
            batch: SurfaceFeatureBatchId(8),
            ..exact
        };
        assert_eq!(
            wrong_batch.validate_for(&request),
            Err(SurfaceFeatureValidationError::MismatchedOutcomeBatch {
                expected: request.batch,
                actual: SurfaceFeatureBatchId(8),
            })
        );

        let wrong_support = SurfaceFeaturePlacementOutcome {
            result: SurfaceFeaturePlacementResult::Applied(feature(11, support(2, -1, 4))),
            ..exact
        };
        assert_eq!(
            wrong_support.validate_for(&request),
            Err(SurfaceFeatureValidationError::AppliedSupportMismatch {
                expected: request.support,
                actual: support(2, -1, 4),
            })
        );

        for rejection in SurfaceFeaturePlacementRejection::PRECEDENCE {
            let rejected = SurfaceFeaturePlacementOutcome {
                batch: request.batch,
                result: SurfaceFeaturePlacementResult::Rejected(rejection),
            };
            assert!(rejected.is_consistent_with(&request));
            assert!(matches!(
                rejected.result,
                SurfaceFeaturePlacementResult::Rejected(_)
            ));
        }
    }

    #[test]
    fn applied_answer_must_appear_exactly_in_the_next_complete_projection() {
        let request = request();
        let exact = applied(&request, 11);
        let record = feature(11, request.support);
        let projection = SurfaceFeatures::from_features([record]).expect("valid projection");
        assert_eq!(exact.validate_for_projection(&request, &projection), Ok(()));

        assert_eq!(
            exact.validate_for_projection(&request, &SurfaceFeatures::new()),
            Err(
                SurfaceFeatureValidationError::AppliedFeatureMissingFromProjection(
                    SurfaceFeatureId(11)
                )
            )
        );

        let different = SurfaceFeatures::from_features([feature(11, support(3, -1, 3))])
            .expect("projection is internally valid");
        assert_eq!(
            exact.validate_for_projection(&request, &different),
            Err(
                SurfaceFeatureValidationError::AppliedFeatureProjectionMismatch(SurfaceFeatureId(
                    11
                ))
            )
        );

        let rejected = SurfaceFeaturePlacementOutcome {
            batch: request.batch,
            result: SurfaceFeaturePlacementResult::Rejected(
                SurfaceFeaturePlacementRejection::FeatureConflict,
            ),
        };
        assert_eq!(
            rejected.validate_for_projection(&request, &SurfaceFeatures::new()),
            Ok(())
        );
    }

    #[test]
    fn one_pending_obligation_requires_exactly_one_first_outcome() {
        let request = request();
        let exact = applied(&request, 11);
        assert_eq!(
            validate_surface_feature_outcomes(&request, &[exact]),
            Ok(&exact)
        );
        assert_eq!(
            validate_surface_feature_outcomes(&request, &[]),
            Err(SurfaceFeatureValidationError::MissingOutcome(request.batch))
        );
        assert_eq!(
            validate_surface_feature_outcomes(&request, &[exact, exact]),
            Err(SurfaceFeatureValidationError::DuplicateOutcome {
                batch: request.batch,
                count: 2,
            })
        );
    }

    #[test]
    fn rejection_order_is_the_required_precedence() {
        let mut reasons = [
            SurfaceFeaturePlacementRejection::FeatureConflict,
            SurfaceFeaturePlacementRejection::InvalidSupport,
            SurfaceFeaturePlacementRejection::UnsupportedKind,
            SurfaceFeaturePlacementRejection::TerrainUnavailable,
            SurfaceFeaturePlacementRejection::ReusedBatch,
        ];
        reasons.sort_unstable();
        assert_eq!(reasons, SurfaceFeaturePlacementRejection::PRECEDENCE);
        for (expected, reason) in (0_u8..).zip(reasons) {
            assert_eq!(reason.precedence(), expected);
        }
    }

    #[test]
    fn present_empty_and_missing_projections_are_distinct() {
        let mut world = World::new();
        assert!(world.get_resource::<SurfaceFeatures>().is_none());

        world.insert_resource(SurfaceFeatures::new());
        assert!(world
            .get_resource::<SurfaceFeatures>()
            .is_some_and(|projection| projection.is_empty()));
    }
}
