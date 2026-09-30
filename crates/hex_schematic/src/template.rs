//! Packaged Grand V3 reference-template loading.

use crate::SchematicTemplateV1;

/// Exact packaged Grand V3 template RON.
pub const GRAND_V3_TEMPLATE_RON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/config/schematics/grand-v3-template.ron"
));

/// Parses and structurally validates the packaged Grand V3 reference template.
pub fn grand_v3_reference_template() -> Result<SchematicTemplateV1, ron::error::SpannedError> {
    ron::from_str(GRAND_V3_TEMPLATE_RON)
}
