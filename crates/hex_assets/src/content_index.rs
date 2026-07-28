//! Cross-file integrity for the content pipeline.
//!
//! Each content file validates its own invariants at parse
//! ([`ElementFile`](crate::ElementFile), [`SpellFile`](crate::SpellFile)), but a
//! single file cannot see the others. [`ContentIndex`] is where the references
//! *between* files are resolved: every element a spell requires must exist in the
//! [`ElementCatalog`], and every substance a spell's effect names must exist in the
//! [`SubstanceTable`]. A dangling reference is reported loudly and the last valid
//! index is kept — the same last-valid-on-bad-reload behaviour the settings loader
//! has.
//!
//! It is rebuilt only outside [`Screen::Gameplay`], like the tables it draws on, so
//! resolved ids never shift under a live world. It also holds the spell requirements
//! **resolved to [`ElementId`]s**, the exact shape `hex_lattice::SpellTable` reads. The
//! `hex_lattice` edge is drawn as of HEX-12's prep, so the trait implementation belongs
//! in this crate, beside the content it reads — the engine's designed seat. Nothing
//! implements it yet; the accessors below are the whole input it needs.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use hex_core::{ElementId, Screen, SpellId};
use hex_lattice::{Casting, FusionTable, Requirement, SpellTable};
use thiserror::Error;

use crate::elements::ElementCatalog;
use crate::spells::{CastingAxis, SpellBook};
use crate::substances::SubstanceTable;

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
}

impl ContentIndex {
    /// Resolves every cross-file reference, or returns every failure found.
    ///
    /// Pure and table-only, so the shipped-content test can call it without an
    /// [`App`]. Substance references are checked for existence but not stored: effects
    /// are applied downstream against the live [`SubstanceTable`], keeping names —
    /// not session-local ids — as the durable form.
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
                    if substances.id(substance).is_none() {
                        errors.push(ContentError::UnknownSubstance {
                            spell: name.to_owned(),
                            substance: substance.to_owned(),
                        });
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
            Ok(Self { spells: resolved })
        } else {
            Err(errors)
        }
    }

    /// A spell's requirements resolved to `(element, mana)` — the shape
    /// `hex_lattice::SpellTable::requirements` will read (HEX-12).
    #[must_use]
    pub fn requirements(&self, spell: SpellId) -> Option<&[(ElementId, u16)]> {
        self.spells
            .get(&spell)
            .map(|resolved| resolved.requirements.as_slice())
    }

    /// How a spell spends its mana — the value `hex_lattice::SpellTable::casting` will
    /// read (HEX-12).
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
    app.register_type::<ContentIndex>();
    app.add_systems(
        Update,
        build_content_index.run_if(not(in_state(Screen::Gameplay))),
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

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;

    use super::*;
    use crate::elements::{ElementFile, FusionInput};
    use crate::spells::{
        CastingAxis, Effect, GemRequirement, ManaAxis, Spell, SpellFile, TargetShape, TargetingSpec,
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
        let mut map = HashMap::default();
        for name in ["air", "stone"] {
            map.insert(
                name.to_owned(),
                Substance {
                    color: (0.5, 0.5, 0.5),
                    solid: true,
                    diggable: true,
                },
            );
        }
        SubstanceTable::from_file(&SubstanceFile { substances: map })
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
                needs_los: false,
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
            spell(vec![gem("Aether")], vec![Effect::Burn { amount: 1 }]),
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
            spell(vec![gem("Aether")], vec![Effect::Burn { amount: 1 }]),
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
}
