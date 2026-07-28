//! What a faction knows about a hostile lattice.
//!
//! # Two channels, deliberately separate
//!
//! Spatial perception answers *where is that unit, and can I see it*. It is the
//! world owner's, and the future `hex_perception` crate owns it. This module
//! answers a different question — *what do I know about that unit's lattice* —
//! and the answer is not derivable from the first one. **Seeing a unit reveals
//! nothing about its gems, its fusions, or what it can cast.** Divination owns
//! those facts.
//!
//! Nothing here observes, and nothing here decides what is visible. It consumes
//! spatial observation where it needs it and duplicates none of it.
//!
//! # Why the store is not keyed on visibility
//!
//! The obvious implementation — remember what a faction can currently see — is
//! the one that has to be thrown away. Divination exists precisely to reveal what
//! a faction *cannot* see, so a fact may arrive from a cast with a lifetime of
//! its own: "revealed information decays or is one-time, unless the divination is
//! an enchantment" (`docs/design/game.md`).
//!
//! So every entry carries its [`KnowledgeSource`](hex_core::KnowledgeSource) and
//! its own [`KnowledgeExpiry`](hex_core::KnowledgeExpiry), and decay runs per
//! entry rather than by recomputing visibility. An observation-sourced fact and a
//! divination-sourced one differ in nothing but those two tags, which is the
//! point: the store does not care how a fact arrived, only that it says so and
//! says when it lapses.
//!
//! # One accessor
//!
//! [`FactionKnowledge::view`] is **the** read path. Anything wanting to know
//! something about a hostile lattice goes through it — the AI included — and
//! reading a hostile [`LatticeState`](hex_lattice::LatticeState) directly is a bug.
//!
//! # What is not wired yet
//!
//! No unit carries a lattice: `LatticeSpec` and `LatticeState` are components
//! today, but nothing attaches them, and `lattices.ron` does not exist. Wiring
//! them onto units is HEX-12's first deliverable. Until it lands the publishing
//! systems below match no entities and the store stays empty — which is why the
//! tests spawn the components directly rather than proving the store works by
//! watching the running game, where it currently has nothing to describe.
//!
//! `Reveal` (the shipped "Scrying Eye") reaches this store through the cast path,
//! which HEX-12 also lands. This half is the store and the accessor.

use std::collections::BTreeMap;

use bevy::prelude::*;

use hex_core::{
    AppSystems, KnowledgeExpiry, KnowledgeSource, LatticeCoord, PausableSystems, RoundElapsed,
    Screen, UnitId,
};
use hex_lattice::{CellKind, LatticeSpec, LatticeState};
use hex_units::Faction;

/// A total order on factions, so a view can key a [`BTreeMap`] deterministically.
///
/// [`Faction`] deliberately does not derive `Ord` — a side has no natural order,
/// and the type belongs to `hex_units`. The store still has to iterate in the
/// same order on every run, so it keys on this local ordinal instead. The numbers
/// carry no game meaning and are never serialized; a new faction variant becomes
/// a compile error here, which is the right place to be told.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FactionKey(u8);

impl From<Faction> for FactionKey {
    fn from(faction: Faction) -> Self {
        Self(match faction {
            Faction::Player => 0,
            Faction::Hostile => 1,
        })
    }
}

/// The facts about a lattice its owner cannot hide.
///
/// **A lattice's shape is public; its contents are not.** How many cells a
/// character has is apparent from looking at them, so it is available without any
/// reveal at all — which is what makes the v1 readout "unknown lattice, N hexes"
/// honest rather than a placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseVisibility {
    /// The side the subject fights for.
    pub faction: Faction,
    /// How many cells the subject's lattice has.
    pub capacity: usize,
}

/// One known cell, with where the knowledge came from and when it lapses.
///
/// The source and expiry are stored per cell rather than per lattice because a
/// reveal is partial: a tier-1 divination may expose two cells of a nine-cell
/// lattice, and those two decay on their own schedule while the rest stay
/// unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownCell {
    /// What the inscription puts in the cell.
    pub kind: CellKind,
    /// Mana held when the fact was learned.
    ///
    /// [`None`] when the reveal exposed the cell's existence but not its charge,
    /// which is a different thing from an empty gem and must not read as one.
    pub mana: Option<u16>,
    /// Whether the cell was disabled when the fact was learned.
    pub disabled: bool,
    /// Which channel wrote this fact.
    pub source: KnowledgeSource,
    /// What remains of this fact's lifetime.
    pub expiry: KnowledgeExpiry,
}

/// What one faction knows about one unit's lattice.
///
/// Base visibility is always present; the cell map holds only what has actually
/// been revealed, so an absent coordinate means *unknown* rather than *empty*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatticeKnowledge {
    base: BaseVisibility,
    cells: BTreeMap<LatticeCoord, KnownCell>,
}

impl LatticeKnowledge {
    /// Creates knowledge holding only the facts nobody can hide.
    #[must_use]
    pub fn new(base: BaseVisibility) -> Self {
        Self {
            base,
            cells: BTreeMap::new(),
        }
    }

    /// The facts available without any reveal.
    #[must_use]
    pub fn base(&self) -> BaseVisibility {
        self.base
    }

    /// What is known about one cell, or [`None`] if it has not been revealed.
    #[must_use]
    pub fn cell(&self, coord: LatticeCoord) -> Option<KnownCell> {
        self.cells.get(&coord).copied()
    }

    /// Every revealed cell, in coordinate order.
    pub fn cells(&self) -> impl Iterator<Item = (LatticeCoord, KnownCell)> + '_ {
        self.cells.iter().map(|(&coord, &cell)| (coord, cell))
    }

    /// How many cells have been revealed.
    #[must_use]
    pub fn revealed_count(&self) -> usize {
        self.cells.len()
    }

    /// How many cells remain unknown.
    ///
    /// Saturating rather than wrapping: base visibility and the revealed set are
    /// written by different systems, and a capacity that has not caught up with a
    /// reveal must read as zero unknown rather than as four billion.
    #[must_use]
    pub fn unknown_count(&self) -> usize {
        self.base.capacity.saturating_sub(self.cells.len())
    }

    /// Whether nothing at all has been revealed about this lattice.
    #[must_use]
    pub fn is_opaque(&self) -> bool {
        self.cells.is_empty()
    }

    /// Records one revealed cell, replacing whatever was known about it.
    ///
    /// Replacement rather than merge, deliberately: a later reveal is a fresher
    /// observation of the same cell, and keeping the older mana figure because it
    /// was more detailed would report a charge that has since been spent.
    pub fn learn(&mut self, coord: LatticeCoord, cell: KnownCell) -> Option<KnownCell> {
        self.cells.insert(coord, cell)
    }

    /// Drops what was known about one cell.
    pub fn forget(&mut self, coord: LatticeCoord) -> Option<KnownCell> {
        self.cells.remove(&coord)
    }

    /// Ages every revealed cell by one round, dropping the lapsed ones.
    ///
    /// Base visibility is untouched — it never decays, because it was never
    /// hidden.
    fn decay(&mut self) {
        self.cells.retain(|_, cell| match cell.expiry.tick() {
            Some(remaining) => {
                cell.expiry = remaining;
                true
            }
            None => false,
        });
    }
}

/// What every faction knows about every lattice.
///
/// Keyed by stable [`UnitId`] and a faction ordinal, never by [`Entity`]: entity
/// indices are recycled and differ across runs and saves, so a store keyed on
/// them silently reshuffles — the randomness-by-another-name the design rules
/// out.
///
/// Not `Reflect`, for the same reason [`LocalMapKnowledge`](hex_core::LocalMapKnowledge)
/// is not: the tuple-keyed maps do not project usefully into the inspector, and a
/// derive that has to be worked around is worse than its absence.
#[derive(Resource, Debug, Default)]
pub struct FactionKnowledge {
    by_view: BTreeMap<(FactionKey, UnitId), LatticeKnowledge>,
    /// Ground truth, populated only while the dev reveal-all toggle is on.
    ///
    /// A separate layer rather than a flag on each entry, so switching the toggle
    /// off cannot strip a fact the game legitimately knew. Empty in the shipped
    /// build, where it costs one map lookup that always misses.
    truth: BTreeMap<UnitId, LatticeKnowledge>,
}

impl FactionKnowledge {
    /// **The** accessor. UI and AI read hostile lattices through here or not at all.
    ///
    /// Returns [`None`] when the viewer knows nothing whatsoever about the
    /// subject — not even that it exists. A subject that is known but wholly
    /// unrevealed returns knowledge whose [`LatticeKnowledge::is_opaque`] is
    /// true, which is a different answer and must stay one.
    ///
    /// The dev reveal-all layer is consulted first, so a designer sees the truth
    /// through the same call the game uses rather than through a second path
    /// that could drift from it.
    #[must_use]
    pub fn view(&self, viewer: Faction, subject: UnitId) -> Option<&LatticeKnowledge> {
        self.truth
            .get(&subject)
            .or_else(|| self.by_view.get(&(viewer.into(), subject)))
    }

    /// Publishes the facts a subject cannot hide from a viewer.
    ///
    /// Idempotent: re-publishing updates base visibility and leaves revealed
    /// cells alone, so a capacity that changes mid-fight does not erase what
    /// divination has already exposed.
    pub fn observe_base(&mut self, viewer: Faction, subject: UnitId, base: BaseVisibility) {
        self.by_view
            .entry((viewer.into(), subject))
            .and_modify(|known| known.base = base)
            .or_insert_with(|| LatticeKnowledge::new(base));
    }

    /// Records one revealed cell for one viewer.
    ///
    /// Does nothing when the viewer has no base visibility of the subject. That
    /// is deliberate rather than defensive: a reveal names a unit the caster
    /// targeted, and targeting requires an Observed anchor, so a reveal against a
    /// subject with no entry at all means the publishing order is wrong and
    /// inventing an entry here would hide it.
    pub fn learn(
        &mut self,
        viewer: Faction,
        subject: UnitId,
        coord: LatticeCoord,
        cell: KnownCell,
    ) -> bool {
        match self.by_view.get_mut(&(viewer.into(), subject)) {
            Some(known) => {
                known.learn(coord, cell);
                true
            }
            None => false,
        }
    }

    /// Drops what a viewer knew about one cell.
    pub fn forget(&mut self, viewer: Faction, subject: UnitId, coord: LatticeCoord) {
        if let Some(known) = self.by_view.get_mut(&(viewer.into(), subject)) {
            known.forget(coord);
        }
    }

    /// Drops everything every faction knew about one subject.
    ///
    /// For a unit leaving the fight: knowledge of a lattice that is no longer
    /// present would otherwise outlive it and be published to a later unit that
    /// reused the id.
    pub fn forget_subject(&mut self, subject: UnitId) {
        self.by_view.retain(|&(_, known), _| known != subject);
        self.truth.remove(&subject);
    }

    /// Ages every view by one round, dropping the facts that have lapsed.
    ///
    /// The dev truth layer is untouched: it is rebuilt from the live lattices
    /// every frame the toggle is on, so decaying it would only make it flicker.
    pub fn decay(&mut self) {
        for known in self.by_view.values_mut() {
            known.decay();
        }
    }

    /// Whether any view is held at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_view.is_empty()
    }

    /// Forgets everything, including the dev layer.
    pub fn clear(&mut self) {
        self.by_view.clear();
        self.truth.clear();
    }
}

/// Shows a designer the truth behind the fog while playing.
///
/// A dev affordance, not a game rule, which is why it is a separate resource
/// rather than a field on [`FactionKnowledge`]: the store's contents stay
/// exactly what the sim believes, and the toggle adds a layer over the top that
/// [`FactionKnowledge::view`] consults. Turning it off restores the honest
/// answer without having to work out which facts the toggle invented.
#[derive(Resource, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[reflect(Resource)]
pub struct RevealAll(pub bool);

/// Registers the knowledge seam.
pub(crate) fn plugin(app: &mut App) {
    app.register_type::<KnowledgeSource>()
        .register_type::<KnowledgeExpiry>()
        .register_type::<RevealAll>()
        .init_resource::<FactionKnowledge>()
        .init_resource::<RevealAll>()
        .add_systems(
            Update,
            (publish_base_visibility, mirror_truth)
                .chain()
                .in_set(AppSystems::Update)
                .in_set(PausableSystems)
                // Publishing must precede the rollover, or a lattice that
                // appeared this round would be decayed before it was ever known.
                .before(crate::CombatSystems::Advance)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            decay_on_round
                .in_set(AppSystems::Update)
                .in_set(PausableSystems)
                // A shared set, not `.chain()`: `RoundElapsed` is written inside
                // `Advance`, and reading it in a system merely *declared* later
                // in the same tuple would race.
                .after(crate::CombatSystems::Advance)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(OnExit(Screen::Gameplay), reset);
}

/// Clears knowledge on leaving gameplay, so a new session cannot inherit views
/// of units that no longer exist.
fn reset(mut knowledge: ResMut<FactionKnowledge>, mut reveal: ResMut<RevealAll>) {
    knowledge.clear();
    *reveal = RevealAll(false);
}

/// Publishes what nobody can hide: every faction learns every lattice's size.
///
/// Base visibility is published to every faction present rather than only to
/// hostile ones. A side knowing its own capacity is trivially true, and the
/// uniform rule means a future neutral or allied-fog case needs no new writer.
///
/// The viewer set is every faction with a unit in the world, **not** every
/// faction that owns a lattice. Those differ: a side whose units carry no lattice
/// yet still has to be able to look at one, and keying the viewers off the
/// subject query would silently deny it a view.
///
/// Publishes for every unit carrying a [`LatticeSpec`], which since HEX-12's content
/// PR is every unit an archetype resolved for.
fn publish_base_visibility(
    mut knowledge: ResMut<FactionKnowledge>,
    viewers: Query<&Faction>,
    subjects: Query<(&UnitId, &Faction, &LatticeSpec)>,
) {
    // Collected and sorted rather than iterated straight out of the queries:
    // query order is not stable, and the write order decides nothing here today
    // but would the moment a writer starts allocating.
    let mut present: Vec<Faction> = Vec::new();
    for faction in &viewers {
        if !present.iter().any(|seen| seen == faction) {
            present.push(*faction);
        }
    }
    let mut rows: Vec<(UnitId, Faction, usize)> = subjects
        .iter()
        .map(|(unit, faction, spec)| (*unit, *faction, spec.capacity()))
        .collect();
    present.sort_by_key(|faction| FactionKey::from(*faction));
    rows.sort_by_key(|&(unit, _, _)| unit);

    for (unit, faction, capacity) in rows {
        let base = BaseVisibility { faction, capacity };
        for viewer in &present {
            knowledge.observe_base(*viewer, unit, base);
        }
    }
}

/// Mirrors live lattices into the dev truth layer while reveal-all is on.
///
/// Rebuilt every frame rather than diffed: the layer is a debugging view of
/// state that changes underneath it, and a stale reveal-all is worse than no
/// reveal-all because it looks authoritative.
fn mirror_truth(
    mut knowledge: ResMut<FactionKnowledge>,
    reveal: Res<RevealAll>,
    subjects: Query<(&UnitId, &Faction, &LatticeSpec, Option<&LatticeState>)>,
) {
    if !reveal.0 {
        // Only pay the clear when there is something to clear, so the shipped
        // build's every-frame cost is one boolean and one emptiness check.
        if !knowledge.truth.is_empty() {
            knowledge.truth.clear();
        }
        return;
    }

    let mut truth = BTreeMap::new();
    for (unit, faction, spec, state) in &subjects {
        let mut known = LatticeKnowledge::new(BaseVisibility {
            faction: *faction,
            capacity: spec.capacity(),
        });
        for (coord, kind) in spec.cells() {
            known.learn(
                coord,
                KnownCell {
                    kind,
                    // A lattice with no state has been inscribed but not yet
                    // rolled into a fight; reporting zero mana would be a claim,
                    // and `None` is the honest answer.
                    mana: state.map(|state| state.mana(coord)),
                    disabled: state.is_some_and(|state| state.is_disabled(coord)),
                    // The toggle shows the truth; it does not pretend the truth
                    // was scried. Tagging it as observation keeps the source
                    // vocabulary meaning what it says.
                    source: KnowledgeSource::Observation,
                    expiry: KnowledgeExpiry::Sustained,
                },
            );
        }
        truth.insert(*unit, known);
    }
    knowledge.truth = truth;
}

/// Ages knowledge one round each time the order wraps.
///
/// Reads [`RoundElapsed`] rather than watching [`TurnOrder::round`](crate::TurnOrder)
/// for changes, so every per-round consumer agrees on exactly when a round ended
/// instead of each re-deriving it from the counter.
fn decay_on_round(
    mut knowledge: ResMut<FactionKnowledge>,
    mut rounds: MessageReader<RoundElapsed>,
) {
    // `count`, not `is_empty`: two rollovers in one frame is not reachable
    // today, but decaying once for both would silently extend every reveal.
    for _ in 0..rounds.read().count() {
        knowledge.decay();
    }
}

#[cfg(test)]
mod tests {
    use hex_core::ElementId;

    use super::*;

    fn base() -> BaseVisibility {
        BaseVisibility {
            faction: Faction::Hostile,
            capacity: 4,
        }
    }

    fn cell(expiry: KnowledgeExpiry, source: KnowledgeSource) -> KnownCell {
        KnownCell {
            kind: CellKind::Gem {
                element: ElementId(0),
            },
            mana: Some(3),
            disabled: false,
            source,
            expiry,
        }
    }

    #[test]
    fn a_subject_nobody_has_seen_is_absent_rather_than_opaque() {
        let knowledge = FactionKnowledge::default();
        assert!(knowledge.view(Faction::Player, UnitId(1)).is_none());
    }

    /// Knowing a lattice exists and knowing nothing about its contents are
    /// different answers, and collapsing them would make "unknown lattice, N
    /// hexes" unrenderable.
    #[test]
    fn base_visibility_is_available_without_any_reveal() {
        let mut knowledge = FactionKnowledge::default();
        knowledge.observe_base(Faction::Player, UnitId(1), base());

        let view = knowledge.view(Faction::Player, UnitId(1)).expect("a view");
        assert!(view.is_opaque(), "nothing has been revealed");
        assert_eq!(view.base().capacity, 4);
        assert_eq!(view.unknown_count(), 4);
        assert_eq!(view.revealed_count(), 0);
    }

    #[test]
    fn knowledge_is_per_viewer() {
        let mut knowledge = FactionKnowledge::default();
        knowledge.observe_base(Faction::Player, UnitId(1), base());
        knowledge.learn(
            Faction::Player,
            UnitId(1),
            LatticeCoord::ORIGIN,
            cell(KnowledgeExpiry::Sustained, KnowledgeSource::Divination),
        );

        assert!(knowledge
            .view(Faction::Player, UnitId(1))
            .expect("a view")
            .cell(LatticeCoord::ORIGIN)
            .is_some());
        assert!(
            knowledge.view(Faction::Hostile, UnitId(1)).is_none(),
            "one faction's reveal must not leak into another's view"
        );
    }

    /// The constraint the whole store exists for: a fact that arrived from a
    /// cast survives on its own schedule, with no observation behind it.
    #[test]
    fn a_divined_fact_decays_on_its_own_schedule() {
        let mut knowledge = FactionKnowledge::default();
        knowledge.observe_base(Faction::Player, UnitId(1), base());
        knowledge.learn(
            Faction::Player,
            UnitId(1),
            LatticeCoord::ORIGIN,
            cell(KnowledgeExpiry::Rounds(1), KnowledgeSource::Divination),
        );

        knowledge.decay();
        let view = knowledge.view(Faction::Player, UnitId(1)).expect("a view");
        assert_eq!(
            view.cell(LatticeCoord::ORIGIN).map(|cell| cell.expiry),
            Some(KnowledgeExpiry::Rounds(0)),
            "one round of a two-round reveal has been spent"
        );

        knowledge.decay();
        let view = knowledge.view(Faction::Player, UnitId(1)).expect("a view");
        assert!(view.is_opaque(), "the reveal has lapsed");
        assert_eq!(
            view.base().capacity,
            4,
            "base visibility must survive the decay that took the reveal"
        );
    }

    /// The design's one-time reveal, spelled `Rounds(0)`: known for the rest of
    /// the current round, gone at the next rollover.
    #[test]
    fn a_one_time_reveal_lasts_until_the_next_rollover() {
        let mut knowledge = FactionKnowledge::default();
        knowledge.observe_base(Faction::Player, UnitId(1), base());
        knowledge.learn(
            Faction::Player,
            UnitId(1),
            LatticeCoord::ORIGIN,
            cell(KnowledgeExpiry::Rounds(0), KnowledgeSource::Divination),
        );

        assert!(!knowledge
            .view(Faction::Player, UnitId(1))
            .expect("a view")
            .is_opaque());
        knowledge.decay();
        assert!(knowledge
            .view(Faction::Player, UnitId(1))
            .expect("a view")
            .is_opaque());
    }

    /// An enchantment-backed divination outlives the rounds a decaying reveal is
    /// measured in; only its writer ends it.
    #[test]
    fn a_sustained_fact_survives_any_number_of_rounds() {
        let mut knowledge = FactionKnowledge::default();
        knowledge.observe_base(Faction::Player, UnitId(1), base());
        knowledge.learn(
            Faction::Player,
            UnitId(1),
            LatticeCoord::ORIGIN,
            cell(KnowledgeExpiry::Sustained, KnowledgeSource::Divination),
        );

        for _ in 0..64 {
            knowledge.decay();
        }
        assert!(!knowledge
            .view(Faction::Player, UnitId(1))
            .expect("a view")
            .is_opaque());

        knowledge.forget(Faction::Player, UnitId(1), LatticeCoord::ORIGIN);
        assert!(knowledge
            .view(Faction::Player, UnitId(1))
            .expect("a view")
            .is_opaque());
    }

    /// Observation- and divination-sourced facts differ in their tags alone; the
    /// store applies the same decay rule to both.
    #[test]
    fn source_does_not_change_how_a_fact_decays() {
        let mut knowledge = FactionKnowledge::default();
        knowledge.observe_base(Faction::Player, UnitId(1), base());
        knowledge.observe_base(Faction::Player, UnitId(2), base());
        knowledge.learn(
            Faction::Player,
            UnitId(1),
            LatticeCoord::ORIGIN,
            cell(KnowledgeExpiry::Rounds(0), KnowledgeSource::Observation),
        );
        knowledge.learn(
            Faction::Player,
            UnitId(2),
            LatticeCoord::ORIGIN,
            cell(KnowledgeExpiry::Rounds(0), KnowledgeSource::Divination),
        );

        knowledge.decay();

        for unit in [UnitId(1), UnitId(2)] {
            assert!(knowledge
                .view(Faction::Player, unit)
                .expect("a view")
                .is_opaque());
        }
    }

    /// A reveal against a subject with no base visibility means the publishing
    /// order is wrong. Inventing an entry would hide that.
    #[test]
    fn learning_about_an_unpublished_subject_is_refused() {
        let mut knowledge = FactionKnowledge::default();
        let accepted = knowledge.learn(
            Faction::Player,
            UnitId(1),
            LatticeCoord::ORIGIN,
            cell(KnowledgeExpiry::Sustained, KnowledgeSource::Divination),
        );

        assert!(!accepted);
        assert!(knowledge.view(Faction::Player, UnitId(1)).is_none());
    }

    #[test]
    fn republishing_base_visibility_keeps_revealed_cells() {
        let mut knowledge = FactionKnowledge::default();
        knowledge.observe_base(Faction::Player, UnitId(1), base());
        knowledge.learn(
            Faction::Player,
            UnitId(1),
            LatticeCoord::ORIGIN,
            cell(KnowledgeExpiry::Sustained, KnowledgeSource::Divination),
        );

        knowledge.observe_base(
            Faction::Player,
            UnitId(1),
            BaseVisibility {
                faction: Faction::Hostile,
                capacity: 9,
            },
        );

        let view = knowledge.view(Faction::Player, UnitId(1)).expect("a view");
        assert_eq!(view.base().capacity, 9);
        assert_eq!(
            view.revealed_count(),
            1,
            "a reveal must survive a republish"
        );
        assert_eq!(view.unknown_count(), 8);
    }

    #[test]
    fn a_departed_subject_is_forgotten_by_every_viewer() {
        let mut knowledge = FactionKnowledge::default();
        knowledge.observe_base(Faction::Player, UnitId(1), base());
        knowledge.observe_base(Faction::Hostile, UnitId(1), base());
        knowledge.observe_base(Faction::Player, UnitId(2), base());

        knowledge.forget_subject(UnitId(1));

        assert!(knowledge.view(Faction::Player, UnitId(1)).is_none());
        assert!(knowledge.view(Faction::Hostile, UnitId(1)).is_none());
        assert!(
            knowledge.view(Faction::Player, UnitId(2)).is_some(),
            "forgetting one subject must not touch another"
        );
    }

    /// Capacity and the revealed set are written by different systems, so a
    /// revealed count that outruns capacity must read as zero unknown rather
    /// than underflowing.
    #[test]
    fn unknown_count_cannot_underflow() {
        let mut view = LatticeKnowledge::new(BaseVisibility {
            faction: Faction::Hostile,
            capacity: 0,
        });
        view.learn(
            LatticeCoord::ORIGIN,
            cell(KnowledgeExpiry::Sustained, KnowledgeSource::Divination),
        );
        assert_eq!(view.unknown_count(), 0);
    }

    /// A later reveal replaces an earlier one rather than merging with it: the
    /// older mana figure has since been spent.
    #[test]
    fn a_fresher_reveal_replaces_a_stale_one() {
        let mut knowledge = FactionKnowledge::default();
        knowledge.observe_base(Faction::Player, UnitId(1), base());
        knowledge.learn(
            Faction::Player,
            UnitId(1),
            LatticeCoord::ORIGIN,
            cell(KnowledgeExpiry::Sustained, KnowledgeSource::Divination),
        );
        knowledge.learn(
            Faction::Player,
            UnitId(1),
            LatticeCoord::ORIGIN,
            KnownCell {
                mana: Some(0),
                ..cell(KnowledgeExpiry::Rounds(2), KnowledgeSource::Observation)
            },
        );

        let view = knowledge.view(Faction::Player, UnitId(1)).expect("a view");
        let known = view.cell(LatticeCoord::ORIGIN).expect("the cell");
        assert_eq!(known.mana, Some(0));
        assert_eq!(known.source, KnowledgeSource::Observation);
        assert_eq!(known.expiry, KnowledgeExpiry::Rounds(2));
        assert_eq!(view.revealed_count(), 1);
    }

    #[test]
    fn clearing_forgets_every_view() {
        let mut knowledge = FactionKnowledge::default();
        knowledge.observe_base(Faction::Player, UnitId(1), base());
        assert!(!knowledge.is_empty());

        knowledge.clear();
        assert!(knowledge.is_empty());
        assert!(knowledge.view(Faction::Player, UnitId(1)).is_none());
    }
}
