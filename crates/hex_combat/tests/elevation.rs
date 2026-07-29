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

use std::collections::{BTreeMap, BTreeSet};

use bevy::app::PluginsState;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;

use hex_anim::Transformation;
use hex_assets::{
    ArtPalette, PaletteSwatch, PlayerSettings, SrgbColor, Substance, SubstanceFile, SubstanceTable,
    SwatchId,
};
use hex_combat::Initiative;
use hex_core::{
    GameCommand, Headroom, HexCoord, HexSpan, HexTile, Level, LightDomain, Mode, Screen,
    SubstanceId, TilePos, TraversalProfile, Turn, UnitId, MAX_HEADROOM,
};
use hex_perception::{
    apply_observations, FactionMapKnowledge, FactionObservation, FactionObservations, ObservedUnit,
    SurfaceSnapshot, SurfaceSnapshots,
};
use hex_units::{Body, Downed, Faction, Standing, StandsOn, UnitAllocator, UnitRegistry};

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
    // The shipped combat.ron values; production loads the file instead.
    app.insert_resource(hex_assets::CombatSettings::default());
    app.add_sub_state::<Mode>();
    app.insert_resource(substance_table());
    app.insert_resource(PlayerSettings {
        scale: 0.25,
        speed: 5.0,
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
/// The deck is eight levels up — far beyond the ordinary walker profile, so nothing
/// can walk between the two surfaces and any connection a test observes is a bug
/// rather than a route.
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

#[expect(
    clippy::expect_used,
    reason = "invalid compile-time fixture data should fail the test immediately"
)]
fn substance_table() -> SubstanceTable {
    let stone_id = SwatchId::new("terrain/stone").expect("the fixture swatch id should be valid");
    let stone = PaletteSwatch::new(
        "Stone",
        SrgbColor::new(0.5, 0.5, 0.5).expect("the fixture color should be valid"),
        BTreeSet::from(["test".to_owned()]),
    )
    .expect("the fixture swatch should be valid");
    let palette = ArtPalette::new(BTreeMap::from([(stone_id.clone(), stone)]))
        .expect("the fixture palette should be valid");
    let mut substances = bevy::platform::collections::HashMap::default();
    substances.insert("air".to_owned(), Substance::invisible(false, false));
    substances.insert(
        "stone".to_owned(),
        Substance::from_swatch(stone_id, true, true),
    );
    SubstanceTable::from_file(&SubstanceFile { substances }, &palette)
        .expect("the fixture substance should resolve through its palette")
}

fn spawn_unit(app: &mut App, faction: Faction, coord: HexCoord, level: Level) -> Entity {
    spawn_unit_with_profile(app, faction, coord, level, TraversalProfile::WALKER)
}

fn spawn_unit_with_profile(
    app: &mut App,
    faction: Faction,
    coord: HexCoord,
    level: Level,
    profile: TraversalProfile,
) -> Entity {
    let id = app.world_mut().resource_mut::<UnitAllocator>().allocate();
    let standing = Standing {
        pos: TilePos::new(coord, level),
        span: span_at(level),
    };
    let mut unit = app.world_mut().spawn((
        faction,
        id,
        StandsOn(standing),
        Body::new(profile),
        Initiative(10),
        Transform::from_translation(standing.world_position()),
    ));
    if faction == Faction::Hostile {
        unit.insert(hex_units::Enemy);
    } else {
        unit.insert(hex_units::Player);
    }
    let entity = unit.id();
    app.world_mut()
        .resource_mut::<UnitRegistry>()
        .register(id, entity);
    entity
}

fn spawn_surface(app: &mut App, coord: HexCoord, level: Level, headroom: Level) {
    app.world_mut().spawn((
        HexTile,
        coord,
        TilePos::new(coord, level),
        span_at(level),
        STONE,
        Headroom(headroom),
    ));
}

fn enter_gameplay(app: &mut App) {
    enter_gameplay_with_hidden_unit(app, None);
}

fn enter_gameplay_with_hidden_unit(app: &mut App, hidden: Option<Entity>) {
    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Gameplay);
    app.update();
    publish_fixture_knowledge(app, hidden);
    app.update();
    app.update();
}

#[expect(
    clippy::expect_used,
    reason = "invalid deterministic fixture projections should fail at their construction seam"
)]
fn publish_fixture_knowledge(app: &mut App, hidden: Option<Entity>) {
    let surfaces = {
        let world = app.world_mut();
        let mut tiles =
            world.query_filtered::<(&TilePos, &HexSpan, &SubstanceId, &Headroom), With<HexTile>>();
        SurfaceSnapshots::try_from_iter(tiles.iter(world).map(
            |(&pos, &span, &substance, &headroom)| SurfaceSnapshot {
                pos,
                span,
                substance,
                headroom,
                is_solid: true,
                blocked: false,
                domain: LightDomain::Exterior,
            },
        ))
        .expect("the fixture should publish unique terrain surfaces")
    };
    let units = {
        let world = app.world_mut();
        let mut query = world.query::<(Entity, &UnitId, &Faction, &StandsOn)>();
        query
            .iter(world)
            .filter(|(entity, ..)| Some(*entity) != hidden)
            .map(|(_, &id, &faction, standing)| ObservedUnit {
                id,
                faction,
                pos: standing.0.pos,
                provides_sight: true,
            })
            .collect::<Vec<_>>()
    };
    let mut observation = FactionObservation::new();
    for (position, _) in surfaces.iter() {
        observation.insert_surface(position);
    }
    for unit in units {
        observation
            .try_insert_unit(unit)
            .expect("fixture unit identities should be unique");
    }
    let observations = FactionObservations::from_factions(observation.clone(), observation);
    let mut knowledge = FactionMapKnowledge::new();
    apply_observations(&mut knowledge, &surfaces, &observations);
    app.insert_resource(knowledge);
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

/// A profile that may descend farther than it climbs still has symmetric melee reach.
///
/// Checking only attacker-to-target movement would let this enemy punch eight levels
/// down while the same two surfaces fail in the other direction. Melee is a shared
/// boundary between the units, not a directional movement benefit.
#[test]
fn asymmetric_drop_does_not_grant_downhill_melee() {
    let mut app = test_app();
    let player_coord = HexCoord::new_cubic(1, 0, -1);
    let player = spawn_unit(&mut app, Faction::Player, player_coord, GROUND_LEVEL);
    let hostile = spawn_unit_with_profile(
        &mut app,
        Faction::Hostile,
        HexCoord::ORIGIN,
        DECK_LEVEL,
        TraversalProfile {
            levels_tall: 2,
            max_climb: 1,
            max_drop: DECK_LEVEL - GROUND_LEVEL,
        },
    );
    app.world_mut().entity_mut(hostile).insert(Initiative(20));
    enter_gameplay(&mut app);
    assert_eq!(
        mode(&mut app),
        Mode::Combat,
        "setup failed — no fight, so melee was never evaluated"
    );
    app.update();
    app.update();

    assert!(
        app.world().get::<Transformation>(player).is_none(),
        "a long-drop profile granted a one-sided downhill melee attack"
    );
}

/// Melee uses the same complete transition boundary as walking. Each unit can fit at
/// its endpoint here, but the one-level ramp leaves only one shared clear level beneath
/// the lower ceiling, so neither can swing through the lintel.
#[test]
fn low_lintel_does_not_admit_melee_between_standable_rooms() {
    let mut app = test_app();
    let low_coord = HexCoord::new_cubic(7, -7, 0);
    let high_coord = HexCoord::new_cubic(8, -8, 0);
    let low_level = 4;
    let high_level = 5;
    spawn_surface(&mut app, low_coord, low_level, 2);
    spawn_surface(&mut app, high_coord, high_level, 2);

    let player = spawn_unit(&mut app, Faction::Player, low_coord, low_level);
    let hostile = spawn_unit(&mut app, Faction::Hostile, high_coord, high_level);
    app.world_mut().entity_mut(hostile).insert(Initiative(20));
    enter_gameplay(&mut app);
    assert_eq!(
        mode(&mut app),
        Mode::Combat,
        "adjacent units should enter combat before melee is evaluated"
    );
    app.update();
    app.update();

    assert!(
        app.world().get::<Transformation>(player).is_none(),
        "the enemy attacked through a one-level lateral aperture"
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
        hex_units::in_reach(deck, ground, 4, 5),
        "eight levels up should buy the fifth hex"
    );
    assert!(
        !hex_units::in_reach(ground, deck, 4, 5),
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
/// world-level test that fails against the old one. Five hexes is outside the shipped
/// `engage_range`; eight levels of height buys the fifth hex and the fight starts.
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
/// The stalemate named in `docs/planning/status.md`: a melee-only unit separated by terrain it
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

#[test]
fn hidden_hostile_truth_never_enters_ai_observation_or_legal_actions() {
    let mut app = test_app();
    let enemy = spawn_unit(&mut app, Faction::Hostile, HexCoord::ORIGIN, GROUND_LEVEL);
    app.world_mut().entity_mut(enemy).insert(Initiative(20));
    let hidden = spawn_unit(
        &mut app,
        Faction::Player,
        HexCoord::new_cubic(1, 0, -1),
        GROUND_LEVEL,
    );
    spawn_unit(
        &mut app,
        Faction::Player,
        HexCoord::new_cubic(3, 0, -3),
        GROUND_LEVEL,
    );
    let hidden_id = *app
        .world()
        .get::<UnitId>(hidden)
        .expect("fixture units carry stable ids");

    enter_gameplay_with_hidden_unit(&mut app, Some(hidden));

    let traces = app.world().resource::<hex_combat::AiDecisionTraces>();
    let trace = traces
        .entries
        .first()
        .expect("the hostile should make one decision");
    assert!(trace
        .observation
        .hostiles
        .iter()
        .all(|hostile| hostile.unit != hidden_id));
    assert!(!trace.observation.turn_order.contains(&hidden_id));
    assert!(trace.legal_actions.actions().iter().all(|action| {
        !matches!(
            action.command,
            GameCommand::Strike { target, .. } if target == hidden_id
        )
    }));
}

#[test]
fn observed_downed_hostile_is_not_an_offensive_goal() {
    let mut app = test_app();
    let enemy = spawn_unit(&mut app, Faction::Hostile, HexCoord::ORIGIN, GROUND_LEVEL);
    app.world_mut().entity_mut(enemy).insert(Initiative(20));
    let downed = spawn_unit(
        &mut app,
        Faction::Player,
        HexCoord::new_cubic(1, 0, -1),
        GROUND_LEVEL,
    );
    app.world_mut().entity_mut(downed).insert(Downed);
    spawn_unit(
        &mut app,
        Faction::Player,
        HexCoord::new_cubic(3, 0, -3),
        GROUND_LEVEL,
    );
    let downed_id = *app
        .world()
        .get::<UnitId>(downed)
        .expect("fixture units carry stable ids");

    enter_gameplay(&mut app);

    let traces = app.world().resource::<hex_combat::AiDecisionTraces>();
    let trace = traces
        .entries
        .first()
        .expect("the hostile should make one decision");
    assert!(trace
        .observation
        .hostiles
        .iter()
        .any(|hostile| hostile.unit == downed_id && hostile.downed));
    assert!(trace.legal_actions.actions().iter().all(|action| {
        !matches!(
            action.command,
            GameCommand::Strike { target, .. } if target == downed_id
        )
    }));
    assert!(
        !matches!(
            trace.command,
            Some(GameCommand::Strike { target, .. }) if target == downed_id
        ),
        "the baseline must ignore a downed offensive target"
    );
}

/// A nearer coordinate is not a better target when no terrain route reaches its
/// surface.
///
/// The deck target is only one hex away horizontally but eight levels above the
/// enemy. A second player is three hexes away on connected ground. Ranking by map
/// distance alone burns the turn staring at the deck; ranking planned routes moves
/// toward the ground target.
#[test]
fn a_routable_foe_beats_a_nearer_unreachable_one() {
    let mut app = test_app();
    let enemy = spawn_unit(&mut app, Faction::Hostile, HexCoord::ORIGIN, GROUND_LEVEL);
    app.world_mut().entity_mut(enemy).insert(Initiative(20));
    spawn_unit(
        &mut app,
        Faction::Player,
        HexCoord::new_cubic(1, 0, -1),
        DECK_LEVEL,
    );
    let reachable = HexCoord::new_cubic(3, 0, -3);
    spawn_unit(&mut app, Faction::Player, reachable, GROUND_LEVEL);
    enter_gameplay(&mut app);

    let moving = app
        .world()
        .get::<hex_units::MovingTo>(enemy)
        .expect("the enemy should approach the foe connected by ground");
    let destination = moving
        .path
        .last()
        .expect("an approach contains its starting surface and at least one step")
        .pos;

    assert_eq!(destination.level, GROUND_LEVEL);
    assert_eq!(
        destination.coord.distance(reachable),
        1,
        "the enemy did not stop adjacent to the routable target"
    );
}
