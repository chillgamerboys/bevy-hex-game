//! What an enemy does with its turn.
//!
//! **This is a placeholder and should read as one.** It closes the distance and
//! swings; that is the whole repertoire. It exists so a turn visibly passes and so
//! the colleague testing terrain has something that walks over it, not because it is
//! the enemy behaviour the game wants.
//!
//! Real behaviour needs things that do not exist yet — lattices to decide what an
//! enemy *can* cast, hidden information to decide what it knows, and a rout threshold
//! to decide when it stops. All three are in the design and none are built.
//!
//! # One thing per turn
//!
//! An enemy either moves or attacks, then ends its turn. Not "move and attack", even
//! though a player gets both, because a placeholder that spends a full turn's economy
//! invites being tuned rather than replaced.
//!
//! # An emitter, not an actor
//!
//! The AI decides; it does not do. Its whole output is commands pushed into the
//! [`CommandQueue`] — a move or a strike, then the end of its turn — and the one
//! applier validates and executes them exactly as it would a player's. That is
//! not ceremony: it is what makes an enemy turn replayable from the same log as
//! everything else, and what stops "the AI cheats" from ever being a bug class.

use bevy::prelude::*;

use hex_assets::SubstanceTable;
use hex_core::{
    Busy, CommandQueue, ControlOwner, GameCommand, Headroom, HexSpan, HexTile, IssuedCommand, Mode,
    PausableSystems, SubstanceId, TilePos, Turn, UnitId,
};
use hex_units::{route, Body, Enemy, Faction, Footing, Standing, StandsOn, UnitRegistry};

use crate::turns::TurnOrder;

/// Tiles, as the AI needs them to work out where it can walk.
type TileQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static TilePos,
        &'static HexSpan,
        &'static SubstanceId,
        &'static Headroom,
    ),
    With<HexTile>,
>;

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        take_enemy_turn
            .in_set(crate::CombatSystems::Act)
            .in_set(PausableSystems)
            .run_if(in_state(Mode::Combat)),
    );
}

/// Emits a move toward the nearest enemy, or a strike if already next to one.
///
/// Runs only for the unit whose turn it is, and only while that unit is not
/// [`Busy`] — a second decision taken mid-presentation would queue commands on
/// top of the ones still playing out. The `acted` flag the applier sets is
/// what stops it deciding twice.
fn take_enemy_turn(
    turn_order: Res<TurnOrder>,
    registry: Res<UnitRegistry>,
    mut queue: ResMut<CommandQueue>,
    acting: Query<
        (
            &Turn,
            &StandsOn,
            &Body,
            &Faction,
            Option<&UnitId>,
            Option<&ControlOwner>,
        ),
        (With<Enemy>, Without<Busy>),
    >,
    others: Query<(Entity, Option<&UnitId>, &Faction, &StandsOn)>,
    tiles: TileQuery,
    table: Option<Res<SubstanceTable>>,
) {
    let Some(table) = table else {
        return;
    };
    let Some(current) = turn_order
        .current()
        .and_then(|unit| registry.entity_of(unit))
    else {
        return;
    };
    let Ok((turn, standing, body, faction, unit, owner)) = acting.get(current) else {
        // Not an enemy's turn, or it is still mid-presentation.
        return;
    };
    if turn.acted {
        return;
    }
    let Some(my_id) = unit.copied() else {
        // `begin_combat` deals ids before any turn is taken, so this is a
        // wiring bug — and it must be loud, or the fight stalls silently on a
        // unit that can never issue its own end-turn.
        warn!("enemy {current:?} holds the turn but carries no UnitId; its turn cannot be taken");
        return;
    };
    let seat = owner.copied().unwrap_or_default().0;

    let footing = Footing::from_tiles(tiles.iter(), &table, *body);
    let plan = best_foe(&others, *faction, standing.0, &footing, turn.movement_left);

    match plan.map(|plan| plan.action) {
        Some(FoeAction::Attack(target)) => {
            queue.push(IssuedCommand {
                seat,
                command: GameCommand::Strike {
                    unit: my_id,
                    target,
                },
            });
        }
        Some(FoeAction::Move(approach)) => {
            queue.push(IssuedCommand {
                seat,
                command: GameCommand::MoveAlong {
                    unit: my_id,
                    path: approach.steps.iter().map(|step| step.pos).collect(),
                },
            });
        }
        // Nothing to fight, no way to reach it, or a target no spawn path
        // identified. Ending the turn regardless keeps the order moving
        // rather than stalling on a unit with nothing it can do.
        Some(FoeAction::Wait) | None => {}
    }
    queue.push(IssuedCommand {
        seat,
        command: GameCommand::EndTurn { unit: my_id },
    });
}

/// What the enemy can do about one foe this turn.
enum FoeAction {
    /// Already within melee reach of this identified target.
    ///
    /// Carries the target's stable id because a strike is issued as a command,
    /// and commands name units, never entities.
    Attack(UnitId),
    /// A terrain route exists and this is the affordable prefix of it.
    Move(Approach),
    /// No terrain route reaches this foe — or it is in reach but no spawn
    /// path identified it, so no command can name it.
    Wait,
}

/// One candidate target and the action available against it.
struct FoePlan {
    /// `None` for a unit no spawn path identified; such a target sorts last
    /// rather than becoming invisible (symmetric with `MovementCrossings`).
    unit: Option<UnitId>,
    target: Standing,
    action: FoeAction,
}

impl FoePlan {
    /// Deterministic target priority: attack, routable approach, unreachable.
    ///
    /// Route cost decides between two approachable foes, horizontal distance is
    /// a stable secondary signal, and the stable [`UnitId`] resolves exact
    /// ties. Query iteration order is deliberately absent from the decision —
    /// and so are entity bits, which are not stable across runs or saves.
    fn priority(&self, from: Standing) -> (u8, usize, u32, bool, Option<UnitId>) {
        let (kind, route_cost) = match &self.action {
            FoeAction::Attack(_) => (0, 0),
            FoeAction::Move(approach) => (1, approach.route_cost),
            FoeAction::Wait => (2, usize::MAX),
        };
        (
            kind,
            route_cost,
            from.pos.coord.distance(self.target.pos.coord),
            // `is_none` first so an unidentified unit genuinely sorts last.
            self.unit.is_none(),
            self.unit,
        )
    }
}

/// The hostile unit that offers the best action from this terrain position.
///
/// Horizontal nearness is not routability on stacked terrain: a target on a bridge
/// directly overhead may be impossible to approach while another target two hexes
/// away has open ground all the way to it. Every candidate is planned before it is
/// ranked so the unreachable one cannot consume the turn merely by looking nearer on
/// the map.
fn best_foe(
    others: &Query<(Entity, Option<&UnitId>, &Faction, &StandsOn)>,
    faction: Faction,
    from: Standing,
    footing: &Footing,
    budget: u32,
) -> Option<FoePlan> {
    others
        .iter()
        .filter(|(_, _, other, _)| faction.is_hostile_to(**other))
        .map(|(_, unit, _, standing)| {
            let unit = unit.copied();
            let target = standing.0;
            let in_melee = footing.admits_step(from.pos, target.pos)
                && footing.admits_step(target.pos, from.pos);
            // **Reach, not range.** Melee gets no high-ground bonus: an attacker
            // five levels up must not acquire a two-hex punch.
            let action = match (in_melee, unit) {
                (true, Some(target_id)) => FoeAction::Attack(target_id),
                (true, None) => FoeAction::Wait,
                (false, _) => {
                    approach(from, target, footing, budget).map_or(FoeAction::Wait, FoeAction::Move)
                }
            };
            FoePlan {
                unit,
                target,
                action,
            }
        })
        .min_by_key(|plan| plan.priority(from))
}

/// A full route's tactical distance and the prefix affordable this turn.
struct Approach {
    steps: Vec<Standing>,
    route_cost: usize,
}

/// The steps to take toward `target`, stopping adjacent to it and within `budget`.
///
/// [`None`] when there is nowhere to go — no route, already adjacent, or no movement
/// left. `route` searches the whole standable graph, so an enemy behind a wall walks
/// around it rather than standing there, and the clamp to `budget` below is the
/// ordinary case rather than the rare one: closing a long distance simply takes
/// several turns.
fn approach(from: Standing, target: Standing, footing: &Footing, budget: u32) -> Option<Approach> {
    if budget == 0 {
        return None;
    }
    let full = route(from, target, footing)?;

    // `full` runs from where we stand to the target's own surface. Stopping one short
    // leaves the attacker adjacent, which is where it wants to be anyway.
    let adjacent_index = full.len().checked_sub(2)?;
    let reachable = adjacent_index.min(budget as usize);
    if reachable == 0 {
        return None;
    }
    full.get(..=reachable).map(|steps| Approach {
        steps: steps.to_vec(),
        route_cost: adjacent_index,
    })
}
