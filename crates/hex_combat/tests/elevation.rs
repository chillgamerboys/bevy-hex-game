//! What combat does when the ground is not flat.
//!
//! Every other fixture in this crate is a single plane: `enemy.rs` spawns one run per
//! coordinate all at level 1, and `loop.rs` spawns no terrain at all and teleports
//! units by writing `StandsOn` directly. So until this file existed, **`hex_combat`
//! had never run against terrain with a second surface in it** — and two rules that
//! discard the vertical axis passed every check.
//!
//! Both were found by review rather than by a test, which is the argument for this
//! fixture existing at all:
//!
//! - engagement compared coordinates, so a unit on a bridge and one on the ground
//!   beneath it were **zero hexes apart** and always in a fight;
//! - melee compared coordinates, so the one on the bridge could swing at the one
//!   underneath.
//!
//! The showcase map added in #47 has exactly this shape — a bridge over a river — so
//! it stopped being hypothetical the moment that map landed.
//!
//! # The rule being checked
//!
//! Height is an **advantage, not a separation**. A caster gains range for standing
//! above its target and the target gains nothing back; melee gets no such bonus and
//! keeps the one-level step rule. Those are deliberately different rules, and the
//! tests below are mostly about telling them apart.

use bevy::app::PluginsState;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;

use hex_anim::Transformation;
use hex_assets::{PlayerSettings, Substance, SubstanceFile, SubstanceTable};
use hex_combat::Initiative;
use hex_core::{
    Headroom, HexCoord, HexSpan, HexTile, Level, Mode, Screen, SubstanceId, TilePos, Turn,
    MAX_HEADROOM,
};
use hex_units::{Body, Faction, Standing, StandsOn};

/// World height of one level in this fixture.
const LEVEL_HEIGHT: f32 = 1.0;

/// The ground everything is built on.
const GROUND_LEVEL: Level = 1;

/// The bridge deck, high enough above the ground that no step rule connects them.
const DECK_LEVEL: Level = 9;

const STONE: SubstanceId = SubstanceId(1);

/// Where the deck runs: a line of coordinates carrying a high surface as well as ground.
///
/// It covers the origin, so two units can share a coordinate and differ only in height,
/// and it runs six hexes out, so one can also stand *beyond base engage range* and still
/// be up there. Both cases are needed: the first is the one a coordinate-only metric
/// reports as zero, the second is the only one where the high-ground bonus changes an
/// answer.
fn bridged(coord: HexCoord) -> bool {
    coord.y() == 0 && coord.x().abs() <= 6
}

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, bevy::input::InputPlugin));
    app.init_state::<Screen>();
    app.add_sub_state::<Mode>();
    app.insert_resource(substance_table());
    app.insert_resource(PlayerSettings {
        scale: 0.25,
        speed: 5.0,
        color: (1.0, 0.2, 0.2),
        levels_tall: 2,
    });
    app.add_systems(OnEnter(Screen::Gameplay), spawn_terrain);
    app.add_plugins(hex_combat::plugin);

    while app.plugins_state() != PluginsState::Cleaned {
        app.finish();
        app.cleanup();
    }
    app
}

/// Ground everywhere, plus a bridge deck over one line of it.
///
/// The deck is eight levels up — far beyond `MAX_STEP`, so nothing can walk between
/// the two surfaces and any connection a test observes is a bug rather than a route.
fn spawn_terrain(mut commands: Commands) {
    for coord in HexCoord::ORIGIN.within_radius(10) {
        commands.spawn((
            HexTile,
            coord,
            TilePos::new(coord, GROUND_LEVEL),
            span_at(GROUND_LEVEL),
            STONE,
            Headroom(MAX_HEADROOM),
        ));
        if bridged(coord) {
            commands.spawn((
                HexTile,
                coord,
                TilePos::new(coord, DECK_LEVEL),
                span_at(DECK_LEVEL),
                STONE,
                Headroom(MAX_HEADROOM),
            ));
        }
    }
}

/// The rendered extent of a one-level run whose surface is at `level`.
fn span_at(level: Level) -> HexSpan {
    #[expect(
        clippy::cast_precision_loss,
        reason = "test levels are single digits, exact in f32"
    )]
    let top = level as f32 * LEVEL_HEIGHT;
    HexSpan::new(top - LEVEL_HEIGHT, top)
}

fn substance_table() -> SubstanceTable {
    let mut substances = bevy::platform::collections::HashMap::default();
    substances.insert(
        "air".to_owned(),
        Substance {
            color: (0.0, 0.0, 0.0),
            solid: false,
            diggable: false,
        },
    );
    substances.insert(
        "stone".to_owned(),
        Substance {
            color: (0.5, 0.5, 0.5),
            solid: true,
            diggable: true,
        },
    );
    SubstanceTable::from_file(&SubstanceFile { substances })
}

fn spawn_unit(app: &mut App, faction: Faction, coord: HexCoord, level: Level) -> Entity {
    let standing = Standing {
        pos: TilePos::new(coord, level),
        span: span_at(level),
    };
    let mut unit = app.world_mut().spawn((
        faction,
        StandsOn(standing),
        Body { levels_tall: 2 },
        Initiative(10),
        Transform::from_translation(standing.world_position()),
    ));
    if faction == Faction::Hostile {
        unit.insert(hex_units::Enemy);
    } else {
        unit.insert(hex_units::Player);
    }
    unit.id()
}

fn enter_gameplay(app: &mut App) {
    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Gameplay);
    app.update();
    app.update();
    app.update();
}

/// An app with both units already standing, then run into gameplay.
///
/// **Units before `enter_gameplay`, not after.** `engagement` is gated on
/// `in_state(Screen::Gameplay)`, so units introduced afterwards need another frame
/// before anything looks at them — and a test that forgets it does not fail loudly, it
/// quietly observes `Mode::Exploring` and passes any assertion phrased as "did not
/// happen". Two tests here did exactly that before this helper existed.
fn fight(player_level: Level, enemy: HexCoord, enemy_level: Level) -> (App, Entity, Entity) {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, player_level);
    let hostile = spawn_unit(&mut app, Faction::Hostile, enemy, enemy_level);
    enter_gameplay(&mut app);
    (app, player, hostile)
}

fn mode(app: &mut App) -> Mode {
    *app.world().resource::<State<Mode>>().get()
}

/// Two units at one coordinate, eight levels apart, are **in** a fight.
///
/// This one characterises rather than discriminates: the old coordinate-only rule said
/// the same thing. It is here because the review asked for the opposite and the answer
/// is deliberately "no" — without it written down, a later reader has every reason to
/// think the stack collapse was simply missed.
///
/// Deliberately the opposite of what a "fix the collapsed stack" instinct produces.
/// Horizontal separation really is zero, and someone directly overhead can act on you
/// — that is the high ground working, not a stack being flattened by accident. The
/// stacking rule governs *movement*; it does not make people invisible to each other.
#[test]
fn a_unit_overhead_is_still_a_fight() {
    let (mut app, _, _) = fight(GROUND_LEVEL, HexCoord::ORIGIN, DECK_LEVEL);

    assert_eq!(mode(&mut app), Mode::Combat);
}

/// But it does not let the one above **hit** the one below.
///
/// The distinction the whole module exists for: the enemy on the deck is in the fight,
/// takes its turn, and must not land a blow through eight levels of air. Melee is
/// reach, and reach is the step rule.
#[test]
fn height_does_not_lengthen_a_punch() {
    // **Adjacent, not identical.** With both units on the same coordinate the old
    // coordinate-only rule did not swing either — `distance` is 0, and it tested for
    // exactly 1 — so a test phrased that way passes against the bug it is meant to
    // catch. One hex across and eight levels up is the case that separates them.
    let (mut app, player, _) = fight(GROUND_LEVEL, HexCoord::new_cubic(1, 0, -1), DECK_LEVEL);
    assert_eq!(
        mode(&mut app),
        Mode::Combat,
        "setup failed — no fight, so nothing was ever swung"
    );
    app.update();
    app.update();

    // An attack is an animation on the *target* — the recoil. Its absence is what
    // "was not hit" looks like from outside, since nothing deals damage yet.
    assert!(
        app.world().get::<Transformation>(player).is_none(),
        "the enemy swung at somebody eight levels below it"
    );
}

/// Standing above someone lengthens what a *ranged* thing can do, and only downhill.
///
/// Checked directly against the targeting rule rather than through combat, because the
/// asymmetry is the mechanic and a fight only reports whether *anyone* is in range. A
/// symmetric metric passes a test that looks one way.
#[test]
fn height_lengthens_range_downhill_only() {
    let deck = TilePos::new(HexCoord::new_cubic(0, 0, 0), DECK_LEVEL);
    let ground = TilePos::new(HexCoord::new_cubic(5, -5, 0), GROUND_LEVEL);

    assert!(
        hex_units::in_reach(deck, ground, 4),
        "eight levels up should buy the fifth hex"
    );
    assert!(
        !hex_units::in_reach(ground, deck, 4),
        "the unit below should gain nothing from the same gap"
    );
}

/// Flat ground behaves exactly as it did before any of this.
///
/// The safety property. Every threshold in `CombatSettings` was tuned on a plane, and
/// a change to how distance is measured has to leave those numbers meaning what they
/// meant or it is not a fix, it is a rebalance nobody asked for.
#[test]
fn flat_ground_engages_at_the_same_range_as_ever() {
    let (mut near, _, _) = fight(GROUND_LEVEL, HexCoord::new_cubic(4, -4, 0), GROUND_LEVEL);
    assert_eq!(mode(&mut near), Mode::Combat, "four hexes should engage");

    let (mut far, _, _) = fight(GROUND_LEVEL, HexCoord::new_cubic(5, -5, 0), GROUND_LEVEL);
    assert_eq!(mode(&mut far), Mode::Exploring, "five hexes should not");
}

/// Standing high starts a fight from further off than standing level does.
///
/// The only behaviour the new engagement rule actually changes, and therefore the only
/// world-level test that fails against the old one. Five hexes is outside
/// `ENGAGE_RANGE`; eight levels of height buys the fifth hex and the fight starts.
#[test]
fn height_engages_from_further_away() {
    let high = HexCoord::new_cubic(5, 0, -5);

    let (mut up, _, _) = fight(GROUND_LEVEL, high, DECK_LEVEL);
    assert_eq!(
        mode(&mut up),
        Mode::Combat,
        "height should have bought the extra hex"
    );

    let (mut level, _, _) = fight(GROUND_LEVEL, high, GROUND_LEVEL);
    assert_eq!(
        mode(&mut level),
        Mode::Exploring,
        "the same five hexes on the flat should not engage"
    );
}

/// An enemy that cannot reach its target does not stall the turn order.
///
/// The stalemate named in `GAMEPLAY_LOOP.md`: a melee-only unit separated by terrain it
/// cannot cross has nothing useful to do. It must still **end its turn**, or the fight
/// hangs and the player cannot even walk away from it.
#[test]
fn an_unreachable_target_does_not_hang_the_fight() {
    let (mut app, _, enemy) = fight(GROUND_LEVEL, HexCoord::ORIGIN, DECK_LEVEL);
    assert_eq!(
        mode(&mut app),
        Mode::Combat,
        "setup failed — no fight to hang"
    );
    app.update();
    app.update();

    let held = app.world().get::<Turn>(enemy);
    assert!(
        held.is_none_or(|turn| turn.acted),
        "an enemy with nothing it can do must still finish its turn"
    );
}
