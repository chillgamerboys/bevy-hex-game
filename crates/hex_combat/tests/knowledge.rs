//! Integration tests for the knowledge and divination seam.
//!
//! The store's own rules are unit-tested beside it. What these prove is the
//! wiring: that the publishing systems actually run, that decay is ordered
//! against the round rollover rather than left to luck, and that the dev toggle
//! reaches [`FactionLatticeKnowledge::view`].
//!
//! **These tests attach `LatticeSpec` and `LatticeState` to units by hand.**
//! Shipped units receive those components from content, but these fixtures stay
//! deliberately small so the knowledge seam is tested independently of scenario
//! loading and the gameplay readout.

use std::collections::BTreeMap;

use bevy::app::PluginsState;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;

use hex_combat::{FactionLatticeKnowledge, Initiative, KnownCell, RevealAll, TurnOrder};
use hex_core::{
    CommandQueue, ElementId, GameCommand, Headroom, HexCoord, HexSpan, IssuedCommand,
    KnowledgeExpiry, KnowledgeSource, LatticeCoord, LightDomain, Mode, PlayerSeat, Screen,
    SubstanceId, TilePos, UnitId,
};
use hex_lattice::{CellKind, LatticeSpec, LatticeState, LatticeStats};
use hex_perception::{
    apply_observations, FactionMapKnowledge, FactionObservation, FactionObservations, ObservedUnit,
    SurfaceSnapshot, SurfaceSnapshots,
};
use hex_units::{Faction, Standing, StandsOn};

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, bevy::input::InputPlugin));
    app.init_state::<Screen>();
    app.insert_resource(hex_assets::CombatSettings::default());
    app.add_sub_state::<Mode>();
    app.add_plugins(hex_combat::plugin);

    while app.plugins_state() != PluginsState::Cleaned {
        app.finish();
        app.cleanup();
    }
    app
}

/// A three-cell lattice: two gems and a blank.
fn spec() -> LatticeSpec {
    LatticeSpec::default()
        .with(
            LatticeCoord::ORIGIN,
            CellKind::Gem {
                element: ElementId(0),
            },
        )
        .with(
            LatticeCoord::new(1, 0),
            CellKind::Gem {
                element: ElementId(1),
            },
        )
        .with(LatticeCoord::new(0, 1), CellKind::Blank)
}

/// Element 0 holds five mana a gem; element 1 is unattuned and holds none.
fn stats() -> LatticeStats {
    LatticeStats::new(BTreeMap::from([(ElementId(0), 5)]), BTreeMap::new())
}

/// A unit carrying only what `hex_combat` reads, optionally with a lattice.
fn spawn_unit(
    app: &mut App,
    faction: Faction,
    coord: HexCoord,
    initiative: u32,
    lattice: bool,
) -> Entity {
    let mut unit = app.world_mut().spawn((
        faction,
        StandsOn(Standing {
            pos: TilePos::new(coord, 1),
            span: HexSpan::new(0.0, 1.0),
        }),
        Initiative(initiative),
    ));
    if lattice {
        let spec = spec();
        let state = LatticeState::new(&spec, &stats());
        unit.insert((spec, state));
    }
    unit.id()
}

fn enter_gameplay(app: &mut App) {
    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Gameplay);
    app.update();
    app.update();
    publish_spatial_knowledge(app, None);
    app.update();
}

/// Publishes an explicit world-owned observation snapshot for the combat adapter.
///
/// `player_visible = None` means every unit is visible; `Some(ids)` narrows only
/// the player's view. The hostile fixture continues to observe every unit because
/// these tests exercise player-facing lattice knowledge.
#[expect(
    clippy::expect_used,
    reason = "duplicate test identities or surfaces invalidate the fixture"
)]
fn publish_spatial_knowledge(app: &mut App, player_visible: Option<&[UnitId]>) {
    let rows: Vec<(UnitId, Faction, TilePos, HexSpan)> = {
        let world = app.world_mut();
        let mut query = world.query::<(&UnitId, &Faction, &StandsOn)>();
        query
            .iter(world)
            .map(|(id, faction, standing)| (*id, *faction, standing.0.pos, standing.0.span))
            .collect()
    };
    let current =
        SurfaceSnapshots::try_from_iter(rows.iter().map(|&(_, _, pos, span)| SurfaceSnapshot {
            pos,
            span,
            substance: SubstanceId(0),
            headroom: Headroom(2),
            is_solid: true,
            blocked: false,
            domain: LightDomain::Exterior,
        }))
        .expect("test units occupy unique surfaces");

    let observation = |visible: Option<&[UnitId]>| {
        let mut observation = FactionObservation::new();
        for &(id, faction, pos, _) in &rows {
            if visible.is_none_or(|ids| ids.contains(&id)) {
                observation.insert_surface(pos);
                observation
                    .try_insert_unit(ObservedUnit { id, faction, pos })
                    .expect("test unit ids are unique");
            }
        }
        observation
    };
    let observations =
        FactionObservations::from_factions(observation(player_visible), observation(None));
    let mut spatial = FactionMapKnowledge::new();
    apply_observations(&mut spatial, &current, &observations);
    app.insert_resource(spatial);
}

#[expect(
    clippy::expect_used,
    reason = "test helper outside a #[test] fn; a missing id IS the failure"
)]
fn unit_id(app: &App, entity: Entity) -> UnitId {
    *app.world()
        .entity(entity)
        .get::<UnitId>()
        .expect("combat should have dealt this unit a stable id")
}

/// Advances the simulation without coupling knowledge tests to an input binding.
#[expect(
    clippy::expect_used,
    reason = "an active combatant is a precondition for this integration-test helper"
)]
fn end_turn(app: &mut App) {
    let unit = app
        .world()
        .resource::<TurnOrder>()
        .current()
        .expect("combat should have a current unit");
    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat::default(),
            command: GameCommand::EndTurn { unit },
        });
    app.update();
}

/// A faction whose own units carry no lattice must still be able to look at one.
#[test]
fn base_visibility_reaches_a_faction_that_owns_no_lattice() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, false);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        true,
    );
    enter_gameplay(&mut app);
    app.update();

    let enemy_id = unit_id(&app, enemy);
    let knowledge = app.world().resource::<FactionLatticeKnowledge>();
    let view = knowledge
        .view(Faction::Player, enemy_id)
        .expect("the player should know a hostile lattice exists");

    assert_eq!(view.base().faction, Faction::Hostile);
    assert_eq!(view.known_capacity(), None, "capacity requires divination");
    assert!(
        view.is_opaque(),
        "seeing a unit must reveal nothing about its lattice contents"
    );
    assert_eq!(view.unknown_count(), None);
}

/// Seeing a unit establishes where it is and nothing else. This is the whole
/// point of the two channels being separate, so it is pinned rather than assumed.
#[test]
fn observation_alone_reveals_no_cell_contents() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, true);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        true,
    );
    enter_gameplay(&mut app);
    for _ in 0..8 {
        app.update();
    }

    let enemy_id = unit_id(&app, enemy);
    let knowledge = app.world().resource::<FactionLatticeKnowledge>();
    let view = knowledge.view(Faction::Player, enemy_id).expect("a view");
    assert_eq!(
        view.revealed_count(),
        0,
        "no amount of looking should reveal a gem"
    );
    assert!(view.cell(LatticeCoord::ORIGIN).is_none());
}

/// Spatial perception owns whether the subject exists to the viewer. Divination
/// facts retain their own lifetime while hidden, but cannot disclose the unit.
#[test]
fn losing_spatial_observation_hides_and_then_restores_unexpired_divination() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, true);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        true,
    );
    enter_gameplay(&mut app);
    let enemy_id = unit_id(&app, enemy);

    assert!(
        app.world_mut()
            .resource_mut::<FactionLatticeKnowledge>()
            .learn(
                Faction::Player,
                enemy_id,
                LatticeCoord::ORIGIN,
                KnownCell {
                    kind: CellKind::Gem {
                        element: ElementId(0),
                    },
                    mana: Some(5),
                    disabled: false,
                    source: KnowledgeSource::Divination,
                    expiry: KnowledgeExpiry::Sustained,
                },
            ),
        "an observed subject accepts divination"
    );

    publish_spatial_knowledge(&mut app, Some(&[]));
    app.update();
    assert!(
        app.world()
            .resource::<FactionLatticeKnowledge>()
            .view(Faction::Player, enemy_id)
            .is_none(),
        "stored lattice facts must not disclose a hidden unit"
    );
    assert!(
        !app.world_mut()
            .resource_mut::<FactionLatticeKnowledge>()
            .learn(
                Faction::Player,
                enemy_id,
                LatticeCoord::new(0, 1),
                KnownCell {
                    kind: CellKind::Blank,
                    mana: None,
                    disabled: false,
                    source: KnowledgeSource::Divination,
                    expiry: KnowledgeExpiry::Sustained,
                },
            ),
        "a hidden subject cannot receive a new targeted reveal"
    );

    publish_spatial_knowledge(&mut app, None);
    app.update();
    assert!(
        app.world()
            .resource::<FactionLatticeKnowledge>()
            .view(Faction::Player, enemy_id)
            .and_then(|known| known.cell(LatticeCoord::ORIGIN))
            .is_some(),
        "unexpired divination becomes readable again after re-observation"
    );
}

#[test]
fn divined_cells_refresh_from_live_truth_without_resetting_expiry() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, true);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        true,
    );
    enter_gameplay(&mut app);
    app.update();
    let enemy_id = unit_id(&app, enemy);

    let accepted = app
        .world_mut()
        .resource_mut::<FactionLatticeKnowledge>()
        .learn(
            Faction::Player,
            enemy_id,
            LatticeCoord::ORIGIN,
            KnownCell {
                kind: CellKind::Blank,
                mana: Some(99),
                disabled: false,
                source: KnowledgeSource::Divination,
                expiry: KnowledgeExpiry::Rounds(1),
            },
        );
    assert!(accepted, "precondition: base visibility exists");
    {
        let mut entity = app.world_mut().entity_mut(enemy);
        let mut state = entity
            .get_mut::<LatticeState>()
            .expect("the enemy has live lattice state");
        hex_lattice::apply_disables(&mut state, &[LatticeCoord::ORIGIN]);
    }
    app.update();

    let refreshed = app
        .world()
        .resource::<FactionLatticeKnowledge>()
        .view(Faction::Player, enemy_id)
        .and_then(|known| known.cell(LatticeCoord::ORIGIN))
        .expect("the divined cell remains known");
    assert_eq!(
        refreshed.kind,
        CellKind::Gem {
            element: ElementId(0),
        }
    );
    assert_eq!(refreshed.mana, Some(5), "mana is current live truth");
    assert!(refreshed.disabled, "disabled state is current live truth");
    assert_eq!(
        refreshed.expiry,
        KnowledgeExpiry::Rounds(1),
        "refreshing values must not extend or spend the reveal"
    );
}

/// The ordering that must not be left to luck: decay reads `RoundElapsed`, which
/// is written inside `CombatSystems::Advance`, so a reveal placed during a round
/// must survive that round and lapse at the rollover.
#[test]
fn a_one_time_reveal_lapses_at_the_round_rollover() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, false);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        true,
    );
    enter_gameplay(&mut app);
    app.update();

    let enemy_id = unit_id(&app, enemy);
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(unit_id(&app, player))
    );

    // A divination lands mid-round.
    let accepted = app
        .world_mut()
        .resource_mut::<FactionLatticeKnowledge>()
        .learn(
            Faction::Player,
            enemy_id,
            LatticeCoord::ORIGIN,
            KnownCell {
                kind: CellKind::Gem {
                    element: ElementId(0),
                },
                mana: Some(5),
                disabled: false,
                source: KnowledgeSource::Divination,
                expiry: KnowledgeExpiry::Rounds(0),
            },
        );
    assert!(
        accepted,
        "base visibility should already have been published"
    );

    // Passing one turn is not a round.
    end_turn(&mut app);
    assert_eq!(app.world().resource::<TurnOrder>().round, 0);
    assert_eq!(
        app.world()
            .resource::<FactionLatticeKnowledge>()
            .view(Faction::Player, enemy_id)
            .expect("a view")
            .revealed_count(),
        1,
        "a reveal must survive the turns inside its own round"
    );

    // Wrapping to the front is.
    end_turn(&mut app);
    assert_eq!(app.world().resource::<TurnOrder>().round, 1);
    let view = app
        .world()
        .resource::<FactionLatticeKnowledge>()
        .view(Faction::Player, enemy_id)
        .expect("a view");
    assert!(view.is_opaque(), "the one-time reveal should have lapsed");
    assert_eq!(view.base().faction, Faction::Hostile);
    assert_eq!(view.known_capacity(), None);
}

/// The dev toggle has to surface the truth through the same accessor the game
/// reads, or a designer is looking at a second path that can drift from it.
#[test]
fn reveal_all_shows_the_truth_through_the_accessor() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, false);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        true,
    );
    enter_gameplay(&mut app);
    app.update();
    let enemy_id = unit_id(&app, enemy);

    assert!(app
        .world()
        .resource::<FactionLatticeKnowledge>()
        .view(Faction::Player, enemy_id)
        .expect("a view")
        .is_opaque());

    *app.world_mut().resource_mut::<RevealAll>() = RevealAll(true);
    app.update();

    let view = app
        .world()
        .resource::<FactionLatticeKnowledge>()
        .view(Faction::Player, enemy_id)
        .expect("a view");
    assert_eq!(view.revealed_count(), 3, "every cell should be exposed");
    assert_eq!(view.known_capacity(), Some(3));
    assert_eq!(view.unknown_count(), Some(0));
    let gem = view.cell(LatticeCoord::ORIGIN).expect("the origin gem");
    assert_eq!(
        gem.mana,
        Some(5),
        "an attuned gem opens full to its capacity"
    );
    assert_eq!(
        view.cell(LatticeCoord::new(1, 0)).map(|cell| cell.mana),
        Some(Some(0)),
        "an unattuned element's gem holds nothing"
    );

    // And turning it off restores the honest answer rather than leaving the
    // revealed cells behind as knowledge the game never earned.
    *app.world_mut().resource_mut::<RevealAll>() = RevealAll(false);
    app.update();
    assert!(app
        .world()
        .resource::<FactionLatticeKnowledge>()
        .view(Faction::Player, enemy_id)
        .expect("a view")
        .is_opaque());
}

/// A new session must not inherit views of units that no longer exist.
#[test]
fn leaving_gameplay_forgets_everything() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, false);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        true,
    );
    enter_gameplay(&mut app);
    app.update();
    assert!(!app.world().resource::<FactionLatticeKnowledge>().is_empty());

    *app.world_mut().resource_mut::<RevealAll>() = RevealAll(true);
    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();

    assert!(
        app.world().resource::<FactionLatticeKnowledge>().is_empty(),
        "knowledge should not survive leaving gameplay"
    );
    assert_eq!(
        *app.world().resource::<RevealAll>(),
        RevealAll(false),
        "the dev toggle should not persist into a new session"
    );
}
