//! Cross-file integrity for the content pipeline.
//!
//! Each content file validates its own invariants at parse
//! ([`ElementFile`], [`SpellFile`]), but a
//! single file cannot see the others. [`ContentIndex`] is where the references
//! *between* files are resolved: every element a spell requires must exist in the
//! [`ElementCatalog`], and every substance a spell's effect names must exist in the
//! [`SubstanceTable`]. Construction effects additionally require the world-owned
//! substance policy to admit spell conjuration. A dangling or non-conjurable reference
//! is reported loudly and the last valid index is kept — the same
//! last-valid-on-bad-reload behaviour the settings loader has.
//!
//! It is rebuilt only outside [`Screen::Gameplay`], like the tables it draws on, so
//! resolved ids never shift under a live world. It also holds the spell requirements
//! **resolved to [`ElementId`]s**, the exact shape `hex_lattice::SpellTable` reads. The
//! trait implementation belongs in this crate, beside the content it reads — the
//! engine's designed seat.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use hex_core::{ElementId, Screen, SpellId};
use hex_lattice::{Casting, FusionTable, Requirement, SpellTable};
use thiserror::Error;

use crate::elements::{ElementCatalog, ElementFile};
use crate::fingerprint::FingerprintEncoder;
use crate::lattices::{LatticeFile, LatticeLibrary};
use crate::spells::{CastingAxis, SpellBook, SpellFile};
use crate::substances::{SubstanceFile, SubstanceTable};
use crate::ArtPalette;

/// A cross-file reference that did not resolve.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ContentError {
    /// A spell requires an element no `elements.ron` entry defines.
    #[error("spell '{spell}' requires element '{element}', which is not defined in elements.ron")]
    UnknownElement {
        /// The spell with the dangling requirement.
        spell: String,
        /// The element name that did not resolve.
        element: String,
    },
    /// A spell effect names a substance no `substances.ron` entry defines.
    #[error(
        "spell '{spell}' effect names substance '{substance}', which is not defined in substances.ron"
    )]
    UnknownSubstance {
        /// The spell with the dangling effect.
        spell: String,
        /// The substance name that did not resolve.
        substance: String,
    },
    /// A construction effect names a defined substance the world does not admit.
    #[error("spell '{spell}' effect names substance '{substance}', which is not conjurable")]
    NonConjurableSubstance {
        /// The spell with the refused construction effect.
        spell: String,
        /// The defined but non-conjurable substance.
        substance: String,
    },
}

/// A spell with its cross-file references resolved to ids.
#[derive(Debug, Clone)]
struct ResolvedSpell {
    /// Requirements resolved to `(element id, mana)` — `hex_lattice::Requirement`'s
    /// shape.
    requirements: Vec<(ElementId, u16)>,
    /// How the spell spends its mana (carries no names, so it needs no resolution).
    casting: CastingAxis,
}

/// The resolved, cross-checked view of the content files.
///
/// Absent until every reference resolves; a failed rebuild keeps the previous value.
#[derive(Resource, Reflect, Debug, Clone, Default)]
#[reflect(Resource)]
pub struct ContentIndex {
    #[reflect(ignore)]
    spells: HashMap<SpellId, ResolvedSpell>,
    #[reflect(ignore)]
    source_elements: u64,
    #[reflect(ignore)]
    source_spells: u64,
    #[reflect(ignore)]
    source_substances: u64,
}

impl ContentIndex {
    /// Resolves every cross-file reference, or returns every failure found.
    ///
    /// Pure and table-only, so the shipped-content test can call it without an
    /// [`App`]. Substance references are checked for existence and world-owned
    /// conjuration admission but are not stored: effects are applied downstream
    /// against the live [`SubstanceTable`], keeping names — not session-local ids —
    /// as the durable form.
    pub fn build(
        elements: &ElementCatalog,
        spells: &SpellBook,
        substances: &SubstanceTable,
    ) -> Result<Self, Vec<ContentError>> {
        let mut resolved: HashMap<SpellId, ResolvedSpell> = HashMap::default();
        let mut errors = Vec::new();

        for (id, name, spell) in spells.iter() {
            let mut requirements = Vec::with_capacity(spell.requirements.len());
            for requirement in &spell.requirements {
                match elements.id(&requirement.element) {
                    Some(element) => requirements.push((element, requirement.mana)),
                    None => errors.push(ContentError::UnknownElement {
                        spell: name.to_owned(),
                        element: requirement.element.clone(),
                    }),
                }
            }
            for effect in &spell.effects {
                if let Some(substance) = effect.substance() {
                    match substances.id(substance) {
                        None => errors.push(ContentError::UnknownSubstance {
                            spell: name.to_owned(),
                            substance: substance.to_owned(),
                        }),
                        Some(id) if !substances.is_conjurable(id) => {
                            errors.push(ContentError::NonConjurableSubstance {
                                spell: name.to_owned(),
                                substance: substance.to_owned(),
                            });
                        }
                        Some(_) => {}
                    }
                }
            }
            resolved.insert(
                id,
                ResolvedSpell {
                    requirements,
                    casting: spell.casting,
                },
            );
        }

        if errors.is_empty() {
            Ok(Self {
                spells: resolved,
                source_elements: elements.source_fingerprint(),
                source_spells: spells.source_fingerprint(),
                source_substances: substances.semantic_fingerprint(),
            })
        } else {
            Err(errors)
        }
    }

    /// A spell's requirements resolved to `(element, mana)` — the shape read by
    /// `hex_lattice::SpellTable::requirements`.
    #[must_use]
    pub fn requirements(&self, spell: SpellId) -> Option<&[(ElementId, u16)]> {
        self.spells
            .get(&spell)
            .map(|resolved| resolved.requirements.as_slice())
    }

    /// How a spell spends its mana — the value read by
    /// `hex_lattice::SpellTable::casting`.
    #[must_use]
    pub fn casting(&self, spell: SpellId) -> Option<CastingAxis> {
        self.spells.get(&spell).map(|resolved| resolved.casting)
    }

    /// Bridges the loaded content tables to the engine's lookup traits.
    ///
    /// Borrows rather than owns, so it is built where it is used and never goes stale
    /// against the tables it reads.
    #[must_use]
    pub fn tables<'a>(&'a self, elements: &'a ElementCatalog) -> ContentTables<'a> {
        ContentTables {
            index: self,
            elements,
        }
    }

    /// How many spells the index holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.spells.len()
    }

    /// Whether the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spells.is_empty()
    }

    /// Whether this index was resolved from these exact table semantics.
    #[must_use]
    pub fn matches_sources(
        &self,
        elements: &ElementCatalog,
        spells: &SpellBook,
        substances: &SubstanceTable,
    ) -> bool {
        self.source_elements == elements.source_fingerprint()
            && self.source_spells == spells.source_fingerprint()
            && self.source_substances == substances.semantic_fingerprint()
    }

    fn source_revision(&self) -> u64 {
        let mut encoder = FingerprintEncoder::new(b"hex-content-index-sources-v1");
        encoder.u64(self.source_elements);
        encoder.u64(self.source_spells);
        encoder.u64(self.source_substances);
        encoder.finish()
    }
}

/// One fully cross-checked revision of the authored gameplay content graph.
///
/// The last valid [`ContentIndex`] and [`LatticeLibrary`] intentionally remain
/// available after a rejected edit, but this resource is removed until every raw
/// source, direct catalog and derived table agrees again. Loading gates on this
/// acceptance marker instead of Bevy change ticks.
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq, Eq)]
#[reflect(Resource)]
pub struct AcceptedContentRevision {
    content_sources: u64,
    lattice_sources: u64,
}

impl AcceptedContentRevision {
    /// Stable identity of the complete accepted semantic revision.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let mut encoder = FingerprintEncoder::new(b"hex-accepted-content-revision-v1");
        encoder.u64(self.content_sources);
        encoder.u64(self.lattice_sources);
        encoder.finish()
    }

    /// Whether these are still the resolved tables accepted by this revision.
    #[must_use]
    pub fn matches_resolved(&self, content: &ContentIndex, lattices: &LatticeLibrary) -> bool {
        self.content_sources == content.source_revision()
            && self.lattice_sources == lattices.source_revision()
    }
}

/// Ordering hook for consumers that must see acceptance changes in the same frame.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContentReadinessSystems {
    /// Publishes or withdraws [`AcceptedContentRevision`].
    PublishAcceptedRevision,
}

/// The engine's content lookups, over the loaded tables.
///
/// Build one with [`ContentIndex::tables`]. The two `None` arms are the load-bearing
/// part and must stay as they are: an unknown spell has to remain **uncastable**, and an
/// empty requirement list would instead read as a tier-0 spell that costs nothing — so
/// the fallback is a single `u16::MAX`-mana requirement, beyond any attunement content
/// can name. An unknown casting axis falls to `Evocation`, the one that spends rather
/// than locks, because an unknown spell that quietly tied mana up forever would be worse.
pub struct ContentTables<'a> {
    index: &'a ContentIndex,
    elements: &'a ElementCatalog,
}

impl SpellTable for ContentTables<'_> {
    fn requirements(&self, spell: SpellId) -> Vec<Requirement> {
        match self.index.requirements(spell) {
            Some(requirements) => requirements
                .iter()
                .map(|&(element, mana)| Requirement { element, mana })
                .collect(),
            None => vec![Requirement {
                // Any real element does — the cost is what blocks the cast — and an
                // empty catalog blocks everything anyway, so the default is fine there.
                element: self.elements.wheel().first().copied().unwrap_or_default(),
                mana: u16::MAX,
            }],
        }
    }

    fn casting(&self, spell: SpellId) -> Casting {
        match self.index.casting(spell) {
            Some(CastingAxis::Enchantment { defense }) => Casting::Enchantment { defense },
            Some(CastingAxis::Evocation) | None => Casting::Evocation,
        }
    }
}

impl FusionTable for ContentTables<'_> {
    fn recipe(&self, output: ElementId) -> Option<Vec<Requirement>> {
        // `None` passes straight through, and correctly: it means "a basic element, not
        // a fusion output", which is not a failure.
        self.elements.recipe(output).map(|inputs| {
            inputs
                .iter()
                .map(|&(element, mana)| Requirement { element, mana })
                .collect()
        })
    }
}

/// Registers the cross-file content index.
pub fn plugin(app: &mut App) {
    app.register_type::<ContentIndex>()
        .register_type::<AcceptedContentRevision>();
    app.add_systems(
        Update,
        build_content_index.run_if(not(in_state(Screen::Gameplay))),
    );
    app.add_systems(
        PostUpdate,
        publish_accepted_content_revision
            .in_set(ContentReadinessSystems::PublishAcceptedRevision)
            .run_if(not(in_state(Screen::Gameplay))),
    );
}

/// Rebuilds the index whenever one of the tables it draws on changes — the frame the
/// last of the three first arrives, and on any later hot-reload. Gating on *changed*
/// (rather than on the index being absent) means a persistently bad file is logged
/// once per edit, not once per frame.
fn build_content_index(
    mut commands: Commands,
    elements: Option<Res<ElementCatalog>>,
    spells: Option<Res<SpellBook>>,
    substances: Option<Res<SubstanceTable>>,
) {
    let (Some(elements), Some(spells), Some(substances)) = (elements, spells, substances) else {
        return;
    };
    if !elements.is_changed() && !spells.is_changed() && !substances.is_changed() {
        return;
    }
    match ContentIndex::build(&elements, &spells, &substances) {
        Ok(index) => commands.insert_resource(index),
        Err(errors) => {
            for error in &errors {
                error!("content: {error}");
            }
            // Keep the last valid index (insert nothing), mirroring the settings
            // loader's last-valid-on-bad-reload.
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the acceptance boundary must compare every raw and derived content resource"
)]
fn publish_accepted_content_revision(
    mut commands: Commands,
    element_file: Option<Res<ElementFile>>,
    elements: Option<Res<ElementCatalog>>,
    spell_file: Option<Res<SpellFile>>,
    spells: Option<Res<SpellBook>>,
    substance_file: Option<Res<SubstanceFile>>,
    palette: Option<Res<ArtPalette>>,
    substances: Option<Res<SubstanceTable>>,
    lattice_file: Option<Res<LatticeFile>>,
    content: Option<Res<ContentIndex>>,
    lattices: Option<Res<LatticeLibrary>>,
    accepted: Option<Res<AcceptedContentRevision>>,
) {
    let current = match (
        element_file.as_deref(),
        elements.as_deref(),
        spell_file.as_deref(),
        spells.as_deref(),
        substance_file.as_deref(),
        palette.as_deref(),
        substances.as_deref(),
        lattice_file.as_deref(),
        content.as_deref(),
        lattices.as_deref(),
    ) {
        (
            Some(element_file),
            Some(elements),
            Some(spell_file),
            Some(spells),
            Some(substance_file),
            Some(palette),
            Some(substances),
            Some(lattice_file),
            Some(content),
            Some(lattices),
        ) if elements.matches_source(element_file)
            && spells.matches_source(spell_file)
            && substances.matches_sources(substance_file, palette)
            && content.matches_sources(elements, spells, substances)
            && lattices.matches_sources(lattice_file, elements, spells) =>
        {
            Some(AcceptedContentRevision {
                content_sources: content.source_revision(),
                lattice_sources: lattices.source_revision(),
            })
        }
        _ => None,
    };

    match (current, accepted.as_deref()) {
        (Some(current), Some(previous)) if current == *previous => {}
        (Some(current), _) => commands.insert_resource(current),
        (None, Some(_)) => commands.remove_resource::<AcceptedContentRevision>(),
        (None, None) => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use bevy::state::app::StatesPlugin;

    use super::*;
    use crate::art_palette::{ArtPalette, PaletteSwatch, SrgbColor, SwatchId};
    use crate::elements::{ElementFile, FusionInput};
    use crate::spells::{
        CastingAxis, Effect, GemRequirement, ManaAxis, Spell, SpellFile, TargetShape,
        TargetingSpec, Trajectory,
    };
    use crate::substances::{Substance, SubstanceFile};

    fn elements() -> ElementCatalog {
        let mut fusions = HashMap::default();
        fusions.insert(
            "Lightning".to_owned(),
            vec![
                FusionInput {
                    element: "Light".to_owned(),
                    mana: 1,
                },
                FusionInput {
                    element: "Fire".to_owned(),
                    mana: 1,
                },
            ],
        );
        let file = ElementFile {
            wheel: ["Light", "Air", "Fire", "Metal", "Earth", "Water"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            fusions,
        };
        ElementCatalog::from_file(&file)
    }

    fn substances() -> SubstanceTable {
        substances_with_stone_admission(true)
    }

    fn substances_with_stone_admission(conjurable: bool) -> SubstanceTable {
        let stone_swatch = SwatchId::new("test/stone").expect("the test swatch id should be valid");
        let palette = ArtPalette::new(BTreeMap::from([(
            stone_swatch.clone(),
            PaletteSwatch::new(
                "Test Stone",
                SrgbColor::new(0.5, 0.5, 0.5).expect("the test color should be valid"),
                BTreeSet::from(["test".to_owned()]),
            )
            .expect("the test swatch should be valid"),
        )]))
        .expect("the test palette should be valid");
        let mut map = HashMap::default();
        map.insert("air".to_owned(), Substance::invisible(false, false));
        map.insert(
            "stone".to_owned(),
            Substance::from_swatch(stone_swatch, true, true).with_conjurable(conjurable),
        );
        SubstanceTable::from_file(&SubstanceFile { substances: map }, &palette)
            .expect("the test substances should resolve through the palette")
    }

    fn spell(requirements: Vec<GemRequirement>, effects: Vec<Effect>) -> Spell {
        Spell {
            requirements,
            casting: CastingAxis::Evocation,
            mana: ManaAxis::Fixed,
            co_castable: false,
            targeting: TargetingSpec {
                range: 1,
                shape: TargetShape::Single,
                trajectory: Trajectory::None,
            },
            effects,
        }
    }

    fn gem(element: &str) -> GemRequirement {
        GemRequirement {
            element: element.to_owned(),
            mana: 1,
        }
    }

    fn book(spells: Vec<(&str, Spell)>) -> SpellBook {
        let mut map = HashMap::default();
        for (name, spell) in spells {
            map.insert(name.to_owned(), spell);
        }
        SpellBook::from_file(&SpellFile { spells: map })
    }

    fn shipped_sources() -> (
        ElementFile,
        SpellFile,
        SubstanceFile,
        ArtPalette,
        LatticeFile,
    ) {
        (
            ron::from_str(include_str!("../../../assets/config/elements.ron"))
                .expect("shipped elements should parse"),
            ron::from_str(include_str!("../../../assets/config/spells.ron"))
                .expect("shipped spells should parse"),
            ron::from_str(include_str!("../../../assets/config/substances.ron"))
                .expect("shipped substances should parse"),
            ron::from_str(include_str!("../../../assets/art/palette.ron"))
                .expect("shipped palette should parse"),
            ron::from_str(include_str!("../../../assets/config/lattices.ron"))
                .expect("shipped lattices should parse"),
        )
    }

    #[test]
    fn resolves_requirements_to_element_ids() {
        let elements = elements();
        let book = book(vec![(
            "Ember",
            spell(
                vec![gem("Fire")],
                vec![Effect::DisableHexes {
                    count: 1,
                    targeted: false,
                }],
            ),
        )]);
        let index = ContentIndex::build(&elements, &book, &substances()).expect("resolves");

        let ember = book.id("Ember").expect("Ember is in the book");
        let requirements = index.requirements(ember).expect("Ember resolved");
        assert_eq!(requirements.len(), 1);
        assert_eq!(requirements.first().map(|(id, _)| *id), elements.id("Fire"));
    }

    #[test]
    fn a_dangling_element_reference_fails() {
        let book = book(vec![(
            "Ghost",
            spell(vec![gem("Aether")], vec![Effect::Burn { turns: 1 }]),
        )]);
        let errors = ContentIndex::build(&elements(), &book, &substances())
            .expect_err("Aether is not an element");
        assert!(errors.iter().any(|error| matches!(
            error,
            ContentError::UnknownElement { element, .. } if element == "Aether"
        )));
    }

    #[test]
    fn a_dangling_substance_reference_fails() {
        let book = book(vec![(
            "Conjure",
            spell(
                vec![gem("Earth")],
                vec![Effect::SpawnWall {
                    substance: "adamant".to_owned(),
                }],
            ),
        )]);
        let errors = ContentIndex::build(&elements(), &book, &substances())
            .expect_err("adamant is not a substance");
        assert!(errors.iter().any(|error| matches!(
            error,
            ContentError::UnknownSubstance { substance, .. } if substance == "adamant"
        )));
    }

    #[test]
    fn a_defined_but_non_conjurable_substance_fails() {
        let book = book(vec![(
            "Protected Wall",
            spell(
                vec![gem("Earth")],
                vec![Effect::SpawnWall {
                    substance: "stone".to_owned(),
                }],
            ),
        )]);
        let errors =
            ContentIndex::build(&elements(), &book, &substances_with_stone_admission(false))
                .expect_err("existence alone must not admit a construction material");

        assert_eq!(
            errors,
            vec![ContentError::NonConjurableSubstance {
                spell: "Protected Wall".to_owned(),
                substance: "stone".to_owned(),
            }]
        );
    }

    /// A failed rebuild must keep the previous valid index, not clear it.
    #[test]
    fn a_bad_rebuild_keeps_the_last_valid_index() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.insert_state(Screen::Title);
        app.add_systems(
            Update,
            build_content_index.run_if(not(in_state(Screen::Gameplay))),
        );

        // A good index is built from valid tables.
        app.insert_resource(elements());
        app.insert_resource(substances());
        app.insert_resource(book(vec![(
            "Ember",
            spell(
                vec![gem("Fire")],
                vec![Effect::DisableHexes {
                    count: 1,
                    targeted: false,
                }],
            ),
        )]));
        app.update();
        assert_eq!(
            app.world().resource::<ContentIndex>().len(),
            1,
            "good content builds"
        );

        // A spell book with a dangling element replaces the old one; the rebuild fails.
        app.insert_resource(book(vec![(
            "Broken",
            spell(vec![gem("Aether")], vec![Effect::Burn { turns: 1 }]),
        )]));
        app.update();

        let index = app.world().resource::<ContentIndex>();
        assert_eq!(
            index.len(),
            1,
            "the last valid index is kept when a rebuild fails"
        );
        assert!(
            index.requirements(SpellId(0)).is_some(),
            "the previously resolved spell is still there"
        );
    }

    #[test]
    fn rejected_cross_file_revision_stays_unaccepted_until_repaired() {
        let (element_file, spell_file, substance_file, palette, lattice_file) = shipped_sources();
        let elements = ElementCatalog::from_file(&element_file);
        let spells = SpellBook::from_file(&spell_file);
        let substances = SubstanceTable::from_file(&substance_file, &palette)
            .expect("shipped substances should resolve");
        let content =
            ContentIndex::build(&elements, &spells, &substances).expect("content should resolve");
        let lattices = LatticeLibrary::build(&lattice_file, &elements, &spells)
            .expect("lattices should resolve");

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(element_file);
        app.insert_resource(elements);
        app.insert_resource(spell_file.clone());
        app.insert_resource(spells);
        app.insert_resource(substance_file);
        app.insert_resource(palette);
        app.insert_resource(substances);
        app.insert_resource(lattice_file);
        app.insert_resource(content);
        app.insert_resource(lattices);
        app.add_systems(Update, build_content_index);
        app.add_systems(PostUpdate, publish_accepted_content_revision);

        app.update();
        assert!(
            app.world().contains_resource::<AcceptedContentRevision>(),
            "the initial coherent graph should be accepted"
        );

        let mut broken = spell_file.clone();
        broken
            .spells
            .get_mut("Ember")
            .expect("shipped spells contain Ember")
            .requirements
            .first_mut()
            .expect("Ember has a requirement")
            .element = "Aether".to_owned();
        app.insert_resource(SpellBook::from_file(&broken));
        app.insert_resource(broken);

        for _ in 0..4 {
            app.update();
            assert!(
                !app.world().contains_resource::<AcceptedContentRevision>(),
                "settled change ticks must not accept a retained stale index"
            );
        }

        app.insert_resource(SpellBook::from_file(&spell_file));
        app.insert_resource(spell_file);
        app.update();
        assert!(
            app.world().contains_resource::<AcceptedContentRevision>(),
            "repairing the source graph should publish a new accepted revision"
        );
    }

    #[test]
    fn inserted_names_cannot_pair_shifted_ids_with_stale_derived_tables() {
        let (element_file, mut spell_file, substance_file, palette, lattice_file) =
            shipped_sources();
        let elements = ElementCatalog::from_file(&element_file);
        let original_spells = SpellBook::from_file(&spell_file);
        let substances = SubstanceTable::from_file(&substance_file, &palette)
            .expect("shipped substances should resolve");
        let stale_content = ContentIndex::build(&elements, &original_spells, &substances)
            .expect("shipped content should resolve");
        let stale_lattices = LatticeLibrary::build(&lattice_file, &elements, &original_spells)
            .expect("shipped lattices should resolve");

        let inserted = spell_file
            .spells
            .get("Ember")
            .expect("shipped spells contain Ember")
            .clone();
        spell_file.spells.insert("Aardvark".to_owned(), inserted);
        let shifted_spells = SpellBook::from_file(&spell_file);

        assert!(
            !stale_content.matches_sources(&elements, &shifted_spells, &substances),
            "a stale spell id index must reject a catalog whose sorted ids shifted"
        );
        assert!(
            !stale_lattices.matches_sources(&lattice_file, &elements, &shifted_spells),
            "stale lattice spell ids must reject the shifted catalog"
        );

        let rebuilt_content = ContentIndex::build(&elements, &shifted_spells, &substances)
            .expect("the inserted spell remains semantically valid");
        let rebuilt_lattices = LatticeLibrary::build(&lattice_file, &elements, &shifted_spells)
            .expect("lattices should rebuild against shifted ids");
        assert!(rebuilt_content.matches_sources(&elements, &shifted_spells, &substances));
        assert!(rebuilt_lattices.matches_sources(&lattice_file, &elements, &shifted_spells));
    }
}
