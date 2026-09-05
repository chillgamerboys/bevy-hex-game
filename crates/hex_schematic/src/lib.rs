//! Pure, renderer-independent contracts for deterministic world schematics.
//!
//! The original modules preserve the frozen V3 coarse schematic contract.
//! [`v4`] compiles reusable authored regions into independent world packages.
//! Neither path depends on rendering, filesystem loading, or gameplay scheduling.

pub mod fingerprint;
pub mod generator;
pub mod metrics;
pub mod model;
pub mod template;
pub mod v4;
pub mod validate;

pub use fingerprint::semantic_fingerprint;
pub use generator::{
    generate, reference_plan, GeneratedSchematic, GenerationError, CANDIDATE_ATTEMPTS,
};
pub use metrics::{
    AccessCountsV1, BoundedRegionMetricsV1, ClimateCountsV1, LandformCountsV1, OverlayCountsV1,
    SchematicMetricsV1, SurfaceCountsV1, VegetationCountsV1,
};

pub use model::{
    bounded_envelope, canonical_cell_id, canonical_coordinate_index, canonical_coordinates,
    traced_twenty_percent_range, AccessIntent, BoundedRegionKind, BoundedRegionRule, BoundedTarget,
    CellFacts, CellId, CellPlan, CellProvenance, ClimateKind, CountRange, FeatureClaim,
    FeatureKind, GenerationSettings, LandformKind, LayerProvenance, ModelError, Network,
    NetworkEdge, NetworkKind, NetworkNode, NetworkNodeKind, OverlayProvenance, PercentRange,
    PlanProvenance, SchematicCoord, SchematicPlan, SchematicPlanParts, SchematicPlanV1,
    SchematicTemplate, SchematicTemplateV1, StableId, SurfaceKind, VegetationDensity,
    SCHEMATIC_CELL_COUNT, SCHEMATIC_RADIUS, SCHEMATIC_SCHEMA_VERSION,
};
pub use template::{grand_v3_reference_template, GRAND_V3_TEMPLATE_RON};
pub use validate::{
    validate_plan, validate_template, ValidationCode, ValidationError, ValidationIssue,
};
