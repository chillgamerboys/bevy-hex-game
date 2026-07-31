//! Stable, name-based authoring contracts for the in-game creator.
//!
//! Runtime element and spell ids are rebuilt from sorted content names. Persisting
//! those ids would silently reinterpret a saved character after content changes, so
//! creator files keep names and opaque local ids and resolve them only for launch.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use hex_core::LatticeCoord;
use serde::{Deserialize, Serialize};

use crate::{
    AxialPair, CastingAxis, Effect, ElementCatalog, LatticeFile, ManaAxis, Spell, SpellBook,
    SpellFile, TargetShape, TargetingSpec, UnvalidatedArchetype, UnvalidatedCell, UnvalidatedEntry,
};
use crate::{LoadSettings, CONFIG_EXTENSIONS};

/// Current on-disk creator schema.
pub const CREATION_SCHEMA_VERSION: u32 = 1;
/// Technical lattice guardrail, not a balance rule.
pub const MAX_CREATION_CELLS: usize = 64;
/// Axial radius guardrail for malformed or mechanically extended drafts.
pub const MAX_CREATION_RADIUS: i32 = 64;
/// Player-facing creator names are deliberately compact.
pub const MAX_CREATION_NAME_CHARS: usize = 32;

/// Whether a packaged record is offered to humans or reserved for automation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresetAudience {
    /// Read-only, visible, and duplicable in the Creator and Sandbox.
    HumanTemplate,
    /// Immutable data addressed only by fixed fixture ids.
    AutomationFixture,
}

/// One packaged character using the same record shape as local persistence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackagedCharacter {
    /// Stable packaged key, independent of display name.
    pub key: String,
    /// Which UI may offer this record.
    pub audience: PresetAudience,
    /// Immutable creator-format definition.
    pub character: SavedCharacter,
}

/// One packaged spell using the same record shape as local persistence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackagedSpell {
    /// Stable packaged key, independent of display name.
    pub key: String,
    /// Which UI may offer this record.
    pub audience: PresetAudience,
    /// Immutable creator-format definition.
    pub spell: SavedSpell,
}

/// Shipped read-only creator records.
#[derive(Asset, Resource, TypePath, Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreationPresetCatalog {
    /// Catalog schema, currently aligned with the local creation file.
    pub version: u32,
    /// Packaged characters.
    pub characters: Vec<PackagedCharacter>,
    /// Packaged spells.
    pub spells: Vec<PackagedSpell>,
}

impl CreationPresetCatalog {
    /// Materializes an isolated library for one audience.
    #[must_use]
    pub fn library_for(&self, audience: PresetAudience) -> CreationLibraryFile {
        let characters = self
            .characters
            .iter()
            .filter(|record| record.audience == audience)
            .map(|record| record.character.clone())
            .collect::<Vec<_>>();
        let spells = self
            .spells
            .iter()
            .filter(|record| record.audience == audience)
            .map(|record| record.spell.clone())
            .collect::<Vec<_>>();
        CreationLibraryFile {
            version: self.version,
            next_character_id: characters
                .iter()
                .map(|record| record.id.0)
                .max()
                .unwrap_or(0)
                .saturating_add(1),
            next_spell_id: spells
                .iter()
                .map(|record| record.id.0)
                .max()
                .unwrap_or(0)
                .saturating_add(1),
            characters,
            spells,
        }
    }
}

/// Loads immutable creator presets.
pub fn plugin(app: &mut App) {
    app.load_settings::<CreationPresetCatalog>("config/creation_presets.ron", CONFIG_EXTENSIONS);
}

/// Stable local identity for an editable character.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct CustomCharacterId(pub u64);

/// Stable local identity for an editable spell.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct CustomSpellId(pub u64);

/// A spell cell references shipped content by name or editable content by stable id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpellReference {
    /// Immutable content from `spells.ron`.
    Shipped(String),
    /// A spell in the same creation library.
    Custom(CustomSpellId),
}

/// One name-based creator cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CreationCellKind {
    /// A basic element gem.
    Gem(String),
    /// A higher-order fusion output.
    Fusion(String),
    /// A shipped or custom spell inscription.
    Spell(SpellReference),
    /// Durability with no magical content.
    Blank,
}

/// One cell in an editable lattice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreationCell {
    /// First axial coordinate.
    pub q: i32,
    /// Second axial coordinate.
    pub r: i32,
    /// Cell contents.
    pub kind: CreationCellKind,
}

impl CreationCell {
    /// Coordinate in the pure lattice engine's vocabulary.
    #[must_use]
    pub const fn coord(&self) -> LatticeCoord {
        LatticeCoord::new(self.q, self.r)
    }
}

/// A saved character draft. Invalid drafts are intentionally representable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedCharacter {
    /// Stable local identity.
    pub id: CustomCharacterId,
    /// Editable display name.
    pub name: String,
    /// Name-based lattice drawing.
    pub cells: Vec<CreationCell>,
    /// Mana capacity per element.
    #[serde(default)]
    pub attunement: BTreeMap<String, u16>,
    /// Mana restored by Channel per element.
    #[serde(default)]
    pub channelling: BTreeMap<String, u16>,
}

impl SavedCharacter {
    /// A blank draft with an origin cell, ready for the editor to shape.
    #[must_use]
    pub fn blank(id: CustomCharacterId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            cells: vec![CreationCell {
                q: 0,
                r: 0,
                kind: CreationCellKind::Blank,
            }],
            attunement: BTreeMap::new(),
            channelling: BTreeMap::new(),
        }
    }

    /// Custom spell ids referenced by this lattice, in stable order.
    pub fn custom_spell_references(&self) -> impl Iterator<Item = CustomSpellId> + '_ {
        self.cells.iter().filter_map(|cell| match cell.kind {
            CreationCellKind::Spell(SpellReference::Custom(id)) => Some(id),
            _ => None,
        })
    }
}

/// A saved spell draft. Its status is always derived, never persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSpell {
    /// Stable local identity.
    pub id: CustomSpellId,
    /// Editable display and runtime content name.
    pub name: String,
    /// Structurally representable spell draft.
    pub spell: Spell,
}

impl SavedSpell {
    /// An intentionally incomplete draft.
    #[must_use]
    pub fn blank(id: CustomSpellId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            spell: Spell {
                requirements: Vec::new(),
                casting: CastingAxis::Evocation,
                mana: ManaAxis::Fixed,
                co_castable: false,
                targeting: TargetingSpec {
                    range: 3,
                    shape: TargetShape::Single,
                    needs_los: false,
                },
                effects: Vec::new(),
            },
        }
    }
}

/// The one atomic local creator file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreationLibraryFile {
    /// On-disk schema version.
    pub version: u32,
    /// Next monotonic character id.
    pub next_character_id: u64,
    /// Next monotonic spell id.
    pub next_spell_id: u64,
    /// Editable local characters.
    #[serde(default)]
    pub characters: Vec<SavedCharacter>,
    /// Editable local spells.
    #[serde(default)]
    pub spells: Vec<SavedSpell>,
}

impl Default for CreationLibraryFile {
    fn default() -> Self {
        Self {
            version: CREATION_SCHEMA_VERSION,
            next_character_id: 1,
            next_spell_id: 1,
            characters: Vec::new(),
            spells: Vec::new(),
        }
    }
}

impl CreationLibraryFile {
    /// Allocates a stable character id without reuse.
    #[must_use]
    pub fn allocate_character_id(&mut self) -> CustomCharacterId {
        let id = CustomCharacterId(self.next_character_id.max(1));
        self.next_character_id = id.0.saturating_add(1);
        id
    }

    /// Allocates a stable spell id without reuse.
    #[must_use]
    pub fn allocate_spell_id(&mut self) -> CustomSpellId {
        let id = CustomSpellId(self.next_spell_id.max(1));
        self.next_spell_id = id.0.saturating_add(1);
        id
    }

    /// File-level integrity that must hold even when individual records are drafts.
    pub fn validate_integrity(&self) -> Result<(), Vec<String>> {
        let mut issues = Vec::new();
        if self.version != CREATION_SCHEMA_VERSION {
            issues.push(format!(
                "creation library version {} is unsupported; expected {}",
                self.version, CREATION_SCHEMA_VERSION
            ));
        }

        let mut character_ids = BTreeSet::new();
        let mut spell_ids = BTreeSet::new();
        let mut character_names = BTreeSet::new();
        let mut spell_names = BTreeSet::new();

        for character in &self.characters {
            if character.id.0 == 0 || !character_ids.insert(character.id) {
                issues.push(format!("duplicate or zero character id {}", character.id.0));
            }
            if let Err(issue) = validate_name(&character.name) {
                issues.push(format!("character {}: {issue}", character.id.0));
            }
            if !character_names.insert(normalized_name(&character.name)) {
                issues.push(format!("character name {:?} is duplicated", character.name));
            }
        }

        for spell in &self.spells {
            if spell.id.0 == 0 || !spell_ids.insert(spell.id) {
                issues.push(format!("duplicate or zero spell id {}", spell.id.0));
            }
            if let Err(issue) = validate_name(&spell.name) {
                issues.push(format!("spell {}: {issue}", spell.id.0));
            }
            if !spell_names.insert(normalized_name(&spell.name)) {
                issues.push(format!("spell name {:?} is duplicated", spell.name));
            }
        }

        for character in &self.characters {
            for id in character.custom_spell_references() {
                if !spell_ids.contains(&id) {
                    issues.push(format!(
                        "character {:?} references missing custom spell {}",
                        character.name, id.0
                    ));
                }
            }
        }

        let largest_character = character_ids.iter().map(|id| id.0).max().unwrap_or(0);
        let largest_spell = spell_ids.iter().map(|id| id.0).max().unwrap_or(0);
        if self.next_character_id <= largest_character {
            issues.push("next_character_id would reuse an existing id".to_owned());
        }
        if self.next_spell_id <= largest_spell {
            issues.push("next_spell_id would reuse an existing id".to_owned());
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }

    /// Saved characters that currently inscribe `id`.
    #[must_use]
    pub fn spell_dependents(&self, id: CustomSpellId) -> Vec<&SavedCharacter> {
        self.characters
            .iter()
            .filter(|character| character.custom_spell_references().any(|used| used == id))
            .collect()
    }

    /// Deletes an unreferenced spell or returns dependent character names.
    pub fn delete_spell(&mut self, id: CustomSpellId) -> Result<(), Vec<String>> {
        let dependents = self.spell_dependents(id);
        if !dependents.is_empty() {
            return Err(dependents
                .iter()
                .map(|character| character.name.clone())
                .collect());
        }
        self.spells.retain(|spell| spell.id != id);
        Ok(())
    }

    /// Deletes one character while preserving monotonic identity.
    pub fn delete_character(&mut self, id: CustomCharacterId) {
        self.characters.retain(|character| character.id != id);
    }

    /// Current display/runtime name for a custom spell id.
    #[must_use]
    pub fn custom_spell_name(&self, id: CustomSpellId) -> Option<&str> {
        self.spells
            .iter()
            .find(|spell| spell.id == id)
            .map(|spell| spell.name.as_str())
    }
}

/// Trims the comparison domain to case-insensitive player-facing identity.
#[must_use]
pub fn normalized_name(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Validates the shared player-facing naming contract.
pub fn validate_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("name cannot be empty".to_owned());
    }
    if trimmed.chars().count() > MAX_CREATION_NAME_CHARS {
        return Err(format!(
            "name is longer than {MAX_CREATION_NAME_CHARS} characters"
        ));
    }
    if trimmed.chars().any(char::is_control) {
        return Err("name cannot contain control characters".to_owned());
    }
    if trimmed != name {
        return Err("name cannot begin or end with whitespace".to_owned());
    }
    Ok(())
}

/// Structural and creator-scope validation for one spell draft.
#[must_use]
pub fn creator_spell_issues(saved: &SavedSpell, elements: &ElementCatalog) -> Vec<String> {
    let mut issues = Vec::new();
    if let Err(issue) = validate_name(&saved.name) {
        issues.push(issue);
    }

    let mut spells = HashMap::default();
    spells.insert(saved.name.clone(), saved.spell.clone());
    if let Err(issue) = (SpellFile { spells }).validate() {
        issues.push(issue);
    }

    if saved.spell.mana != ManaAxis::Fixed {
        issues.push("creator spells must use fixed mana".to_owned());
    }
    if saved.spell.co_castable {
        issues.push("co-casting is not supported by the creator".to_owned());
    }
    if saved.spell.targeting.needs_los {
        issues.push("creator spells cannot require line of sight yet".to_owned());
    }
    if !matches!(
        saved.spell.targeting.shape,
        TargetShape::SelfCast | TargetShape::Single
    ) {
        issues.push("creator spells must target Self or Single".to_owned());
    }
    if saved.spell.targeting.range > 16 {
        issues.push("creator spell range cannot exceed 16".to_owned());
    }

    for requirement in &saved.spell.requirements {
        if elements.id(&requirement.element).is_none() {
            issues.push(format!(
                "requirement names unknown element {:?}",
                requirement.element
            ));
        }
        if requirement.mana > 64 {
            issues.push(format!(
                "requirement for {:?} exceeds the mana guardrail of 64",
                requirement.element
            ));
        }
    }

    for effect in &saved.spell.effects {
        if !matches!(
            effect,
            Effect::DisableHexes {
                targeted: false,
                ..
            } | Effect::Burn { .. }
                | Effect::RestoreHexes { .. }
                | Effect::Reveal { .. }
        ) {
            issues.push(format!("effect {effect:?} is not delivered by Wave 6"));
        }
    }
    issues
}

/// Character validation independent of runtime id assignment.
#[must_use]
pub fn creator_character_issues(
    character: &SavedCharacter,
    library: &CreationLibraryFile,
    elements: &ElementCatalog,
    shipped_spells: &SpellBook,
) -> Vec<String> {
    let mut issues = Vec::new();
    if let Err(issue) = validate_name(&character.name) {
        issues.push(issue);
    }
    if character.cells.is_empty() {
        issues.push("lattice must contain at least one cell".to_owned());
        return issues;
    }
    if character.cells.len() > MAX_CREATION_CELLS {
        issues.push(format!(
            "lattice has {} cells; maximum is {MAX_CREATION_CELLS}",
            character.cells.len()
        ));
    }

    let mut coords = BTreeSet::new();
    for cell in &character.cells {
        let radius = cell
            .q
            .unsigned_abs()
            .max(cell.r.unsigned_abs())
            .max(cell.q.saturating_add(cell.r).unsigned_abs());
        if radius > MAX_CREATION_RADIUS.unsigned_abs() {
            issues.push(format!(
                "cell ({}, {}) lies outside the creator radius of {MAX_CREATION_RADIUS}",
                cell.q, cell.r
            ));
        }
        if !coords.insert(cell.coord()) {
            issues.push(format!("two cells occupy ({}, {})", cell.q, cell.r));
        }
        match &cell.kind {
            CreationCellKind::Gem(name) => match elements.id(name) {
                Some(id) if elements.is_higher_order(id) => {
                    issues.push(format!("{name:?} is a fusion output, not a gem"));
                }
                Some(_) => {}
                None => issues.push(format!("unknown gem element {name:?}")),
            },
            CreationCellKind::Fusion(name) => match elements.id(name) {
                Some(id) if elements.is_higher_order(id) => {}
                Some(_) => issues.push(format!("{name:?} has no fusion recipe")),
                None => issues.push(format!("unknown fusion output {name:?}")),
            },
            CreationCellKind::Spell(SpellReference::Shipped(name)) => {
                if shipped_spells.id(name).is_none() {
                    issues.push(format!("unknown shipped spell {name:?}"));
                }
            }
            CreationCellKind::Spell(SpellReference::Custom(id)) => {
                match library.spells.iter().find(|spell| spell.id == *id) {
                    Some(spell) => {
                        for issue in creator_spell_issues(spell, elements) {
                            issues.push(format!("spell {:?}: {issue}", spell.name));
                        }
                    }
                    None => issues.push(format!("missing custom spell {}", id.0)),
                }
            }
            CreationCellKind::Blank => {}
        }
    }

    if !coords.contains(&LatticeCoord::ORIGIN) {
        issues.push("lattice must contain the origin cell (0, 0)".to_owned());
    }
    if !coords.is_empty() {
        let mut reached = BTreeSet::new();
        let Some(first) = coords.first().copied() else {
            return issues;
        };
        let mut queue = VecDeque::from([first]);
        while let Some(coord) = queue.pop_front() {
            if !reached.insert(coord) {
                continue;
            }
            queue.extend(
                coord
                    .neighbors()
                    .into_iter()
                    .filter(|neighbor| coords.contains(neighbor)),
            );
        }
        if reached.len() != coords.len() {
            issues.push("lattice cells must form one contiguous shape".to_owned());
        }
    }

    for (field, values) in [
        ("attunement", &character.attunement),
        ("channelling", &character.channelling),
    ] {
        for (element, amount) in values {
            if elements.id(element).is_none() {
                issues.push(format!("{field} names unknown element {element:?}"));
            }
            if *amount > 64 {
                issues.push(format!("{field} for {element:?} exceeds 64"));
            }
        }
    }
    issues
}

/// Converts a saved character into the existing authored lattice shape.
///
/// Custom spell ids are resolved to their current display names at this boundary.
pub fn character_lattice_file(
    character: &SavedCharacter,
    library: &CreationLibraryFile,
) -> Result<LatticeFile, String> {
    let cells = character
        .cells
        .iter()
        .map(|cell| {
            let kind = match &cell.kind {
                CreationCellKind::Gem(name) => UnvalidatedCell::Gem(name.clone()),
                CreationCellKind::Fusion(name) => UnvalidatedCell::Fusion(name.clone()),
                CreationCellKind::Spell(SpellReference::Shipped(name)) => {
                    UnvalidatedCell::Spell(name.clone())
                }
                CreationCellKind::Spell(SpellReference::Custom(id)) => {
                    let name = library
                        .custom_spell_name(*id)
                        .ok_or_else(|| format!("missing custom spell {}", id.0))?;
                    UnvalidatedCell::Spell(name.to_owned())
                }
                CreationCellKind::Blank => UnvalidatedCell::Blank,
            };
            Ok(UnvalidatedEntry {
                at: AxialPair {
                    q: cell.q,
                    r: cell.r,
                },
                kind,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(LatticeFile {
        archetypes: BTreeMap::from([(
            character_runtime_key(character.id),
            UnvalidatedArchetype {
                cells,
                attunement: character.attunement.clone(),
                channelling: character.channelling.clone(),
                ai_profile: Some("baseline".to_owned()),
            },
        )]),
    })
}

/// Runtime key deliberately independent from the editable display name.
#[must_use]
pub fn character_runtime_key(id: CustomCharacterId) -> String {
    format!("custom-character-{}", id.0)
}

/// A combined spell file for a frozen creator launch.
///
/// Only Ready custom spells should be passed; name collisions fail explicitly.
pub fn combined_spell_file(
    shipped: &SpellFile,
    custom: impl IntoIterator<Item = SavedSpell>,
) -> Result<SpellFile, String> {
    let mut spells = shipped.spells.clone();
    for saved in custom {
        if spells.insert(saved.name.clone(), saved.spell).is_some() {
            return Err(format!(
                "custom spell {:?} collides with shipped content",
                saved.name
            ));
        }
    }
    let file = SpellFile { spells };
    file.validate()?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_monotonic_and_never_start_at_zero() {
        let mut library = CreationLibraryFile::default();
        assert_eq!(library.allocate_character_id(), CustomCharacterId(1));
        assert_eq!(library.allocate_character_id(), CustomCharacterId(2));
        assert_eq!(library.allocate_spell_id(), CustomSpellId(1));
    }

    #[test]
    fn legacy_clear_terrain_library_records_decode_but_remain_invalid_drafts() {
        let mut library = CreationLibraryFile::default();
        let mut saved = SavedSpell::blank(library.allocate_spell_id(), "Old Dig");
        saved.spell.requirements.push(crate::GemRequirement {
            element: "Earth".to_owned(),
            mana: 1,
        });
        saved.spell.effects.push(Effect::ClearTerrain);
        library.spells.push(saved);

        let encoded = ron::to_string(&library).expect("legacy-compatible library serializes");
        let decoded: CreationLibraryFile =
            ron::from_str(&encoded).expect("legacy ClearTerrain remains decode-compatible");
        assert!(decoded.validate_integrity().is_ok());

        let saved = decoded
            .spells
            .first()
            .expect("the decoded library retains its legacy spell");
        let mut spells = HashMap::default();
        spells.insert(saved.name.clone(), saved.spell.clone());
        let issue = (SpellFile { spells })
            .validate()
            .expect_err("legacy effect is retained only as an invalid draft");
        assert!(issue.contains("decode-only"), "{issue}");
    }

    #[test]
    fn referenced_spell_cannot_be_deleted() {
        let mut library = CreationLibraryFile::default();
        let spell_id = library.allocate_spell_id();
        library.spells.push(SavedSpell::blank(spell_id, "Draft"));
        let character_id = library.allocate_character_id();
        let mut character = SavedCharacter::blank(character_id, "Tester");
        if let Some(origin) = character.cells.first_mut() {
            origin.kind = CreationCellKind::Spell(SpellReference::Custom(spell_id));
        }
        library.characters.push(character);

        assert_eq!(
            library.delete_spell(spell_id),
            Err(vec!["Tester".to_owned()])
        );
        assert_eq!(library.spells.len(), 1);
    }

    #[test]
    fn connectivity_and_origin_are_checked() {
        let character = SavedCharacter {
            id: CustomCharacterId(1),
            name: "Broken".to_owned(),
            cells: vec![
                CreationCell {
                    q: 1,
                    r: 0,
                    kind: CreationCellKind::Blank,
                },
                CreationCell {
                    q: 3,
                    r: 0,
                    kind: CreationCellKind::Blank,
                },
            ],
            attunement: BTreeMap::new(),
            channelling: BTreeMap::new(),
        };
        let coords = character
            .cells
            .iter()
            .map(CreationCell::coord)
            .collect::<BTreeSet<_>>();
        assert!(!coords.contains(&LatticeCoord::ORIGIN));
        assert!(!coords
            .iter()
            .any(|coord| coord.neighbors().into_iter().any(|n| coords.contains(&n))));
    }

    #[test]
    fn shipped_preset_catalog_is_creator_format_and_integral() {
        let catalog: CreationPresetCatalog = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/creation_presets.ron"
        )))
        .expect("creation_presets.ron parses");
        assert_eq!(catalog.version, CREATION_SCHEMA_VERSION);
        for audience in [
            PresetAudience::HumanTemplate,
            PresetAudience::AutomationFixture,
        ] {
            catalog
                .library_for(audience)
                .validate_integrity()
                .expect("packaged records keep stable ids and references");
        }
        assert!(catalog
            .characters
            .iter()
            .any(|record| record.key == "fixture-caster"));
    }
}
