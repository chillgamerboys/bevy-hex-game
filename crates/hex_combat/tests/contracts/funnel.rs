//! Contract tests for the command funnel: one applier, replayable input.
//!
//! Commands are pushed straight into the [`CommandQueue`] — the same resource
//! every emitter writes — so these tests cover the applier's contract without
//! caring which input produced the command. Emission itself is covered where
//! the emitters live: the click observer in `hex_units`' tests, the end-turn
//! key in `loop.rs`, the AI in `enemy.rs`.
//!
//! The heart of the file is the replay test: the funnel exists so that the
//! sim's entire input is an ordered command sequence, and a sequence applied
//! twice from the same spawn state must land the same world twice.

use std::collections::BTreeMap;

use bevy::prelude::*;

use hex_assets::{ElementCatalog, ElementFile, FormationCatalog, PlayerSettings, SubstanceTable};
use hex_combat::{
    CombatData, CombatEvent, CombatSummary, CommandRefusal, Initiative, PartyMoveRefusal, TurnOrder,
};
use hex_core::{
    Busy, CommandQueue, ControlOwner, ElementId, FormationPreset, FormationSlot, GameCommand,
    Headroom, HexCoord, HexSpan, HexTile, IssuedCommand, LatticeCoord, Mode, PartyFormation,
    PartyPath, PlayerSeat, Screen, SpellId, SubstanceId, TilePos, TraversalBlockers, Turn, UnitId,
    MAX_HEADROOM,
};
use hex_lattice::{
    apply_cast, castable, Casting, CellKind, FusionTable, LatticeSpec, LatticeState, LatticeStats,
    Requirement, SpellTable,
};
use hex_test_support::{SyntheticArena, TestAppBuilder, STONE};
use hex_units::{
    route, Body, Downed, Faction, Footing, OccupancyBlock, Party, Standing, StandsOn, UnitRegistry,
};

const GROUND: f32 = 2.0;
const GROUND_LEVEL: hex_core::Level = 1;
#[expect(
    clippy::expect_used,
    reason = "invalid shared deterministic fixture data must fail during construction"
)]
fn test_app() -> App {
    let mut builder = TestAppBuilder::new()
        .with_arena(SyntheticArena::flat_radius(10, GROUND_LEVEL))
        .expect("the shared synthetic arena must be valid");
    let app = builder.app_mut();
    // The shipped combat.ron values; production loads the file instead.
    app.insert_resource(hex_assets::CombatSettings::default());
    app.insert_resource(shipped_elements());
    app.insert_resource(PlayerSettings {
        scale: 0.25,
        speed: 5.0,
    });
    app.add_plugins((
        hex_anim::plugin,
        hex_units::movement::plugin,
        hex_combat::plugin,
    ));

    builder.build()
}

fn shipped_elements() -> ElementCatalog {
    ElementCatalog::from_file(&ElementFile {
        wheel: vec!["Fire".to_owned(), "Water".to_owned()],
        fusions: bevy::platform::collections::HashMap::default(),
    })
}

struct ChannelTables {
    fire: ElementId,
}

impl FusionTable for ChannelTables {
    fn recipe(&self, _output: ElementId) -> Option<Vec<Requirement>> {
        None
    }
}

impl SpellTable for ChannelTables {
    fn requirements(&self, _spell: SpellId) -> Vec<Requirement> {
        vec![Requirement {
            element: self.fire,
            mana: 2,
        }]
    }

    fn casting(&self, _spell: SpellId) -> Casting {
        Casting::Evocation
    }
}

/// A unit with an explicit, pre-registered id.
///
/// Deliberately no `Enemy` marker on hostiles: the AI emits for marked enemies,
/// and these tests script every command themselves. `begin_combat` upserts the
/// carried ids, so the explicit registration and the dealt order agree.
fn spawn_unit(
    app: &mut App,
    faction: Faction,
    coord: HexCoord,
    initiative: u32,
    id: u64,
) -> Entity {
    let standing = Standing {
        pos: TilePos::new(coord, GROUND_LEVEL),
        span: HexSpan::new(GROUND - 1.0, GROUND),
    };
    let entity = app
        .world_mut()
        .spawn((
            faction,
            StandsOn(standing),
            Body::new(hex_core::TraversalProfile::WALKER),
            Initiative(initiative),
            UnitId(id),
            Transform::from_translation(standing.world_position()),
        ))
        .id();
    app.world_mut()
        .resource_mut::<UnitRegistry>()
        .register(UnitId(id), entity);
    entity
}

fn enter_gameplay(app: &mut App) {
    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Gameplay);
    app.update();
    app.update();
}

fn mode(app: &App) -> Mode {
    *app.world().resource::<State<Mode>>().get()
}

fn push(app: &mut App, command: GameCommand) {
    push_as(app, PlayerSeat(0), command);
}

fn push_as(app: &mut App, seat: PlayerSeat, command: GameCommand) {
    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand { seat, command });
}

fn take_events(app: &mut App) -> Vec<CombatEvent> {
    app.world_mut()
        .resource_mut::<Messages<CombatEvent>>()
        .drain()
        .collect()
}

#[test]
fn combat_commands_fail_closed_when_the_authority_cannot_initialize() {
    let mut builder = TestAppBuilder::new();
    let app = builder.app_mut();
    app.insert_resource(hex_assets::CombatSettings::default());
    app.add_plugins(hex_combat::plugin);
    app.init_resource::<UnitRegistry>();
    let mut app = builder.build();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        2,
    );
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");
    assert!(
        hex_combat::authority_snapshot(app.world()).is_err(),
        "the fixture intentionally omits published arena facts"
    );
    take_events(&mut app);

    let before = *app
        .world()
        .get::<Turn>(
            app.world()
                .resource::<UnitRegistry>()
                .entity_of(UnitId(1))
                .expect("the fixture unit is registered"),
        )
        .expect("the first unit owns the turn");
    let command = GameCommand::EndTurn { unit: UnitId(1) };
    push(&mut app, command.clone());
    app.update();

    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(UnitId(1)),
        "missing authority must never fall back to the legacy mutator"
    );
    let after = *app
        .world()
        .get::<Turn>(
            app.world()
                .resource::<UnitRegistry>()
                .entity_of(UnitId(1))
                .expect("the fixture unit stays registered"),
        )
        .expect("the refused command preserves the turn");
    assert_eq!(after, before);
    assert!(
        take_events(&mut app).contains(&CombatEvent::CommandRefused {
            command,
            refusal: CommandRefusal::MissingCombatData {
                data: CombatData::AuthorityState,
            },
        })
    );
}

/// Runs frames until the queue is drained and nothing is mid-presentation.
///
/// Bounded so a regression stalls the test with a message instead of hanging
/// the suite.
fn settle(app: &mut App) {
    for _ in 0..300 {
        let busy = {
            let mut busy = app.world_mut().query_filtered::<Entity, With<Busy>>();
            busy.iter(app.world()).next().is_some()
        };
        if !busy && app.world().resource::<CommandQueue>().is_empty() {
            app.update();
            return;
        }
        app.update();
    }
}

/// A surface path along adjacent coordinates at ground level.
fn path(coords: &[HexCoord]) -> Vec<TilePos> {
    coords
        .iter()
        .map(|coord| TilePos::new(*coord, GROUND_LEVEL))
        .collect()
}

fn standing_of(app: &mut App, entity: Entity) -> Option<TilePos> {
    app.world().get::<StandsOn>(entity).map(|s| s.0.pos)
}

fn budget_of(app: &App, entity: Entity) -> Option<u32> {
    app.world()
        .get::<Turn>(entity)
        .map(|turn| turn.movement_left)
}

fn pair_formation(app: &mut App) {
    let preset = FormationPreset {
        name: "Pair".to_owned(),
        slots: vec![
            FormationSlot {
                offset: HexCoord::ORIGIN,
                anchor: true,
            },
            FormationSlot {
                offset: HexCoord::from_axial(-1, 0),
                anchor: false,
            },
        ],
    };
    app.insert_resource(FormationCatalog {
        presets: vec![preset.clone()],
    });
    app.world_mut().resource_mut::<Party>().members = vec![UnitId(1), UnitId(2)];
    app.world_mut()
        .resource_mut::<PartyFormation>()
        .select_preset(&preset, &[UnitId(1), UnitId(2)]);
}

fn pair_party_app() -> (App, Entity, Entity) {
    let mut app = test_app();
    let anchor = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    let rear = spawn_unit(
        &mut app,
        Faction::Player,
        HexCoord::from_axial(-1, 0),
        10,
        2,
    );
    pair_formation(&mut app);
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Exploring);
    (app, anchor, rear)
}

#[expect(
    clippy::expect_used,
    reason = "invalid deterministic fixture content should fail at construction"
)]
fn insert_depleted_channel_lattice(app: &mut App, entity: Entity) {
    let fire = app
        .world()
        .resource::<ElementCatalog>()
        .id("Fire")
        .expect("the shipped catalog defines Fire");
    let spell = LatticeCoord::ORIGIN;
    let [gem, ..] = spell.neighbors();
    let spec = LatticeSpec::default()
        .with(spell, CellKind::Spell { spell: SpellId(0) })
        .with(gem, CellKind::Gem { element: fire });
    let stats = LatticeStats::new(BTreeMap::from([(fire, 3)]), BTreeMap::from([(fire, 2)]));
    let mut state = LatticeState::new(&spec, &stats);
    let tables = ChannelTables { fire };
    let plan = castable(&spec, &state, spell, &tables).expect("fixture spell can drain its gem");
    assert!(apply_cast(&mut state, &plan, &tables));
    app.world_mut()
        .entity_mut(entity)
        .insert((spec, state, stats));
}

fn party_command(anchor: UnitId, paths: Vec<(UnitId, &[HexCoord])>) -> GameCommand {
    GameCommand::MoveParty {
        anchor,
        paths: paths
            .into_iter()
            .map(|(member, coords)| PartyPath {
                member,
                path: path(coords),
            })
            .collect(),
    }
}

/// Out of combat there is no turn and no budget: a valid move command simply
/// starts the walk. The funnel is the write path in both tempos.
#[test]
fn an_exploring_move_flows_through_the_funnel() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Exploring, "precondition: nobody to fight");

    let destination = HexCoord::new_cubic(1, -1, 0);
    push(
        &mut app,
        GameCommand::MoveAlong {
            unit: UnitId(1),
            path: path(&[HexCoord::ORIGIN, destination]),
        },
    );
    settle(&mut app);

    assert_eq!(
        standing_of(&mut app, player),
        Some(TilePos::new(destination, GROUND_LEVEL)),
        "the commanded walk should land"
    );
}

#[test]
fn command_grounding_rejects_generated_feature_blockers() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    let destination = HexCoord::new_cubic(1, -1, 0);
    let mut blockers = TraversalBlockers::new();
    assert!(blockers.insert(TilePos::new(destination, GROUND_LEVEL)));
    app.insert_resource(blockers);
    enter_gameplay(&mut app);

    push(
        &mut app,
        GameCommand::MoveAlong {
            unit: UnitId(1),
            path: path(&[HexCoord::ORIGIN, destination]),
        },
    );
    settle(&mut app);

    assert_eq!(
        standing_of(&mut app, player),
        Some(TilePos::new(HexCoord::ORIGIN, GROUND_LEVEL)),
        "the authoritative applier must not ground a path through a tree root"
    );
}

#[test]
fn occupied_endpoints_and_route_steps_are_refused_exactly() {
    let cases = [
        (
            HexCoord::from_axial(1, 0),
            vec![HexCoord::ORIGIN, HexCoord::from_axial(1, 0)],
            OccupancyBlock::Destination {
                position: TilePos::new(HexCoord::from_axial(1, 0), GROUND_LEVEL),
                occupant: UnitId(2),
            },
        ),
        (
            HexCoord::from_axial(1, 0),
            vec![
                HexCoord::ORIGIN,
                HexCoord::from_axial(1, 0),
                HexCoord::from_axial(2, 0),
            ],
            OccupancyBlock::Route {
                position: TilePos::new(HexCoord::from_axial(1, 0), GROUND_LEVEL),
                occupant: UnitId(2),
            },
        ),
    ];

    for (blocker, coords, block) in cases {
        let mut app = test_app();
        spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
        spawn_unit(&mut app, Faction::Player, blocker, 10, 2);
        enter_gameplay(&mut app);
        let command = GameCommand::MoveAlong {
            unit: UnitId(1),
            path: path(&coords),
        };
        push(&mut app, command.clone());
        app.update();
        assert_eq!(
            take_events(&mut app),
            vec![CombatEvent::CommandRefused {
                command,
                refusal: CommandRefusal::Occupied { block },
            }]
        );
    }
}

#[test]
fn a_body_on_a_lower_stacked_surface_does_not_block_the_upper_route() {
    let mut app = test_app();
    let destination = HexCoord::from_axial(1, 0);
    let mover = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    spawn_unit(&mut app, Faction::Player, destination, 10, 2);
    for coord in [HexCoord::ORIGIN, destination] {
        app.world_mut().spawn((
            HexTile,
            coord,
            TilePos::new(coord, 3),
            HexSpan::new(3.0, 4.0),
            STONE,
            Headroom(MAX_HEADROOM),
        ));
    }
    let high = Standing {
        pos: TilePos::new(HexCoord::ORIGIN, 3),
        span: HexSpan::new(3.0, 4.0),
    };
    app.world_mut().entity_mut(mover).insert((
        StandsOn(high),
        Transform::from_translation(high.world_position()),
    ));
    enter_gameplay(&mut app);

    push(
        &mut app,
        GameCommand::MoveAlong {
            unit: UnitId(1),
            path: vec![high.pos, TilePos::new(destination, 3)],
        },
    );
    settle(&mut app);
    assert_eq!(
        standing_of(&mut app, mover),
        Some(TilePos::new(destination, 3))
    );
}

#[test]
fn a_downed_body_still_owns_its_surface_until_removed() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    let destination = HexCoord::from_axial(1, 0);
    let blocker = spawn_unit(&mut app, Faction::Player, destination, 10, 2);
    app.world_mut().entity_mut(blocker).insert(Downed);
    enter_gameplay(&mut app);

    let command = GameCommand::MoveAlong {
        unit: UnitId(1),
        path: path(&[HexCoord::ORIGIN, destination]),
    };
    push(&mut app, command.clone());
    app.update();
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::CommandRefused {
            command,
            refusal: CommandRefusal::Occupied {
                block: OccupancyBlock::Destination {
                    position: TilePos::new(destination, GROUND_LEVEL),
                    occupant: UnitId(2),
                },
            },
        }]
    );
}

#[test]
fn an_in_flight_route_reserves_its_endpoint() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    let destination = HexCoord::from_axial(2, 0);
    spawn_unit(&mut app, Faction::Player, HexCoord::from_axial(3, 0), 10, 2);
    enter_gameplay(&mut app);
    push(
        &mut app,
        GameCommand::MoveAlong {
            unit: UnitId(1),
            path: path(&[HexCoord::ORIGIN, HexCoord::from_axial(1, 0), destination]),
        },
    );
    app.update();

    let command = GameCommand::MoveAlong {
        unit: UnitId(2),
        path: path(&[HexCoord::from_axial(3, 0), destination]),
    };
    push(&mut app, command.clone());
    app.update();
    assert_eq!(
        take_events(&mut app)
            .into_iter()
            .find(|event| matches!(event, CombatEvent::CommandRefused { .. })),
        Some(CombatEvent::CommandRefused {
            command,
            refusal: CommandRefusal::Occupied {
                block: OccupancyBlock::Destination {
                    position: TilePos::new(destination, GROUND_LEVEL),
                    occupant: UnitId(1),
                },
            },
        })
    );
}

#[test]
fn an_atomic_party_move_commits_every_member() {
    let (mut app, anchor, rear) = pair_party_app();
    let destination = HexCoord::from_axial(1, 0);
    let command = party_command(
        UnitId(1),
        vec![
            (UnitId(1), &[HexCoord::ORIGIN, destination]),
            (UnitId(2), &[HexCoord::from_axial(-1, 0), HexCoord::ORIGIN]),
        ],
    );
    push(&mut app, command.clone());
    app.update();
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::PartyMoved {
            anchor: UnitId(1),
            paths: match command {
                GameCommand::MoveParty { paths, .. } => paths,
                _ => unreachable!("the fixture is a party command"),
            },
        }]
    );
    settle(&mut app);
    assert_eq!(
        standing_of(&mut app, anchor),
        Some(TilePos::new(destination, GROUND_LEVEL))
    );
    assert_eq!(
        standing_of(&mut app, rear),
        Some(TilePos::new(HexCoord::ORIGIN, GROUND_LEVEL))
    );
}

#[test]
fn every_party_validation_failure_is_atomic() {
    let cases = [
        (
            PlayerSeat(0),
            party_command(
                UnitId(2),
                vec![
                    (UnitId(1), &[HexCoord::ORIGIN, HexCoord::from_axial(1, 0)]),
                    (UnitId(2), &[HexCoord::from_axial(-1, 0), HexCoord::ORIGIN]),
                ],
            ),
            CommandRefusal::PartyMove {
                reason: PartyMoveRefusal::WrongAnchor,
            },
        ),
        (
            PlayerSeat(0),
            party_command(
                UnitId(1),
                vec![
                    (UnitId(1), &[HexCoord::ORIGIN, HexCoord::from_axial(1, 0)]),
                    (UnitId(1), &[HexCoord::ORIGIN, HexCoord::from_axial(0, 1)]),
                ],
            ),
            CommandRefusal::PartyMove {
                reason: PartyMoveRefusal::DuplicateMember { member: UnitId(1) },
            },
        ),
        (
            PlayerSeat(0),
            party_command(
                UnitId(1),
                vec![(UnitId(1), &[HexCoord::ORIGIN, HexCoord::from_axial(1, 0)])],
            ),
            CommandRefusal::PartyMove {
                reason: PartyMoveRefusal::MissingMember { member: UnitId(2) },
            },
        ),
        (
            PlayerSeat(0),
            party_command(
                UnitId(1),
                vec![
                    (UnitId(1), &[HexCoord::ORIGIN, HexCoord::from_axial(1, 0)]),
                    (UnitId(2), &[HexCoord::ORIGIN, HexCoord::from_axial(0, 1)]),
                ],
            ),
            CommandRefusal::PartyMove {
                reason: PartyMoveRefusal::InvalidStart { member: UnitId(2) },
            },
        ),
        (
            PlayerSeat(0),
            party_command(
                UnitId(1),
                vec![
                    (UnitId(1), &[HexCoord::ORIGIN, HexCoord::from_axial(1, 0)]),
                    (
                        UnitId(2),
                        &[HexCoord::from_axial(-1, 0), HexCoord::from_axial(2, 0)],
                    ),
                ],
            ),
            CommandRefusal::PartyMove {
                reason: PartyMoveRefusal::InvalidMemberPath { member: UnitId(2) },
            },
        ),
        (
            PlayerSeat(0),
            party_command(
                UnitId(1),
                vec![
                    (UnitId(1), &[HexCoord::ORIGIN, HexCoord::from_axial(-1, 0)]),
                    (UnitId(2), &[HexCoord::from_axial(-1, 0), HexCoord::ORIGIN]),
                ],
            ),
            CommandRefusal::PartyMove {
                reason: PartyMoveRefusal::Occupied {
                    block: OccupancyBlock::Route {
                        position: TilePos::new(HexCoord::from_axial(-1, 0), GROUND_LEVEL),
                        occupant: UnitId(1),
                    },
                },
            },
        ),
        (
            PlayerSeat(0),
            party_command(
                UnitId(1),
                vec![
                    (UnitId(1), &[HexCoord::ORIGIN, HexCoord::from_axial(1, 0)]),
                    (
                        UnitId(2),
                        &[
                            HexCoord::from_axial(-1, 0),
                            HexCoord::ORIGIN,
                            HexCoord::from_axial(1, 0),
                        ],
                    ),
                ],
            ),
            CommandRefusal::PartyMove {
                reason: PartyMoveRefusal::DuplicateDestination {
                    destination: TilePos::new(HexCoord::from_axial(1, 0), GROUND_LEVEL),
                },
            },
        ),
    ];

    for (seat, command, expected) in cases {
        let (mut app, anchor, rear) = pair_party_app();
        let before = (standing_of(&mut app, anchor), standing_of(&mut app, rear));
        push_as(&mut app, seat, command.clone());
        app.update();
        assert_eq!(
            take_events(&mut app),
            vec![CombatEvent::CommandRefused {
                command,
                refusal: expected,
            }]
        );
        assert_eq!(
            (standing_of(&mut app, anchor), standing_of(&mut app, rear)),
            before,
            "no member may move when any validation fails"
        );
        let mut moving = app
            .world_mut()
            .query_filtered::<Entity, Or<(With<Busy>, With<hex_units::MovingTo>)>>();
        assert_eq!(moving.iter(app.world()).count(), 0);
    }
}

#[test]
fn a_wrongly_owned_party_member_rejects_the_whole_move() {
    let (mut app, anchor, rear) = pair_party_app();
    app.world_mut()
        .entity_mut(rear)
        .insert(ControlOwner(PlayerSeat(1)));
    let command = party_command(
        UnitId(1),
        vec![
            (UnitId(1), &[HexCoord::ORIGIN, HexCoord::from_axial(1, 0)]),
            (UnitId(2), &[HexCoord::from_axial(-1, 0), HexCoord::ORIGIN]),
        ],
    );
    push(&mut app, command.clone());
    app.update();
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::CommandRefused {
            command,
            refusal: CommandRefusal::WrongSeat {
                issued_by: PlayerSeat(0),
                owned_by: PlayerSeat(1),
            },
        }]
    );
    assert_eq!(
        standing_of(&mut app, anchor),
        Some(TilePos::new(HexCoord::ORIGIN, GROUND_LEVEL))
    );
    assert_eq!(
        standing_of(&mut app, rear),
        Some(TilePos::new(HexCoord::from_axial(-1, 0), GROUND_LEVEL))
    );
}

#[test]
fn a_party_route_cannot_enter_a_nonparty_body() {
    let (mut app, anchor, rear) = pair_party_app();
    let destination = HexCoord::from_axial(1, 0);
    spawn_unit(&mut app, Faction::Player, destination, 5, 3);
    let command = party_command(
        UnitId(1),
        vec![
            (UnitId(1), &[HexCoord::ORIGIN, destination]),
            (UnitId(2), &[HexCoord::from_axial(-1, 0), HexCoord::ORIGIN]),
        ],
    );
    push(&mut app, command.clone());
    app.update();
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::CommandRefused {
            command,
            refusal: CommandRefusal::PartyMove {
                reason: PartyMoveRefusal::Occupied {
                    block: OccupancyBlock::Destination {
                        position: TilePos::new(destination, GROUND_LEVEL),
                        occupant: UnitId(3),
                    },
                },
            },
        }]
    );
    assert_eq!(
        standing_of(&mut app, anchor),
        Some(TilePos::new(HexCoord::ORIGIN, GROUND_LEVEL))
    );
    assert_eq!(
        standing_of(&mut app, rear),
        Some(TilePos::new(HexCoord::from_axial(-1, 0), GROUND_LEVEL))
    );
}

#[test]
fn combat_interrupts_every_party_member_on_a_whole_surface() {
    let (mut app, anchor, rear) = pair_party_app();
    push(
        &mut app,
        party_command(
            UnitId(1),
            vec![
                (
                    UnitId(1),
                    &[
                        HexCoord::ORIGIN,
                        HexCoord::from_axial(1, 0),
                        HexCoord::from_axial(2, 0),
                    ],
                ),
                (
                    UnitId(2),
                    &[
                        HexCoord::from_axial(-1, 0),
                        HexCoord::ORIGIN,
                        HexCoord::from_axial(1, 0),
                    ],
                ),
            ],
        ),
    );
    app.update();
    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();

    for entity in [anchor, rear] {
        assert!(
            app.world().get::<hex_units::MovingTo>(entity).is_none(),
            "combat should remove every in-flight party route"
        );
        assert!(
            app.world()
                .get::<hex_anim::Transformation>(entity)
                .is_none(),
            "combat should remove every in-flight party animation"
        );
        let standing = standing_of(&mut app, entity).expect("the member remains grounded");
        let mut tiles = app.world_mut().query_filtered::<&TilePos, With<HexTile>>();
        assert!(
            tiles
                .iter(app.world())
                .any(|position| *position == standing),
            "the interrupted member must land on an exact live surface"
        );
    }
}

/// The same command sequence from the same spawn state lands the same world.
///
/// This is the funnel's reason to exist: every sim mutation flows through the
/// drained queue, so the sequence *is* the input, and applying it twice must
/// be indistinguishable — same turn order, positions, budgets, successful commands,
/// and structured events.
#[test]
fn a_replayed_sequence_lands_identically() {
    let script = [
        GameCommand::MoveAlong {
            unit: UnitId(1),
            path: path(&[HexCoord::ORIGIN, HexCoord::new_cubic(1, -1, 0)]),
        },
        GameCommand::EndTurn { unit: UnitId(1) },
        GameCommand::EndTurn { unit: UnitId(2) },
        GameCommand::MoveAlong {
            unit: UnitId(1),
            path: path(&[HexCoord::new_cubic(1, -1, 0), HexCoord::new_cubic(1, 0, -1)]),
        },
    ];

    let run = || {
        let mut app = test_app();
        let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
        let hostile = spawn_unit(
            &mut app,
            Faction::Hostile,
            HexCoord::new_cubic(2, -2, 0),
            10,
            2,
        );
        enter_gameplay(&mut app);
        assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");

        for command in script.clone() {
            push(&mut app, command);
            settle(&mut app);
        }

        let order = app.world().resource::<TurnOrder>();
        (
            order.order().to_vec(),
            order.current(),
            order.round,
            standing_of(&mut app, player),
            standing_of(&mut app, hostile),
            budget_of(&app, player),
            app.world().resource::<CombatSummary>().clone(),
        )
    };

    assert_eq!(run(), run(), "a replay must not diverge");
}

#[test]
fn channel_restores_named_mana_and_spends_exactly_one_action() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        2,
    );
    insert_depleted_channel_lattice(&mut app, player);
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");

    let command = GameCommand::Channel { unit: UnitId(1) };
    push(&mut app, command.clone());
    app.update();
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::Channelled {
            unit: UnitId(1),
            restored: BTreeMap::from([("Fire".to_owned(), 2)]),
        }]
    );
    assert!(
        app.world()
            .get::<Turn>(player)
            .is_some_and(|turn| turn.acted),
        "Channel consumes the acting unit's one action"
    );
    let summary = app.world().resource::<CombatSummary>();
    assert_eq!(summary.channels, 1);
    assert_eq!(summary.channelled_mana.get("Fire"), Some(&2));

    push(&mut app, command.clone());
    app.update();
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::CommandRefused {
            command,
            refusal: CommandRefusal::ActionAlreadySpent,
        }],
        "a repeated Channel cannot grant or spend another action"
    );
}

#[test]
fn a_downed_unit_receives_the_exact_channel_refusal() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        2,
    );
    spawn_unit(
        &mut app,
        Faction::Player,
        HexCoord::new_cubic(1, -1, 0),
        5,
        3,
    );
    insert_depleted_channel_lattice(&mut app, player);
    app.world_mut().entity_mut(player).insert(Downed);
    enter_gameplay(&mut app);

    let command = GameCommand::Channel { unit: UnitId(1) };
    push(&mut app, command.clone());
    app.update();
    let events = take_events(&mut app);
    assert_eq!(
        events.first(),
        Some(&CombatEvent::CommandRefused {
            command,
            refusal: CommandRefusal::ActingUnitDowned { unit: UnitId(1) },
        }),
        "the refusal is emitted before the resulting terminal outcome"
    );
}

/// A command from a unit that is not acting is refused, not deferred.
#[test]
fn an_end_turn_from_the_wrong_unit_is_dropped() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        2,
    );
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(UnitId(1)),
        "precondition: the player acts first"
    );

    let refused = GameCommand::EndTurn { unit: UnitId(2) };
    push(&mut app, refused.clone());
    app.update();
    app.update();

    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(UnitId(1)),
        "somebody else's end-turn must not pass the player's turn"
    );
    assert!(
        app.world().get::<Turn>(player).is_some(),
        "the player should still hold the turn marker"
    );
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::CommandRefused {
            command: refused,
            refusal: CommandRefusal::NotCurrentTurn {
                current: Some(UnitId(1)),
            },
        }]
    );
}

/// A path that teleports is refused whole; a command applies entirely or not
/// at all.
#[test]
fn an_unwalkable_path_is_dropped() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        2,
    );
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");

    // Origin to three hexes out in one "step".
    let refused = GameCommand::MoveAlong {
        unit: UnitId(1),
        path: path(&[HexCoord::ORIGIN, HexCoord::new_cubic(3, -3, 0)]),
    };
    push(&mut app, refused.clone());
    app.update();
    app.update();

    assert_eq!(
        standing_of(&mut app, player),
        Some(TilePos::new(HexCoord::ORIGIN, GROUND_LEVEL)),
        "an unwalkable path must not move the piece"
    );
    assert_eq!(
        budget_of(&app, player),
        Some(4),
        "a refused path must not be billed"
    );
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::CommandRefused {
            command: refused,
            refusal: CommandRefusal::InvalidPath,
        }]
    );
}

/// A path longer than the remaining budget is refused before anything moves.
#[test]
fn an_over_budget_path_is_dropped() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        2,
    );
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");

    // Five adjacent steps against a budget of four.
    let refused = GameCommand::MoveAlong {
        unit: UnitId(1),
        path: path(&[
            HexCoord::ORIGIN,
            HexCoord::new_cubic(0, 1, -1),
            HexCoord::new_cubic(0, 2, -2),
            HexCoord::new_cubic(0, 3, -3),
            HexCoord::new_cubic(0, 4, -4),
            HexCoord::new_cubic(0, 5, -5),
        ]),
    };
    push(&mut app, refused.clone());
    app.update();
    app.update();

    assert_eq!(
        standing_of(&mut app, player),
        Some(TilePos::new(HexCoord::ORIGIN, GROUND_LEVEL)),
        "an over-budget path must not move the piece"
    );
    assert_eq!(
        budget_of(&app, player),
        Some(4),
        "a refused path must not be billed"
    );
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::CommandRefused {
            command: refused,
            refusal: CommandRefusal::MovementBudgetExceeded {
                cost: 5,
                remaining: 4,
            },
        }]
    );
}

/// A command missing required runtime content is refused and changes nothing.
#[test]
fn an_unbuilt_verb_is_dropped_and_changes_nothing() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        2,
    );
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");

    let refused = GameCommand::Cast {
        unit: UnitId(1),
        spell: "Ember".to_owned(),
        target: TilePos::new(HexCoord::ORIGIN, GROUND_LEVEL),
        facing: None,
        mana: None,
    };
    push(&mut app, refused.clone());
    app.update();
    app.update();

    assert!(
        app.world().resource::<CommandQueue>().is_empty(),
        "the unbuilt verb should have been drained"
    );
    assert_eq!(
        budget_of(&app, player),
        Some(4),
        "an unbuilt verb must change nothing"
    );
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(UnitId(1)),
        "an unbuilt verb must not consume the turn"
    );
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::CommandRefused {
            command: refused,
            refusal: CommandRefusal::MissingCombatData {
                data: CombatData::SpellBook,
            },
        }]
    );
}

/// One command per unit per presentation: the second move in a single drain is
/// refused and, above all, never billed.
///
/// This is the applier-side half of the old double-charge bug. The click
/// emitter also suppresses mid-walk clicks, but the budget lives here, so the
/// authoritative guard has to hold even for emitters that forget.
#[test]
fn a_busy_unit_cannot_start_a_second_move() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        2,
    );
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");

    let first = HexCoord::new_cubic(1, -1, 0);
    push(
        &mut app,
        GameCommand::MoveAlong {
            unit: UnitId(1),
            path: path(&[HexCoord::ORIGIN, first]),
        },
    );
    push(
        &mut app,
        GameCommand::MoveAlong {
            unit: UnitId(1),
            path: path(&[HexCoord::ORIGIN, HexCoord::new_cubic(0, 1, -1)]),
        },
    );
    settle(&mut app);

    assert_eq!(
        standing_of(&mut app, player),
        Some(TilePos::new(first, GROUND_LEVEL)),
        "only the first move should have been committed"
    );
    assert_eq!(
        budget_of(&app, player),
        Some(3),
        "exactly one step should have been billed"
    );
}

/// The ownership check is real: a seat cannot command another seat's unit.
///
/// Every shipped unit is seat 0 today, so this is the one place the co-op
/// seam is exercised at all — the branch must hold before it ever matters.
#[test]
fn a_command_from_the_wrong_seat_is_dropped() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        2,
    );
    // The acting unit belongs to seat 1 in this session.
    app.world_mut()
        .entity_mut(player)
        .insert(ControlOwner(PlayerSeat(1)));
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");

    push_as(
        &mut app,
        PlayerSeat(0),
        GameCommand::EndTurn { unit: UnitId(1) },
    );
    app.update();
    app.update();

    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(UnitId(1)),
        "a seat that does not own the unit must not end its turn"
    );

    push_as(
        &mut app,
        PlayerSeat(1),
        GameCommand::EndTurn { unit: UnitId(1) },
    );
    settle(&mut app);

    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(UnitId(2)),
        "the owning seat's identical command should pass the turn"
    );
}

/// The rules live in the applier: allies cannot be made to swing at each
/// other, whatever a forged or replayed log claims.
#[test]
fn a_strike_on_a_friendly_unit_is_dropped() {
    let mut app = test_app();
    let striker = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    let ally = spawn_unit(
        &mut app,
        Faction::Player,
        HexCoord::new_cubic(1, -1, 0),
        10,
        2,
    );
    // A hostile close enough to start the fight, far enough to stay out of
    // the strike under test.
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(3, -3, 0),
        5,
        3,
    );
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(UnitId(1)),
        "precondition: the striker acts first"
    );

    push(
        &mut app,
        GameCommand::Strike {
            unit: UnitId(1),
            target: UnitId(2),
        },
    );
    app.update();
    app.update();

    let turn = app
        .world()
        .get::<Turn>(striker)
        .expect("the striker should still hold its turn");
    assert!(!turn.acted, "a refused strike must not consume the action");
    assert!(
        app.world().get::<hex_anim::Transformation>(ally).is_none(),
        "the ally must not have been made to recoil"
    );
}

/// Downing keeps an entity registered for restoration, but it does not leave that
/// entity as a legal damage sink for forged or replayed commands.
#[test]
fn a_strike_on_a_downed_unit_is_dropped_without_spending_the_action() {
    let mut app = test_app();
    let striker = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    let downed = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(1, -1, 0),
        10,
        2,
    );
    app.world_mut().entity_mut(downed).insert(Downed);
    // A live hostile keeps combat active after the downed target is excluded.
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        5,
        3,
    );
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");

    let command = GameCommand::Strike {
        unit: UnitId(1),
        target: UnitId(2),
    };
    push(&mut app, command.clone());
    app.update();

    let turn = app
        .world()
        .get::<Turn>(striker)
        .expect("the striker should still hold its turn");
    assert!(!turn.acted, "a refused strike must not consume the action");
    assert!(
        app.world()
            .get::<hex_anim::Transformation>(downed)
            .is_none(),
        "the downed target must not be made to recoil"
    );
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::CommandRefused {
            command,
            refusal: CommandRefusal::TargetDowned { target: UnitId(2) },
        }]
    );
}

/// The emitter's route vocabulary and the applier's grounding agree.
///
/// The click observer commits nothing itself, so a disagreement between
/// `route`'s output and `ground_path`'s acceptance would surface only as a
/// warned drop and a dead click in game. Feeding a real routed path through
/// the applier pins the seam headlessly.
#[test]
fn a_routed_path_grounds_and_applies() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        2,
    );
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");

    // Exactly what the click observer does: resolve footing, route to the
    // clicked surface, and emit the step positions.
    let destination = HexCoord::new_cubic(0, 2, -2);
    let path: Vec<TilePos> = {
        let body = *app
            .world()
            .get::<Body>(player)
            .expect("the player has a body");
        let from = app
            .world()
            .get::<StandsOn>(player)
            .expect("the player stands somewhere")
            .0;
        let mut tiles = app
            .world_mut()
            .query_filtered::<(&TilePos, &HexSpan, &SubstanceId, &Headroom), With<HexTile>>();
        let world = app.world();
        let footing = Footing::from_tiles(
            tiles.iter(world),
            world.resource::<SubstanceTable>(),
            body,
            None,
        );
        let to = footing
            .at(TilePos::new(destination, GROUND_LEVEL))
            .expect("the destination is standable");
        route(from, to, &footing)
            .expect("open ground routes")
            .iter()
            .map(|step| step.pos)
            .collect()
    };

    push(
        &mut app,
        GameCommand::MoveAlong {
            unit: UnitId(1),
            path,
        },
    );
    settle(&mut app);

    assert_eq!(
        standing_of(&mut app, player),
        Some(TilePos::new(destination, GROUND_LEVEL)),
        "the routed path should ground and land"
    );
    assert_eq!(
        budget_of(&app, player),
        Some(2),
        "two routed steps should bill two"
    );
}
