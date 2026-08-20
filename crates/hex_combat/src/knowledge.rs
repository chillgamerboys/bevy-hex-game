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
//! [`FactionLatticeKnowledge::view`] is **the** read path. Anything wanting to know
//! something about a hostile lattice goes through it — the AI included — and
//! reading a hostile [`LatticeState`](hex_lattice::LatticeState) directly is a bug.
//!
//! Units receive lattices from `lattices.ron` during gameplay setup. `Reveal`
//! (the shipped "Scrying Eye") writes complete, expiring views here through the
//! cast applier; the systems below keep those already-earned facts current without
//! extending their lifetime.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;

use hex_core::{
    AppSystems, AuthoritativeSystems, KnowledgeExpiry, KnowledgeSource, LatticeCoord,
    PausableSystems, PerceptionSystems, RoundElapsed, Screen, UnitId,
};
use hex_lattice::{CellKind, LatticeSpec, LatticeState};
use hex_perception::FactionMapKnowledge;
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

/// The facts about a unit that ordinary visibility establishes.
///
/// Existence and faction are public once a subject is observed. Lattice capacity,
/// formation, cell contents, mana, and disabled state all require divination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseVisibility {
    /// The side the subject fights for.
    pub faction: Faction,
}

/// One known cell, with where the knowledge came from and when it lapses.
///
/// Source and expiry stay per-cell so facts retain their provenance and can
/// expire independently. The current Scrying Eye reveals every cell at once,
/// while this representation also supports future partial-knowledge sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownCell {
    /// What the inscription puts in the cell.
    pub kind: CellKind,
    /// Mana currently held while this fact remains divined.
    ///
    /// [`None`] when the reveal exposed the cell's existence but not its charge,
    /// which is a different thing from an empty gem and must not read as one.
    pub mana: Option<u16>,
    /// Whether the cell is currently disabled while this fact remains divined.
    pub disabled: bool,
    /// Which channel wrote this fact.
    pub source: KnowledgeSource,
    /// What remains of this fact's lifetime.
    pub expiry: KnowledgeExpiry,
}

/// What one faction knows about one unit's lattice.
///
/// A stored entry retains its base facts; [`FactionLatticeKnowledge`] separately
/// gates whether that entry is currently readable. The cell map holds only what
/// has actually been revealed, so an absent coordinate means *unknown* rather
/// than *empty*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatticeKnowledge {
    base: BaseVisibility,
    capacity: Option<KnownCapacity>,
    cells: BTreeMap<LatticeCoord, KnownCell>,
}

/// A learned capacity and the clock that governs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KnownCapacity {
    value: usize,
    expiry: KnowledgeExpiry,
}

impl LatticeKnowledge {
    /// Creates knowledge holding only the facts nobody can hide.
    #[must_use]
    pub fn new(base: BaseVisibility) -> Self {
        Self {
            base,
            capacity: None,
            cells: BTreeMap::new(),
        }
    }

    /// The facts available without any reveal.
    #[must_use]
    pub fn base(&self) -> BaseVisibility {
        self.base
    }

    /// The divined lattice capacity, or [`None`] while capacity remains hidden.
    #[must_use]
    pub fn known_capacity(&self) -> Option<usize> {
        self.capacity.map(|known| known.value)
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

    /// How many cells remain unknown, when capacity itself has been learned.
    ///
    /// Saturating rather than wrapping: base visibility and the revealed set are
    /// written by different systems, and a capacity that has not caught up with a
    /// reveal must read as zero unknown rather than as four billion.
    #[must_use]
    pub fn unknown_count(&self) -> Option<usize> {
        self.known_capacity()
            .map(|capacity| capacity.saturating_sub(self.cells.len()))
    }

    /// Whether every cell promised by the learned capacity has been revealed.
    ///
    /// This is the knowledge-only authorization check used before consulting an
    /// authoritative hostile lattice. Equality is deliberate: a stale or malformed
    /// capacity smaller than the revealed set must fail closed rather than inheriting
    /// the saturating behavior of [`Self::unknown_count`].
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.known_capacity() == Some(self.cells.len())
    }

    /// Whether a complete view exactly matches an already-authorized lattice spec.
    ///
    /// This is an integrity/test predicate, not the initial hostile-information gate:
    /// callers must establish [`Self::is_complete`] before reading an authoritative
    /// hostile spec. Once access is authorized, this stronger comparison also checks
    /// every exact coordinate and cell kind.
    #[must_use]
    pub fn is_complete_for(&self, spec: &LatticeSpec) -> bool {
        let expected = spec.cells().count();
        self.is_complete()
            && self.cells.len() == expected
            && spec.cells().all(|(coord, kind)| {
                self.cells
                    .get(&coord)
                    .is_some_and(|known| known.kind == kind)
            })
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

    /// Records the lattice capacity with the same expiry vocabulary as known cells.
    fn learn_capacity(&mut self, value: usize, expiry: KnowledgeExpiry) {
        self.capacity = Some(KnownCapacity { value, expiry });
    }

    /// Drops what was known about one cell.
    pub fn forget(&mut self, coord: LatticeCoord) -> Option<KnownCell> {
        self.cells.remove(&coord)
    }

    /// Ages every revealed cell by one round, dropping the lapsed ones.
    ///
    /// Stored base facts are untouched. Current spatial observation gates whether
    /// the containing view is readable.
    fn decay(&mut self) {
        self.capacity = self.capacity.and_then(|mut known| {
            known.expiry.tick().map(|remaining| {
                known.expiry = remaining;
                known
            })
        });
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
pub struct FactionLatticeKnowledge {
    by_view: BTreeMap<(FactionKey, UnitId), LatticeKnowledge>,
    /// Subjects currently observed according to world-owned spatial knowledge.
    ///
    /// Divined facts remain in `by_view` while a subject is hidden, but this set
    /// gates every ordinary read and write so stored facts cannot reveal that a
    /// currently unseen unit exists.
    observed: BTreeSet<(FactionKey, UnitId)>,
    /// Ground truth, populated only while the dev reveal-all toggle is on.
    ///
    /// A separate layer rather than a flag on each entry, so switching the toggle
    /// off cannot strip a fact the game legitimately knew. Empty in the shipped
    /// build, where it costs one map lookup that always misses.
    truth: BTreeMap<UnitId, LatticeKnowledge>,
}

impl FactionLatticeKnowledge {
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
        self.truth.get(&subject).or_else(|| {
            let key = (viewer.into(), subject);
            if self.observed.contains(&key) {
                self.by_view.get(&key)
            } else {
                None
            }
        })
    }

    /// Publishes the facts a subject cannot hide from a viewer.
    ///
    /// Idempotent: re-publishing updates faction and leaves every divined fact
    /// alone.
    pub fn observe_base(&mut self, viewer: Faction, subject: UnitId, base: BaseVisibility) {
        let key = (viewer.into(), subject);
        self.observed.insert(key);
        self.by_view
            .entry(key)
            .and_modify(|known| known.base = base)
            .or_insert_with(|| LatticeKnowledge::new(base));
    }

    /// Replaces current visibility with the authoritative spatial observation.
    ///
    /// Stored divination facts are deliberately retained for their own expiry
    /// window. They become readable again only if the subject is re-observed.
    fn replace_observed(
        &mut self,
        observations: impl IntoIterator<Item = (Faction, UnitId, BaseVisibility)>,
    ) {
        self.observed.clear();
        for (viewer, subject, base) in observations {
            self.observe_base(viewer, subject, base);
        }
    }

    /// Records one revealed cell for one viewer.
    ///
    /// Does nothing when the viewer does not currently observe the subject. That
    /// is deliberate rather than defensive: a reveal names a unit the caster
    /// targeted, and targeting requires an Observed anchor, so a reveal without
    /// current spatial authority means the publishing order is wrong.
    pub fn learn(
        &mut self,
        viewer: Faction,
        subject: UnitId,
        coord: LatticeCoord,
        cell: KnownCell,
    ) -> bool {
        let key = (viewer.into(), subject);
        if !self.observed.contains(&key) {
            return false;
        }
        match self.by_view.get_mut(&key) {
            Some(known) => {
                known.learn(coord, cell);
                true
            }
            None => false,
        }
    }

    /// Reveals a subject's complete live lattice for one expiry window.
    ///
    /// Returns the exact coordinates revealed, or [`None`] when the subject is not
    /// currently observed. Refusing to reveal a retained hidden entry preserves the
    /// distinction between "unknown subject" and "observed subject with an opaque
    /// lattice".
    pub(crate) fn reveal(
        &mut self,
        viewer: Faction,
        subject: UnitId,
        spec: &LatticeSpec,
        state: &LatticeState,
        expiry: KnowledgeExpiry,
    ) -> Option<Vec<LatticeCoord>> {
        let key = (viewer.into(), subject);
        if !self.observed.contains(&key) {
            return None;
        }
        let known = self.by_view.get_mut(&key)?;
        known.learn_capacity(spec.capacity(), expiry);
        let mut revealed = Vec::with_capacity(spec.capacity());
        for (coord, kind) in spec.cells() {
            known.learn(
                coord,
                KnownCell {
                    kind,
                    mana: Some(state.mana(coord)),
                    disabled: state.is_disabled(coord),
                    source: KnowledgeSource::Divination,
                    expiry,
                },
            );
            revealed.push(coord);
        }
        Some(revealed)
    }

    /// Drops what a viewer knew about one cell.
    pub fn forget(&mut self, viewer: Faction, subject: UnitId, coord: LatticeCoord) {
        if let Some(known) = self.by_view.get_mut(&(viewer.into(), subject)) {
            known.forget(coord);
        }
    }

    /// Drops everything every faction knew about one subject.
    ///
    /// For actual despawn or session teardown, never for downing: a downed entity
    /// keeps its stable id and lattice for restoration and revival.
    pub fn forget_subject(&mut self, subject: UnitId) {
        self.by_view.retain(|&(_, known), _| known != subject);
        self.observed.retain(|&(_, known)| known != subject);
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
        self.observed.clear();
        self.truth.clear();
    }
}

/// Shows a designer the truth behind the fog while playing.
///
/// A dev affordance, not a game rule, which is why it is a separate resource
/// rather than a field on [`FactionLatticeKnowledge`]: the store's contents stay
/// exactly what the sim believes, and the toggle adds a layer over the top that
/// [`FactionLatticeKnowledge::view`] consults. Turning it off restores the honest
/// answer without having to work out which facts the toggle invented.
#[derive(Resource, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[reflect(Resource)]
pub struct RevealAll(pub bool);

/// Registers the knowledge seam.
pub(crate) fn plugin(app: &mut App) {
    app.register_type::<KnowledgeSource>()
        .register_type::<KnowledgeExpiry>()
        .register_type::<RevealAll>()
        .init_resource::<FactionLatticeKnowledge>()
        .init_resource::<RevealAll>()
        .add_systems(
            Update,
            sync_spatial_visibility
                .in_set(AppSystems::Update)
                .in_set(AuthoritativeSystems)
                .in_set(PausableSystems)
                // World perception publishes the authoritative observation first;
                // combat only adapts it into the lattice read seam.
                .after(PerceptionSystems::PublishKnowledge)
                // AI and player commands in this frame must consume the same current
                // spatial publication. A Reveal applied later still needs the
                // existence entry first.
                .before(crate::CombatSystems::Act)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            (refresh_known_truth, mirror_truth)
                .chain()
                .in_set(AppSystems::Update)
                .in_set(AuthoritativeSystems)
                .in_set(PausableSystems)
                // Payment, damage and Reveal all write facts this projection reads.
                .after(crate::CombatSystems::Apply)
                .before(crate::CombatSystems::Advance)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            decay_on_round
                .in_set(AppSystems::Update)
                .in_set(AuthoritativeSystems)
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
fn reset(mut knowledge: ResMut<FactionLatticeKnowledge>, mut reveal: ResMut<RevealAll>) {
    knowledge.clear();
    *reveal = RevealAll(false);
}

/// Adapts world-owned current observations into the lattice knowledge seam.
///
/// The adapter filters to actual lattice subjects, but never derives observation
/// from their ECS presence. If spatial knowledge is absent or a subject falls out
/// of sight, its ordinary lattice view disappears immediately.
fn sync_spatial_visibility(
    mut knowledge: ResMut<FactionLatticeKnowledge>,
    spatial: Option<Res<FactionMapKnowledge>>,
    subjects: Query<&UnitId, With<LatticeSpec>>,
) {
    let lattice_subjects: BTreeSet<UnitId> = subjects.iter().copied().collect();
    let mut observed = Vec::new();
    if let Some(spatial) = spatial {
        for viewer in [Faction::Player, Faction::Hostile] {
            for (unit, snapshot) in spatial.faction(viewer).units() {
                if lattice_subjects.contains(&unit) {
                    observed.push((
                        viewer,
                        unit,
                        BaseVisibility {
                            faction: snapshot.faction,
                        },
                    ));
                }
            }
        }
    }
    knowledge.replace_observed(observed);
}

/// Refreshes facts already earned by divination from current battle truth.
///
/// Only value fields change. Source and expiry remain untouched, so spending mana or
/// taking damage appears immediately without a frame of stale authority and without
/// silently extending the reveal.
fn refresh_known_truth(
    mut knowledge: ResMut<FactionLatticeKnowledge>,
    subjects: Query<(&UnitId, &LatticeSpec, &LatticeState)>,
) {
    for (subject, spec, state) in &subjects {
        for ((_, known_subject), known) in &mut knowledge.by_view {
            if known_subject != subject {
                continue;
            }
            if let Some(capacity) = &mut known.capacity {
                capacity.value = spec.capacity();
            }
            for (coord, kind) in spec.cells() {
                if let Some(cell) = known.cells.get_mut(&coord) {
                    if cell.source != KnowledgeSource::Divination {
                        continue;
                    }
                    cell.kind = kind;
                    cell.mana = Some(state.mana(coord));
                    cell.disabled = state.is_disabled(coord);
                }
            }
        }
    }
}

/// Mirrors live lattices into the dev truth layer while reveal-all is on.
///
/// Rebuilt every frame rather than diffed: the layer is a debugging view of
/// state that changes underneath it, and a stale reveal-all is worse than no
/// reveal-all because it looks authoritative.
fn mirror_truth(
    mut knowledge: ResMut<FactionLatticeKnowledge>,
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
        let mut known = LatticeKnowledge::new(BaseVisibility { faction: *faction });
        known.learn_capacity(spec.capacity(), KnowledgeExpiry::Sustained);
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
    mut knowledge: ResMut<FactionLatticeKnowledge>,
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
        let knowledge = FactionLatticeKnowledge::default();
        assert!(knowledge.view(Faction::Player, UnitId(1)).is_none());
    }

    /// Knowing a lattice exists reveals neither its capacity nor its contents.
    #[test]
    fn base_visibility_is_available_without_any_reveal() {
        let mut knowledge = FactionLatticeKnowledge::default();
        knowledge.observe_base(Faction::Player, UnitId(1), base());

        let view = knowledge.view(Faction::Player, UnitId(1)).expect("a view");
        assert!(view.is_opaque(), "nothing has been revealed");
        assert_eq!(view.base().faction, Faction::Hostile);
        assert_eq!(view.known_capacity(), None);
        assert_eq!(view.unknown_count(), None);
        assert_eq!(view.revealed_count(), 0);
    }

    #[test]
    fn knowledge_is_per_viewer() {
        let mut knowledge = FactionLatticeKnowledge::default();
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
        let mut knowledge = FactionLatticeKnowledge::default();
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
        assert_eq!(view.base().faction, Faction::Hostile);
        assert_eq!(view.known_capacity(), None);
    }

    /// The design's one-time reveal, spelled `Rounds(0)`: known for the rest of
    /// the current round, gone at the next rollover.
    #[test]
    fn a_one_time_reveal_lasts_until_the_next_rollover() {
        let mut knowledge = FactionLatticeKnowledge::default();
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
        let mut knowledge = FactionLatticeKnowledge::default();
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
        let mut knowledge = FactionLatticeKnowledge::default();
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
        let mut knowledge = FactionLatticeKnowledge::default();
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
        let mut knowledge = FactionLatticeKnowledge::default();
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
                faction: Faction::Player,
            },
        );

        let view = knowledge.view(Faction::Player, UnitId(1)).expect("a view");
        assert_eq!(view.base().faction, Faction::Player);
        assert_eq!(
            view.revealed_count(),
            1,
            "a reveal must survive a republish"
        );
        assert_eq!(view.known_capacity(), None);
    }

    #[test]
    fn a_departed_subject_is_forgotten_by_every_viewer() {
        let mut knowledge = FactionLatticeKnowledge::default();
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
        });
        view.learn_capacity(0, KnowledgeExpiry::Sustained);
        view.learn(
            LatticeCoord::ORIGIN,
            cell(KnowledgeExpiry::Sustained, KnowledgeSource::Divination),
        );
        assert_eq!(view.unknown_count(), Some(0));
    }

    #[test]
    fn completeness_matches_the_exact_current_spec_not_saturating_unknown_count() {
        let spec = LatticeSpec::default()
            .with(
                LatticeCoord::ORIGIN,
                CellKind::Gem {
                    element: ElementId(0),
                },
            )
            .with(
                LatticeCoord::new(1, 0),
                CellKind::Gem {
                    element: ElementId(0),
                },
            );
        let mut view = LatticeKnowledge::new(base());
        view.learn_capacity(0, KnowledgeExpiry::Sustained);
        view.learn(
            LatticeCoord::ORIGIN,
            cell(KnowledgeExpiry::Sustained, KnowledgeSource::Divination),
        );
        assert_eq!(view.unknown_count(), Some(0));
        assert!(!view.is_complete());
        assert!(!view.is_complete_for(&spec));

        view.learn_capacity(2, KnowledgeExpiry::Sustained);
        assert!(!view.is_complete_for(&spec));
        view.learn(
            LatticeCoord::new(1, 0),
            cell(KnowledgeExpiry::Sustained, KnowledgeSource::Divination),
        );
        assert!(view.is_complete());
        assert!(view.is_complete_for(&spec));

        view.forget(LatticeCoord::new(1, 0));
        view.learn(
            LatticeCoord::new(0, 1),
            cell(KnowledgeExpiry::Sustained, KnowledgeSource::Divination),
        );
        assert!(
            view.is_complete(),
            "capacity alone cannot distinguish a stale same-size lattice shape"
        );
        assert!(
            !view.is_complete_for(&spec),
            "exact current coordinates remain mandatory after the knowledge-only gate"
        );
    }

    /// A later reveal replaces an earlier one rather than merging with it: the
    /// older mana figure has since been spent.
    #[test]
    fn a_fresher_reveal_replaces_a_stale_one() {
        let mut knowledge = FactionLatticeKnowledge::default();
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
        let mut knowledge = FactionLatticeKnowledge::default();
        knowledge.observe_base(Faction::Player, UnitId(1), base());
        assert!(!knowledge.is_empty());

        knowledge.clear();
        assert!(knowledge.is_empty());
        assert!(knowledge.view(Faction::Player, UnitId(1)).is_none());
    }
}
