//! Renderer-free composition contracts for spell-to-terrain resolution.
//!
//! This target deliberately installs only the real map, unit, perception, and combat
//! plugins over a tiny deterministic fixture. It never installs `AppPlugin`, a window,
//! a renderer, a viewport, `hex_ui::UiPlugin`, gameplay-app support, screenshots, or a
//! visual walk. The assertions exercise simulation facts that pixels cannot prove:
//! impact correlation, terrain publication, actor settlement, exact authority
//! adoption, and release ordering.

#![expect(
    clippy::expect_used,
    reason = "invalid compile-time fixtures should fail these contracts immediately"
)]

use std::collections::BTreeMap;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use hex_assets::{
    ArtPalette, CastingAxis, CombatSettings, ContentIndex, Effect, ElementCatalog, ElementFile,
    Encounter, GameAssets, GemRequirement, ManaAxis, PerceptionSettings, PlayerSettings, Spell,
    SpellBook, SpellFile, SubstanceFile, SubstanceTable, TargetShape, TargetingSpec,
    TerrainDamageFile, TerrainDamagePair, TerrainDamageTable, Trajectory,
};
use hex_combat::{
    Initiative, PersistentEffects, SpellResolutionFailure, SpellResolutionState,
    SpellResolutionStatus, TurnOrder,
};
use hex_core::{
    AppSystems, CommandQueue, ControlOwner, ExteriorIllumination, GameCommand, GameplayPhase,
    GameplaySetup, HexCoord, IlluminationLevel, IssuedCommand, LatticeCoord, Mode, PausableSystems,
    PendingDecision, PerceptionSystems, PlayerSeat, Screen, TerrainBatchId, TerrainImpact,
    TerrainImpactOutcome, TerrainImpactRejection, TerrainImpactResult, TilePos, TraversalProfile,
    Turn, UnitId,
};
use hex_lattice::{CellKind, LatticeSpec, LatticeState, LatticeStats};
use hex_map::{MapSettings, PerlinSettings, TerrainSettings, VoxelMap};
use hex_test_support::{enter_gameplay, TestAppBuilder};
use hex_units::{Body, Faction, Party, Player, Standing, StandsOn, UnitRegistry};

const CASTER: UnitId = UnitId(1);
const DEFENDER: UnitId = UnitId(2);
const SURFACE_LEVEL: i32 = 1;
const CASTER_COORD: HexCoord = HexCoord::ORIGIN;
const DEFENDER_COORD: HexCoord = HexCoord::new_cubic(1, -1, 0);

const IMPACT: &str = "Impact";
const IMPACT_AND_DISABLE: &str = "Impact and Disable";
const DOUBLE_IMPACT: &str = "Double Impact";
const SCORCH: &str = "Scorch";

#[derive(Resource)]
struct FixtureSpell(String);

fn target() -> TilePos {
    TilePos::new(DEFENDER_COORD, SURFACE_LEVEL)
}

fn caster_surface() -> TilePos {
    TilePos::new(CASTER_COORD, SURFACE_LEVEL)
}

fn elements() -> ElementCatalog {
    let file = ElementFile {
        wheel: vec!["Fire".to_owned(), "Water".to_owned()],
        fusions: HashMap::default(),
    };
    file.validate()
        .expect("the two-element fixture should be valid");
    ElementCatalog::from_file(&file)
}

fn impact() -> Effect {
    Effect::Impact {
        element: "Fire".to_owned(),
        power: 1,
    }
}

fn spell(effects: Vec<Effect>) -> Spell {
    Spell {
        requirements: vec![GemRequirement {
            element: "Fire".to_owned(),
            mana: 1,
        }],
        casting: CastingAxis::Evocation,
        mana: ManaAxis::Fixed,
        co_castable: false,
        targeting: TargetingSpec {
            range: 2,
            shape: TargetShape::Single,
            trajectory: Trajectory::None,
        },
        effects,
    }
}

fn spells() -> SpellBook {
    let file = SpellFile {
        spells: HashMap::from_iter([
            (IMPACT.to_owned(), spell(vec![impact()])),
            (
                IMPACT_AND_DISABLE.to_owned(),
                spell(vec![
                    impact(),
                    Effect::DisableHexes {
                        count: 1,
                        targeted: false,
                    },
                ]),
            ),
            (DOUBLE_IMPACT.to_owned(), spell(vec![impact(), impact()])),
            (
                SCORCH.to_owned(),
                spell(vec![impact(), Effect::Burn { turns: 1 }]),
            ),
        ]),
    };
    file.validate()
        .expect("the spell-resolution fixture should be valid");
    SpellBook::from_file(&file)
}

fn authored_assets() -> (ArtPalette, SubstanceTable) {
    let palette: ArtPalette = ron::from_str(include_str!("../../../assets/art/palette.ron"))
        .expect("the tracked art palette should parse");
    let substances: SubstanceFile =
        ron::from_str(include_str!("../../../assets/config/substances.ron"))
            .expect("the tracked substance catalog should parse");
    let table = SubstanceTable::from_file(&substances, &palette)
        .expect("the tracked substances should resolve through the tracked palette");
    (palette, table)
}

fn damage_content(
    elements: &ElementCatalog,
    substances: &SubstanceTable,
    damages_grass: bool,
) -> (TerrainDamageFile, TerrainDamageTable) {
    let damaging_pairs = if damages_grass {
        vec![TerrainDamagePair {
            element: "Fire".to_owned(),
            substance: "grass".to_owned(),
        }]
    } else {
        Vec::new()
    };
    let file = TerrainDamageFile { damaging_pairs };
    let table = TerrainDamageTable::from_file(&file, elements, substances)
        .expect("the terrain-damage fixture should resolve");
    (file, table)
}

fn caster_lattice(
    book: &SpellBook,
    elements: &ElementCatalog,
    name: &str,
) -> (LatticeSpec, LatticeState, LatticeStats) {
    let spell = book
        .id(name)
        .expect("the selected fixture spell should exist");
    let fire = elements
        .id("Fire")
        .expect("the fixture should resolve Fire");
    let spec = LatticeSpec::default()
        .with(LatticeCoord::ORIGIN, CellKind::Spell { spell })
        .with(LatticeCoord::new(1, 0), CellKind::Gem { element: fire });
    let stats = LatticeStats::new(BTreeMap::from([(fire, 6)]), BTreeMap::new());
    let state = LatticeState::new(&spec, &stats);
    (spec, state, stats)
}

fn defender_lattice() -> (LatticeSpec, LatticeState, LatticeStats) {
    let spec = LatticeSpec::default()
        .with(LatticeCoord::ORIGIN, CellKind::Blank)
        .with(LatticeCoord::new(1, 0), CellKind::Blank)
        .with(LatticeCoord::new(-1, 0), CellKind::Blank);
    let stats = LatticeStats::new(BTreeMap::new(), BTreeMap::new());
    let state = LatticeState::new(&spec, &stats);
    (spec, state, stats)
}

fn fixture_standing(
    coord: HexCoord,
    tiles: &Query<(&TilePos, &hex_core::HexSpan, &hex_core::Headroom), With<hex_core::HexTile>>,
) -> Standing {
    tiles
        .iter()
        .filter(|(position, _, headroom)| position.coord == coord && headroom.0 > 0)
        .map(|(position, span, _)| Standing {
            pos: *position,
            span: *span,
        })
        .max_by_key(|standing| standing.pos)
        .expect("the flat Perlin fixture should publish the requested surface")
}

fn spawn_fixture_units(
    mut commands: Commands,
    fixture: Res<FixtureSpell>,
    book: Res<SpellBook>,
    elements: Res<ElementCatalog>,
    tiles: Query<(&TilePos, &hex_core::HexSpan, &hex_core::Headroom), With<hex_core::HexTile>>,
    mut registry: ResMut<UnitRegistry>,
    mut party: ResMut<Party>,
) {
    let caster_standing = fixture_standing(CASTER_COORD, &tiles);
    let defender_standing = fixture_standing(DEFENDER_COORD, &tiles);
    assert_eq!(caster_standing.pos, caster_surface());
    assert_eq!(defender_standing.pos, target());

    let (caster_spec, caster_state, caster_stats) = caster_lattice(&book, &elements, &fixture.0);
    let caster = commands
        .spawn((
            CASTER,
            Faction::Player,
            Player,
            ControlOwner::default(),
            Body::new(TraversalProfile::WALKER),
            Initiative(20),
            StandsOn(caster_standing),
            Transform::from_translation(caster_standing.world_position()),
            caster_spec,
            caster_state,
            caster_stats,
        ))
        .id();
    registry.register(CASTER, caster);
    party.members.push(CASTER);

    let (defender_spec, defender_state, defender_stats) = defender_lattice();
    let defender = commands
        .spawn((
            DEFENDER,
            Faction::Hostile,
            // The fixture scripts exact defender answers. Marking it human prevents
            // the headless AI policy from answering before an assertion can inspect
            // the modal seam; faction truth remains Hostile.
            Player,
            ControlOwner::default(),
            Body::new(TraversalProfile::WALKER),
            Initiative(10),
            StandsOn(defender_standing),
            Transform::from_translation(defender_standing.world_position()),
            defender_spec,
            defender_state,
            defender_stats,
        ))
        .id();
    registry.register(DEFENDER, defender);
}

fn test_app(selected_spell: &str, damages_grass: bool) -> App {
    let mut builder = TestAppBuilder::new();
    let app = builder.app_mut();

    let (palette, substances) = authored_assets();
    let elements = elements();
    let spells = spells();
    let content = ContentIndex::build(&elements, &spells, &substances)
        .expect("the fixture content should resolve");
    let (damage_file, damage_table) = damage_content(&elements, &substances, damages_grass);

    app.init_resource::<GameplayPhase>()
        .insert_resource(GameAssets {
            hex_tile: Handle::default(),
            player_pieces: [Handle::default(), Handle::default()],
        })
        .insert_resource(palette)
        .insert_resource(substances)
        .insert_resource(elements)
        .insert_resource(spells)
        .insert_resource(content)
        .insert_resource(damage_file)
        .insert_resource(damage_table)
        .insert_resource(MapSettings {
            grid_radius: 2,
            level_height: 1.0,
            terrain: TerrainSettings::Perlin(PerlinSettings {
                seed: Some(20_260_802),
                // Empty octaves are the deterministic flat case, while still going
                // through the real Perlin map producer and publication path.
                steps: Vec::new(),
            }),
        })
        .insert_resource(PlayerSettings {
            scale: 0.25,
            speed: 5.0,
        })
        .insert_resource(CombatSettings::default())
        .insert_resource(PerceptionSettings::default())
        .insert_resource(ExteriorIllumination::new(IlluminationLevel::Bright))
        .insert_resource(Encounter {
            name: "Spell resolution composition".to_owned(),
            // Runtime actor systems are installed, but the two exact actors are
            // authored in Restore so perception observes them in its first pass.
            rosters: Vec::new(),
        })
        .insert_resource(FixtureSpell(selected_spell.to_owned()));

    app.configure_sets(
        Update,
        PausableSystems
            .run_if(in_state(hex_core::Pause(false)))
            .run_if(resource_equals(GameplayPhase::Active)),
    );
    app.configure_sets(
        Update,
        (
            PerceptionSystems::PublishAmbient,
            PerceptionSystems::ResolveIllumination,
            PerceptionSystems::ResolveObservation,
            PerceptionSystems::PublishKnowledge,
            PerceptionSystems::ApplyPresentation,
        )
            .chain()
            .in_set(AppSystems::Update),
    );
    app.configure_sets(
        OnEnter(Screen::Gameplay),
        (
            PerceptionSystems::PublishAmbient,
            PerceptionSystems::ResolveIllumination,
            PerceptionSystems::ResolveObservation,
            PerceptionSystems::PublishKnowledge,
            PerceptionSystems::ApplyPresentation,
        )
            .chain()
            .in_set(GameplaySetup::Perception),
    );
    app.add_plugins((
        hex_map::plugin,
        hex_units::plugin,
        hex_perception::plugin,
        hex_combat::plugin,
    ));
    app.add_systems(
        OnEnter(Screen::Gameplay),
        spawn_fixture_units.in_set(GameplaySetup::Restore),
    );

    let mut app = builder.build();
    enter_gameplay(&mut app);
    assert_eq!(app.world().resource::<State<Mode>>().get(), &Mode::Combat);
    let authority = hex_combat::authority_snapshot(app.world())
        .expect("the real published fixture should initialize combat authority");
    assert_eq!(authority.current(), Some(CASTER));
    app
}

fn push(app: &mut App, command: GameCommand) {
    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat::default(),
            command,
        });
}

fn cast(app: &mut App, spell: &str, target: TilePos) {
    push(
        app,
        GameCommand::Cast {
            unit: CASTER,
            spell: spell.to_owned(),
            target,
            facing: None,
            mana: None,
        },
    );
    app.update();
}

fn standing(app: &App, unit: UnitId) -> TilePos {
    let entity = app
        .world()
        .resource::<UnitRegistry>()
        .entity_of(unit)
        .expect("the fixture unit should remain registered");
    app.world()
        .get::<StandsOn>(entity)
        .expect("the fixture unit should remain on an exact surface")
        .0
        .pos
}

fn disabled_cells(app: &App, unit: UnitId) -> Vec<LatticeCoord> {
    let entity = app
        .world()
        .resource::<UnitRegistry>()
        .entity_of(unit)
        .expect("the fixture unit should remain registered");
    let spec = app
        .world()
        .get::<LatticeSpec>(entity)
        .expect("the fixture unit should retain its lattice spec");
    let state = app
        .world()
        .get::<LatticeState>(entity)
        .expect("the fixture unit should retain its lattice state");
    spec.cells()
        .filter_map(|(coord, _)| state.is_disabled(coord).then_some(coord))
        .collect()
}

fn assert_authority_position(app: &App, unit: UnitId, expected: TilePos) {
    let authority = hex_combat::authority_snapshot(app.world())
        .expect("combat authority should remain available");
    assert_eq!(
        authority.units.get(&unit).map(|actor| actor.position),
        Some(expected)
    );
}

fn assert_released(app: &App) {
    assert_eq!(
        app.world().resource::<SpellResolutionState>().status(),
        SpellResolutionStatus::Idle
    );
    let authority = hex_combat::authority_snapshot(app.world())
        .expect("combat authority should remain available");
    assert!(!authority.external_resolution_is_held());
}

fn take_impacts(app: &mut App) -> Vec<TerrainImpact> {
    app.world_mut()
        .resource_mut::<Messages<TerrainImpact>>()
        .drain()
        .collect()
}

fn replay_impacts_reversed(app: &mut App, impacts: &[TerrainImpact]) {
    for impact in impacts.iter().rev().cloned() {
        app.world_mut().write_message(impact);
    }
}

#[test]
fn applied_impact_rebuilds_occupancy_settles_the_actor_and_releases_exact_authority() {
    let mut app = test_app(IMPACT, true);
    assert_eq!(standing(&app, DEFENDER), target());

    cast(&mut app, IMPACT, target());
    assert!(matches!(
        app.world().resource::<SpellResolutionState>().status(),
        SpellResolutionStatus::Pending {
            pending_terrain_batches: 1,
            ..
        }
    ));
    assert!(hex_combat::authority_snapshot(app.world())
        .expect("authority should remain inspectable while held")
        .external_resolution_is_held());

    app.update();

    let landed = TilePos::new(DEFENDER_COORD, 0);
    assert_eq!(standing(&app, DEFENDER), landed);
    assert_authority_position(&app, DEFENDER, landed);
    assert!(app.world().resource::<VoxelMap>().get(target()).is_air());
    assert_released(&app);
}

#[test]
fn terrain_unavailable_rejection_releases_without_tentative_settlement() {
    let mut app = test_app(IMPACT, true);
    app.world_mut().remove_resource::<TerrainDamageTable>();

    cast(&mut app, IMPACT, target());
    assert!(matches!(
        app.world().resource::<SpellResolutionState>().status(),
        SpellResolutionStatus::Pending { .. }
    ));
    app.update();

    assert_eq!(standing(&app, DEFENDER), target());
    assert_authority_position(&app, DEFENDER, target());
    assert!(!app.world().resource::<VoxelMap>().get(target()).is_air());
    assert_released(&app);
}

#[test]
fn foreign_answer_freezes_and_an_outstanding_defender_answer_cannot_mutate() {
    let mut app = test_app(IMPACT_AND_DISABLE, false);
    cast(&mut app, IMPACT_AND_DISABLE, target());

    let pending = PendingDecision::ChooseDisables {
        decider: DEFENDER,
        count: 1,
        source: CASTER,
    };
    assert_eq!(*app.world().resource::<PendingDecision>(), pending);
    assert!(disabled_cells(&app, DEFENDER).is_empty());

    app.world_mut().write_message(TerrainImpactOutcome {
        batch: TerrainBatchId(99),
        result: TerrainImpactResult::Rejected(TerrainImpactRejection::TerrainUnavailable),
    });
    app.update();

    assert_eq!(
        app.world().resource::<SpellResolutionState>().status(),
        SpellResolutionStatus::Frozen(SpellResolutionFailure::ForeignOutcome {
            batch: TerrainBatchId(99),
        })
    );
    assert_eq!(*app.world().resource::<PendingDecision>(), pending);

    push(
        &mut app,
        GameCommand::ChooseDisables {
            unit: DEFENDER,
            cells: vec![LatticeCoord::ORIGIN],
        },
    );
    app.update();

    assert!(disabled_cells(&app, DEFENDER).is_empty());
    assert_eq!(*app.world().resource::<PendingDecision>(), pending);
    assert!(hex_combat::authority_snapshot(app.world())
        .expect("frozen authority evidence should remain inspectable")
        .external_resolution_is_held());
}

#[test]
fn reversed_multi_batch_answers_and_a_later_cast_never_reuse_settlement_adoption() {
    let mut app = test_app(DOUBLE_IMPACT, true);

    cast(&mut app, DOUBLE_IMPACT, target());
    let first = take_impacts(&mut app);
    assert_eq!(
        first.iter().map(|impact| impact.batch).collect::<Vec<_>>(),
        vec![TerrainBatchId(0), TerrainBatchId(1)]
    );
    replay_impacts_reversed(&mut app, &first);
    app.update();

    let defender_landing = TilePos::new(DEFENDER_COORD, 0);
    assert_eq!(standing(&app, DEFENDER), defender_landing);
    assert_authority_position(&app, DEFENDER, defender_landing);
    assert_released(&app);

    push(&mut app, GameCommand::EndTurn { unit: CASTER });
    app.update();
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(DEFENDER)
    );
    push(&mut app, GameCommand::EndTurn { unit: DEFENDER });
    app.update();
    assert_eq!(app.world().resource::<TurnOrder>().current(), Some(CASTER));

    cast(&mut app, DOUBLE_IMPACT, caster_surface());
    let second = take_impacts(&mut app);
    assert_eq!(
        second.iter().map(|impact| impact.batch).collect::<Vec<_>>(),
        vec![TerrainBatchId(2), TerrainBatchId(3)]
    );
    replay_impacts_reversed(&mut app, &second);
    app.update();

    let caster_landing = TilePos::new(CASTER_COORD, 0);
    assert_eq!(standing(&app, CASTER), caster_landing);
    assert_authority_position(&app, CASTER, caster_landing);
    assert_authority_position(&app, DEFENDER, defender_landing);
    assert_released(&app);
}

#[test]
fn terrain_release_precedes_end_turn_and_the_next_actors_start_effect() {
    let mut app = test_app(SCORCH, false);

    cast(&mut app, SCORCH, target());
    assert_eq!(app.world().resource::<PersistentEffects>().len(), 1);
    push(&mut app, GameCommand::EndTurn { unit: CASTER });

    // The map answers and ConsumeOutcomes releases first. The already-queued EndTurn
    // then reduces in CombatSystems::Apply in this same update.
    app.update();
    assert_released(&app);
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(DEFENDER)
    );

    // The newly granted turn is a deferred projection. Its Added<Turn> edge opens the
    // burn decision on the following bounded gameplay update, before any actor runs.
    app.update();
    assert_eq!(
        *app.world().resource::<PendingDecision>(),
        PendingDecision::ChooseDisables {
            decider: DEFENDER,
            count: 1,
            source: CASTER,
        }
    );
    assert!(disabled_cells(&app, DEFENDER).is_empty());
    let defender_entity = app
        .world()
        .resource::<UnitRegistry>()
        .entity_of(DEFENDER)
        .expect("the defender should remain registered");
    assert!(app.world().get::<Turn>(defender_entity).is_some());
}
