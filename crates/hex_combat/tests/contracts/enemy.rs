//! Contract tests for what an enemy does with its turn.
//!
//! The AI is a placeholder, but "closes the distance and swings" is still a claim
//! that can be wrong in specific ways — walking onto the player, overshooting its
//! movement budget, or never ending its turn and stalling the fight. Those are what
//! these check.
//!
//! Terrain is spawned by the test, because `hex_combat` cannot see `hex_map` and does
//! not need to: it consumes `TilePos`, `HexSpan`, `SubstanceId` and `Headroom`.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use bevy::prelude::*;

use hex_anim::Transformation;
use hex_assets::{ElementCatalog, ElementFile, PlayerSettings};
use hex_combat::{
    AiDecisionTraces, CombatSummary, CombatTranscriptRecorder, EncounterOutcome,
    EncounterResolution, Initiative, TurnOrder, MAX_AI_DECISION_TRACES, MAX_COMBAT_SUMMARY_DETAILS,
};
use hex_core::{
    CommandQueue, ControlOwner, GameCommand, Headroom, HexCoord, HexSpan, HexTile, IssuedCommand,
    LatticeCoord, LightDomain, Mode, PendingDecision, PlayerSeat, Screen, SubstanceId, TilePos,
    Turn, UnitId,
};
use hex_lattice::{CellKind, LatticeSpec, LatticeState, LatticeStats};
use hex_perception::{
    apply_observations, FactionMapKnowledge, FactionObservation, FactionObservations, ObservedUnit,
    SurfaceSnapshot, SurfaceSnapshots,
};
use hex_test_support::{SyntheticArena, TestAppBuilder};
use hex_units::{Body, Faction, HexPathingLine, MovingTo, Standing, StandsOn, UnitAllocator};

const GROUND: f32 = 2.0;
const GROUND_LEVEL: hex_core::Level = 1;
#[expect(
    clippy::expect_used,
    reason = "invalid shared deterministic fixture data must fail during construction"
)]
fn test_app() -> App {
    let mut builder = TestAppBuilder::new()
        .with_fixed_step(Duration::ZERO)
        .with_arena(SyntheticArena::flat_radius(10, GROUND_LEVEL))
        .expect("the shared synthetic arena must be valid");
    let app = builder.app_mut();
    // The shipped combat.ron values; production loads the file instead.
    app.insert_resource(hex_assets::CombatSettings::default());
    app.insert_resource(PlayerSettings {
        scale: 0.25,
        speed: 5.0,
    });
    // `hex_units::movement::plugin`, not the whole of `hex_units::plugin`: this is what
    // keeps `StandsOn` honest as a unit walks, and combat is meaningless without it.
    // The full plugin would also read the active scenario placements and spawn its own
    // pieces on top of the ones these tests place by hand.
    app.add_plugins((
        hex_anim::plugin,
        hex_units::authored_object_occupancy::plugin,
        hex_units::movement::plugin,
        hex_combat::plugin,
    ));

    builder.build()
}

/// The stable id combat dealt this entity when the fight began.
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

fn spawn_unit(app: &mut App, faction: Faction, coord: HexCoord, initiative: u32) -> Entity {
    let id = app.world_mut().resource_mut::<UnitAllocator>().allocate();
    let standing = Standing {
        pos: TilePos::new(coord, GROUND_LEVEL),
        span: HexSpan::new(GROUND - 1.0, GROUND),
    };
    let mut unit = app.world_mut().spawn((
        faction,
        id,
        StandsOn(standing),
        Body::new(hex_core::TraversalProfile::WALKER),
        Initiative(initiative),
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
    publish_fixture_knowledge(app);
    app.update();
    app.update();
}

#[expect(
    clippy::expect_used,
    reason = "invalid deterministic fixture projections should fail at their construction seam"
)]
fn publish_fixture_knowledge(app: &mut App) {
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
        let mut query = world.query::<(&UnitId, &Faction, &StandsOn)>();
        query
            .iter(world)
            .map(|(&id, &faction, standing)| ObservedUnit {
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

/// Advances the bounded domain route to completion.
///
/// The fixture normally freezes virtual time at zero so tests choose every
/// transition explicitly. Temporarily advancing that clock proves movement can
/// settle without inspecting or removing its presentation component.
fn finish_moving(app: &mut App, entity: Entity) {
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        Duration::from_millis(100),
    ));
    for _ in 0..32 {
        if app.world().get::<MovingTo>(entity).is_none() {
            break;
        }
        app.update();
    }
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        Duration::ZERO,
    ));
    app.update();
}

/// Where a unit currently stands.
///
/// Returns an `Option` rather than unwrapping: the restriction lints are only relaxed
/// inside `#[test]` functions, not in helpers they call, so a panic belongs at the
/// call site where it is also more informative.
fn coord_of(app: &App, entity: Entity) -> Option<HexCoord> {
    Some(app.world().get::<StandsOn>(entity)?.0.pos.coord)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StalemateFingerprint {
    outcome: Option<EncounterOutcome>,
    current: Option<UnitId>,
    round: u32,
    idle_turns: u32,
    ai_selection_count: u64,
    ai_selection_fingerprint: u64,
    event_count: u64,
    event_fingerprint: u64,
    retained_traces: usize,
    retained_ai_selections: usize,
    retained_events: usize,
}

fn run_ten_thousand_turn_stalemate(turns: u32) -> StalemateFingerprint {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
    );
    enter_gameplay(&mut app);
    app.update();
    assert_eq!(*app.world().resource::<State<Mode>>().get(), Mode::Combat);

    let player_id = unit_id(&app, player);
    let enemy_id = unit_id(&app, enemy);
    // Both units remain physically within engagement range, but the hostile faction
    // has no authorized target. Its only legal action is therefore EndTurn.
    app.insert_resource(FactionMapKnowledge::new());

    for turn in 0..turns {
        let before = app.world().resource::<TurnOrder>().current();
        if before == Some(player_id) {
            app.world_mut()
                .resource_mut::<CommandQueue>()
                .push(IssuedCommand {
                    seat: PlayerSeat::default(),
                    command: GameCommand::EndTurn { unit: player_id },
                });
        } else {
            assert_eq!(
                before,
                Some(enemy_id),
                "stalemate lost its current unit at turn {turn}"
            );
        }
        app.update();
        assert_ne!(
            app.world().resource::<TurnOrder>().current(),
            before,
            "turn {turn} deadlocked"
        );
        assert!(
            app.world().resource::<CommandQueue>().is_empty(),
            "turn {turn} left an undrained command"
        );
    }

    let summary = app.world().resource::<CombatSummary>();
    let traces = app.world().resource::<AiDecisionTraces>();
    let transcript = app.world().resource::<CombatTranscriptRecorder>();
    assert_eq!(summary.idle_turns, turns);
    assert_eq!(summary.ai_selection_count, u64::from(turns / 2));
    assert_eq!(traces.entries.len(), MAX_AI_DECISION_TRACES);
    assert_eq!(
        summary.ai_selections.len(),
        MAX_COMBAT_SUMMARY_DETAILS,
        "the long soak should exercise the retained AI-detail cap"
    );
    assert!(summary.events.len() <= MAX_COMBAT_SUMMARY_DETAILS);
    assert!(!transcript.is_enabled());
    assert!(transcript.ai_selections().is_empty());
    assert!(transcript.events().is_empty());

    StalemateFingerprint {
        outcome: app.world().resource::<EncounterResolution>().outcome(),
        current: app.world().resource::<TurnOrder>().current(),
        round: app.world().resource::<TurnOrder>().round,
        idle_turns: summary.idle_turns,
        ai_selection_count: summary.ai_selection_count,
        ai_selection_fingerprint: summary.ai_selection_fingerprint,
        event_count: summary.event_count,
        event_fingerprint: summary.event_fingerprint,
        retained_traces: traces.entries.len(),
        retained_ai_selections: summary.ai_selections.len(),
        retained_events: summary.events.len(),
    }
}

#[test]
#[ignore = "manual release-mode 10,000-turn combat stalemate soak"]
fn ten_thousand_turn_stalemate_keeps_diagnostics_bounded() {
    let started = Instant::now();
    let first = run_ten_thousand_turn_stalemate(10_000);
    let second = run_ten_thousand_turn_stalemate(10_000);
    assert_eq!(first, second, "identical stalemates diverged");
    assert_eq!(first.outcome, None);
    assert_ne!(first.ai_selection_fingerprint, 0);
    eprintln!(
        "COMBAT_SOAK turns_per_run=10000 runs=2 elapsed_ms={} rounds={} \
         ai_count={} ai_fingerprint={} event_count={} event_fingerprint={} \
         retained_traces={} retained_ai={} retained_events={}",
        started.elapsed().as_millis(),
        first.round,
        first.ai_selection_count,
        first.ai_selection_fingerprint,
        first.event_count,
        first.event_fingerprint,
        first.retained_traces,
        first.retained_ai_selections,
        first.retained_events,
    );
}

/// The enemy has a lower initiative here, so the player acts first. Ending the
/// player's turn hands over, and the enemy closes the distance on its own.
#[test]
fn an_enemy_closes_the_distance_on_its_turn() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(4, -4, 0),
        10,
    );
    enter_gameplay(&mut app);

    let before = coord_of(&app, enemy).expect("the enemy exists");
    assert_eq!(before.distance(HexCoord::ORIGIN), 4, "precondition");

    end_turn(&mut app);
    app.update();
    finish_moving(&mut app, enemy);

    let after = coord_of(&app, enemy).expect("the enemy exists");
    assert!(
        after.distance(HexCoord::ORIGIN) < before.distance(HexCoord::ORIGIN),
        "the enemy should have moved closer; it is at {after:?}"
    );
}

/// It must stop *next to* the player, never on top of it. Walking onto the target
/// would put two units on one surface, which the model has no way to express.
#[test]
fn an_enemy_stops_adjacent_rather_than_on_top() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(3, -3, 0),
        10,
    );
    enter_gameplay(&mut app);

    end_turn(&mut app);
    app.update();
    finish_moving(&mut app, enemy);

    let after = coord_of(&app, enemy).expect("the enemy exists");
    assert_ne!(after, HexCoord::ORIGIN, "the enemy walked onto the player");
    assert!(
        after.distance(HexCoord::ORIGIN) >= 1,
        "the enemy should stop adjacent at the closest"
    );
}

/// A turn's movement budget is a limit, not a suggestion. An enemy far away must not
/// cross the whole map in one go.
#[test]
fn an_enemy_cannot_outrun_its_movement_budget() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    // Distance 6, inside the disengage margin so the fight holds, but further than
    // one turn's movement.
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(6, -6, 0),
        10,
    );
    enter_gameplay(&mut app);

    // Force the fight: this distance is outside engage range on its own.
    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();
    app.update();

    let before = coord_of(&app, enemy).expect("the enemy exists");
    end_turn(&mut app);
    app.update();
    finish_moving(&mut app, enemy);
    let after = coord_of(&app, enemy).expect("the enemy exists");

    let travelled = before.distance(after);
    assert!(
        travelled <= 4,
        "the enemy moved {travelled} hexes on a four-hex budget"
    );
    assert!(travelled > 0, "the enemy should have moved at all");
}

/// Equal tactical choices resolve by the stable unit id rather than query
/// iteration order or entity bits — neither of which survives a save or a
/// second run.
#[test]
fn equally_routable_foes_have_a_deterministic_tie_break() {
    let mut app = test_app();
    let enemy = spawn_unit(&mut app, Faction::Hostile, HexCoord::ORIGIN, 30);
    let first = HexCoord::new_cubic(3, -3, 0);
    let second = HexCoord::new_cubic(0, 3, -3);
    let first_entity = spawn_unit(&mut app, Faction::Player, first, 20);
    let second_entity = spawn_unit(&mut app, Faction::Player, second, 10);
    // Explicit ids, with the LOWER one on the later spawn: the winner must
    // follow the stable id, not spawn order and not entity allocation.
    app.world_mut().entity_mut(first_entity).insert(UnitId(2));
    app.world_mut().entity_mut(second_entity).insert(UnitId(1));
    let (expected, other) = (second, first);
    enter_gameplay(&mut app);

    let destination = app
        .world()
        .get::<MovingTo>(enemy)
        .and_then(|moving| moving.path.last())
        .expect("the enemy should commit an approach")
        .pos
        .coord;
    assert_eq!(
        destination.distance(expected),
        1,
        "an exact tie did not choose the lower unit id"
    );
    assert!(
        destination.distance(other) > 1,
        "the route headed toward the other equally distant foe"
    );
}

/// A turn must not pass while its unit is still mid-stride. Advancing early cuts the
/// animation off and strands the piece between two hexes.
///
/// Note the test does **not** wait for the animation to play: `app.update()` in a
/// tight loop advances the clock by microseconds, so a real 0.36s lunge would never
/// finish however many frames were run. The contract is expressed in terms of the
/// component, so the test is too.
#[test]
fn a_turn_does_not_pass_while_a_unit_is_still_moving() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(3, -3, 0),
        10,
    );
    enter_gameplay(&mut app);

    end_turn(&mut app);

    assert!(
        app.world().get::<Transformation>(enemy).is_some(),
        "precondition: the enemy should be moving"
    );
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(unit_id(&app, enemy)),
        "the turn must stay with a unit that is still animating"
    );
}

/// Once domain movement finishes, an enemy reconsiders its still-unused action.
///
/// The AI host deliberately does not prequeue EndTurn beside movement: doing so would
/// throw away the same move-then-act economy the player receives.
#[test]
fn an_enemy_reconsiders_after_its_move_finishes() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(3, -3, 0),
        10,
    );
    enter_gameplay(&mut app);

    end_turn(&mut app);

    finish_moving(&mut app, enemy);
    // One domain tick projects the follow-up strike; the next lets the policy
    // explicitly yield its unused movement without waiting for that swing animation.
    app.update();

    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(unit_id(&app, player)),
        "presentation must not retain gameplay authority after the follow-up strike"
    );
    assert!(
        app.world().get::<Transformation>(enemy).is_some(),
        "finishing the domain move should still project an adjacent follow-up strike"
    );
    assert!(
        app.world().get::<Turn>(player).is_some(),
        "the player should hold the turn marker while the prior swing still presents"
    );
}

/// An enemy already next to the player attacks instead of moving — and an attack
/// leaves it exactly where it started.
#[test]
fn an_adjacent_enemy_attacks_without_moving() {
    let mut app = test_app();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        Duration::from_millis(100),
    ));
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let adjacent = HexCoord::new_cubic(1, -1, 0);
    let enemy = spawn_unit(&mut app, Faction::Hostile, adjacent, 10);
    enter_gameplay(&mut app);

    end_turn(&mut app);

    assert!(
        app.world().get::<Transformation>(enemy).is_some(),
        "an adjacent enemy should commit an attack animation"
    );
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(unit_id(&app, enemy)),
        "the attacker should keep its turn while the lunge is running"
    );

    for _ in 0..10 {
        app.update();
        let attack_finished = app.world().get::<Transformation>(enemy).is_none();
        let turn_advanced =
            app.world().resource::<TurnOrder>().current() == Some(unit_id(&app, player));
        if attack_finished && turn_advanced {
            break;
        }
    }

    assert!(
        app.world().get::<Transformation>(enemy).is_none(),
        "the deterministic clock did not finish the enemy's attack"
    );
    assert!(
        app.world().get::<Transformation>(player).is_none(),
        "the target's recoil outlived the completed attack"
    );
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(unit_id(&app, player)),
        "the turn should advance after the attack animation completes"
    );
    assert!(
        app.world().get::<Turn>(player).is_some(),
        "the player should receive the next turn"
    );
    assert_eq!(
        coord_of(&app, enemy),
        Some(adjacent),
        "an attack should not change which surface the enemy stands on"
    );
}

#[test]
fn a_depleted_enemy_channels_only_from_its_canonical_legal_actions() {
    let mut app = test_app();
    let elements = ElementCatalog::from_file(&ElementFile {
        wheel: vec!["Fire".to_owned(), "Water".to_owned()],
        fusions: bevy::platform::collections::HashMap::default(),
    });
    let Some(fire) = elements.id("Fire") else {
        unreachable!("the fixture defines Fire")
    };
    app.insert_resource(elements);
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(3, -3, 0),
        10,
    );
    let gem = LatticeCoord::ORIGIN;
    let spec = LatticeSpec::default().with(gem, CellKind::Gem { element: fire });
    let empty = LatticeStats::default();
    let state = LatticeState::new(&spec, &empty);
    let stats = LatticeStats::new(BTreeMap::from([(fire, 3)]), BTreeMap::from([(fire, 2)]));
    app.world_mut()
        .entity_mut(enemy)
        .insert((spec, state, stats));
    enter_gameplay(&mut app);

    end_turn(&mut app);

    let enemy_id = unit_id(&app, enemy);
    let traces = app.world().resource::<AiDecisionTraces>();
    let Some(trace) = traces.entries.last() else {
        panic!("the enemy should produce a decision trace")
    };
    assert!(
        trace
            .legal_actions
            .actions()
            .iter()
            .any(|action| action.command == GameCommand::Channel { unit: enemy_id }),
        "combat must place Channel in the canonical set before the algorithm can choose it"
    );
    assert_eq!(
        trace.command,
        Some(GameCommand::Channel { unit: enemy_id }),
        "the baseline's depleted-lattice choice is deterministic"
    );
    assert_eq!(
        app.world()
            .get::<LatticeState>(enemy)
            .map(|state| state.mana(gem)),
        Some(2)
    );
    assert_eq!(app.world().resource::<CombatSummary>().channels, 1);
}

#[test]
fn enemy_legal_routes_never_enter_an_allied_occupied_surface() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let actor = spawn_unit(&mut app, Faction::Hostile, HexCoord::from_axial(3, 0), 10);
    let blocker = spawn_unit(&mut app, Faction::Hostile, HexCoord::from_axial(2, 0), 5);
    enter_gameplay(&mut app);

    end_turn(&mut app);

    let actor_id = unit_id(&app, actor);
    let blocked = app
        .world()
        .get::<StandsOn>(blocker)
        .map(|standing| standing.0.pos);
    let traces = app.world().resource::<AiDecisionTraces>();
    let Some(trace) = traces.entries.last() else {
        panic!("the acting enemy should produce a decision trace")
    };
    assert_eq!(trace.actor, actor_id);
    assert!(trace.legal_actions.actions().iter().all(|action| {
        match &action.command {
            GameCommand::MoveAlong { path, .. } => {
                !path.iter().any(|position| Some(*position) == blocked)
            }
            _ => true,
        }
    }));
}

/// A hostile strike can park resolution on a human defender choice. The AI must not
/// prequeue its end-turn beside that strike — the modal gate correctly refuses every
/// command except the matching answer — but it must still end the spent turn once the
/// answer and presentation have both finished.
#[test]
fn a_player_defence_choice_does_not_strand_the_enemy_turn() {
    let mut app = test_app();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        Duration::from_millis(100),
    ));
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(1, -1, 0),
        10,
    );
    let spec = LatticeSpec::default()
        .with(LatticeCoord::ORIGIN, CellKind::Blank)
        .with(LatticeCoord::new(1, 0), CellKind::Blank);
    let stats = LatticeStats::default();
    let state = LatticeState::new(&spec, &stats);
    app.world_mut()
        .entity_mut(player)
        .insert((ControlOwner::default(), spec, state, stats));
    app.world_mut()
        .entity_mut(enemy)
        .insert(ControlOwner::default());
    enter_gameplay(&mut app);

    let player_id = unit_id(&app, player);
    let enemy_id = unit_id(&app, enemy);
    end_turn(&mut app);

    assert_eq!(
        *app.world().resource::<PendingDecision>(),
        PendingDecision::ChooseDisables {
            decider: player_id,
            count: 1,
            source: enemy_id,
        },
        "the adjacent strike should wait for the player to name its disabled cell"
    );
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(enemy_id),
        "the attacker owns the turn while its damage is unresolved"
    );

    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat::default(),
            command: GameCommand::ChooseDisables {
                unit: player_id,
                cells: vec![LatticeCoord::ORIGIN],
            },
        });
    app.update();

    for _ in 0..20 {
        app.update();
        if app.world().resource::<TurnOrder>().current() == Some(player_id) {
            break;
        }
    }

    assert!(
        !app.world().resource::<PendingDecision>().is_open(),
        "the matching player answer should resolve the parked damage"
    );
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(player_id),
        "the AI should end its already-spent turn after the decision and lunge resolve"
    );
    assert!(
        app.world().get::<Turn>(player).is_some(),
        "the player should receive the next turn"
    );
}

/// Resolving one defender choice must not make the hostile policy dependent on the
/// keyboard for later rounds. The gameplay walk used to hide this by pressing Space
/// while the hostile still held its second spent turn.
#[test]
fn repeated_player_defence_choices_do_not_strand_a_later_enemy_turn() {
    let mut app = test_app();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        Duration::from_millis(4),
    ));
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(1, -1, 0),
        10,
    );
    let first = LatticeCoord::ORIGIN;
    let second = LatticeCoord::new(1, 0);
    let spec = LatticeSpec::default()
        .with(first, CellKind::Blank)
        .with(second, CellKind::Blank)
        .with(LatticeCoord::new(0, 1), CellKind::Blank);
    let stats = LatticeStats::default();
    let state = LatticeState::new(&spec, &stats);
    app.world_mut()
        .entity_mut(player)
        .insert((ControlOwner::default(), spec, state, stats));
    app.world_mut()
        .entity_mut(enemy)
        .insert(ControlOwner::default());
    enter_gameplay(&mut app);

    let player_id = unit_id(&app, player);
    for cell in [first, second] {
        end_turn(&mut app);
        assert!(
            app.world().resource::<PendingDecision>().is_open(),
            "each adjacent hostile turn should ask the player to choose a cell"
        );
        app.world_mut()
            .resource_mut::<CommandQueue>()
            .push(IssuedCommand {
                seat: PlayerSeat::default(),
                command: GameCommand::ChooseDisables {
                    unit: player_id,
                    cells: vec![cell],
                },
            });

        for _ in 0..240 {
            app.update();
            if app.world().resource::<TurnOrder>().current() == Some(player_id) {
                break;
            }
        }
        assert_eq!(
            app.world().resource::<TurnOrder>().current(),
            Some(player_id),
            "the hostile's spent turn should end after defence choice at {cell:?}"
        );
    }
}

/// Advances the simulation without coupling enemy-policy tests to an input binding.
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
    // The command advances to the hostile during Apply/Advance. Its policy runs
    // in Act on the following frame, preserving the production schedule boundary.
    app.update();
    app.update();
}

/// The turn cannot pass while the unit holding it is still walking.
///
/// Acting is half immediate and half deferred: `spend` mutates `Turn` in place, but the
/// walk animation is inserted through `Commands`. Until `CombatSystems` existed,
/// `take_enemy_turn` and `advance_turn` were unordered — so `advance_turn` could see a
/// turn already marked finished with no `Transformation` yet attached to say the unit
/// was moving, and hand the turn on before the enemy had taken a step.
///
/// The observable consequence is this: while a `Transformation` is running, the order
/// must still be pointing at its owner. Anything else means somebody else can act
/// while the enemy is mid-stride.
#[test]
fn the_turn_does_not_pass_while_its_unit_is_still_walking() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(4, -4, 0),
        10,
    );
    enter_gameplay(&mut app);

    end_turn(&mut app);
    app.update();

    assert!(
        app.world().get::<Transformation>(enemy).is_some(),
        "precondition: the enemy should be walking by now"
    );
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(unit_id(&app, enemy)),
        "the turn was handed on while the enemy was still mid-walk"
    );
}

/// A fight starts when a unit **arrives**, not when it sets off.
///
/// The whole point of splitting `StandsOn` from `MovingTo`. Engagement asks where units
/// are; while the two were one component that answer was the destination, so committing
/// to a walk started the fight instantly at the far end of the route — and a walk whose
/// endpoint happened to be out of range could pass straight through engaging distance
/// without anything noticing.
///
/// Covered here rather than in `hex_units` because `engagement` lives in this crate and
/// the claim is about the two halves agreeing.
#[test]
fn a_fight_starts_on_arrival_not_on_departure() {
    let mut app = test_app();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        Duration::from_millis(100),
    ));
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let start = HexCoord::new_cubic(5, -5, 0);
    let destination = HexCoord::new_cubic(4, -4, 0);
    let enemy = spawn_unit(&mut app, Faction::Hostile, start, 10);
    enter_gameplay(&mut app);

    assert_eq!(
        *app.world().resource::<State<Mode>>().get(),
        Mode::Exploring,
        "precondition: five hexes apart is no fight"
    );

    // One real leg crosses exactly from outside engagement range to its boundary.
    // Making its speed equal to its world-space length gives the leg a one-second
    // duration under the deterministic 100 ms test clock.
    let path: Vec<Standing> = start
        .line_between(destination)
        .into_iter()
        .map(|coord| Standing {
            pos: TilePos::new(coord, GROUND_LEVEL),
            span: HexSpan::new(GROUND - 1.0, GROUND),
        })
        .collect();
    let [from, to] = path.as_slice() else {
        panic!("an adjacent coordinate pair should produce a two-step route");
    };
    let speed = from.world_position().distance(to.world_position());
    app.world_mut().entity_mut(enemy).insert((
        MovingTo::new(path.clone(), speed),
        Transformation::from(HexPathingLine::new(&path, speed)),
    ));

    // Several active ticks, but less than the leg's one-second duration.
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(
        coord_of(&app, enemy),
        Some(start),
        "logical position advanced before the route reached its first waypoint"
    );
    assert!(
        app.world().get::<Transformation>(enemy).is_some(),
        "the route finished before the arrival assertion"
    );
    assert!(matches!(
        app.world().resource::<NextState<Mode>>(),
        &NextState::Unchanged
    ));

    for _ in 0..10 {
        app.update();
        if coord_of(&app, enemy) == Some(destination) {
            break;
        }
    }

    assert_eq!(
        coord_of(&app, enemy),
        Some(destination),
        "the real route never arrived at the engagement boundary"
    );
    assert_eq!(
        *app.world().resource::<State<Mode>>().get(),
        Mode::Exploring,
        "the queued combat transition should apply on the next frame"
    );
    assert!(
        matches!(
            app.world().resource::<NextState<Mode>>(),
            &NextState::Pending(Mode::Combat)
        ),
        "arrival at engagement range did not queue combat"
    );

    app.update();

    assert_eq!(
        *app.world().resource::<State<Mode>>().get(),
        Mode::Combat,
        "arriving next to the player should start the fight"
    );
}

/// Intermediate whole steps are positions too, even when one frame crosses several.
///
/// Both endpoints are outside engage range, so an implementation that updates
/// `StandsOn` only at final arrival misses the fight completely even though the route
/// passes directly through the player's tile. The deliberately high speed makes the
/// first active tick jump from distance eight to distance six on the other side, so
/// sampling only one `StandsOn` per frame also misses every in-range waypoint.
#[test]
fn crossing_engage_range_mid_route_starts_fight_and_stops_walk() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let start = HexCoord::new_cubic(-8, 8, 0);
    let end = HexCoord::new_cubic(8, -8, 0);
    let enemy = spawn_unit(&mut app, Faction::Hostile, start, 10);
    enter_gameplay(&mut app);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        Duration::from_millis(250),
    ));

    let path: Vec<Standing> = start
        .line_between(end)
        .into_iter()
        .map(|coord| Standing {
            pos: TilePos::new(coord, GROUND_LEVEL),
            span: HexSpan::new(GROUND - 1.0, GROUND),
        })
        .collect();
    let speed = 100.0;
    app.world_mut().entity_mut(enemy).insert((
        MovingTo::new(path.clone(), speed),
        Transformation::from(HexPathingLine::new(&path, speed)),
    ));

    for _ in 0..100 {
        app.update();
        if *app.world().resource::<State<Mode>>().get() == Mode::Combat {
            break;
        }
    }

    assert_eq!(
        *app.world().resource::<State<Mode>>().get(),
        Mode::Combat,
        "the route crossed engage range without starting a fight"
    );
    assert!(
        app.world().get::<Transformation>(enemy).is_none(),
        "the walk continued after combat began"
    );
    assert!(
        app.world().get::<MovingTo>(enemy).is_none(),
        "the interrupted unit kept its obsolete route"
    );

    let stopped = app
        .world()
        .get::<StandsOn>(enemy)
        .expect("the enemy should still have a logical position")
        .0
        .pos;
    assert!(path.iter().any(|step| step.pos == stopped));
    assert!(
        stopped.coord.distance(HexCoord::ORIGIN) <= 4,
        "combat began before the route reached engagement distance"
    );
    assert_ne!(
        stopped.coord, end,
        "the interrupted walk was delivered to its out-of-range endpoint"
    );
}
