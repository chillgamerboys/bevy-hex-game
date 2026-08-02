//! Pure Sandbox composition and navigation state.

use std::fmt;

use bevy_ecs::prelude::Resource;
use hex_core::TilePos;
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

/// Stable identity for one occupied slot in a Sandbox deployment queue.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SandboxDeploymentSlot {
    /// Roster side that owns the character.
    pub side: SandboxSide,
    /// Original sparse roster slot; deployment never compacts this identity.
    pub slot: SandboxSlotIndex,
}

impl SandboxDeploymentSlot {
    /// Creates one exact side-local deployment identity.
    #[must_use]
    pub const fn new(side: SandboxSide, slot: SandboxSlotIndex) -> Self {
        Self { side, slot }
    }
}

impl fmt::Display for SandboxDeploymentSlot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} slot {}", self.side, self.slot)
    }
}

/// Current step in the guided Sandbox deployment task.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxDeploymentStage {
    /// One exact occupied roster slot owns the next valid terrain click.
    Placing(SandboxDeploymentSlot),
    /// Every occupied slot has an exact placement and may be reviewed or started.
    Review,
}

/// Typed reason a Sandbox placement or launch action was refused.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxPlacementRefusal {
    /// Published terrain facts are not ready for placement validation.
    TerrainUnavailable,
    /// The clicked exact surface does not admit the canonical walker body.
    InvalidFooting,
    /// Another exact roster slot already owns the clicked surface.
    Occupied {
        /// Slot whose placement must remain unique.
        occupant: SandboxDeploymentSlot,
    },
    /// Review owns the task until the player selects a character to reposition.
    SelectCharacter,
    /// Live actors no longer match the frozen ordered roster.
    RosterChanged,
    /// At least one occupied roster slot still needs an exact placement.
    Incomplete,
    /// The active scenario, content, encounter, or provenance is incomplete.
    LaunchIdentityUnavailable,
}

impl SandboxPlacementRefusal {
    /// Stable player-facing refusal copy.
    #[must_use]
    pub fn message(self) -> String {
        self.to_string()
    }
}

impl fmt::Display for SandboxPlacementRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TerrainUnavailable => {
                formatter.write_str("Sandbox terrain is still loading.")
            }
            Self::InvalidFooting => formatter
                .write_str("Choose a solid surface with enough room for this character."),
            Self::Occupied { occupant } => {
                write!(formatter, "That surface is already occupied by {occupant}.")
            }
            Self::SelectCharacter => {
                formatter.write_str("Select a character before choosing another surface.")
            }
            Self::RosterChanged => formatter.write_str(
                "Sandbox deployment no longer matches its roster. Return to Sandbox and start again.",
            ),
            Self::Incomplete => {
                formatter.write_str("Place every character before starting combat.")
            }
            Self::LaunchIdentityUnavailable => formatter.write_str(
                "Sandbox launch identity is unavailable. Return to Sandbox and start again.",
            ),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
struct SandboxDeploymentEdit {
    slot: SandboxDeploymentSlot,
    previous: Option<TilePos>,
    previous_stage: SandboxDeploymentStage,
}

/// Renderer-free authority for guided, exact-surface Sandbox deployment.
///
/// The model owns sparse slot identity, Party-then-Enemies progression, exact
/// occupancy, review, and undo. The application adapter validates a clicked
/// [`TilePos`] against live published terrain before calling [`Self::place_validated`].
#[derive(Resource, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SandboxDeploymentModel {
    order: Vec<SandboxDeploymentSlot>,
    party: [Option<TilePos>; SANDBOX_ROSTER_SIZE],
    enemies: [Option<TilePos>; SANDBOX_ROSTER_SIZE],
    undo: Vec<SandboxDeploymentEdit>,
    /// Current guided placement or final review stage.
    pub stage: SandboxDeploymentStage,
    /// Most recent typed refusal, cleared by a successful transition.
    pub refusal: Option<SandboxPlacementRefusal>,
}

impl SandboxDeploymentModel {
    /// Builds a stable Party-then-Enemies queue from occupied sparse draft slots.
    #[must_use]
    pub fn from_draft<CustomId>(draft: &SandboxDraft<CustomId>) -> Self {
        let order = SandboxSide::ALL
            .into_iter()
            .flat_map(|side| {
                SandboxSlotIndex::ALL.into_iter().filter_map(move |slot| {
                    draft
                        .character(side, slot)
                        .is_some()
                        .then_some(SandboxDeploymentSlot::new(side, slot))
                })
            })
            .collect::<Vec<_>>();
        let stage = order.first().copied().map_or(
            SandboxDeploymentStage::Review,
            SandboxDeploymentStage::Placing,
        );
        Self {
            order,
            party: [None; SANDBOX_ROSTER_SIZE],
            enemies: [None; SANDBOX_ROSTER_SIZE],
            undo: Vec::new(),
            stage,
            refusal: None,
        }
    }

    /// Ordered occupied sparse slots, with Party slots before Enemy slots.
    #[must_use]
    pub fn order(&self) -> &[SandboxDeploymentSlot] {
        &self.order
    }

    /// Active placement owner, or `None` while reviewing a complete deployment.
    #[must_use]
    pub const fn active_slot(&self) -> Option<SandboxDeploymentSlot> {
        match self.stage {
            SandboxDeploymentStage::Placing(slot) => Some(slot),
            SandboxDeploymentStage::Review => None,
        }
    }

    /// Exact placement for one sparse slot.
    #[must_use]
    pub fn placement(&self, slot: SandboxDeploymentSlot) -> Option<TilePos> {
        self.placements(slot.side)
            .get(slot.slot.index())
            .copied()
            .flatten()
    }

    /// One-based progress position and total occupied slot count.
    #[must_use]
    pub fn progress(&self, slot: SandboxDeploymentSlot) -> Option<(usize, usize)> {
        self.order
            .iter()
            .position(|candidate| *candidate == slot)
            .map(|index| (index + 1, self.order.len()))
    }

    /// Whether one exact placement edit is available to restore.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Selects any occupied slot for placement or repositioning.
    #[must_use]
    pub fn select_slot(&mut self, slot: SandboxDeploymentSlot) -> bool {
        if !self.order.contains(&slot) {
            return false;
        }
        self.stage = SandboxDeploymentStage::Placing(slot);
        self.refusal = None;
        true
    }

    /// Records a live-adapter refusal without mutating placement or progression.
    pub fn refuse(&mut self, refusal: SandboxPlacementRefusal) {
        self.refusal = Some(refusal);
    }

    /// Commits a terrain-validated exact surface for the active slot.
    ///
    /// Exact occupancy is transactional. A refused position changes neither the
    /// slot's previous placement nor the guided stage.
    pub fn place_validated(
        &mut self,
        position: TilePos,
    ) -> Result<SandboxDeploymentStage, SandboxPlacementRefusal> {
        let Some(slot) = self.active_slot() else {
            let refusal = SandboxPlacementRefusal::SelectCharacter;
            self.refuse(refusal);
            return Err(refusal);
        };
        if let Some(occupant) =
            self.order.iter().copied().find(|candidate| {
                *candidate != slot && self.placement(*candidate) == Some(position)
            })
        {
            let refusal = SandboxPlacementRefusal::Occupied { occupant };
            self.refuse(refusal);
            return Err(refusal);
        }

        let previous = self.placement(slot);
        let previous_stage = self.stage;
        if let Some(target) = self.placements_mut(slot.side).get_mut(slot.slot.index()) {
            *target = Some(position);
        }
        self.undo.push(SandboxDeploymentEdit {
            slot,
            previous,
            previous_stage,
        });
        self.refusal = None;
        self.stage = self.next_stage_after(slot);
        Ok(self.stage)
    }

    /// Restores the last placement or repositioning edit and its exact stage.
    #[must_use]
    pub fn undo(&mut self) -> bool {
        let Some(edit) = self.undo.pop() else {
            return false;
        };
        if let Some(target) = self
            .placements_mut(edit.slot.side)
            .get_mut(edit.slot.slot.index())
        {
            *target = edit.previous;
        }
        self.stage = edit.previous_stage;
        self.refusal = None;
        true
    }

    /// Whether both non-empty sides have one unique exact placement per occupied slot.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        SandboxSide::ALL.into_iter().all(|side| {
            let side_slots = self.order.iter().filter(|slot| slot.side == side);
            let mut found = false;
            for slot in side_slots {
                found = true;
                if self.placement(*slot).is_none() {
                    return false;
                }
            }
            found
        })
    }

    /// Exact placements for one side in stable occupied-slot launch order.
    pub fn ordered_placements(&self, side: SandboxSide) -> impl Iterator<Item = TilePos> + '_ {
        self.order
            .iter()
            .copied()
            .filter(move |slot| slot.side == side)
            .filter_map(|slot| self.placement(slot))
    }

    /// Complete ordered Party and Enemy placements, ready to freeze into launch identity.
    #[must_use]
    pub fn frozen_placements(&self) -> Option<(Vec<TilePos>, Vec<TilePos>)> {
        self.is_complete().then(|| {
            (
                self.ordered_placements(SandboxSide::Party).collect(),
                self.ordered_placements(SandboxSide::Enemies).collect(),
            )
        })
    }

    fn placements(&self, side: SandboxSide) -> &[Option<TilePos>; SANDBOX_ROSTER_SIZE] {
        match side {
            SandboxSide::Party => &self.party,
            SandboxSide::Enemies => &self.enemies,
        }
    }

    fn placements_mut(&mut self, side: SandboxSide) -> &mut [Option<TilePos>; SANDBOX_ROSTER_SIZE] {
        match side {
            SandboxSide::Party => &mut self.party,
            SandboxSide::Enemies => &mut self.enemies,
        }
    }

    fn next_stage_after(&self, slot: SandboxDeploymentSlot) -> SandboxDeploymentStage {
        let Some(current) = self.order.iter().position(|candidate| *candidate == slot) else {
            return SandboxDeploymentStage::Review;
        };
        self.order
            .iter()
            .skip(current + 1)
            .chain(self.order.iter().take(current + 1))
            .copied()
            .find(|candidate| self.placement(*candidate).is_none())
            .map_or(
                SandboxDeploymentStage::Review,
                SandboxDeploymentStage::Placing,
            )
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
    fn guided_deployment_preserves_sparse_slot_order_and_exact_stack_identity() {
        let mut draft: SandboxDraft<u64> = SandboxDraft::default();
        draft.clear(SandboxSide::Party);
        draft.set_character(
            SandboxSide::Party,
            SandboxSlotIndex::Two,
            Some(Character::Custom(7)),
        );
        draft.set_character(
            SandboxSide::Party,
            SandboxSlotIndex::Six,
            Some(Character::Custom(7)),
        );
        draft.clear(SandboxSide::Enemies);
        draft.set_character(
            SandboxSide::Enemies,
            SandboxSlotIndex::Three,
            Some(template("raider")),
        );

        let mut deployment = SandboxDeploymentModel::from_draft(&draft);
        let party_two = SandboxDeploymentSlot::new(SandboxSide::Party, SandboxSlotIndex::Two);
        let party_six = SandboxDeploymentSlot::new(SandboxSide::Party, SandboxSlotIndex::Six);
        let enemy_three = SandboxDeploymentSlot::new(SandboxSide::Enemies, SandboxSlotIndex::Three);
        assert_eq!(deployment.order(), [party_two, party_six, enemy_three]);
        assert_eq!(deployment.stage, SandboxDeploymentStage::Placing(party_two));

        let lower = TilePos::new(hex_core::HexCoord::ORIGIN, 1);
        let upper = TilePos::new(hex_core::HexCoord::ORIGIN, 4);
        let hostile = TilePos::new(hex_core::HexCoord::from_axial(1, 0), 2);
        assert_eq!(
            deployment.place_validated(lower),
            Ok(SandboxDeploymentStage::Placing(party_six))
        );
        assert_eq!(
            deployment.place_validated(upper),
            Ok(SandboxDeploymentStage::Placing(enemy_three)),
            "stacked exact surfaces at one coordinate must remain distinct"
        );
        assert_eq!(
            deployment.place_validated(lower),
            Err(SandboxPlacementRefusal::Occupied {
                occupant: party_two,
            })
        );
        assert_eq!(deployment.placement(enemy_three), None);
        assert_eq!(
            deployment.place_validated(hostile),
            Ok(SandboxDeploymentStage::Review)
        );
        assert!(deployment.is_complete());
        assert_eq!(
            deployment.frozen_placements(),
            Some((vec![lower, upper], vec![hostile]))
        );
    }

    #[test]
    fn guided_deployment_reselection_and_undo_restore_the_exact_previous_stage() {
        let draft: SandboxDraft<u64> = SandboxDraft::default();
        let mut deployment = SandboxDeploymentModel::from_draft(&draft);
        let party = SandboxDeploymentSlot::new(SandboxSide::Party, SandboxSlotIndex::One);
        let enemy = SandboxDeploymentSlot::new(SandboxSide::Enemies, SandboxSlotIndex::One);
        let party_first = TilePos::new(hex_core::HexCoord::ORIGIN, 1);
        let party_moved = TilePos::new(hex_core::HexCoord::from_axial(1, 0), 1);
        let hostile = TilePos::new(hex_core::HexCoord::from_axial(-1, 0), 1);

        assert_eq!(
            deployment.place_validated(party_first),
            Ok(SandboxDeploymentStage::Placing(enemy))
        );
        assert_eq!(
            deployment.place_validated(hostile),
            Ok(SandboxDeploymentStage::Review)
        );
        assert!(deployment.select_slot(party));
        assert_eq!(
            deployment.place_validated(party_moved),
            Ok(SandboxDeploymentStage::Review)
        );
        assert_eq!(deployment.placement(party), Some(party_moved));

        assert!(deployment.undo());
        assert_eq!(deployment.placement(party), Some(party_first));
        assert_eq!(
            deployment.stage,
            SandboxDeploymentStage::Placing(party),
            "undo must restore the stage that owned the repositioning click"
        );
        assert_eq!(deployment.progress(party), Some((1, 2)));
        assert_eq!(deployment.progress(enemy), Some((2, 2)));
    }

    #[test]
    fn guided_deployment_rejects_unknown_slot_without_mutation() {
        let draft: SandboxDraft<u64> = SandboxDraft::default();
        let mut deployment = SandboxDeploymentModel::from_draft(&draft);
        let before = deployment.clone();
        assert!(!deployment.select_slot(SandboxDeploymentSlot::new(
            SandboxSide::Party,
            SandboxSlotIndex::Six,
        )));
        assert_eq!(deployment, before);
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
