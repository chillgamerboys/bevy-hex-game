//! World-owned elemental admission for voxel damage.
//!
//! `assets/config/terrain_damage.ron` is intentionally a Boolean allow-list. Spell
//! content announces an element and power; this table answers only whether that
//! element may damage that material. Toughness and all mutation policy remain with
//! the world.

use std::collections::BTreeSet;

use bevy::prelude::*;
use hex_core::{ElementId, Screen, SubstanceId};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use thiserror::Error;

use crate::elements::ElementCatalog;
use crate::fingerprint::FingerprintEncoder;
use crate::substances::SubstanceTable;
use crate::{LoadSettings, CONFIG_EXTENSIONS};

/// One stable-name `(element, substance)` damage admission.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerrainDamagePair {
    /// Stable element name from `elements.ron`.
    pub element: String,
    /// Stable substance name from `substances.ron`.
    pub substance: String,
}

/// The raw Boolean terrain-damage allow-list.
///
/// Deserialization rejects duplicate stable-name pairs before the file can replace
/// the last valid resource. Cross-file references are resolved by
/// [`TerrainDamageTable::build`].
#[derive(Asset, Resource, Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(Resource)]
pub struct TerrainDamageFile {
    /// Pairs that permit damage. Every absent pair resists.
    pub damaging_pairs: Vec<TerrainDamagePair>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedTerrainDamageFile {
    damaging_pairs: Vec<TerrainDamagePair>,
}

impl<'de> Deserialize<'de> for TerrainDamageFile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedTerrainDamageFile::deserialize(deserializer)?;
        let file = Self {
            damaging_pairs: raw.damaging_pairs,
        };
        file.validate_duplicates().map_err(D::Error::custom)?;
        Ok(file)
    }
}

impl TerrainDamageFile {
    fn validate_duplicates(&self) -> Result<(), TerrainDamageError> {
        let mut seen = BTreeSet::new();
        for pair in &self.damaging_pairs {
            if !seen.insert((pair.element.as_str(), pair.substance.as_str())) {
                return Err(TerrainDamageError::DuplicatePair {
                    element: pair.element.clone(),
                    substance: pair.substance.clone(),
                });
            }
        }
        Ok(())
    }

    fn semantic_fingerprint(&self) -> u64 {
        let mut pairs: Vec<_> = self.damaging_pairs.iter().collect();
        pairs.sort();

        let mut encoder = FingerprintEncoder::new(b"hex-terrain-damage-file-v1");
        encoder.usize(pairs.len());
        for pair in pairs {
            encoder.string(&pair.element);
            encoder.string(&pair.substance);
        }
        encoder.finish()
    }
}

/// Why a terrain-damage allow-list could not be resolved.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TerrainDamageError {
    /// The raw allow-list repeated an exact stable-name pair.
    #[error(
        "terrain damage lists element '{element}' against substance '{substance}' more than once"
    )]
    DuplicatePair {
        /// Repeated element name.
        element: String,
        /// Repeated substance name.
        substance: String,
    },
    /// A pair names no current authored element.
    #[error("terrain damage references unknown element '{element}'")]
    UnknownElement {
        /// Missing element name.
        element: String,
    },
    /// A pair names no current authored substance.
    #[error("terrain damage references unknown substance '{substance}'")]
    UnknownSubstance {
        /// Missing substance name.
        substance: String,
    },
    /// A pair names a substance that has no maximum health.
    #[error("terrain damage references indestructible substance '{substance}'")]
    IndestructibleSubstance {
        /// Substance with `toughness: None`.
        substance: String,
    },
}

/// Resolved Boolean damage admission indexed by transient runtime ids.
#[derive(Resource, Reflect, Debug, Clone, Default)]
#[reflect(Resource)]
pub struct TerrainDamageTable {
    #[reflect(ignore)]
    damaging_pairs: BTreeSet<(ElementId, SubstanceId)>,
    #[reflect(ignore)]
    source_file: Option<TerrainDamageFile>,
    source_file_fingerprint: u64,
    source_elements: u64,
    source_substances: u64,
}

impl TerrainDamageTable {
    /// Resolves every stable-name pair through the current element and substance
    /// tables, returning every cross-file failure found.
    pub fn from_file(
        file: &TerrainDamageFile,
        elements: &ElementCatalog,
        substances: &SubstanceTable,
    ) -> Result<Self, Vec<TerrainDamageError>> {
        let mut errors = Vec::new();
        if let Err(error) = file.validate_duplicates() {
            errors.push(error);
        }

        let mut damaging_pairs = BTreeSet::new();
        for pair in &file.damaging_pairs {
            let element = elements.id(&pair.element);
            if element.is_none() {
                errors.push(TerrainDamageError::UnknownElement {
                    element: pair.element.clone(),
                });
            }

            let substance = substances.id(&pair.substance);
            match substance {
                None => errors.push(TerrainDamageError::UnknownSubstance {
                    substance: pair.substance.clone(),
                }),
                Some(substance) if substances.toughness(substance).is_none() => {
                    errors.push(TerrainDamageError::IndestructibleSubstance {
                        substance: pair.substance.clone(),
                    });
                }
                Some(_) => {}
            }

            if let (Some(element), Some(substance)) = (element, substance) {
                if substances.toughness(substance).is_some() {
                    damaging_pairs.insert((element, substance));
                }
            }
        }

        if errors.is_empty() {
            Ok(Self {
                damaging_pairs,
                source_file: Some(file.clone()),
                source_file_fingerprint: file.semantic_fingerprint(),
                source_elements: elements.source_fingerprint(),
                source_substances: substances.semantic_fingerprint(),
            })
        } else {
            Err(errors)
        }
    }

    /// Alias for [`Self::from_file`] used by cross-file content builders.
    pub fn build(
        file: &TerrainDamageFile,
        elements: &ElementCatalog,
        substances: &SubstanceTable,
    ) -> Result<Self, Vec<TerrainDamageError>> {
        Self::from_file(file, elements, substances)
    }

    /// Whether this element damages this substance.
    ///
    /// Unknown ids and every pair absent from the authored allow-list resist.
    #[must_use]
    pub fn damages(&self, element: ElementId, substance: SubstanceId) -> bool {
        self.damaging_pairs.contains(&(element, substance))
    }

    /// Number of admitted element/material pairs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.damaging_pairs.len()
    }

    /// Whether no element/material pair permits damage.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.damaging_pairs.is_empty()
    }

    /// Whether this table was resolved from these exact current source semantics.
    #[must_use]
    pub fn matches_sources(
        &self,
        file: &TerrainDamageFile,
        elements: &ElementCatalog,
        substances: &SubstanceTable,
    ) -> bool {
        self.matches_file(file)
            && self.source_elements == elements.source_fingerprint()
            && self.source_substances == substances.semantic_fingerprint()
    }

    fn matches_file(&self, file: &TerrainDamageFile) -> bool {
        self.source_file_fingerprint == file.semantic_fingerprint()
    }

    pub(crate) fn source_revision(&self) -> u64 {
        let mut encoder = FingerprintEncoder::new(b"hex-terrain-damage-table-v1");
        encoder.u64(self.source_file_fingerprint);
        encoder.u64(self.source_elements);
        encoder.u64(self.source_substances);
        encoder.finish()
    }

    fn accepted_source(&self) -> Option<TerrainDamageFile> {
        self.source_file.clone()
    }
}

/// One rejected cross-file candidate, retained so a repair in another source file can
/// complete it without requiring the designer to touch `terrain_damage.ron` again.
#[derive(Resource, Debug, Clone)]
struct FailedTerrainDamageBuild {
    file: TerrainDamageFile,
    source_elements: u64,
    source_substances: u64,
}

impl FailedTerrainDamageBuild {
    fn new(
        file: &TerrainDamageFile,
        elements: &ElementCatalog,
        substances: &SubstanceTable,
    ) -> Self {
        Self {
            file: file.clone(),
            source_elements: elements.source_fingerprint(),
            source_substances: substances.semantic_fingerprint(),
        }
    }

    fn matches_sources(
        &self,
        file: &TerrainDamageFile,
        elements: &ElementCatalog,
        substances: &SubstanceTable,
    ) -> bool {
        self.file.semantic_fingerprint() == file.semantic_fingerprint()
            && self.source_elements == elements.source_fingerprint()
            && self.source_substances == substances.semantic_fingerprint()
    }
}

/// Marks a raw-resource reinsertion performed by the recovery path rather than a new
/// asset edit.
#[derive(Resource, Debug, Clone, Copy)]
struct AcceptedTerrainDamageSourceRestoration;

/// Registers the world-owned terrain-damage content and its resolved table.
pub fn plugin(app: &mut App) {
    app.register_type::<TerrainDamagePair>()
        .register_type::<TerrainDamageFile>()
        .register_type::<TerrainDamageTable>();
    app.load_settings::<TerrainDamageFile>("config/terrain_damage.ron", CONFIG_EXTENSIONS);
    register_table_builder(app);
}

fn register_table_builder(app: &mut App) {
    app.add_systems(
        Update,
        build_table_when_loaded.run_if(not(in_state(Screen::Gameplay))),
    );
}

fn build_table_when_loaded(
    mut commands: Commands,
    file: Option<Res<TerrainDamageFile>>,
    elements: Option<Res<ElementCatalog>>,
    substances: Option<Res<SubstanceTable>>,
    table: Option<Res<TerrainDamageTable>>,
    failed: Option<Res<FailedTerrainDamageBuild>>,
    restoration: Option<Res<AcceptedTerrainDamageSourceRestoration>>,
) {
    let (Some(file), Some(elements), Some(substances)) = (file, elements, substances) else {
        return;
    };

    if table
        .as_deref()
        .is_some_and(|table| table.matches_sources(&file, &elements, &substances))
    {
        let is_recovery_write = restoration
            .as_ref()
            .is_some_and(|marker| marker.last_changed() == file.last_changed());
        if failed.is_some() && file.is_changed() && !is_recovery_write {
            commands.remove_resource::<FailedTerrainDamageBuild>();
            commands.remove_resource::<AcceptedTerrainDamageSourceRestoration>();
        } else if failed.is_none() && restoration.is_some() {
            commands.remove_resource::<AcceptedTerrainDamageSourceRestoration>();
        }
        return;
    }

    let candidate = match (table.as_deref(), failed.as_deref()) {
        (Some(table), Some(failed)) if table.matches_file(&file) => failed.file.clone(),
        _ => file.as_ref().clone(),
    };

    if failed
        .as_deref()
        .is_some_and(|failed| failed.matches_sources(&candidate, &elements, &substances))
    {
        if let Some(accepted) = table
            .as_deref()
            .filter(|table| !table.matches_file(&file))
            .and_then(TerrainDamageTable::accepted_source)
        {
            commands.insert_resource(accepted);
            commands.insert_resource(AcceptedTerrainDamageSourceRestoration);
        }
        return;
    }

    match TerrainDamageTable::build(&candidate, &elements, &substances) {
        Ok(rebuilt) => {
            commands.remove_resource::<FailedTerrainDamageBuild>();
            commands.remove_resource::<AcceptedTerrainDamageSourceRestoration>();
            commands.insert_resource(candidate);
            commands.insert_resource(rebuilt);
        }
        Err(errors) => {
            for error in &errors {
                error!("terrain damage content: {error}");
            }
            commands.insert_resource(FailedTerrainDamageBuild::new(
                &candidate,
                &elements,
                &substances,
            ));
            if let Some(accepted) = table
                .as_deref()
                .and_then(TerrainDamageTable::accepted_source)
            {
                warn!(
                    "restoring the previous valid config/terrain_damage.ron while its rejected candidate remains available for cross-file repair"
                );
                commands.insert_resource(accepted);
                commands.insert_resource(AcceptedTerrainDamageSourceRestoration);
            } else {
                error!(
                    "could not resolve initial config/terrain_damage.ron; Loading remains blocked"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use bevy::platform::collections::HashMap;
    use hex_test_app::HeadlessAppBuilder;

    use super::*;
    use crate::{ArtPalette, PaletteSwatch, SrgbColor, Substance, SubstanceFile, SwatchId};

    fn pair(element: &str, substance: &str) -> TerrainDamagePair {
        TerrainDamagePair {
            element: element.to_owned(),
            substance: substance.to_owned(),
        }
    }

    fn file(pairs: Vec<TerrainDamagePair>) -> TerrainDamageFile {
        TerrainDamageFile {
            damaging_pairs: pairs,
        }
    }

    fn elements() -> ElementCatalog {
        ElementCatalog::from_file(&crate::ElementFile {
            wheel: vec!["Fire".to_owned(), "Water".to_owned()],
            fusions: HashMap::default(),
        })
    }

    fn substances() -> SubstanceTable {
        let stone_swatch = SwatchId::new("test/stone").expect("test swatch id should be valid");
        let water_swatch = SwatchId::new("test/water").expect("test swatch id should be valid");
        let palette = ArtPalette::new(BTreeMap::from([
            (
                stone_swatch.clone(),
                PaletteSwatch::new(
                    "Stone",
                    SrgbColor::new(0.5, 0.5, 0.5).expect("test color should be valid"),
                    BTreeSet::from(["test".to_owned()]),
                )
                .expect("test swatch should be valid"),
            ),
            (
                water_swatch.clone(),
                PaletteSwatch::new(
                    "Water",
                    SrgbColor::new(0.1, 0.2, 0.8).expect("test color should be valid"),
                    BTreeSet::from(["test".to_owned()]),
                )
                .expect("test swatch should be valid"),
            ),
        ]))
        .expect("test palette should be valid");
        let substances = HashMap::from([
            ("air".to_owned(), Substance::invisible(false, false)),
            (
                "stone".to_owned(),
                Substance::from_swatch(stone_swatch, true, true).with_toughness(Some(4)),
            ),
            (
                "water".to_owned(),
                Substance::from_swatch(water_swatch, false, true),
            ),
        ]);
        SubstanceTable::from_file(&SubstanceFile { substances }, &palette)
            .expect("test substances should resolve")
    }

    #[test]
    fn duplicate_pairs_are_rejected_during_deserialization() {
        let error = ron::from_str::<TerrainDamageFile>(
            r#"(
                damaging_pairs: [
                    (element: "Fire", substance: "stone"),
                    (element: "Fire", substance: "stone"),
                ],
            )"#,
        )
        .expect_err("duplicates must not parse");
        assert!(error.to_string().contains("more than once"));
    }

    #[test]
    fn listed_pairs_damage_and_missing_pairs_resist() {
        let elements = elements();
        let substances = substances();
        let table =
            TerrainDamageTable::build(&file(vec![pair("Fire", "stone")]), &elements, &substances)
                .expect("the pair should resolve");

        assert!(table.damages(
            elements.id("Fire").expect("Fire should resolve"),
            substances.id("stone").expect("stone should resolve")
        ));
        assert!(!table.damages(
            elements.id("Water").expect("Water should resolve"),
            substances.id("stone").expect("stone should resolve")
        ));
        assert!(!table.damages(ElementId(999), SubstanceId(999)));
    }

    #[test]
    fn unknown_and_indestructible_references_are_rejected() {
        let errors = TerrainDamageTable::build(
            &file(vec![
                pair("Void", "stone"),
                pair("Fire", "adamant"),
                pair("Water", "water"),
            ]),
            &elements(),
            &substances(),
        )
        .expect_err("every invalid reference should fail the build");

        assert_eq!(
            errors,
            vec![
                TerrainDamageError::UnknownElement {
                    element: "Void".to_owned(),
                },
                TerrainDamageError::UnknownSubstance {
                    substance: "adamant".to_owned(),
                },
                TerrainDamageError::IndestructibleSubstance {
                    substance: "water".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn pair_order_does_not_change_the_resolved_fingerprint() {
        let elements = elements();
        let substances = substances();
        let first = TerrainDamageTable::build(
            &file(vec![pair("Fire", "stone"), pair("Water", "stone")]),
            &elements,
            &substances,
        )
        .expect("first matrix should resolve");
        let second = TerrainDamageTable::build(
            &file(vec![pair("Water", "stone"), pair("Fire", "stone")]),
            &elements,
            &substances,
        )
        .expect("reordered matrix should resolve");

        assert_eq!(first.source_revision(), second.source_revision());
    }

    #[test]
    fn shipped_matrix_covers_every_damageable_material_and_no_indestructible_one() {
        let element_file: crate::ElementFile =
            ron::from_str(include_str!("../../../assets/config/elements.ron"))
                .expect("shipped elements should parse");
        let substance_file: SubstanceFile =
            ron::from_str(include_str!("../../../assets/config/substances.ron"))
                .expect("shipped substances should parse");
        let palette: ArtPalette = ron::from_str(include_str!("../../../assets/art/palette.ron"))
            .expect("shipped palette should parse");
        let damage_file: TerrainDamageFile =
            ron::from_str(include_str!("../../../assets/config/terrain_damage.ron"))
                .expect("shipped terrain damage should parse");
        let elements = ElementCatalog::from_file(&element_file);
        let substances = SubstanceTable::from_file(&substance_file, &palette)
            .expect("shipped substances should resolve");
        let table = TerrainDamageTable::build(&damage_file, &elements, &substances)
            .expect("shipped terrain damage should resolve");

        let mut expected = 0;
        for element_index in 0..elements.len() {
            let element = ElementId(u16::try_from(element_index).expect("element id should fit"));
            for substance_index in 0..substances.len() {
                let substance =
                    SubstanceId(u16::try_from(substance_index).expect("substance id should fit"));
                if substances.toughness(substance).is_some() {
                    expected += 1;
                    assert!(table.damages(element, substance));
                } else {
                    assert!(!table.damages(element, substance));
                }
            }
        }
        assert_eq!(table.len(), expected);
        assert_eq!(expected, 70);
    }

    #[test]
    fn invalid_hot_reload_restores_the_previous_file_and_table() {
        let elements = elements();
        let substances = substances();
        let accepted_file = file(vec![pair("Fire", "stone")]);
        let accepted = TerrainDamageTable::build(&accepted_file, &elements, &substances)
            .expect("accepted matrix should resolve");

        let mut builder = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_state_plugin();
        builder.app_mut().insert_state(Screen::Title);
        builder.app_mut().insert_resource(elements.clone());
        builder.app_mut().insert_resource(substances.clone());
        builder.app_mut().insert_resource(accepted_file.clone());
        builder.app_mut().insert_resource(accepted);
        register_table_builder(builder.app_mut());
        let mut app = builder.build();
        app.update();

        let rejected = file(vec![pair("Void", "stone")]);
        app.insert_resource(rejected.clone());
        app.update();

        assert_eq!(
            *app.world().resource::<TerrainDamageFile>(),
            accepted_file,
            "the rejected raw resource should not remain beside the accepted table"
        );
        assert!(app.world().resource::<TerrainDamageTable>().damages(
            elements.id("Fire").expect("Fire should resolve"),
            substances.id("stone").expect("stone should resolve")
        ));
        assert!(app
            .world()
            .resource::<FailedTerrainDamageBuild>()
            .matches_sources(&rejected, &elements, &substances));
    }

    #[test]
    fn a_cross_file_repair_retries_the_retained_candidate() {
        let original_elements = elements();
        let substances = substances();
        let accepted_file = file(vec![pair("Fire", "stone")]);
        let accepted = TerrainDamageTable::build(&accepted_file, &original_elements, &substances)
            .expect("accepted matrix should resolve");

        let mut builder = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_state_plugin();
        builder.app_mut().insert_state(Screen::Title);
        builder.app_mut().insert_resource(original_elements);
        builder.app_mut().insert_resource(substances.clone());
        builder.app_mut().insert_resource(accepted_file);
        builder.app_mut().insert_resource(accepted);
        register_table_builder(builder.app_mut());
        let mut app = builder.build();
        app.update();

        let repaired_candidate = file(vec![pair("Lightning", "stone")]);
        app.insert_resource(repaired_candidate.clone());
        app.update();
        let mut fusions = HashMap::default();
        fusions.insert(
            "Lightning".to_owned(),
            vec![
                crate::FusionInput {
                    element: "Fire".to_owned(),
                    mana: 1,
                },
                crate::FusionInput {
                    element: "Water".to_owned(),
                    mana: 1,
                },
            ],
        );
        let repaired_elements = ElementCatalog::from_file(&crate::ElementFile {
            wheel: vec!["Fire".to_owned(), "Water".to_owned()],
            fusions,
        });
        app.insert_resource(repaired_elements.clone());
        app.update();

        assert_eq!(
            *app.world().resource::<TerrainDamageFile>(),
            repaired_candidate
        );
        assert!(app.world().resource::<TerrainDamageTable>().damages(
            repaired_elements
                .id("Lightning")
                .expect("Lightning should resolve after repair"),
            substances.id("stone").expect("stone should resolve")
        ));
        assert!(!app.world().contains_resource::<FailedTerrainDamageBuild>());
    }
}
