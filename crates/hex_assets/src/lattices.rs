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

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use hex_core::{ElementId, LatticeCoord, Screen, SpellId};
use hex_lattice::{CellKind, LatticeSpec, LatticeStats};
use serde::Deserialize;
use thiserror::Error;

use crate::elements::ElementCatalog;
use crate::fingerprint::FingerprintEncoder;
use crate::spells::{Effect, SpellBook};
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
    /// Cells form more than one disconnected island.
    #[error(
        "archetype '{archetype}' has a disconnected lattice: all authored cells must \
         join one contiguous hex arrangement"
    )]
    Disconnected {
        /// Which archetype.
        archetype: String,
    },
    /// A spell cell holding an area spell with an unsupported unit effect.
    ///
    /// `hex_units::volumes` resolves a shape to an exact voxel set and the area applier
    /// delivers Disable and Burn to every snapshotted occupant. Restore and Reveal
    /// remain fail-closed because their exact-choice and hidden-information policies
    /// are not settled for an area.
    ///
    /// Refusing at load keeps the unsupported promise out of gameplay rather than
    /// silently reducing it to the selected anchor.
    #[error(
        "archetype '{archetype}' inscribes '{spell}', whose area shape carries a unit \
         effect that is not safely delivered to every occupant"
    )]
    AreaEffectUnapplied {
        /// Which archetype.
        archetype: String,
        /// The spell whose area effect remains unsupported.
        spell: String,
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
    /// Default AI profile for hostile units of this archetype.
    pub ai_profile: Option<String>,
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
    #[reflect(ignore)]
    source_file: u64,
    #[reflect(ignore)]
    source_elements: u64,
    #[reflect(ignore)]
    source_spells: u64,
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
            Ok(Self {
                archetypes,
                source_file: lattice_file_fingerprint(file),
                source_elements: elements.source_fingerprint(),
                source_spells: spells.source_fingerprint(),
            })
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

    /// Whether this library was resolved from these exact source semantics.
    #[must_use]
    pub fn matches_sources(
        &self,
        file: &LatticeFile,
        elements: &ElementCatalog,
        spells: &SpellBook,
    ) -> bool {
        self.source_file == lattice_file_fingerprint(file)
            && self.source_elements == elements.source_fingerprint()
            && self.source_spells == spells.source_fingerprint()
    }

    pub(crate) fn source_revision(&self) -> u64 {
        let mut encoder = FingerprintEncoder::new(b"hex-lattice-library-sources-v1");
        encoder.u64(self.source_file);
        encoder.u64(self.source_elements);
        encoder.u64(self.source_spells);
        encoder.finish()
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

    let coordinates: BTreeSet<_> = raw
        .cells
        .iter()
        .map(|entry| LatticeCoord::new(entry.at.q, entry.at.r))
        .collect();
    if coordinates.len() == raw.cells.len() && !lattice_is_connected(&coordinates) {
        errors.push(LatticeError::Disconnected {
            archetype: name.to_owned(),
        });
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
                Some(id) if area_effect_is_unapplied(spells, id) => {
                    errors.push(LatticeError::AreaEffectUnapplied {
                        archetype: name.to_owned(),
                        spell: spell.clone(),
                    });
                    None
                }
                Some(id) => Some(CellKind::Spell { spell: id }),
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
            ai_profile: raw.ai_profile.clone(),
        })
    } else {
        Err(errors)
    }
}

fn lattice_is_connected(cells: &BTreeSet<LatticeCoord>) -> bool {
    let Some(&first) = cells.first() else {
        return false;
    };
    let mut reached = BTreeSet::from([first]);
    let mut frontier = vec![first];
    while let Some(current) = frontier.pop() {
        for neighbor in current.neighbors() {
            if cells.contains(&neighbor) && reached.insert(neighbor) {
                frontier.push(neighbor);
            }
        }
    }
    reached.len() == cells.len()
}

fn lattice_file_fingerprint(file: &LatticeFile) -> u64 {
    let mut encoder = FingerprintEncoder::new(b"hex-lattice-file-v1");
    encoder.usize(file.archetypes.len());
    for (name, archetype) in &file.archetypes {
        encoder.string(name);

        let mut cells: Vec<_> = archetype.cells.iter().collect();
        cells.sort_by_key(|entry| (entry.at.q, entry.at.r));
        encoder.usize(cells.len());
        for entry in cells {
            encoder.i32(entry.at.q);
            encoder.i32(entry.at.r);
            match &entry.kind {
                UnvalidatedCell::Gem(element) => {
                    encoder.u8(0);
                    encoder.string(element);
                }
                UnvalidatedCell::Fusion(element) => {
                    encoder.u8(1);
                    encoder.string(element);
                }
                UnvalidatedCell::Spell(spell) => {
                    encoder.u8(2);
                    encoder.string(spell);
                }
                UnvalidatedCell::Blank => encoder.u8(3),
            }
        }

        encoder.usize(archetype.attunement.len());
        for (element, amount) in &archetype.attunement {
            encoder.string(element);
            encoder.u16(*amount);
        }
        encoder.usize(archetype.channelling.len());
        for (element, amount) in &archetype.channelling {
            encoder.string(element);
            encoder.u16(*amount);
        }
        if let Some(profile) = &archetype.ai_profile {
            encoder.u8(1);
            encoder.string(profile);
        } else {
            encoder.u8(0);
        }
    }
    encoder.finish()
}

/// Whether `spell` would have the interface promise more than the applier delivers.
///
/// True when the shape covers more than the anchor and at least one effect still lacks
/// an accepted area policy. Disable and Burn now reach every snapshotted occupant;
/// targeted Disable, Restore, one-shot wards, and Reveal remain fail-closed.
///
/// See [`LatticeError::AreaEffectUnapplied`] for why this is refused rather than clamped.
fn area_effect_is_unapplied(spells: &SpellBook, spell: SpellId) -> bool {
    let Some(spell) = spells.spell(spell) else {
        return false;
    };
    if !spell.targeting.shape.can_cover_multiple_voxels() {
        return false;
    }
    spell.effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::DisableHexes { targeted: true, .. }
                | Effect::RestoreHexes { .. }
                | Effect::ModifyIncomingDisables { .. }
                | Effect::Reveal { .. }
        )
    })
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
    /// Default AI profile for hostile units of this archetype.
    #[serde(default)]
    pub ai_profile: Option<String>,
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

#[cfg(test)]
mod tests {
    use bevy::platform::collections::HashMap;
    use hex_core::{HexCoord, Level};

    use crate::spells::{
        CastingAxis, ManaAxis, Spell, SpellFile, TargetShape, TargetingSpec, Trajectory,
        VoxelOffset,
    };

    use super::*;

    const GROUND: Level = 0;

    fn offset(q: i32, r: i32, level: Level) -> VoxelOffset {
        VoxelOffset {
            coord: HexCoord::from_axial(q, r),
            level,
        }
    }

    fn blank(q: i32, r: i32) -> UnvalidatedEntry {
        UnvalidatedEntry {
            at: AxialPair { q, r },
            kind: UnvalidatedCell::Blank,
        }
    }

    fn file_with_cells(cells: Vec<UnvalidatedEntry>) -> LatticeFile {
        LatticeFile {
            archetypes: BTreeMap::from([(
                "Tester".to_owned(),
                UnvalidatedArchetype {
                    cells,
                    attunement: BTreeMap::new(),
                    channelling: BTreeMap::new(),
                    ai_profile: None,
                },
            )]),
        }
    }

    #[test]
    fn disconnected_authored_lattice_names_its_archetype() {
        let errors = LatticeLibrary::build(
            &file_with_cells(vec![blank(0, 0), blank(2, 0)]),
            &ElementCatalog::default(),
            &SpellBook::default(),
        )
        .expect_err("separate lattice islands must be rejected");

        assert!(errors.iter().any(|error| matches!(
            error,
            LatticeError::Disconnected { archetype } if archetype == "Tester"
        )));
        assert!(errors
            .iter()
            .find_map(|error| matches!(error, LatticeError::Disconnected { .. })
                .then(|| error.to_string()))
            .is_some_and(|message| message.contains("Tester")));
    }

    #[test]
    fn contiguous_lattice_and_cell_reordering_share_one_semantic_source() {
        let elements = ElementCatalog::default();
        let spells = SpellBook::default();
        let first = file_with_cells(vec![blank(0, 0), blank(1, 0), blank(1, -1)]);
        let reordered = file_with_cells(vec![blank(1, -1), blank(0, 0), blank(1, 0)]);

        let library = LatticeLibrary::build(&first, &elements, &spells)
            .expect("one connected hex arrangement should resolve");
        assert!(library.matches_sources(&reordered, &elements, &spells));
    }

    #[test]
    fn only_shapes_that_can_resolve_to_multiple_distinct_voxels_are_area() {
        let single_cardinality = [
            TargetShape::SelfCast,
            TargetShape::Single,
            TargetShape::Sphere { radius: 0 },
            TargetShape::Column { height: 1 },
            TargetShape::Line {
                length: 1,
                width: 0,
            },
            TargetShape::Cone {
                length: 1,
                spread: 0,
            },
            TargetShape::Path {
                offsets: vec![offset(0, 0, GROUND)],
            },
            TargetShape::Path {
                offsets: vec![offset(0, 0, GROUND), offset(0, 0, GROUND)],
            },
        ];
        for shape in single_cardinality {
            assert!(
                !shape.can_cover_multiple_voxels(),
                "{shape:?} resolves to at most one distinct voxel"
            );
        }

        let multiple_cardinality = [
            TargetShape::Sphere { radius: 1 },
            TargetShape::Column { height: 2 },
            TargetShape::Line {
                length: 2,
                width: 0,
            },
            TargetShape::Line {
                length: 1,
                width: 1,
            },
            TargetShape::Cone {
                length: 2,
                spread: 0,
            },
            TargetShape::Cone {
                length: 1,
                spread: 1,
            },
            TargetShape::Path {
                offsets: vec![offset(0, 0, GROUND), offset(1, 0, GROUND)],
            },
            TargetShape::Path {
                offsets: vec![offset(0, 0, GROUND), offset(0, 0, 1)],
            },
        ];
        for shape in multiple_cardinality {
            assert!(
                shape.can_cover_multiple_voxels(),
                "{shape:?} can resolve to multiple distinct voxels"
            );
        }
    }

    fn area_spell_book(effects: Vec<Effect>) -> SpellBook {
        let mut spells = HashMap::default();
        spells.insert(
            "Area".to_owned(),
            Spell {
                requirements: Vec::new(),
                casting: CastingAxis::Evocation,
                mana: ManaAxis::Fixed,
                co_castable: false,
                targeting: TargetingSpec {
                    range: 3,
                    reach: crate::TargetingReach::Ranged,
                    shape: TargetShape::Sphere { radius: 2 },
                    trajectory: Trajectory::None,
                },
                effects,
            },
        );
        SpellBook::from_file(&SpellFile { spells })
    }

    fn file_with_area_spell() -> LatticeFile {
        file_with_cells(vec![UnvalidatedEntry {
            at: AxialPair { q: 0, r: 0 },
            kind: UnvalidatedCell::Spell("Area".to_owned()),
        }])
    }

    #[test]
    fn area_disable_burn_and_impact_are_admitted_to_lattices() {
        let spells = area_spell_book(vec![
            Effect::DisableHexes {
                count: 3,
                targeted: false,
            },
            Effect::Burn { turns: 2 },
            Effect::Impact {
                element: "Fire".to_owned(),
                power: 2,
            },
        ]);

        LatticeLibrary::build(&file_with_area_spell(), &ElementCatalog::default(), &spells)
            .expect("supported area effects should be inscribable");
    }

    #[test]
    fn area_restore_reveal_and_targeted_disable_remain_fail_closed() {
        for effect in [
            Effect::RestoreHexes { count: 1 },
            Effect::Reveal { tier: 1 },
            Effect::DisableHexes {
                count: 1,
                targeted: true,
            },
        ] {
            let spells = area_spell_book(vec![effect]);
            let errors =
                LatticeLibrary::build(&file_with_area_spell(), &ElementCatalog::default(), &spells)
                    .expect_err("unsupported area unit policy must remain fail-closed");
            assert!(errors.iter().any(|error| matches!(
                error,
                LatticeError::AreaEffectUnapplied { spell, .. } if spell == "Area"
            )));
        }
    }
}
