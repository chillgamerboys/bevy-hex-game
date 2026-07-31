//! Pure Creator navigation and lifecycle routing.

/// Creator library/editor surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CreatorSurface {
    /// Character library or editor.
    #[default]
    Characters,
    /// Spell library or editor.
    Spells,
}

/// Explicit entry intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatorEntry {
    /// Top-level character library.
    CharacterLibrary,
    /// Top-level spell library.
    SpellLibrary,
    /// Spell management entered from one character.
    SpellFromCharacter,
}

/// Destination chosen by an unblocked back action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatorDestination {
    /// Remain within Creator and return to the character editor.
    CharacterEditor,
    /// Return to Combat Lab.
    CombatLab,
    /// Return to the title screen.
    Title,
}

/// Navigation facts independent of drafts, widgets, and Bevy state resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CreatorNavigation {
    /// Active Creator surface.
    pub tab: CreatorSurface,
    /// Whether the eventual exit returns to Combat Lab.
    pub return_to_combat_lab: bool,
    /// Whether leaving spell management first returns to its character.
    pub return_to_character_creator: bool,
}

/// Bounded branch-aware edit history independent of the draft payload type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditHistory<Snapshot> {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    limit: usize,
}

impl<Snapshot> Default for EditHistory<Snapshot> {
    fn default() -> Self {
        Self::new(100)
    }
}

impl<Snapshot> EditHistory<Snapshot> {
    /// Creates a history retaining at most `limit` undo snapshots.
    #[must_use]
    pub const fn new(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit,
        }
    }

    /// Records the state before one edit and discards the abandoned redo branch.
    pub fn remember(&mut self, snapshot: Snapshot) {
        if self.limit == 0 {
            self.undo.clear();
        } else {
            self.undo.push(snapshot);
            if self.undo.len() > self.limit {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
    }

    /// Restores the previous snapshot while retaining the matching current state
    /// for redo.
    ///
    /// `current` receives the snapshot about to be restored so a heterogeneous
    /// history can capture the current value of the same kind. Returning `None`
    /// leaves the history unchanged.
    pub fn try_undo(
        &mut self,
        current: impl FnOnce(&Snapshot) -> Option<Snapshot>,
    ) -> Option<Snapshot> {
        let previous = self.undo.pop()?;
        let Some(current) = current(&previous) else {
            self.undo.push(previous);
            return None;
        };
        self.redo.push(current);
        Some(previous)
    }

    /// Restores the next snapshot while retaining the matching current state for
    /// undo. Returning `None` from `current` leaves the history unchanged.
    pub fn try_redo(
        &mut self,
        current: impl FnOnce(&Snapshot) -> Option<Snapshot>,
    ) -> Option<Snapshot> {
        let next = self.redo.pop()?;
        let Some(current) = current(&next) else {
            self.redo.push(next);
            return None;
        };
        self.undo.push(current);
        Some(next)
    }

    /// Number of currently available undo transitions.
    #[must_use]
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// Number of currently available redo transitions.
    #[must_use]
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }
}

impl CreatorNavigation {
    /// Applies one explicit entry, clearing stale cold-launch identity where required.
    pub fn enter(&mut self, entry: CreatorEntry) {
        match entry {
            CreatorEntry::CharacterLibrary => {
                self.tab = CreatorSurface::Characters;
                self.return_to_combat_lab = false;
                self.return_to_character_creator = false;
            }
            CreatorEntry::SpellLibrary => {
                self.tab = CreatorSurface::Spells;
                self.return_to_combat_lab = false;
                self.return_to_character_creator = false;
            }
            CreatorEntry::SpellFromCharacter => {
                self.tab = CreatorSurface::Spells;
                self.return_to_character_creator = true;
            }
        }
    }

    /// Resolves one clean back action and consumes nested return identity once.
    pub fn back(&mut self) -> CreatorDestination {
        if self.tab == CreatorSurface::Spells && self.return_to_character_creator {
            self.tab = CreatorSurface::Characters;
            self.return_to_character_creator = false;
            CreatorDestination::CharacterEditor
        } else if self.return_to_combat_lab {
            CreatorDestination::CombatLab
        } else {
            CreatorDestination::Title
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_library_entry_clears_prior_combat_lab_identity() {
        let mut navigation = CreatorNavigation {
            return_to_combat_lab: true,
            return_to_character_creator: true,
            ..Default::default()
        };
        navigation.enter(CreatorEntry::SpellLibrary);
        assert_eq!(
            navigation,
            CreatorNavigation {
                tab: CreatorSurface::Spells,
                return_to_combat_lab: false,
                return_to_character_creator: false,
            }
        );
        assert_eq!(navigation.back(), CreatorDestination::Title);
    }

    #[test]
    fn spell_management_returns_to_character_then_preserves_lab_route() {
        let mut navigation = CreatorNavigation {
            return_to_combat_lab: true,
            ..Default::default()
        };
        navigation.enter(CreatorEntry::SpellFromCharacter);
        assert_eq!(navigation.back(), CreatorDestination::CharacterEditor);
        assert_eq!(navigation.back(), CreatorDestination::CombatLab);
    }

    #[test]
    fn edit_history_is_bounded_and_new_edits_discard_the_redo_branch() {
        let mut history = EditHistory::new(2);
        history.remember("zero");
        history.remember("one");
        history.remember("two");
        assert_eq!(history.undo_len(), 2);

        assert_eq!(history.try_undo(|_| Some("three")), Some("two"));
        assert_eq!(history.redo_len(), 1);
        history.remember("branch");
        assert_eq!(history.redo_len(), 0);
        assert_eq!(history.try_undo(|_| Some("current")), Some("branch"));
        assert_eq!(history.try_undo(|_| Some("branch")), Some("one"));
        assert_eq!(history.try_undo(|_| Some("one")), None);
    }

    #[test]
    fn unavailable_matching_state_does_not_consume_history() {
        let mut history = EditHistory::new(2);
        history.remember("character");
        assert_eq!(history.try_undo(|_| None), None);
        assert_eq!(history.undo_len(), 1);
        assert_eq!(history.redo_len(), 0);
    }
}
