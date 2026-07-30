//! Whose turn it is, how combat starts and stops, and when a turn ends.
//!
//! # The shape of a round
//!
//! Combat begins when a hostile comes within engaging distance of the party — the
//! thresholds come from `assets/config/combat.ron`
//! ([`CombatSettings`](hex_assets::CombatSettings)). A [`TurnOrder`] is built from
//! everyone present, sorted by [`Initiative`],
//! and the first unit gets a [`Turn`]. Ending a turn hands the [`Turn`] to the next
//! unit; running off the end wraps to the front and the round number goes up.
//!
//! Combat ends when nothing hostile is within `engage_range + disengage_margin`. The
//! margin is not decoration: without it a unit sitting exactly on the boundary would
//! toggle in and out of combat every frame it drifted a hair either way.
//!
//! # Turns wait for presentation
//!
//! A turn cannot end while the acting unit is still [`Busy`](hex_core::Busy) —
//! the marker the command applier maintains for as long as a walk or swing is
//! in flight. Advancing on the input frame instead would cut the piece off
//! mid-stride and leave it standing between two hexes.
//!
//! # Ending a turn is a command
//!
//! The end-turn key does not touch the order. It emits an end-turn
//! [`GameCommand`](hex_core::GameCommand) into the
//! [`CommandQueue`](hex_core::CommandQueue) like every other intent, the
//! applier yields the unit's remaining budget, and `advance_turn` passes the
//! turn once the unit is spent and still. One consequence is deliberate:
//! pressing the key while the piece is still walking now registers — the yield
//! applies at once and the turn passes when the walk lands, instead of the
//! press being silently lost to the animation.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use std::collections::BTreeMap;

use hex_assets::CombatSettings;
use hex_core::{
    AppSystems, Busy, CommandQueue, ControlOwner, GameCommand, InputAction, InputBindings,
    IssuedCommand, Mode, PausableSystems, PendingDecision, RoundElapsed, Screen, TilePos, Turn,
    UnitId,
};
use hex_lattice::{LatticeSpec, LatticeState};
use hex_units::{
    either_in_reach, Downed, Faction, MovementCrossings, Standing, StandsOn, StopMovingAt,
    UnitAllocator, UnitRegistry,
};

/// Where a unit sits in the turn order. Higher acts first.
///
/// A component so the rule is swappable without touching anything that reads it. The
/// design proposes deriving this from lattice size — which could also give a large
/// lattice several slots in the order, solving boss action economy with the same
/// mechanic — but that policy remains unsettled, so this is a number on a unit.
///
/// **No randomness.** The design is explicit that uncertainty should come from hidden
/// information rather than dice, and a turn order that a player cannot predict makes
/// the multi-caster rituals in the design unplannable.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[reflect(Component)]
pub struct Initiative(pub u32);

/// The running order, and where in it we are.
///
/// Rebuilt whenever combat starts. Cleared when it ends, so its emptiness is the
/// authoritative answer to "are we fighting" and cannot drift from [`Mode`].
///
/// Keyed by stable [`UnitId`], never [`Entity`]: entity indices are recycled
/// and differ across runs and saves, so an order stored as entities silently
/// reshuffles — exactly the randomness-by-another-name the design rules out.
/// Systems that need the actual entity resolve through
/// [`UnitRegistry`].
#[derive(Resource, Debug, Default)]
pub struct TurnOrder {
    /// Units in the order they act.
    order: Vec<UnitId>,
    /// Index into [`Self::order`] of the unit currently acting.
    current: usize,
    /// How many full rounds have elapsed. Purely for display.
    pub round: u32,
}

/// Revived units waiting for the next round boundary before rejoining initiative.
#[derive(Resource, Debug, Default)]
pub(crate) struct PendingRevivals(BTreeMap<UnitId, u32>);

impl PendingRevivals {
    pub(crate) fn schedule(&mut self, unit: UnitId, round: u32) {
        self.0.insert(unit, round);
    }
}

impl TurnOrder {
    /// The unit currently acting, if there is one.
    #[must_use]
    pub fn current(&self) -> Option<UnitId> {
        self.order.get(self.current).copied()
    }

    /// Everyone in the fight, in order.
    #[must_use]
    pub fn order(&self) -> &[UnitId] {
        &self.order
    }

    /// Whether a fight is in progress.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Where a unit sits in the order, counting from zero.
    #[must_use]
    pub fn position_of(&self, unit: UnitId) -> Option<usize> {
        self.order.iter().position(|u| *u == unit)
    }

    /// Moves to the next unit, wrapping and counting a round.
    ///
    /// Returns whether the wrap happened — that is, whether a **round
    /// elapsed**. Several systems need to know: effects that tick per round,
    /// knowledge that decays per round, and anything counting fight length.
    /// Returning it here rather than having each of them re-derive it from
    /// `round` is what keeps them from disagreeing about when a round ended.
    fn advance(&mut self) -> bool {
        if self.order.is_empty() {
            return false;
        }
        self.current += 1;
        if self.current >= self.order.len() {
            self.current = 0;
            self.round += 1;
            return true;
        }
        false
    }

    /// Takes a unit out of the fight, keeping whose turn it is intact.
    ///
    /// Returns whether the unit was in the order at all.
    ///
    /// **`current` is an index, not an id**, so removing somebody earlier in
    /// the order silently hands the turn to the wrong unit unless the index
    /// moves with them. That is the entire reason this is a method rather than
    /// a `Vec::retain` at each call site — death, rout and despawn all need it,
    /// and each would get the off-by-one wrong separately.
    ///
    /// Removing the acting unit leaves `current` pointing at whoever now
    /// occupies that slot — the next unit — which is what a turn order should
    /// do when somebody dies mid-turn. Removing the last unit wraps to the
    /// front **without** counting a round, because no round elapsed: units
    /// simply stopped existing.
    pub fn remove(&mut self, unit: UnitId) -> bool {
        let Some(index) = self.position_of(unit) else {
            return false;
        };
        self.order.remove(index);
        if self.order.is_empty() {
            self.current = 0;
            return true;
        }
        if index < self.current {
            self.current -= 1;
        } else if self.current >= self.order.len() {
            self.current = 0;
        }
        true
    }

    fn clear(&mut self) {
        self.order.clear();
        self.current = 0;
        self.round = 0;
    }
}

/// Registers the loop.
pub fn plugin(app: &mut App) {
    app.add_message::<RoundElapsed>()
        .register_type::<Initiative>()
        .register_type::<Turn>()
        .init_resource::<TurnOrder>()
        .init_resource::<PendingRevivals>()
        // Idempotent alongside hex_units' own init: combat resolves the order
        // through these, so they must exist even in a test app that composes
        // only the combat half.
        .init_resource::<UnitAllocator>()
        .init_resource::<UnitRegistry>()
        .add_systems(OnEnter(Mode::Combat), begin_combat)
        .add_systems(OnExit(Mode::Combat), end_combat)
        // Both are pausable. A fight starting or a turn passing while the player is
        // staring at the pause menu would mean coming back to a world that had moved
        // on without them — the one thing a pause is supposed to prevent.
        .add_systems(
            Update,
            engagement
                .in_set(AppSystems::Update)
                .in_set(PausableSystems)
                .after(hex_units::MovementSystems::Reconcile)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            (
                end_turn_on_space
                    .in_set(AppSystems::RecordInput)
                    // Redundant in the shipped app, where `RecordInput` already
                    // precedes `Update` — but that chain is configured by the
                    // binary, and a test app composing only this crate must not
                    // have the press race the drain.
                    .before(crate::CombatSystems::Apply)
                    .in_set(PausableSystems)
                    .run_if(in_state(Mode::Combat)),
                // Between applying and advancing: a unit that goes down on its own
                // turn must not then be handed that turn.
                check_for_downed
                    .after(crate::CombatSystems::Apply)
                    .in_set(crate::CombatSystems::Resolve)
                    .in_set(PausableSystems)
                    .run_if(in_state(Mode::Combat)),
                crate::resolution::detect_outcome
                    .after(check_for_downed)
                    .in_set(crate::CombatSystems::Resolve)
                    .in_set(PausableSystems)
                    .run_if(in_state(Mode::Combat)),
                advance_turn
                    .in_set(crate::CombatSystems::Advance)
                    .in_set(PausableSystems)
                    .run_if(in_state(Mode::Combat)),
            ),
        )
        .add_systems(OnExit(Screen::Gameplay), reset);
}

/// Clears everything on leaving gameplay, so a new session cannot inherit an order
/// full of despawned entities.
fn reset(mut order: ResMut<TurnOrder>, mut revivals: ResMut<PendingRevivals>) {
    order.clear();
    revivals.0.clear();
}

/// Starts and stops fights based on whether anyone can reach anyone.
///
/// Measured on the hex grid rather than in world units, because that is the unit the
/// rest of the game reasons in and it does not change when someone edits
/// `level_height`.
///
/// **A threshold, not a distance.** There is no single number for how far apart two
/// surfaces are — a unit on a clifftop reaches further down than the one below reaches
/// back up, so the question only has an answer once you say which range you are asking
/// about. [`either_in_reach`] takes that range; the hysteresis is the same two
/// thresholds asked separately rather than one distance compared twice.
fn engagement(
    mut commands: Commands,
    mode: Res<State<Mode>>,
    mut next: ResMut<NextState<Mode>>,
    units: Query<(Entity, &Faction, &StandsOn), Without<Downed>>,
    crossings: Option<Res<MovementCrossings>>,
    settings: Res<CombatSettings>,
) {
    let engage = settings.engage_range;
    let disengage = engage + settings.disengage_margin;
    let levels_per_bonus = settings.levels_per_bonus_range;

    match mode.get() {
        Mode::Exploring => {
            if let Some((entity, stopped)) = crossings.as_deref().and_then(|crossings| {
                first_hostile_crossing(&units, crossings, engage, levels_per_bonus)
            }) {
                // The rendered transform may already have crossed several more legs.
                // Preserve the first engaging waypoint until combat-entry movement
                // reconciliation can snap the unit back to it.
                commands.entity(entity).insert(StopMovingAt::new(stopped));
                next.set(Mode::Combat);
            } else if any_hostile_in_reach(&units, engage, levels_per_bonus) == Some(true) {
                next.set(Mode::Combat);
            }
        }
        // `!= Some(true)` rather than `== Some(false)`, so a fight also ends when one
        // side is gone entirely. That is a different thing from "far apart", and
        // collapsing them would leave combat running with nobody to fight.
        Mode::Combat => {
            // Elimination is terminal and belongs to the retained-world outcome
            // modal. Only two surviving sides that have separated may disengage.
            if any_hostile_in_reach(&units, disengage, levels_per_bonus) == Some(false) {
                next.set(Mode::Exploring);
            }
        }
    }
}

/// Whether any mutually hostile pair can reach each other at `range`.
///
/// [`None`] when one side is absent entirely, which is a different thing from "far
/// apart" and should not start or end a fight by accident.
///
/// Positions are compared as [`TilePos`], **not** as coordinates. Discarding the level
/// put a unit on a bridge and one on the ground beneath it zero hexes apart — true
/// horizontally, and exactly why the answer has to come from a reach rule that knows
/// what height is worth rather than from raw separation.
fn any_hostile_in_reach(
    units: &Query<(Entity, &Faction, &StandsOn), Without<Downed>>,
    range: u32,
    levels_per_bonus: u32,
) -> Option<bool> {
    let mut by_faction: HashMap<Faction, Vec<TilePos>> = HashMap::default();
    for (_, faction, standing) in units.iter() {
        by_faction.entry(*faction).or_default().push(standing.0.pos);
    }

    let mine = by_faction.get(&Faction::Player)?;
    let theirs = by_faction.get(&Faction::Hostile)?;

    Some(mine.iter().any(|a| {
        theirs
            .iter()
            .any(|b| either_in_reach(*a, *b, range, levels_per_bonus))
    }))
}

/// The first completed waypoint this frame that came within hostile reach.
///
/// Reconciliation records every crossed waypoint in route order. Sampling only the
/// final [`StandsOn`] would miss a fast unit that entered and left the engagement
/// radius during one frame.
fn first_hostile_crossing(
    units: &Query<(Entity, &Faction, &StandsOn), Without<Downed>>,
    crossings: &MovementCrossings,
    range: u32,
    levels_per_bonus: u32,
) -> Option<(Entity, Standing)> {
    for (moving_entity, crossed) in crossings.iter() {
        let Ok((_, moving_faction, _)) = units.get(moving_entity) else {
            continue;
        };
        let entered_reach = units.iter().any(|(entity, faction, standing)| {
            entity != moving_entity
                && moving_faction.is_hostile_to(*faction)
                && either_in_reach(crossed.pos, standing.0.pos, range, levels_per_bonus)
        });
        if entered_reach {
            return Some((moving_entity, crossed));
        }
    }
    None
}

/// Builds the order and hands the first unit its turn.
fn begin_combat(
    mut commands: Commands,
    mut turn_order: ResMut<TurnOrder>,
    units: Query<(Entity, Option<&UnitId>, Option<&Initiative>), (With<Faction>, Without<Downed>)>,
    mut allocator: ResMut<UnitAllocator>,
    mut registry: ResMut<UnitRegistry>,
    settings: Res<CombatSettings>,
) {
    // `Option` on both components, so a unit missing one still joins the fight.
    // Requiring them would make the whole order silently empty — every unit
    // filtered out, combat starting with nobody in it and no error anywhere.
    // A unit without a `UnitId` (hand-spawned in a test, or a future spawn path
    // that forgot) is registered here rather than dropped.
    let mut combatants: Vec<(UnitId, Entity, Initiative)> = units
        .iter()
        .map(|(entity, unit, initiative)| {
            let unit = unit.copied().unwrap_or_else(|| {
                // Dealing here re-admits query iteration order into id order —
                // the exact nondeterminism this system exists to remove — so
                // the breach must be observable, never silent.
                warn!("dealing a combat-time id to {entity:?}; a spawn path missed it");
                let id = allocator.allocate();
                commands.entity(entity).insert(id);
                id
            });
            // Upsert unconditionally: a unit carrying an id the registry has
            // not seen (a test's explicit id, a future load path) must still
            // resolve, or its turn silently never advances.
            registry.register(unit, entity);
            let fallback = Initiative(settings.default_initiative);
            (unit, entity, initiative.copied().unwrap_or(fallback))
        })
        .collect();

    // Highest initiative first. Ties break on the stable `UnitId` rather than
    // being left to query order or entity index — the design rules out
    // randomness in resolution, entity indices are not stable across runs or
    // saves, and an order that shuffles between runs would be randomness by
    // another name.
    combatants.sort_by(|(a_unit, _, a_init), (b_unit, _, b_init)| {
        b_init.cmp(a_init).then(a_unit.cmp(b_unit))
    });

    turn_order.clear();
    let first_entity = combatants.first().map(|&(_, entity, _)| entity);
    turn_order.order = combatants.into_iter().map(|(unit, _, _)| unit).collect();

    if let Some(first) = first_entity {
        commands.entity(first).insert(Turn {
            movement_left: settings.movement_per_turn,
            acted: false,
        });
    }
    info!("combat begins: {} combatants", turn_order.order().len());
}

/// Tears the order down and takes the turn marker off whoever holds it.
fn end_combat(
    mut commands: Commands,
    mut turn_order: ResMut<TurnOrder>,
    mut revivals: ResMut<PendingRevivals>,
    acting: Query<Entity, With<Turn>>,
) {
    for entity in &acting {
        commands.entity(entity).remove::<Turn>();
    }
    turn_order.clear();
    revivals.0.clear();
    info!("combat ends");
}

/// Emits an end-turn command when the player presses the key.
///
/// The seat is read off the current player unit itself, so the applier's ownership
/// check passes for exactly the unit this input controls. Enemy turns are deliberately
/// ignored: keyboard input must not become a debug back door that skips hostile actions.
fn end_turn_on_space(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    turn_order: Res<TurnOrder>,
    registry: Res<UnitRegistry>,
    pending: Res<PendingDecision>,
    owners: Query<(Option<&ControlOwner>, &Faction)>,
    mut queue: ResMut<CommandQueue>,
) {
    if !bindings.just_pressed(&keys, InputAction::EndTurn) || pending.is_open() {
        return;
    }
    let Some(current) = turn_order.current() else {
        return;
    };
    let Some(entity) = registry.entity_of(current) else {
        return;
    };
    let Ok((owner, faction)) = owners.get(entity) else {
        return;
    };
    if *faction != Faction::Player {
        return;
    }
    let seat = owner.copied().unwrap_or_default().0;
    queue.push(IssuedCommand {
        seat,
        command: GameCommand::EndTurn { unit: current },
    });
}

/// Passes the turn on when the acting unit is done.
///
/// A unit is done when it has taken its action and spent its movement — which
/// is also the state an applied end-turn command leaves it in. Either way it
/// must not still be [`Busy`]: the marker's removal is what "finished" means,
/// and advancing before then strands the piece between two hexes.
fn advance_turn(
    mut commands: Commands,
    mut turn_order: ResMut<TurnOrder>,
    registry: Res<UnitRegistry>,
    settings: Res<CombatSettings>,
    mut rounds: MessageWriter<RoundElapsed>,
    acting: Query<(Entity, &Turn, Has<Busy>)>,
    initiatives: Query<&Initiative>,
    downed: Query<(), With<Downed>>,
    mut revivals: ResMut<PendingRevivals>,
) {
    let Some(current) = turn_order.current() else {
        return;
    };
    let Some(current_entity) = registry.entity_of(current) else {
        return;
    };
    let Ok((entity, turn, is_busy)) = acting.get(current_entity) else {
        return;
    };

    if is_busy {
        return;
    }

    if !(turn.acted && turn.movement_left == 0) {
        return;
    }

    commands.entity(entity).remove::<Turn>();
    if turn_order.advance() {
        let round = turn_order.round;
        let due: Vec<_> = revivals
            .0
            .iter()
            .filter(|(_, reenters)| **reenters <= round)
            .map(|(&unit, _)| unit)
            .collect();
        for unit in &due {
            revivals.0.remove(unit);
            let active = registry
                .entity_of(*unit)
                .is_some_and(|entity| !downed.contains(entity));
            if active && !turn_order.order.contains(unit) {
                turn_order.order.push(*unit);
            }
        }
        if !due.is_empty() {
            turn_order.order.sort_by(|a, b| {
                let initiative = |unit: &UnitId| {
                    registry
                        .entity_of(*unit)
                        .and_then(|entity| initiatives.get(entity).ok())
                        .copied()
                        .unwrap_or(Initiative(0))
                };
                initiative(b).cmp(&initiative(a)).then(a.cmp(b))
            });
            turn_order.current = 0;
        }
        rounds.write(RoundElapsed);
    }
    if let Some(next) = turn_order
        .current()
        .and_then(|unit| registry.entity_of(unit))
    {
        commands.entity(next).insert(Turn {
            movement_left: settings.movement_per_turn,
            acted: false,
        });
    }
}

/// Movement budget when `combat.ron` has not loaded, for headless harnesses only.
const DEFAULT_MOVEMENT_PER_TURN: u32 = 4;

/// Takes units whose lattice is entirely disabled out of the fight.
///
/// **Downed, not dead.** The design leaves both functional death — a threshold arriving
/// before zero — and permadeath open, and this settles neither: a unit whose every hex
/// is disabled leaves the turn order, gains [`Downed`], and stays on the map with its
/// lattice available to restoration. Renewal removes `Downed` after restoring a cell
/// and schedules the unit to rejoin initiative at the next round boundary. That is a
/// testable starting behaviour, not an answer.
///
/// Runs after the applier, because that is what disables hexes, and before the turn
/// advances, so a unit that goes down on its own turn does not get to take it.
fn check_for_downed(
    mut commands: Commands,
    mut turn_order: ResMut<TurnOrder>,
    registry: Res<UnitRegistry>,
    settings: Option<Res<CombatSettings>>,
    units: Query<(Entity, &UnitId, &LatticeSpec, &LatticeState), Without<Downed>>,
    mut events: MessageWriter<crate::CombatEvent>,
) {
    for (entity, &unit, spec, state) in &units {
        // A lattice with no cells at all is not a downed unit — it is a unit with no
        // lattice, which `all()` would call downed on the vacuous truth.
        if spec.capacity() == 0 || !spec.cells().all(|(coord, _)| state.is_disabled(coord)) {
            continue;
        }
        let held_the_turn = turn_order.current() == Some(unit);
        commands.entity(entity).insert(Downed).remove::<Turn>();
        turn_order.remove(unit);
        events.write(crate::CombatEvent::Downed { unit });
        info!("{unit:?} is down — every hex disabled");

        // **Hand the turn on, or the fight stalls forever.** `advance_turn` only acts
        // on a unit that *holds* a `Turn`, and `TurnOrder::remove` slides `current` onto
        // a successor who has none — so taking the turn-holder out without granting the
        // next one a turn means nobody ever acts again, and only combat ending unwedges
        // it. This is the one path that removes a unit mid-order, so it is the one place
        // that has to do the handover.
        if !held_the_turn {
            continue;
        }
        let budget = settings
            .as_deref()
            .map_or(DEFAULT_MOVEMENT_PER_TURN, |combat| combat.movement_per_turn);
        if let Some(next) = turn_order
            .current()
            .and_then(|unit| registry.entity_of(unit))
        {
            commands.entity(next).insert(Turn {
                movement_left: budget,
                acted: false,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order_of(units: &[UnitId]) -> TurnOrder {
        TurnOrder {
            order: units.to_vec(),
            current: 0,
            round: 0,
        }
    }

    #[test]
    fn an_empty_order_has_nobody_acting() {
        let order = TurnOrder::default();
        assert!(order.is_empty());
        assert_eq!(order.current(), None);
    }

    #[test]
    fn advancing_moves_to_the_next_unit() {
        let mut order = order_of(&[UnitId(1), UnitId(2)]);

        assert_eq!(order.current(), Some(UnitId(1)));
        order.advance();
        assert_eq!(order.current(), Some(UnitId(2)));
    }

    /// Running off the end wraps and counts a round, rather than leaving nobody
    /// acting — which would stall the fight with no way to recover.
    #[test]
    fn the_order_wraps_and_counts_a_round() {
        let mut order = order_of(&[UnitId(1), UnitId(2)]);

        assert_eq!(order.round, 0);
        order.advance();
        order.advance();

        assert_eq!(
            order.current(),
            Some(UnitId(1)),
            "the order should wrap to the front"
        );
        assert_eq!(order.round, 1, "a full cycle is one round");
    }

    /// Advancing an empty order must be a no-op rather than panicking or leaving
    /// `current` pointing past the end.
    #[test]
    fn advancing_an_empty_order_does_nothing() {
        let mut order = TurnOrder::default();
        order.advance();
        assert_eq!(order.current(), None);
        assert_eq!(order.round, 0);
    }

    #[test]
    fn clearing_forgets_the_round_count() {
        let mut order = order_of(&[UnitId(1)]);
        order.advance();
        assert_eq!(order.round, 1);

        order.clear();
        assert!(order.is_empty());
        assert_eq!(order.round, 0, "a new fight starts at round zero");
    }

    /// Removing somebody who already acted must not hand the turn to the
    /// wrong unit. `current` is an index, so a naive `Vec::remove` shifts
    /// everyone left and silently skips whoever was next.
    #[test]
    fn removing_an_earlier_unit_keeps_the_same_unit_acting() {
        let mut order = order_of(&[UnitId(1), UnitId(2), UnitId(3)]);
        order.advance();
        assert_eq!(order.current(), Some(UnitId(2)), "precondition");

        assert!(order.remove(UnitId(1)));

        assert_eq!(
            order.current(),
            Some(UnitId(2)),
            "removing an earlier unit must not change whose turn it is"
        );
    }

    /// Removing whoever is acting passes the turn to the next unit rather than
    /// skipping one, because the survivor slides into the vacated slot.
    #[test]
    fn removing_the_acting_unit_gives_the_turn_to_the_next() {
        let mut order = order_of(&[UnitId(1), UnitId(2), UnitId(3)]);

        assert!(order.remove(UnitId(1)));

        assert_eq!(order.current(), Some(UnitId(2)));
    }

    /// Removing the last unit in the order wraps to the front — and must not
    /// count a round, because none elapsed.
    #[test]
    fn removing_the_last_unit_wraps_without_counting_a_round() {
        let mut order = order_of(&[UnitId(1), UnitId(2)]);
        order.advance();
        assert_eq!(order.current(), Some(UnitId(2)), "precondition");
        assert_eq!(order.round, 0, "precondition");

        assert!(order.remove(UnitId(2)));

        assert_eq!(order.current(), Some(UnitId(1)), "the order should wrap");
        assert_eq!(order.round, 0, "removal is not a round");
    }

    /// The last removal empties the fight without panicking on the index.
    #[test]
    fn removing_everyone_empties_the_order() {
        let mut order = order_of(&[UnitId(1)]);
        assert!(order.remove(UnitId(1)));
        assert!(order.is_empty());
        assert_eq!(order.current(), None);
    }

    #[test]
    fn removing_a_unit_that_is_not_in_the_order_reports_it() {
        let mut order = order_of(&[UnitId(1)]);
        assert!(!order.remove(UnitId(9)));
        assert_eq!(order.order().len(), 1);
    }

    /// The wrap is what a round *is*, and consumers are told about it exactly
    /// once — anything re-deriving it from the counter could double-count.
    #[test]
    fn advance_reports_the_wrap_exactly_once() {
        let mut order = order_of(&[UnitId(1), UnitId(2)]);

        assert!(!order.advance(), "mid-order is not a round boundary");
        assert!(order.advance(), "the wrap is the round boundary");
        assert!(!order.advance(), "and the next step is not");
    }

    #[test]
    fn position_of_finds_a_unit_in_the_order() {
        let order = order_of(&[UnitId(1), UnitId(2)]);

        assert_eq!(order.position_of(UnitId(2)), Some(1));
        assert_eq!(order.position_of(UnitId(9)), None);
    }
}
