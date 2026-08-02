//! Pure Sandbox composition and navigation state.

use std::fmt;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Exact number of ordered character slots on each Sandbox side.
pub const SANDBOX_ROSTER_SIZE: usize = 6;

/// One independently editable Sandbox roster.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SandboxSide {
    /// Human-controlled characters.
    Party,
    /// Baseline-AI characters.
    Enemies,
}

impl SandboxSide {
    /// Both sides in stable validation and launch order.
    pub const ALL: [Self; 2] = [Self::Party, Self::Enemies];
}

impl fmt::Display for SandboxSide {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Party => "Party",
            Self::Enemies => "Enemies",
        })
    }
}

/// Valid identity for one of the six ordered slots on a Sandbox side.
///
/// An enum keeps invalid indices out of routes, intents, and serialized test
/// inputs instead of relying on every adapter to repeat the bounds check.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SandboxSlotIndex {
    /// First roster slot.
    One,
    /// Second roster slot.
    Two,
    /// Third roster slot.
    Three,
    /// Fourth roster slot.
    Four,
    /// Fifth roster slot.
    Five,
    /// Sixth roster slot.
    Six,
}

impl SandboxSlotIndex {
    /// All slots in stable display and launch order.
    pub const ALL: [Self; SANDBOX_ROSTER_SIZE] = [
        Self::One,
        Self::Two,
        Self::Three,
        Self::Four,
        Self::Five,
        Self::Six,
    ];

    /// One-based player-facing slot number.
    #[must_use]
    pub const fn number(self) -> u8 {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Three => 3,
            Self::Four => 4,
            Self::Five => 5,
            Self::Six => 6,
        }
    }

    /// Zero-based array index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.number() as usize - 1
    }

    /// Converts a zero-based array index into a valid slot identity.
    #[must_use]
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::One),
            1 => Some(Self::Two),
            2 => Some(Self::Three),
            3 => Some(Self::Four),
            4 => Some(Self::Five),
            5 => Some(Self::Six),
            _ => None,
        }
    }
}

impl fmt::Display for SandboxSlotIndex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.number())
    }
}

/// One character reference stored in a Sandbox roster slot.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum SandboxCharacter<CustomId> {
    /// Shipped character template key.
    Template(String),
    /// User-authored character identity.
    Custom(CustomId),
}

/// Exact selected map identity frozen into a Sandbox launch.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SandboxMapSelection {
    /// Stable map-catalog identity.
    pub catalog_id: String,
    /// Exact generated seed, or `None` for authored maps.
    pub resolved_seed: Option<u64>,
}

impl SandboxMapSelection {
    /// Creates one exact map selection.
    #[must_use]
    pub fn new(catalog_id: impl Into<String>, resolved_seed: Option<u64>) -> Self {
        Self {
            catalog_id: catalog_id.into(),
            resolved_seed,
        }
    }
}

/// Fixed-size ordered roster with optional sparse slots.
pub type SandboxRoster<CustomId> = [Option<SandboxCharacter<CustomId>>; SANDBOX_ROSTER_SIZE];

/// All persistent-in-memory composition choices for a Sandbox run.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SandboxDraft<CustomId> {
    /// Committed map; pending confirmation is held by [`SandboxModel`].
    pub map: Option<SandboxMapSelection>,
    /// Six ordered human-controlled slots.
    pub party: SandboxRoster<CustomId>,
    /// Six ordered baseline-AI slots.
    pub enemies: SandboxRoster<CustomId>,
}

impl<CustomId> Default for SandboxDraft<CustomId> {
    fn default() -> Self {
        Self {
            map: Some(SandboxMapSelection::new("flat-arena", None)),
            party: [
                Some(SandboxCharacter::Template("hedge-mage".to_owned())),
                None,
                None,
                None,
                None,
                None,
            ],
            enemies: [
                Some(SandboxCharacter::Template("raider".to_owned())),
                None,
                None,
                None,
                None,
                None,
            ],
        }
    }
}

impl<CustomId> SandboxDraft<CustomId> {
    /// Returns one complete fixed-size side.
    #[must_use]
    pub const fn roster(&self, side: SandboxSide) -> &SandboxRoster<CustomId> {
        match side {
            SandboxSide::Party => &self.party,
            SandboxSide::Enemies => &self.enemies,
        }
    }

    /// Returns one character without collapsing sparse slots.
    #[must_use]
    pub fn character(
        &self,
        side: SandboxSide,
        slot: SandboxSlotIndex,
    ) -> Option<&SandboxCharacter<CustomId>> {
        self.roster(side).get(slot.index()).and_then(Option::as_ref)
    }

    /// Iterates occupied characters in stable slot order, preserving duplicates.
    pub fn ordered_characters(
        &self,
        side: SandboxSide,
    ) -> impl Iterator<Item = &SandboxCharacter<CustomId>> {
        self.roster(side).iter().filter_map(Option::as_ref)
    }

    /// Copies occupied characters into launch order without compacting the draft.
    #[must_use]
    pub fn flattened_roster(&self, side: SandboxSide) -> Vec<SandboxCharacter<CustomId>>
    where
        CustomId: Clone,
    {
        self.ordered_characters(side).cloned().collect()
    }

    fn roster_mut(&mut self, side: SandboxSide) -> &mut SandboxRoster<CustomId> {
        match side {
            SandboxSide::Party => &mut self.party,
            SandboxSide::Enemies => &mut self.enemies,
        }
    }

    fn set_character(
        &mut self,
        side: SandboxSide,
        slot: SandboxSlotIndex,
        character: Option<SandboxCharacter<CustomId>>,
    ) {
        if let Some(target) = self.roster_mut(side).get_mut(slot.index()) {
            *target = character;
        }
    }

    fn clear(&mut self, side: SandboxSide) {
        for slot in self.roster_mut(side) {
            *slot = None;
        }
    }
}

/// Renderer-free route within the Sandbox screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SandboxRoute {
    /// Committed map and roster overview.
    #[default]
    Overview,
    /// Map catalog browser.
    MapBrowser,
    /// Confirmation for one pending map selection.
    MapDetail,
    /// Editor for one typed roster side.
    Roster(SandboxSide),
    /// Character selection for one exact side and slot.
    CharacterPicker {
        /// Side that will receive a confirmed character.
        side: SandboxSide,
        /// Slot that will receive a confirmed character.
        slot: SandboxSlotIndex,
    },
}

/// Surface from which the current Sandbox excursion began.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SandboxEntryOrigin {
    /// Main Menu Sandbox action.
    #[default]
    MainMenu,
    /// Character Creator's Open in Sandbox action.
    Creator,
}

/// Typed destination when Back leaves Sandbox Overview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxDestination {
    /// Return to the Main Menu root.
    MainMenu,
    /// Return to the originating Character Creator session.
    Creator,
}

/// Result of one Sandbox Back transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxBackResult {
    /// A Sandbox child route consumed Back.
    Routed,
    /// Back from Overview leaves Sandbox.
    Exit(SandboxDestination),
}

/// Centralized reason Start Sandbox must refuse a launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxStartBlocker {
    /// The curated Sandbox map catalog is not ready.
    MapsLoading,
    /// No map has been committed.
    ChooseMap,
    /// The committed catalog identity cannot be resolved.
    MapUnavailable,
    /// Party contains no occupied slot.
    PartyEmpty,
    /// Enemies contains no occupied slot.
    EnemiesEmpty,
    /// The first stable-order occupied slot that is not Map-ready.
    CharacterNotReady {
        /// Failing roster side.
        side: SandboxSide,
        /// Failing exact slot.
        slot: SandboxSlotIndex,
        /// Adapter-provided readiness reason.
        reason: String,
    },
}

impl SandboxStartBlocker {
    /// Exact player-facing refusal copy.
    #[must_use]
    pub fn message(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for SandboxStartBlocker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MapsLoading => formatter.write_str("Sandbox maps are still loading."),
            Self::ChooseMap => formatter.write_str("Choose a map."),
            Self::MapUnavailable => formatter.write_str("The selected map is unavailable."),
            Self::PartyEmpty => formatter.write_str("Add at least one Party character."),
            Self::EnemiesEmpty => formatter.write_str("Add at least one Enemy character."),
            Self::CharacterNotReady { side, slot, reason } => {
                write!(formatter, "{side} slot {slot} is not Map-ready: {reason}")
            }
        }
    }
}

/// One authoritative renderer-free Sandbox state.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct SandboxModel<CustomId> {
    /// Active child route.
    pub route: SandboxRoute,
    /// Committed map and fixed-size rosters.
    pub draft: SandboxDraft<CustomId>,
    /// Map being inspected; Back discards it and Use Map commits it.
    pub pending_map: Option<SandboxMapSelection>,
    /// Character being previewed; it does not mutate the draft until confirmed.
    pub preview: Option<SandboxCharacter<CustomId>>,
    /// Typed destination for leaving Overview.
    pub entry_origin: SandboxEntryOrigin,
    /// Monotonic immutable-view invalidation token.
    pub revision: u64,
}

impl<CustomId> Default for SandboxModel<CustomId> {
    fn default() -> Self {
        Self {
            route: SandboxRoute::Overview,
            draft: SandboxDraft::default(),
            pending_map: None,
            preview: None,
            entry_origin: SandboxEntryOrigin::MainMenu,
            revision: 0,
        }
    }
}

impl<CustomId> SandboxModel<CustomId> {
    /// Enters Sandbox without resetting its in-memory draft.
    pub fn enter(&mut self, origin: SandboxEntryOrigin) {
        self.route = SandboxRoute::Overview;
        self.pending_map = None;
        self.preview = None;
        self.entry_origin = origin;
        self.bump();
    }

    /// Opens a Map-ready Creator character in Party slot 1.
    ///
    /// The adapter owns saved/clean/Map-ready validation. This transition keeps
    /// the committed map and Enemies, clears the rest of Party, and records the
    /// return to Creator.
    pub fn open_from_creator(&mut self, character: SandboxCharacter<CustomId>) {
        self.draft.clear(SandboxSide::Party);
        self.draft
            .set_character(SandboxSide::Party, SandboxSlotIndex::One, Some(character));
        self.route = SandboxRoute::Roster(SandboxSide::Party);
        self.pending_map = None;
        self.preview = None;
        self.entry_origin = SandboxEntryOrigin::Creator;
        self.bump();
    }

    /// Opens the catalog without changing the committed map.
    pub fn open_map_browser(&mut self) {
        self.route = SandboxRoute::MapBrowser;
        self.pending_map = None;
        self.preview = None;
        self.bump();
    }

    /// Previews one catalog row on the confirmation route.
    pub fn select_map(&mut self, selection: SandboxMapSelection) {
        self.route = SandboxRoute::MapDetail;
        self.pending_map = Some(selection);
        self.preview = None;
        self.bump();
    }

    /// Replaces only the pending map's resolved seed.
    ///
    /// Returns `false` when there is no pending selection. The catalog adapter
    /// controls whether a map is generated and therefore exposes regeneration.
    #[must_use]
    pub fn set_pending_seed(&mut self, resolved_seed: Option<u64>) -> bool {
        let Some(selection) = self.pending_map.as_mut() else {
            return false;
        };
        selection.resolved_seed = resolved_seed;
        self.bump();
        true
    }

    /// Commits the pending map and returns to Overview.
    #[must_use]
    pub fn use_pending_map(&mut self) -> bool {
        let Some(selection) = self.pending_map.take() else {
            return false;
        };
        self.draft.map = Some(selection);
        self.route = SandboxRoute::Overview;
        self.bump();
        true
    }

    /// Opens one typed roster without changing either side.
    pub fn open_roster(&mut self, side: SandboxSide) {
        self.route = SandboxRoute::Roster(side);
        self.preview = None;
        self.bump();
    }

    /// Opens the shared Character Picker for one exact roster slot.
    pub fn open_character_picker(&mut self, side: SandboxSide, slot: SandboxSlotIndex) {
        self.route = SandboxRoute::CharacterPicker { side, slot };
        self.preview = None;
        self.bump();
    }

    /// Previews a character without mutating either roster.
    ///
    /// Returns `false` outside the Character Picker route.
    #[must_use]
    pub fn preview_character(&mut self, character: SandboxCharacter<CustomId>) -> bool {
        if !matches!(self.route, SandboxRoute::CharacterPicker { .. }) {
            return false;
        }
        self.preview = Some(character);
        self.bump();
        true
    }

    /// Applies the preview to the picker side/slot and returns to its roster.
    #[must_use]
    pub fn use_previewed_character(&mut self) -> bool {
        let SandboxRoute::CharacterPicker { side, slot } = self.route else {
            return false;
        };
        let Some(character) = self.preview.take() else {
            return false;
        };
        self.draft.set_character(side, slot, Some(character));
        self.route = SandboxRoute::Roster(side);
        self.bump();
        true
    }

    /// Clears one exact sparse roster slot.
    pub fn clear_character(&mut self, side: SandboxSide, slot: SandboxSlotIndex) {
        self.draft.set_character(side, slot, None);
        self.bump();
    }

    /// Applies one route-aware Back transition.
    pub fn back(&mut self) -> SandboxBackResult {
        match self.route {
            SandboxRoute::Overview => SandboxBackResult::Exit(self.destination()),
            SandboxRoute::MapBrowser => {
                self.route = SandboxRoute::Overview;
                self.pending_map = None;
                self.bump();
                SandboxBackResult::Routed
            }
            SandboxRoute::MapDetail => {
                self.route = SandboxRoute::MapBrowser;
                self.pending_map = None;
                self.bump();
                SandboxBackResult::Routed
            }
            SandboxRoute::Roster(_) => {
                self.route = SandboxRoute::Overview;
                self.preview = None;
                self.bump();
                SandboxBackResult::Routed
            }
            SandboxRoute::CharacterPicker { side, .. } => {
                self.route = SandboxRoute::Roster(side);
                self.preview = None;
                self.bump();
                SandboxBackResult::Routed
            }
        }
    }

    /// Resolves the typed Overview exit destination.
    #[must_use]
    pub const fn destination(&self) -> SandboxDestination {
        match self.entry_origin {
            SandboxEntryOrigin::MainMenu => SandboxDestination::MainMenu,
            SandboxEntryOrigin::Creator => SandboxDestination::Creator,
        }
    }

    /// Returns the first exact-priority reason Start Sandbox must refuse.
    ///
    /// Map readiness comes from the asset adapter. Character readiness comes
    /// from the Creator/content adapter and is evaluated Party-first, then
    /// Enemies, with each side in stable slot order.
    pub fn start_blocker(
        &self,
        maps_loaded: bool,
        map_available: bool,
        mut readiness: impl FnMut(&SandboxCharacter<CustomId>) -> Result<(), String>,
    ) -> Option<SandboxStartBlocker> {
        if !maps_loaded {
            return Some(SandboxStartBlocker::MapsLoading);
        }
        if self.draft.map.is_none() {
            return Some(SandboxStartBlocker::ChooseMap);
        }
        if !map_available {
            return Some(SandboxStartBlocker::MapUnavailable);
        }
        if self
            .draft
            .ordered_characters(SandboxSide::Party)
            .next()
            .is_none()
        {
            return Some(SandboxStartBlocker::PartyEmpty);
        }
        if self
            .draft
            .ordered_characters(SandboxSide::Enemies)
            .next()
            .is_none()
        {
            return Some(SandboxStartBlocker::EnemiesEmpty);
        }
        for side in SandboxSide::ALL {
            for (slot, character) in SandboxSlotIndex::ALL
                .into_iter()
                .zip(self.draft.roster(side))
            {
                let Some(character) = character.as_ref() else {
                    continue;
                };
                if let Err(reason) = readiness(character) {
                    return Some(SandboxStartBlocker::CharacterNotReady { side, slot, reason });
                }
            }
        }
        None
    }

    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Character = SandboxCharacter<u64>;

    fn template(key: &str) -> Character {
        Character::Template(key.to_owned())
    }

    #[test]
    fn defaults_are_the_shipped_flat_arena_party_and_enemy() {
        let model: SandboxModel<u64> = SandboxModel::default();
        assert_eq!(
            model.draft.map,
            Some(SandboxMapSelection::new("flat-arena", None))
        );
        assert_eq!(
            model
                .draft
                .character(SandboxSide::Party, SandboxSlotIndex::One),
            Some(&template("hedge-mage"))
        );
        assert_eq!(
            model
                .draft
                .character(SandboxSide::Enemies, SandboxSlotIndex::One),
            Some(&template("raider"))
        );
    }

    #[test]
    fn map_confirmation_separates_pending_from_committed_and_back_discards() {
        let mut model: SandboxModel<u64> = SandboxModel::default();
        let committed = model.draft.map.clone();
        model.open_map_browser();
        model.select_map(SandboxMapSelection::new("procedural-hills", Some(7)));
        assert_eq!(model.draft.map, committed);
        assert!(model.set_pending_seed(Some(9)));
        assert_eq!(
            model.pending_map,
            Some(SandboxMapSelection::new("procedural-hills", Some(9)))
        );
        assert_eq!(model.back(), SandboxBackResult::Routed);
        assert_eq!(model.route, SandboxRoute::MapBrowser);
        assert_eq!(model.pending_map, None);
        assert_eq!(model.draft.map, committed);

        model.select_map(SandboxMapSelection::new("procedural-hills", Some(11)));
        assert!(model.use_pending_map());
        assert_eq!(model.route, SandboxRoute::Overview);
        assert_eq!(
            model.draft.map,
            Some(SandboxMapSelection::new("procedural-hills", Some(11)))
        );
    }

    #[test]
    fn character_preview_back_and_use_are_transactional_and_slot_typed() {
        let mut model: SandboxModel<u64> = SandboxModel::default();
        model.open_character_picker(SandboxSide::Party, SandboxSlotIndex::Three);
        assert!(model.preview_character(Character::Custom(42)));
        assert_eq!(
            model
                .draft
                .character(SandboxSide::Party, SandboxSlotIndex::Three),
            None
        );
        assert_eq!(model.back(), SandboxBackResult::Routed);
        assert_eq!(model.route, SandboxRoute::Roster(SandboxSide::Party));
        assert_eq!(model.preview, None);

        model.open_character_picker(SandboxSide::Enemies, SandboxSlotIndex::Three);
        assert!(model.preview_character(Character::Custom(42)));
        assert!(model.use_previewed_character());
        assert_eq!(model.route, SandboxRoute::Roster(SandboxSide::Enemies));
        assert_eq!(
            model
                .draft
                .character(SandboxSide::Enemies, SandboxSlotIndex::Three),
            Some(&Character::Custom(42))
        );
    }

    #[test]
    fn sparse_rosters_flatten_in_slot_order_and_preserve_duplicates() {
        let mut model: SandboxModel<u64> = SandboxModel::default();
        model.clear_character(SandboxSide::Party, SandboxSlotIndex::One);
        model.open_character_picker(SandboxSide::Party, SandboxSlotIndex::Two);
        assert!(model.preview_character(Character::Custom(7)));
        assert!(model.use_previewed_character());
        model.open_character_picker(SandboxSide::Party, SandboxSlotIndex::Six);
        assert!(model.preview_character(Character::Custom(7)));
        assert!(model.use_previewed_character());
        assert_eq!(
            model.draft.flattened_roster(SandboxSide::Party),
            [Character::Custom(7), Character::Custom(7)]
        );
    }

    #[test]
    fn entering_and_child_navigation_preserve_the_draft() {
        let mut model: SandboxModel<u64> = SandboxModel::default();
        model.open_character_picker(SandboxSide::Party, SandboxSlotIndex::Five);
        assert!(model.preview_character(Character::Custom(5)));
        assert!(model.use_previewed_character());
        let draft = model.draft.clone();
        model.enter(SandboxEntryOrigin::MainMenu);
        model.open_map_browser();
        assert_eq!(model.back(), SandboxBackResult::Routed);
        assert_eq!(model.draft, draft);
        assert_eq!(
            model.back(),
            SandboxBackResult::Exit(SandboxDestination::MainMenu)
        );
    }

    #[test]
    fn every_route_has_one_exact_back_edge_and_preserves_the_draft() {
        for origin in [SandboxEntryOrigin::MainMenu, SandboxEntryOrigin::Creator] {
            let mut model: SandboxModel<u64> = SandboxModel {
                entry_origin: origin,
                ..Default::default()
            };
            let draft = model.draft.clone();
            assert_eq!(
                model.back(),
                SandboxBackResult::Exit(match origin {
                    SandboxEntryOrigin::MainMenu => SandboxDestination::MainMenu,
                    SandboxEntryOrigin::Creator => SandboxDestination::Creator,
                })
            );
            assert_eq!(model.draft, draft);
        }

        let mut child_routes = vec![
            (SandboxRoute::MapBrowser, SandboxRoute::Overview),
            (SandboxRoute::MapDetail, SandboxRoute::MapBrowser),
        ];
        for side in SandboxSide::ALL {
            child_routes.push((SandboxRoute::Roster(side), SandboxRoute::Overview));
            for slot in SandboxSlotIndex::ALL {
                child_routes.push((
                    SandboxRoute::CharacterPicker { side, slot },
                    SandboxRoute::Roster(side),
                ));
            }
        }
        for (route, expected) in child_routes {
            let mut model: SandboxModel<u64> = SandboxModel {
                route,
                ..Default::default()
            };
            if route == SandboxRoute::MapDetail {
                model.pending_map = Some(SandboxMapSelection::new("procedural-hills", Some(9)));
            }
            if matches!(route, SandboxRoute::CharacterPicker { .. }) {
                model.preview = Some(Character::Custom(42));
            }
            let draft = model.draft.clone();
            assert_eq!(model.back(), SandboxBackResult::Routed, "route {route:?}");
            assert_eq!(model.route, expected, "route {route:?}");
            assert_eq!(model.draft, draft, "route {route:?}");
            assert_eq!(model.preview, None, "route {route:?}");
            if matches!(route, SandboxRoute::MapBrowser | SandboxRoute::MapDetail) {
                assert_eq!(model.pending_map, None, "route {route:?}");
            }
        }
    }

    #[test]
    fn creator_entry_replaces_only_party_and_returns_to_creator() {
        let mut model: SandboxModel<u64> = SandboxModel::default();
        let map = model.draft.map.clone();
        let enemies = model.draft.enemies.clone();
        model.open_character_picker(SandboxSide::Party, SandboxSlotIndex::Four);
        assert!(model.preview_character(Character::Custom(4)));
        assert!(model.use_previewed_character());

        model.open_from_creator(Character::Custom(99));
        assert_eq!(model.draft.map, map);
        assert_eq!(model.draft.enemies, enemies);
        assert_eq!(
            model.draft.flattened_roster(SandboxSide::Party),
            [Character::Custom(99)]
        );
        assert_eq!(model.route, SandboxRoute::Roster(SandboxSide::Party));
        assert_eq!(model.back(), SandboxBackResult::Routed);
        assert_eq!(
            model.back(),
            SandboxBackResult::Exit(SandboxDestination::Creator)
        );
    }

    #[test]
    fn start_blockers_follow_exact_priority_and_stable_slot_order() {
        let mut model: SandboxModel<u64> = SandboxModel::default();
        let ready = |_: &Character| Ok(());
        assert_eq!(
            model.start_blocker(false, false, ready),
            Some(SandboxStartBlocker::MapsLoading)
        );
        model.draft.map = None;
        assert_eq!(
            model.start_blocker(true, false, ready),
            Some(SandboxStartBlocker::ChooseMap)
        );
        model.draft.map = Some(SandboxMapSelection::new("missing", None));
        assert_eq!(
            model.start_blocker(true, false, ready),
            Some(SandboxStartBlocker::MapUnavailable)
        );
        model.draft.clear(SandboxSide::Party);
        assert_eq!(
            model.start_blocker(true, true, ready),
            Some(SandboxStartBlocker::PartyEmpty)
        );
        model.draft.set_character(
            SandboxSide::Party,
            SandboxSlotIndex::Six,
            Some(Character::Custom(6)),
        );
        model.draft.clear(SandboxSide::Enemies);
        assert_eq!(
            model.start_blocker(true, true, ready),
            Some(SandboxStartBlocker::EnemiesEmpty)
        );
        model.draft.set_character(
            SandboxSide::Enemies,
            SandboxSlotIndex::Five,
            Some(Character::Custom(5)),
        );
        let blocker = model.start_blocker(true, true, |character| match character {
            Character::Custom(id) => Err(format!("character {id} needs a spell")),
            Character::Template(_) => Ok(()),
        });
        assert_eq!(
            blocker,
            Some(SandboxStartBlocker::CharacterNotReady {
                side: SandboxSide::Party,
                slot: SandboxSlotIndex::Six,
                reason: "character 6 needs a spell".to_owned(),
            })
        );
        assert_eq!(
            blocker.map(|value| value.message()),
            Some("Party slot 6 is not Map-ready: character 6 needs a spell".to_owned())
        );
    }
}
