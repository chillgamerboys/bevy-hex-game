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
use hex_core::{AppSystems, HexCoord, Mode, PausableSystems, Screen, Turn};
use hex_units::{Faction, StandsOn};

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
#[derive(Resource, Debug, Default)]
pub struct TurnOrder {
    /// Units in the order they act.
    order: Vec<Entity>,
    /// Index into [`Self::order`] of the unit currently acting.
    current: usize,
    /// How many full rounds have elapsed. Purely for display.
    pub round: u32,
}

impl TurnOrder {
    /// The unit currently acting, if there is one.
    #[must_use]
    pub fn current(&self) -> Option<Entity> {
        self.order.get(self.current).copied()
    }

    /// Everyone in the fight, in order.
    #[must_use]
    pub fn order(&self) -> &[Entity] {
        &self.order
    }

    /// Whether a fight is in progress.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Where a unit sits in the order, counting from zero.
    #[must_use]
    pub fn position_of(&self, entity: Entity) -> Option<usize> {
        self.order.iter().position(|e| *e == entity)
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
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            advance_turn
                .in_set(AppSystems::Update)
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

/// Starts and stops fights based on how close the nearest hostile is.
///
/// Distance is measured on the hex grid rather than in world units, because that is
/// the unit the rest of the game reasons in and it does not change when someone edits
/// `level_height`.
fn engagement(
    mode: Res<State<Mode>>,
    mut next: ResMut<NextState<Mode>>,
    units: Query<(&Faction, &StandsOn)>,
) {
    let Some(nearest) = nearest_hostile_distance(&units) else {
        // Nobody left to fight, or nobody to fight with.
        if *mode.get() == Mode::Combat {
            next.set(Mode::Exploring);
        }
        return;
    };

    match mode.get() {
        Mode::Exploring if nearest <= CombatSettings::ENGAGE_RANGE => {
            next.set(Mode::Combat);
        }
        Mode::Combat
            if nearest > CombatSettings::ENGAGE_RANGE + CombatSettings::DISENGAGE_MARGIN =>
        {
            next.set(Mode::Exploring);
        }
        _ => {}
    }
}

/// The distance between the closest pair of mutually hostile units.
///
/// [`None`] when one side is absent entirely, which is a different thing from "far
/// apart" and should not start or end a fight by accident.
fn nearest_hostile_distance(units: &Query<(&Faction, &StandsOn)>) -> Option<u32> {
    let mut by_faction: HashMap<Faction, Vec<HexCoord>> = HashMap::default();
    for (faction, standing) in units.iter() {
        by_faction
            .entry(*faction)
            .or_default()
            .push(standing.0.pos.coord);
    }

    let mine = by_faction.get(&Faction::Player)?;
    let theirs = by_faction.get(&Faction::Hostile)?;

    mine.iter()
        .flat_map(|a| theirs.iter().map(move |b| a.distance(*b)))
        .min()
}

/// Builds the order and hands the first unit its turn.
fn begin_combat(
    mut commands: Commands,
    mut turn_order: ResMut<TurnOrder>,
    units: Query<(Entity, Option<&Initiative>), With<Faction>>,
) {
    // `Option`, so a unit without an explicit initiative still joins the fight.
    // Requiring the component would make the whole order silently empty — every unit
    // filtered out, combat starting with nobody in it and no error anywhere.
    let mut combatants: Vec<(Entity, Initiative)> = units
        .iter()
        .map(|(entity, initiative)| (entity, initiative.copied().unwrap_or_default()))
        .collect();

    // Highest initiative first. Ties break on entity index rather than being left to
    // query order, so the same units always produce the same order — the design rules
    // out randomness in resolution, and an order that shuffles between runs would be
    // randomness by another name.
    combatants.sort_by(|(a_entity, a_init), (b_entity, b_init)| {
        b_init.cmp(a_init).then(a_entity.cmp(b_entity))
    });

    turn_order.clear();
    turn_order.order = combatants.into_iter().map(|(entity, _)| entity).collect();

    if let Some(first) = turn_order.current() {
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
    acting: Query<(Entity, &Turn, Has<Transformation>)>,
) {
    let Some(current) = turn_order.current() else {
        return;
    };
    let Ok((entity, turn, is_moving)) = acting.get(current) else {
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
    if let Some(next) = turn_order.current() {
        commands.entity(next).insert(Turn {
            movement_left: MOVEMENT_PER_TURN,
            acted: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order_of(entities: &[Entity]) -> TurnOrder {
        TurnOrder {
            order: entities.to_vec(),
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
        let a = Entity::from_raw_u32(1).expect("a valid entity id");
        let b = Entity::from_raw_u32(2).expect("a valid entity id");
        let mut order = order_of(&[a, b]);

        assert_eq!(order.current(), Some(a));
        order.advance();
        assert_eq!(order.current(), Some(b));
    }

    /// Running off the end wraps and counts a round, rather than leaving nobody
    /// acting — which would stall the fight with no way to recover.
    #[test]
    fn the_order_wraps_and_counts_a_round() {
        let a = Entity::from_raw_u32(1).expect("a valid entity id");
        let b = Entity::from_raw_u32(2).expect("a valid entity id");
        let mut order = order_of(&[a, b]);

        assert_eq!(order.round, 0);
        order.advance();
        order.advance();

        assert_eq!(
            order.current(),
            Some(a),
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
        let a = Entity::from_raw_u32(1).expect("a valid entity id");
        let mut order = order_of(&[a]);
        order.advance();
        assert_eq!(order.round, 1);

        order.clear();
        assert!(order.is_empty());
        assert_eq!(order.round, 0, "a new fight starts at round zero");
    }

    #[test]
    fn position_of_finds_a_unit_in_the_order() {
        let a = Entity::from_raw_u32(1).expect("a valid entity id");
        let b = Entity::from_raw_u32(2).expect("a valid entity id");
        let order = order_of(&[a, b]);

        assert_eq!(order.position_of(b), Some(1));
        assert_eq!(
            order.position_of(Entity::from_raw_u32(9).expect("a valid entity id")),
            None
        );
    }
}
