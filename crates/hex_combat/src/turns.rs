//! Whose turn it is, how combat starts and stops, and when a turn ends.
//!
//! # The shape of a round
//!
//! Combat begins when a hostile comes within engaging distance of the party — see
//! [`CombatSettings`](crate::turns::CombatSettings). A [`TurnOrder`] is built from
//! everyone present, sorted by [`Initiative`],
//! and the first unit gets a [`Turn`]. Ending a turn hands the [`Turn`] to the next
//! unit; running off the end wraps to the front and the round number goes up.
//!
//! Combat ends when nothing hostile is within `engage_range + disengage_margin`. The
//! margin is not decoration: without it a unit sitting exactly on the boundary would
//! toggle in and out of combat every frame it drifted a hair either way.
//!
//! # Turns wait for animation
//!
//! A turn cannot end while the acting unit still carries a
//! [`Transformation`](hex_anim::Transformation) — the component's *removal* is the
//! signal that a move finished. Advancing on the input frame instead would cut the
//! piece off mid-stride and leave it standing between two hexes.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use hex_anim::Transformation;
use hex_core::{AppSystems, Mode, PausableSystems, Screen, TilePos, Turn, UnitId};
use hex_units::{
    either_in_reach, Faction, MovementCrossings, Standing, StandsOn, StopMovingAt, UnitAllocator,
    UnitRegistry,
};

/// How far apart two units can be and still start a fight, and related knobs.
///
/// Not loaded from RON yet — these are the crudest possible defaults and exist to be
/// argued with. When they earn their place they belong in `assets/config/combat.ron`
/// alongside the rest of the designer-facing settings.
pub struct CombatSettings;

impl CombatSettings {
    /// Hexes between a hostile and the party that start a fight.
    pub const ENGAGE_RANGE: u32 = 4;

    /// Extra hexes a hostile must retreat beyond [`Self::ENGAGE_RANGE`] before combat
    /// ends. Prevents a unit on the boundary flipping in and out every frame.
    pub const DISENGAGE_MARGIN: u32 = 2;
}

/// Where a unit sits in the turn order. Higher acts first.
///
/// A component so the rule is swappable without touching anything that reads it. The
/// design proposes deriving this from lattice size — which would also give a large
/// lattice several slots in the order, solving boss action economy with the same
/// mechanic — but lattices do not exist yet, so this is a number on a unit.
///
/// **No randomness.** The design is explicit that uncertainty should come from hidden
/// information rather than dice, and a turn order that a player cannot predict makes
/// the multi-caster rituals in the design unplannable.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[reflect(Component)]
pub struct Initiative(pub u32);

impl Default for Initiative {
    fn default() -> Self {
        Self(10)
    }
}

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
    fn advance(&mut self) {
        if self.order.is_empty() {
            return;
        }
        self.current += 1;
        if self.current >= self.order.len() {
            self.current = 0;
            self.round += 1;
        }
    }

    fn clear(&mut self) {
        self.order.clear();
        self.current = 0;
        self.round = 0;
    }
}

/// Hexes a unit may move on its turn.
///
/// Provisional. The design's current preference is one or two hexes of free movement
/// plus one action, which keeps big spells feeling categorical while stopping the map
/// from being scenery. Four is generous enough to make terrain worth looking at while
/// the map is the thing being tested.
const MOVEMENT_PER_TURN: u32 = 4;

/// Registers the loop.
pub fn plugin(app: &mut App) {
    app.register_type::<Initiative>()
        .register_type::<Turn>()
        .init_resource::<TurnOrder>()
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
            advance_turn
                .in_set(crate::CombatSystems::Advance)
                .in_set(PausableSystems)
                .run_if(in_state(Mode::Combat)),
        )
        .add_systems(OnExit(Screen::Gameplay), reset);
}

/// Clears everything on leaving gameplay, so a new session cannot inherit an order
/// full of despawned entities.
fn reset(mut order: ResMut<TurnOrder>) {
    order.clear();
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
    units: Query<(Entity, &Faction, &StandsOn)>,
    crossings: Option<Res<MovementCrossings>>,
) {
    let engage = CombatSettings::ENGAGE_RANGE;
    let disengage = engage + CombatSettings::DISENGAGE_MARGIN;

    match mode.get() {
        Mode::Exploring => {
            if let Some((entity, stopped)) = crossings
                .as_deref()
                .and_then(|crossings| first_hostile_crossing(&units, crossings, engage))
            {
                // The rendered transform may already have crossed several more legs.
                // Preserve the first engaging waypoint until combat-entry movement
                // reconciliation can snap the unit back to it.
                commands.entity(entity).insert(StopMovingAt::new(stopped));
                next.set(Mode::Combat);
            } else if any_hostile_in_reach(&units, engage) == Some(true) {
                next.set(Mode::Combat);
            }
        }
        // `!= Some(true)` rather than `== Some(false)`, so a fight also ends when one
        // side is gone entirely. That is a different thing from "far apart", and
        // collapsing them would leave combat running with nobody to fight.
        Mode::Combat => {
            if any_hostile_in_reach(&units, disengage) != Some(true) {
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
fn any_hostile_in_reach(units: &Query<(Entity, &Faction, &StandsOn)>, range: u32) -> Option<bool> {
    let mut by_faction: HashMap<Faction, Vec<TilePos>> = HashMap::default();
    for (_, faction, standing) in units.iter() {
        by_faction.entry(*faction).or_default().push(standing.0.pos);
    }

    let mine = by_faction.get(&Faction::Player)?;
    let theirs = by_faction.get(&Faction::Hostile)?;

    Some(
        mine.iter()
            .any(|a| theirs.iter().any(|b| either_in_reach(*a, *b, range))),
    )
}

/// The first completed waypoint this frame that came within hostile reach.
///
/// Reconciliation records every crossed waypoint in route order. Sampling only the
/// final [`StandsOn`] would miss a fast unit that entered and left the engagement
/// radius during one frame.
fn first_hostile_crossing(
    units: &Query<(Entity, &Faction, &StandsOn)>,
    crossings: &MovementCrossings,
    range: u32,
) -> Option<(Entity, Standing)> {
    for (moving_entity, crossed) in crossings.iter() {
        let Ok((_, moving_faction, _)) = units.get(moving_entity) else {
            continue;
        };
        let entered_reach = units.iter().any(|(entity, faction, standing)| {
            entity != moving_entity
                && moving_faction.is_hostile_to(*faction)
                && either_in_reach(crossed.pos, standing.0.pos, range)
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
    units: Query<(Entity, Option<&UnitId>, Option<&Initiative>), With<Faction>>,
    mut allocator: ResMut<UnitAllocator>,
    mut registry: ResMut<UnitRegistry>,
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
            (unit, entity, initiative.copied().unwrap_or_default())
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
            movement_left: MOVEMENT_PER_TURN,
            acted: false,
        });
    }
    info!("combat begins: {} combatants", turn_order.order().len());
}

/// Tears the order down and takes the turn marker off whoever holds it.
fn end_combat(
    mut commands: Commands,
    mut turn_order: ResMut<TurnOrder>,
    acting: Query<Entity, With<Turn>>,
) {
    for entity in &acting {
        commands.entity(entity).remove::<Turn>();
    }
    turn_order.clear();
    info!("combat ends");
}

/// Passes the turn on when the acting unit is done.
///
/// A unit is done when it has taken its action and spent its movement, or when the
/// player presses the end-turn key. Either way it must not still be moving: the
/// absence of a [`Transformation`] is what "finished moving" means, and advancing
/// before then strands the piece between two hexes.
fn advance_turn(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut turn_order: ResMut<TurnOrder>,
    registry: Res<UnitRegistry>,
    acting: Query<(Entity, &Turn, Has<Transformation>)>,
) {
    let Some(current) = turn_order.current() else {
        return;
    };
    let Some(current_entity) = registry.entity_of(current) else {
        return;
    };
    let Ok((entity, turn, is_moving)) = acting.get(current_entity) else {
        return;
    };

    if is_moving {
        return;
    }

    let spent = turn.acted && turn.movement_left == 0;
    if !spent && !keys.just_pressed(KeyCode::Space) {
        return;
    }

    commands.entity(entity).remove::<Turn>();
    turn_order.advance();
    if let Some(next) = turn_order
        .current()
        .and_then(|unit| registry.entity_of(unit))
    {
        commands.entity(next).insert(Turn {
            movement_left: MOVEMENT_PER_TURN,
            acted: false,
        });
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

    #[test]
    fn position_of_finds_a_unit_in_the_order() {
        let order = order_of(&[UnitId(1), UnitId(2)]);

        assert_eq!(order.position_of(UnitId(2)), Some(1));
        assert_eq!(order.position_of(UnitId(9)), None);
    }
}
