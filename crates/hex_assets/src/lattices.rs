//! Archetype lattices, loaded from `assets/config/lattices.ron`.
//!
//! An enemy's lattice is its entire stat block, so this file is where enemies are
//! authored — there is no separate stat system to keep in step with it.
//!
//! # Why there are two types here
//!
//! [`LatticeFile`] is what the designer writes: cells and stats naming **elements and
//! spells by string**. [`LatticeLibrary`] is what the game reads: the same drawing with
//! every name resolved to a [`LatticeSpec`](hex_lattice::LatticeSpec) the engine can
//! cast from.
//!
//! They cannot be one type, and the reason is worth stating because getting it wrong is
//! silent. `LatticeSpec` already derives serde — but its wire form stores
//! [`ElementId`](hex_core::ElementId) and [`SpellId`](hex_core::SpellId), and **those
//! are session-local**, dealt from sorted names at load. A file written in ids would
//! mean something different the moment a content patch adds an element, and a hot reload
//! could reassign every id under a lattice already built from them. So `LatticeSpec`'s
//! serde is the *save* format; this module is the *file* format, and the gap between
//! them is one resolution pass — the same shape
//! [`ContentIndex`](crate::ContentIndex) uses for spell requirements.
//!
//! Resolution therefore cannot happen in `Deserialize`: it needs the element and spell
//! catalogs, which are resources. It happens in a system, once they exist.

use std::collections::BTreeMap;

use bevy::prelude::*;
use hex_core::{ElementId, LatticeCoord, Screen};
use hex_lattice::{CellKind, LatticeSpec, LatticeStats};
use serde::Deserialize;
use thiserror::Error;

use crate::elements::ElementCatalog;
use crate::spells::SpellBook;
use crate::{LoadSettings, CONFIG_EXTENSIONS};

/// A name in `lattices.ron` that resolves to nothing.
///
/// Each carries the archetype, so a designer reading the log knows which drawing to
/// open rather than only which word was wrong.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum LatticeError {
    /// A cell, or an attunement entry, names an element the wheel does not have.
    #[error("archetype '{archetype}' names element '{element}', which is not in elements.ron")]
    UnknownElement {
        /// Which archetype.
        archetype: String,
        /// The unresolvable name.
        element: String,
    },
    /// A spell cell names a spell the book does not have.
    #[error("archetype '{archetype}' names spell '{spell}', which is not in spells.ron")]
    UnknownSpell {
        /// Which archetype.
        archetype: String,
        /// The unresolvable name.
        spell: String,
    },
    /// A fusion cell names an element that is not a fusion output.
    ///
    /// Separate from [`Self::UnknownElement`] because the failure is different: the
    /// element exists, it just has no recipe, so a fusion of it could never resolve.
    #[error("archetype '{archetype}' fuses '{element}', which has no recipe in elements.ron")]
    NotAFusion {
        /// Which archetype.
        archetype: String,
        /// The element named as a fusion output.
        element: String,
    },
    /// Two cells claim the same coordinate.
    #[error("archetype '{archetype}' places two cells at ({q}, {r})")]
    DuplicateCell {
        /// Which archetype.
        archetype: String,
        /// The contested axial pair.
        q: i32,
        /// The contested axial pair.
        r: i32,
    },
    /// An archetype with no cells at all.
    #[error("archetype '{archetype}' has no cells, so nothing could stand for it")]
    Empty {
        /// Which archetype.
        archetype: String,
    },
    /// A file that defines nobody.
    #[error("lattices.ron defines no archetypes, so every unit would spawn inert")]
    NoArchetypes,
    /// A gem holding a fusion output.
    ///
    /// The mirror of [`Self::NotAFusion`], and it matters for the same reason: a gem of
    /// a higher-order element satisfies a requirement for it **directly**, so the fusion
    /// that was supposed to be the expensive part of reaching that element is bypassed
    /// entirely — silently, with the spell still castable.
    #[error(
        "archetype '{archetype}' holds '{element}' in a gem, but it is a fusion output — \
         it has to be fused from its recipe, not held directly"
    )]
    NotAGem {
        /// Which archetype.
        archetype: String,
        /// The higher-order element found in a gem.
        element: String,
    },
    /// An attunement or channelling figure past what content can plausibly mean.
    #[error(
        "archetype '{archetype}' gives '{element}' {amount} {field}; the maximum is \
         {MAX_ATTUNEMENT}"
    )]
    Implausible {
        /// Which archetype.
        archetype: String,
        /// Which element.
        element: String,
        /// `attunement` or `channelling`.
        field: &'static str,
        /// What was written.
        amount: u16,
    },
}

/// The largest attunement or channelling an archetype may name.
///
/// A guard rail rather than balance, and one with a sharp edge behind it. The engine's
/// unknown-spell fallback is a single requirement costing `u16::MAX`, which is
/// uncastable only for as long as no gem can hold that much — so an attunement of
/// `65535` would make **every unknown spell castable** from one gem of the wheel's first
/// element. Capping well below that keeps the fallback the impossibility it claims to be,
/// and a three-digit attunement is a typo in any case: every shipped spell costs 1.
const MAX_ATTUNEMENT: u16 = 64;

/// One archetype's lattice and the mana rules that go with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archetype {
    /// The drawing: which cell is where, and what each one is.
    pub spec: LatticeSpec,
    /// Attunement and channelling, per element.
    pub stats: LatticeStats,
}

/// Every archetype the game can spawn, resolved and ready to attach.
///
/// Absent until every name resolves; a failed reload keeps the previous value, matching
/// the settings loader and [`ContentIndex`](crate::ContentIndex).
#[derive(Resource, Reflect, Debug, Clone, Default)]
#[reflect(Resource)]
pub struct LatticeLibrary {
    // `LatticeSpec`/`LatticeStats` are `Reflect` but their maps are not registered, so
    // the inspector shows the resource exists rather than its contents. Matches
    // `ContentIndex`, which ignores its own map for the same reason.
    #[reflect(ignore)]
    archetypes: BTreeMap<String, Archetype>,
}

impl LatticeLibrary {
    /// Resolves every name in `file`, or returns every failure found.
    ///
    /// Pure and table-only, so the shipped-content test can call it without an [`App`].
    /// Every error is collected rather than the first returned, because a designer
    /// fixing a file wants the whole list.
    pub fn build(
        file: &LatticeFile,
        elements: &ElementCatalog,
        spells: &SpellBook,
    ) -> Result<Self, Vec<LatticeError>> {
        let mut archetypes = BTreeMap::new();
        let mut errors = Vec::new();

        for (name, raw) in &file.archetypes {
            match resolve_archetype(name, raw, elements, spells) {
                Ok(archetype) => {
                    archetypes.insert(name.clone(), archetype);
                }
                Err(mut found) => errors.append(&mut found),
            }
        }

        // The same reasoning as an archetype with no cells, one level up: a file that
        // defines nobody satisfies the loading gate and puts a whole field of inert
        // units on the map behind one warning each.
        if archetypes.is_empty() && errors.is_empty() {
            errors.push(LatticeError::NoArchetypes);
        }

        if errors.is_empty() {
            Ok(Self { archetypes })
        } else {
            Err(errors)
        }
    }

    /// Adds one archetype, replacing any of the same name.
    ///
    /// For tests and tools that need a library without a file behind them. The game
    /// builds its own with [`Self::build`], which is the path that validates.
    pub fn insert(&mut self, name: String, archetype: Archetype) {
        self.archetypes.insert(name, archetype);
    }

    /// The archetype `name` describes, if there is one.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Archetype> {
        self.archetypes.get(name)
    }

    /// Every archetype, in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Archetype)> {
        self.archetypes
            .iter()
            .map(|(name, archetype)| (name.as_str(), archetype))
    }

    /// How many archetypes are defined.
    #[must_use]
    pub fn len(&self) -> usize {
        self.archetypes.len()
    }

    /// Whether the library is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.archetypes.is_empty()
    }
}

fn resolve_archetype(
    name: &str,
    raw: &UnvalidatedArchetype,
    elements: &ElementCatalog,
    spells: &SpellBook,
) -> Result<Archetype, Vec<LatticeError>> {
    let mut errors = Vec::new();
    let mut cells: BTreeMap<LatticeCoord, CellKind> = BTreeMap::new();

    if raw.cells.is_empty() {
        return Err(vec![LatticeError::Empty {
            archetype: name.to_owned(),
        }]);
    }

    for entry in &raw.cells {
        let coord = LatticeCoord::new(entry.at.q, entry.at.r);
        let kind = match &entry.kind {
            UnvalidatedCell::Blank => Some(CellKind::Blank),
            UnvalidatedCell::Gem(element) => {
                element_id(name, element, elements, &mut errors).and_then(|id| {
                    // A gem of a fusion output satisfies a requirement for it directly,
                    // which quietly deletes the reason fusions are expensive.
                    if elements.is_higher_order(id) {
                        errors.push(LatticeError::NotAGem {
                            archetype: name.to_owned(),
                            element: element.clone(),
                        });
                        None
                    } else {
                        Some(CellKind::Gem { element: id })
                    }
                })
            }
            UnvalidatedCell::Fusion(element) => {
                element_id(name, element, elements, &mut errors).and_then(|output| {
                    // A fusion of an element with no recipe could never resolve, and
                    // the engine would report it as an unsatisfiable cast rather than
                    // as the authoring mistake it is.
                    if elements.is_higher_order(output) {
                        Some(CellKind::Fusion { output })
                    } else {
                        errors.push(LatticeError::NotAFusion {
                            archetype: name.to_owned(),
                            element: element.clone(),
                        });
                        None
                    }
                })
            }
            UnvalidatedCell::Spell(spell) => match spells.id(spell) {
                Some(spell) => Some(CellKind::Spell { spell }),
                None => {
                    errors.push(LatticeError::UnknownSpell {
                        archetype: name.to_owned(),
                        spell: spell.clone(),
                    });
                    None
                }
            },
        };

        if let Some(kind) = kind {
            if cells.insert(coord, kind).is_some() {
                errors.push(LatticeError::DuplicateCell {
                    archetype: name.to_owned(),
                    q: entry.at.q,
                    r: entry.at.r,
                });
            }
        }
    }

    let capacity = element_map(name, "attunement", &raw.attunement, elements, &mut errors);
    let channelling = element_map(name, "channelling", &raw.channelling, elements, &mut errors);

    if errors.is_empty() {
        Ok(Archetype {
            spec: LatticeSpec::new(cells),
            stats: LatticeStats::new(capacity, channelling),
        })
    } else {
        Err(errors)
    }
}

/// Resolves one element name, recording the failure and yielding `None` if it is unknown.
fn element_id(
    archetype: &str,
    element: &str,
    elements: &ElementCatalog,
    errors: &mut Vec<LatticeError>,
) -> Option<ElementId> {
    match elements.id(element) {
        Some(id) => Some(id),
        None => {
            errors.push(LatticeError::UnknownElement {
                archetype: archetype.to_owned(),
                element: element.to_owned(),
            });
            None
        }
    }
}

fn element_map(
    archetype: &str,
    field: &'static str,
    raw: &BTreeMap<String, u16>,
    elements: &ElementCatalog,
    errors: &mut Vec<LatticeError>,
) -> BTreeMap<ElementId, u16> {
    raw.iter()
        .filter_map(|(element, &amount)| {
            if amount > MAX_ATTUNEMENT {
                errors.push(LatticeError::Implausible {
                    archetype: archetype.to_owned(),
                    element: element.clone(),
                    field,
                    amount,
                });
                return None;
            }
            element_id(archetype, element, elements, errors).map(|id| (id, amount))
        })
        .collect()
}

/// `assets/config/lattices.ron` as written, before any name is resolved.
///
/// A `Resource` as well as an `Asset` so the resolving system can read it the way every
/// other settings file is read; nothing outside this module should need it, because the
/// resolved [`LatticeLibrary`] is what the game runs on.
#[derive(Asset, Resource, Reflect, Debug, Clone, Deserialize)]
#[reflect(Resource)]
#[serde(deny_unknown_fields)]
pub struct LatticeFile {
    /// Archetypes by name. A `BTreeMap` so a file with two mistakes reports them in the
    /// same order every run — `SpellFile`'s hash map does not, and its own validation
    /// notes that as a wart.
    pub archetypes: BTreeMap<String, UnvalidatedArchetype>,
}

/// One archetype as written.
#[derive(Reflect, Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnvalidatedArchetype {
    /// The cells, in no particular order — the coordinate on each is what places it.
    pub cells: Vec<UnvalidatedEntry>,
    /// Attunement per element name: how much mana one gem of it holds.
    #[serde(default)]
    pub attunement: BTreeMap<String, u16>,
    /// Channelling per element name: how much a channel action restores.
    #[serde(default)]
    pub channelling: BTreeMap<String, u16>,
}

/// One cell and where it sits.
#[derive(Reflect, Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnvalidatedEntry {
    /// The axial coordinate.
    pub at: AxialPair,
    /// What is there.
    pub kind: UnvalidatedCell,
}

/// An axial coordinate as written.
///
/// Its own type rather than `LatticeCoord` because that one's fields are private, and a
/// tuple would read as `(0, 0)` with nothing saying which is which.
#[derive(Reflect, Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AxialPair {
    /// The first axial component.
    pub q: i32,
    /// The second axial component.
    pub r: i32,
}

/// What a cell holds, by name.
#[derive(Reflect, Debug, Clone, Deserialize)]
pub enum UnvalidatedCell {
    /// A gem of the named element.
    Gem(String),
    /// A fusion producing the named higher-order element.
    Fusion(String),
    /// The named spell.
    Spell(String),
    /// Part of the lattice, holding nothing.
    Blank,
}

/// Loads the file and resolves it into the library.
pub fn plugin(app: &mut App) {
    app.register_type::<LatticeFile>()
        .load_settings::<LatticeFile>("config/lattices.ron", CONFIG_EXTENSIONS);
    app.add_systems(
        Update,
        build_lattice_library.run_if(not(in_state(Screen::Gameplay))),
    );
}

/// Rebuilds the library whenever the file or either catalog changes.
///
/// Gated on *changed* rather than on the library being absent, so a persistently bad
/// file is logged once per edit rather than once per frame. Rebuilt only outside
/// gameplay, like the tables it draws on, so resolved ids never shift under a live
/// world — a lattice already attached to a unit would otherwise be reinterpreted
/// mid-fight.
fn build_lattice_library(
    mut commands: Commands,
    file: Option<Res<LatticeFile>>,
    elements: Option<Res<ElementCatalog>>,
    spells: Option<Res<SpellBook>>,
) {
    let (Some(file), Some(elements), Some(spells)) = (file, elements, spells) else {
        return;
    };
    if !file.is_changed() && !elements.is_changed() && !spells.is_changed() {
        return;
    }
    match LatticeLibrary::build(&file, &elements, &spells) {
        Ok(library) => commands.insert_resource(library),
        Err(errors) => {
            for error in &errors {
                error!("lattices: {error}");
            }
            // Keep the last valid library, mirroring the settings loader.
        }
    }
}
