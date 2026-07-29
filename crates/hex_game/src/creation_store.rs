//! Atomic local persistence for creator records.
//!
//! Characters and spells share one file so a successful write can never leave a
//! character pointing at a spell from a different generation.

use std::io;

use bevy::prelude::*;
use hex_assets::{CreationLibraryFile, SavedCharacter, SavedSpell};
use ron::ser::PrettyConfig;

use crate::storage::{read, write_atomic, StoragePaths};

/// Loaded creator data and the last player-visible storage problem.
#[derive(Resource, Debug, Clone, Default)]
pub(crate) struct CreationStore {
    pub(crate) file: CreationLibraryFile,
    pub(crate) error: Option<String>,
}

impl CreationStore {
    pub(crate) fn save_character(
        &mut self,
        character: SavedCharacter,
        paths: &StoragePaths,
    ) -> Result<(), String> {
        let before = self.file.clone();
        if let Some(existing) = self
            .file
            .characters
            .iter_mut()
            .find(|saved| saved.id == character.id)
        {
            *existing = character;
        } else {
            self.file.characters.push(character);
        }
        self.persist_or_restore(before, paths)
    }

    pub(crate) fn save_spell(
        &mut self,
        spell: SavedSpell,
        paths: &StoragePaths,
    ) -> Result<(), String> {
        let before = self.file.clone();
        if let Some(existing) = self
            .file
            .spells
            .iter_mut()
            .find(|saved| saved.id == spell.id)
        {
            *existing = spell;
        } else {
            self.file.spells.push(spell);
        }
        self.persist_or_restore(before, paths)
    }

    pub(crate) fn delete_character(
        &mut self,
        id: hex_assets::CustomCharacterId,
        paths: &StoragePaths,
    ) -> Result<(), String> {
        let before = self.file.clone();
        self.file.delete_character(id);
        self.persist_or_restore(before, paths)
    }

    pub(crate) fn delete_spell(
        &mut self,
        id: hex_assets::CustomSpellId,
        paths: &StoragePaths,
    ) -> Result<(), String> {
        let before = self.file.clone();
        if let Err(dependents) = self.file.delete_spell(id) {
            return Err(format!("used by {}", dependents.join(", ")));
        }
        self.persist_or_restore(before, paths)
    }

    pub(crate) fn reset(&mut self, paths: &StoragePaths) -> Result<(), String> {
        let before = self.file.clone();
        self.file = CreationLibraryFile::default();
        self.persist_or_restore(before, paths)
    }

    fn persist_or_restore(
        &mut self,
        before: CreationLibraryFile,
        paths: &StoragePaths,
    ) -> Result<(), String> {
        let result = persist(&self.file, paths);
        match result {
            Ok(()) => {
                self.error = None;
                Ok(())
            }
            Err(error) => {
                self.file = before;
                self.error = Some(error.clone());
                Err(error)
            }
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<StoragePaths>()
        .init_resource::<CreationStore>()
        .add_systems(Startup, load_creations);
}

fn load_creations(paths: Res<StoragePaths>, mut store: ResMut<CreationStore>) {
    match read(&paths.creations) {
        Ok(contents) => match ron::from_str::<CreationLibraryFile>(&contents) {
            Ok(file) => match file.validate_integrity() {
                Ok(()) => {
                    store.file = file;
                    store.error = None;
                }
                Err(issues) => {
                    store.error = Some(format!(
                        "creations.ron was preserved but refused: {}",
                        issues.join("; ")
                    ));
                }
            },
            Err(error) => {
                store.error = Some(format!(
                    "creations.ron was preserved but could not be parsed: {error}"
                ));
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            store.file = CreationLibraryFile::default();
            store.error = None;
        }
        Err(error) => {
            store.error = Some(format!("could not read creations.ron: {error}"));
        }
    }
}

fn persist(file: &CreationLibraryFile, paths: &StoragePaths) -> Result<(), String> {
    file.validate_integrity()
        .map_err(|issues| issues.join("; "))?;
    let serialized = ron::ser::to_string_pretty(file, PrettyConfig::new())
        .map_err(|error| format!("could not serialize creations: {error}"))?;
    write_atomic(&paths.creations, &serialized)
        .map_err(|error| format!("could not save creations: {error}"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use hex_assets::{CustomCharacterId, SavedCharacter};

    use super::*;

    fn paths(root: PathBuf) -> StoragePaths {
        StoragePaths {
            preferences: root.join("preferences.ron"),
            resume: root.join("resume.ron"),
            creations: root.join("creations.ron"),
        }
    }

    #[test]
    fn character_and_spell_library_round_trips_as_one_file() {
        let root = std::env::temp_dir().join(format!(
            "hex-game-creations-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let paths = paths(root);
        let mut file = CreationLibraryFile::default();
        let id = file.allocate_character_id();
        file.characters
            .push(SavedCharacter::blank(id, "Round Trip"));
        persist(&file, &paths).expect("write");
        let loaded: CreationLibraryFile =
            ron::from_str(&read(&paths.creations).expect("read")).expect("parse");
        assert_eq!(loaded, file);
        drop(std::fs::remove_file(&paths.creations));
        if let Some(parent) = paths.creations.parent() {
            drop(std::fs::remove_dir(parent));
        }
    }

    #[test]
    fn failed_integrity_does_not_replace_in_memory_data() {
        let root =
            std::env::temp_dir().join(format!("hex-game-invalid-creations-{}", std::process::id()));
        let paths = paths(root);
        let mut store = CreationStore::default();
        store
            .file
            .characters
            .push(SavedCharacter::blank(CustomCharacterId(1), "Existing"));
        let before = store.file.clone();
        let invalid = SavedCharacter::blank(CustomCharacterId(1), " Invalid");
        assert!(store.save_character(invalid, &paths).is_err());
        assert_eq!(store.file, before);
    }
}
