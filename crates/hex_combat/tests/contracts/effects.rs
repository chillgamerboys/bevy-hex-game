//! Contract tests for persistent effects: lighting a fire, and collecting on it.
//!
//! These drive the real applier over the real command queue, with a hand-built content
//! set standing in for `spells.ron`. What they prove is the part no unit test can: that
//! a burn **lands on a later turn than the cast that started it**, that it arrives at the
//! start of the *target's* turn rather than on the round boundary, and that it reaches
//! the lattice through the same defender-chooses seam as any other damage while ignoring
//! the armour that would have stopped a spell.
//!
//! The content is built here rather than loaded, because this crate cannot see asset
//! files and should not need to: what is under test is the runtime, not the roster.

use std::collections::BTreeMap;
use std::time::Duration;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use hex_assets::{
    CastingAxis, CombatSettings, ContentIndex, Effect, ElementCatalog, ElementFile, GemRequirement,
    ManaAxis, Spell, SpellBook, SpellFile, SubstanceTable, TargetShape, TargetingReach,
    TargetingSpec, Trajectory,
};
use hex_combat::{
    CombatEvent, CommandRefusal, FactionLatticeKnowledge, Initiative, PersistentEffects,
    RestorationTargetRefusal, TurnOrder,
};
use hex_combat_core::FrozenTargeting;
use hex_core::{
    CommandQueue, ControlOwner, EffectEnd, EffectPayload, GameCommand, Headroom, HexCoord, HexSpan,
    HexTile, IssuedCommand, LatticeCoord, LightDomain, Mode, PendingDecision, PlayerSeat,
    RunBottom, Screen, SubstanceId, TerrainEdit, TilePos, TraversalProfile, Turn, UnitId,
};
use hex_lattice::{apply_cast, castable, CellKind, LatticeSpec, LatticeState, LatticeStats};
use hex_perception::{
    apply_observations, FactionMapKnowledge, FactionObservation, FactionObservations, ObservedUnit,
    SurfaceSnapshot, SurfaceSnapshots,
};
use hex_test_support::{SurfaceSpec, SyntheticArena, TestAppBuilder, STONE};
use hex_units::{Body, Downed, Faction, Standing, StandsOn, TerrainOccupancy, UnitRegistry};

/// The level every unit in these tests stands on.
const GROUND: hex_core::Level = 1;

/// How much mana each gem starts with. Comfortably more than any spell here costs, so a
/// refused cast is never a refused *payment* in disguise.
const GEM_MANA: u16 = 6;

// --- content -----------------------------------------------------------------

/// Two elements, no fusions. Ids come from sorted names, so nothing here may assume
/// which is which — every lookup goes through the catalog.
fn elements() -> ElementCatalog {
    ElementCatalog::from_file(&ElementFile {
        wheel: vec!["Fire".to_owned(), "Metal".to_owned()],
        fusions: HashMap::default(),
    })
}

/// Implemented effect fixtures plus one area-shaped burn used to prove that spatial
/// authorization applies only to the cast anchor.
///
/// The pair is the whole point of the fixture. "Ward" subtracts 1 from an incoming
/// disable count, which is exactly enough to absorb an ember — and exactly what burn is
/// specified to ignore, so the same lattice can be shown to stop one and not the other.
fn spells(burn_turns: u16) -> SpellBook {
    let single = TargetingSpec {
        range: 3,
        reach: hex_assets::TargetingReach::Ranged,
        shape: TargetShape::Single,
        trajectory: Trajectory::None,
    };
    let mut by_name = HashMap::default();
    by_name.insert(
        "Kindle".to_owned(),
        Spell {
            requirements: vec![GemRequirement {
                element: "Fire".to_owned(),
                mana: 1,
            }],
            casting: CastingAxis::Evocation,
            mana: ManaAxis::Fixed,
            co_castable: false,
            targeting: single.clone(),
            effects: vec![Effect::Burn { turns: burn_turns }],
        },
    );
    by_name.insert(
        "Ember".to_owned(),
        Spell {
            requirements: vec![GemRequirement {
                element: "Fire".to_owned(),
                mana: 1,
            }],
            casting: CastingAxis::Evocation,
            mana: ManaAxis::Fixed,
            co_castable: false,
            targeting: single.clone(),
            effects: vec![Effect::DisableHexes {
                count: 1,
                targeted: false,
            }],
        },
    );
    by_name.insert(
        "Ward".to_owned(),
        Spell {
            requirements: vec![GemRequirement {
                element: "Metal".to_owned(),
                mana: 1,
            }],
            casting: CastingAxis::Enchantment { defense: 1 },
            mana: ManaAxis::Fixed,
            co_castable: false,
            targeting: single.clone(),
            effects: Vec::new(),
        },
    );
    by_name.insert(
        "Self Ward".to_owned(),
        Spell {
            requirements: vec![GemRequirement {
                element: "Metal".to_owned(),
                mana: 1,
            }],
            casting: CastingAxis::Enchantment { defense: 1 },
            mana: ManaAxis::Fixed,
            co_castable: false,
            targeting: TargetingSpec {
                range: 0,
                reach: TargetingReach::Ranged,
                shape: TargetShape::SelfCast,
                trajectory: Trajectory::None,
            },
            effects: Vec::new(),
        },
    );
    by_name.insert(
        "Mend".to_owned(),
        Spell {
            requirements: vec![GemRequirement {
                element: "Fire".to_owned(),
                mana: 1,
            }],
            casting: CastingAxis::Evocation,
            mana: ManaAxis::Fixed,
            co_castable: false,
            targeting: TargetingSpec {
                range: 0,
                reach: TargetingReach::Touch,
                shape: TargetShape::Single,
                trajectory: Trajectory::None,
            },
            effects: vec![Effect::RestoreHexes { count: 1 }],
        },
    );
    by_name.insert(
        "Scry".to_owned(),
        Spell {
            requirements: vec![GemRequirement {
                element: "Fire".to_owned(),
                mana: 1,
            }],
            casting: CastingAxis::Evocation,
            mana: ManaAxis::Fixed,
            co_castable: false,
            targeting: TargetingSpec {
                range: 3,
                reach: hex_assets::TargetingReach::Ranged,
                shape: TargetShape::Single,
                trajectory: Trajectory::None,
            },
            effects: vec![Effect::Reveal { tier: 1 }],
        },
    );
    by_name.insert(
        "Wildfire".to_owned(),
        Spell {
            requirements: vec![GemRequirement {
                element: "Fire".to_owned(),
                mana: 1,
            }],
            casting: CastingAxis::Evocation,
            mana: ManaAxis::Fixed,
            co_castable: false,
            targeting: TargetingSpec {
                range: 3,
                reach: hex_assets::TargetingReach::Ranged,
                shape: TargetShape::Sphere { radius: 2 },
                trajectory: Trajectory::None,
            },
            effects: vec![Effect::Burn { turns: burn_turns }],
        },
    );
    by_name.insert(
        "Stone Shaper".to_owned(),
        Spell {
            requirements: vec![GemRequirement {
                element: "Fire".to_owned(),
                mana: 1,
            }],
            casting: CastingAxis::Evocation,
            mana: ManaAxis::Fixed,
            co_castable: false,
            targeting: single.clone(),
            effects: vec![Effect::SetTerrain {
                substance: "stone".to_owned(),
            }],
        },
    );
    by_name.insert(
        "Earthen Wall".to_owned(),
        Spell {
            requirements: vec![GemRequirement {
                element: "Fire".to_owned(),
                mana: 1,
            }],
            casting: CastingAxis::Evocation,
            mana: ManaAxis::Fixed,
            co_castable: false,
            targeting: TargetingSpec {
                range: 3,
                reach: hex_assets::TargetingReach::Ranged,
                shape: TargetShape::Column { height: 2 },
                trajectory: Trajectory::None,
            },
            effects: vec![Effect::SpawnWall {
                substance: "stone".to_owned(),
            }],
        },
    );
    for (name, trajectory) in [
        ("Direct Ember", Trajectory::Direct),
        ("Arcing Ember", Trajectory::Arc { rise: 3 }),
        ("Phase Ember", Trajectory::None),
    ] {
        by_name.insert(
            name.to_owned(),
            Spell {
                requirements: vec![GemRequirement {
                    element: "Fire".to_owned(),
                    mana: 1,
                }],
                casting: CastingAxis::Evocation,
                mana: ManaAxis::Fixed,
                co_castable: false,
                targeting: TargetingSpec {
                    range: 3,
                    reach: hex_assets::TargetingReach::Ranged,
                    shape: TargetShape::Single,
                    trajectory,
                },
                effects: vec![Effect::DisableHexes {
                    count: 1,
                    targeted: false,
                }],
            },
        );
    }
    SpellBook::from_file(&SpellFile { spells: by_name })
}

// --- lattices ----------------------------------------------------------------

/// A lattice that can cast `spell`, plus `spare` blank cells to lose.
///
/// The spell sits at the origin with its one funding gem adjacent, which is what
/// `castable` looks for; the spares hang off the far side so damage has somewhere to go
/// that is not the spell or the gem paying for it.
#[expect(
    clippy::expect_used,
    reason = "test helper outside a #[test] fn; a fixture naming content it did not \
              define IS the failure"
)]
fn lattice_casting(
    book: &SpellBook,
    catalog: &ElementCatalog,
    spell: &str,
    element: &str,
    spare: u16,
) -> (LatticeSpec, LatticeStats) {
    let spell_id = book.id(spell).expect("the fixture book holds that spell");
    let element_id = catalog.id(element).expect("and the catalog that element");
    let mut spec = LatticeSpec::default()
        .with(LatticeCoord::ORIGIN, CellKind::Spell { spell: spell_id })
        .with(
            LatticeCoord::new(1, 0),
            CellKind::Gem {
                element: element_id,
            },
        );
    for step in 0..i32::from(spare) {
        spec = spec.with(LatticeCoord::new(-1 - step, 0), CellKind::Blank);
    }
    let stats = LatticeStats::new(
        BTreeMap::from([(element_id, GEM_MANA)]),
        BTreeMap::default(),
    );
    (spec, stats)
}

// --- watching the decision seam ----------------------------------------------

/// What the defender-chooses seam was seen doing, between parking and applying.
///
/// The seam opens and closes inside a **single frame** — the tick parks the decision
/// before `Act`, the auto-policy answers during it, and the applier drains that answer
/// during `Apply` — so nothing outside the schedule can observe it. Recording from
/// inside is the only honest way to prove a burn went through it rather than around it.
#[derive(Resource, Debug, Default)]
struct Seam {
    /// Every distinct decision seen parked, in order.
    parked: Vec<PendingDecision>,
    /// Whether an answering **command** was waiting while a decision was parked.
    answered_by_command: bool,
}

#[derive(Resource, Default)]
struct CapturedEvents(Vec<CombatEvent>);

fn capture_events(mut events: MessageReader<CombatEvent>, mut captured: ResMut<CapturedEvents>) {
    captured.0.extend(events.read().cloned());
}

#[derive(Resource, Default)]
struct CapturedTerrainEdits(Vec<TerrainEdit>);

#[derive(Resource, Default)]
struct StripLatticeBeforeApply(Option<Entity>);

fn capture_terrain_edits(
    mut edits: MessageReader<TerrainEdit>,
    mut captured: ResMut<CapturedTerrainEdits>,
) {
    captured.0.extend(edits.read().cloned());
}

#[expect(
    clippy::expect_used,
    reason = "the one-frame hostile privacy fixture must publish a valid authority projection"
)]
fn strip_lattice_before_apply(world: &mut World) {
    let entity = world.resource_mut::<StripLatticeBeforeApply>().0.take();
    let Some(entity) = entity else {
        return;
    };
    world
        .entity_mut(entity)
        .remove::<(LatticeSpec, LatticeState)>();
    hex_combat::publish_combat_adapter_facts(world)
        .expect("the stripped lattice is a complete adapter projection");
}

/// Samples the seam between deciding and applying.
fn watch_seam(pending: Res<PendingDecision>, queue: Res<CommandQueue>, mut seam: ResMut<Seam>) {
    if !pending.is_open() {
        return;
    }
    if seam.parked.last() != Some(&*pending) {
        seam.parked.push(pending.clone());
    }
    if let Some(decider) = pending.decider() {
        seam.answered_by_command |= queue.holds_answer_for(decider);
    }
}

// --- harness -----------------------------------------------------------------

fn test_app(burn_turns: u16) -> App {
    test_app_with_arena(burn_turns, SyntheticArena::flat_radius(12, GROUND))
}

#[expect(
    clippy::expect_used,
    reason = "test helper outside a #[test] fn; fixture content that will not resolve \
              IS the failure"
)]
fn test_app_with_arena(burn_turns: u16, arena: SyntheticArena) -> App {
    let terrain = TerrainOccupancy::from_runs(
        arena
            .surfaces()
            .iter()
            .map(|surface| (surface.position, RunBottom(surface.bottom))),
    )
    .expect("the synthetic arena publishes valid exact runs");
    let mut builder = TestAppBuilder::new()
        .with_fixed_step(Duration::ZERO)
        .with_arena(arena)
        .expect("the shared synthetic arena must be valid");
    let app = builder.app_mut();
    app.insert_resource(CombatSettings::default());
    app.add_plugins(hex_combat::plugin);
    app.init_resource::<UnitRegistry>();

    let catalog = elements();
    let book = spells(burn_turns);
    let substances = app.world().resource::<SubstanceTable>().clone();
    let index =
        ContentIndex::build(&catalog, &book, &substances).expect("the fixture content resolves");
    app.insert_resource(catalog);
    app.insert_resource(book);
    app.insert_resource(index);
    app.insert_resource(terrain);

    app.init_resource::<Seam>();
    app.init_resource::<CapturedEvents>();
    app.init_resource::<CapturedTerrainEdits>();
    app.init_resource::<StripLatticeBeforeApply>();
    app.add_systems(
        Update,
        strip_lattice_before_apply.in_set(hex_combat::CombatSystems::Act),
    );
    app.add_systems(
        Update,
        watch_seam
            .after(hex_combat::CombatSystems::Act)
            .before(hex_combat::CombatSystems::Apply),
    );
    app.add_systems(
        Update,
        capture_events.after(hex_combat::CombatSystems::Advance),
    );
    app.add_systems(
        Update,
        capture_terrain_edits.after(hex_combat::CombatSystems::Apply),
    );

    let mut app = builder.build();
    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Gameplay);
    app.update();
    app
}

#[expect(
    clippy::expect_used,
    reason = "fixture facts must be accepted by the active combat authority"
)]
fn publish_adapter_facts(app: &mut App) {
    hex_combat::publish_combat_adapter_facts(app.world_mut())
        .expect("the fixture projection must be valid");
}

fn publish_downed_fixture(app: &mut App, entity: Entity, unit: UnitId) {
    app.world_mut()
        .entity_mut(entity)
        .insert(Downed)
        .remove::<Turn>();
    app.world_mut().resource_mut::<TurnOrder>().remove(unit);
    publish_adapter_facts(app);
}

fn spawn(
    app: &mut App,
    id: UnitId,
    faction: Faction,
    coord: HexCoord,
    initiative: u32,
    lattice: (LatticeSpec, LatticeStats),
) -> Entity {
    spawn_at(
        app,
        id,
        faction,
        TilePos::new(coord, GROUND),
        initiative,
        lattice,
    )
}

fn spawn_at(
    app: &mut App,
    id: UnitId,
    faction: Faction,
    position: TilePos,
    initiative: u32,
    lattice: (LatticeSpec, LatticeStats),
) -> Entity {
    let (spec, stats) = lattice;
    let state = LatticeState::new(&spec, &stats);
    let entity = app
        .world_mut()
        .spawn((
            faction,
            id,
            ControlOwner::default(),
            StandsOn(Standing {
                pos: position,
                span: HexSpan::new(0.0, 1.0),
            }),
            Body::new(TraversalProfile::WALKER),
            Initiative(initiative),
            spec,
            state,
            stats,
        ))
        .id();
    app.world_mut()
        .resource_mut::<UnitRegistry>()
        .register(id, entity);
    entity
}

fn spawn_latticeless(
    app: &mut App,
    id: UnitId,
    faction: Faction,
    position: TilePos,
    initiative: u32,
) -> Entity {
    let entity = app
        .world_mut()
        .spawn((
            faction,
            id,
            ControlOwner::default(),
            StandsOn(Standing {
                pos: position,
                span: HexSpan::new(0.0, 1.0),
            }),
            Body::new(TraversalProfile::WALKER),
            Initiative(initiative),
        ))
        .id();
    app.world_mut()
        .resource_mut::<UnitRegistry>()
        .register(id, entity);
    entity
}

/// Gives the combat adapter an explicit world-owned observation snapshot.
fn publish_spatial_knowledge(app: &mut App) {
    publish_spatial_knowledge_with_surfaces(app, []);
}

/// Publishes ordinary unit observations plus explicitly observed empty surfaces.
#[expect(
    clippy::expect_used,
    reason = "duplicate test identities or surfaces invalidate the fixture"
)]
fn publish_spatial_knowledge_with_surfaces(
    app: &mut App,
    extra_surfaces: impl IntoIterator<Item = TilePos>,
) {
    let rows: Vec<(UnitId, Faction, TilePos, HexSpan)> = {
        let world = app.world_mut();
        let mut query = world.query::<(&UnitId, &Faction, &StandsOn)>();
        query
            .iter(world)
            .map(|(id, faction, standing)| (*id, *faction, standing.0.pos, standing.0.span))
            .collect()
    };
    let extras: Vec<_> = extra_surfaces.into_iter().collect();
    let current = SurfaceSnapshots::try_from_iter(
        rows.iter()
            .map(|&(_, _, pos, span)| SurfaceSnapshot {
                pos,
                span,
                substance: SubstanceId(0),
                headroom: Headroom(2),
                is_solid: true,
                blocked: false,
                domain: LightDomain::Exterior,
            })
            .chain(extras.iter().copied().map(|pos| SurfaceSnapshot {
                pos,
                span: HexSpan::new(0.0, 1.0),
                substance: SubstanceId(1),
                headroom: Headroom(2),
                is_solid: true,
                blocked: false,
                domain: LightDomain::Exterior,
            })),
    )
    .expect("test units and extra surfaces occupy unique positions");
    let observe_all = || {
        let mut observation = FactionObservation::new();
        for &(id, faction, pos, _) in &rows {
            observation.insert_surface(pos);
            observation
                .try_insert_unit(ObservedUnit {
                    id,
                    faction,
                    pos,
                    provides_sight: true,
                })
                .expect("test unit ids are unique");
        }
        for &pos in &extras {
            observation.insert_surface(pos);
        }
        observation
    };
    let observations = FactionObservations::from_factions(observe_all(), observe_all());
    let mut spatial = FactionMapKnowledge::new();
    apply_observations(&mut spatial, &current, &observations);
    app.insert_resource(spatial);
}

/// The caster, the defender, and the position the defender stands on.
struct Fight {
    caster: Entity,
    defender: Entity,
    defender_pos: TilePos,
}

/// Two units one hex apart, both able to cast, with the caster acting first.
///
/// The defender is given "Ward" rather than "Kindle" so the armour test can raise it,
/// and the initiative gap is what makes the turn order predictable without relying on
/// spawn order.
fn two_casters(app: &mut App) -> Fight {
    let catalog = app.world().resource::<ElementCatalog>().clone();
    let book = app.world().resource::<SpellBook>().clone();
    let defender_coord = HexCoord::new_cubic(1, -1, 0);

    let caster = spawn(
        app,
        UnitId(1),
        Faction::Player,
        HexCoord::ORIGIN,
        20,
        lattice_casting(&book, &catalog, "Kindle", "Fire", 2),
    );
    let defender = spawn(
        app,
        UnitId(2),
        Faction::Hostile,
        defender_coord,
        10,
        lattice_casting(&book, &catalog, "Ward", "Metal", 3),
    );
    publish_spatial_knowledge(app);

    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();

    Fight {
        caster,
        defender,
        defender_pos: TilePos::new(defender_coord, GROUND),
    }
}

fn two_scriers(app: &mut App) -> Fight {
    let catalog = app.world().resource::<ElementCatalog>().clone();
    let book = app.world().resource::<SpellBook>().clone();
    let defender_coord = HexCoord::new_cubic(1, -1, 0);

    let caster = spawn(
        app,
        UnitId(1),
        Faction::Player,
        HexCoord::ORIGIN,
        20,
        lattice_casting(&book, &catalog, "Scry", "Fire", 2),
    );
    let defender = spawn(
        app,
        UnitId(2),
        Faction::Hostile,
        defender_coord,
        10,
        lattice_casting(&book, &catalog, "Ward", "Metal", 3),
    );
    publish_spatial_knowledge(app);

    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();

    Fight {
        caster,
        defender,
        defender_pos: TilePos::new(defender_coord, GROUND),
    }
}

fn two_ember_casters(app: &mut App) -> Fight {
    let catalog = app.world().resource::<ElementCatalog>().clone();
    let book = app.world().resource::<SpellBook>().clone();
    let defender_coord = HexCoord::new_cubic(1, -1, 0);

    let caster = spawn(
        app,
        UnitId(1),
        Faction::Player,
        HexCoord::ORIGIN,
        20,
        lattice_casting(&book, &catalog, "Ember", "Fire", 2),
    );
    let defender = spawn(
        app,
        UnitId(2),
        Faction::Hostile,
        defender_coord,
        10,
        lattice_casting(&book, &catalog, "Ward", "Metal", 3),
    );
    publish_spatial_knowledge(app);

    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();

    Fight {
        caster,
        defender,
        defender_pos: TilePos::new(defender_coord, GROUND),
    }
}

fn two_wildfire_casters(app: &mut App) -> Fight {
    let catalog = app.world().resource::<ElementCatalog>().clone();
    let book = app.world().resource::<SpellBook>().clone();
    let defender_coord = HexCoord::new_cubic(1, -1, 0);

    let caster = spawn(
        app,
        UnitId(1),
        Faction::Player,
        HexCoord::ORIGIN,
        20,
        lattice_casting(&book, &catalog, "Wildfire", "Fire", 2),
    );
    let defender = spawn(
        app,
        UnitId(2),
        Faction::Hostile,
        defender_coord,
        10,
        lattice_casting(&book, &catalog, "Ward", "Metal", 3),
    );
    publish_spatial_knowledge(app);

    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();

    Fight {
        caster,
        defender,
        defender_pos: TilePos::new(defender_coord, GROUND),
    }
}

/// A terrain-capable caster, one hostile keeping combat live, and one observed empty
/// build surface between them.
fn terrain_caster(app: &mut App, spell: &str) -> (Fight, TilePos) {
    let catalog = app.world().resource::<ElementCatalog>().clone();
    let book = app.world().resource::<SpellBook>().clone();
    let defender_coord = HexCoord::from_axial(2, 0);
    let build_surface = TilePos::new(HexCoord::from_axial(1, 0), GROUND);

    let caster = spawn(
        app,
        UnitId(1),
        Faction::Player,
        HexCoord::ORIGIN,
        20,
        lattice_casting(&book, &catalog, spell, "Fire", 2),
    );
    let defender = spawn(
        app,
        UnitId(2),
        Faction::Hostile,
        defender_coord,
        10,
        lattice_casting(&book, &catalog, "Ward", "Metal", 3),
    );
    publish_spatial_knowledge_with_surfaces(app, [build_surface]);

    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();

    (
        Fight {
            caster,
            defender,
            defender_pos: TilePos::new(defender_coord, GROUND),
        },
        build_surface,
    )
}

fn trajectory_caster(app: &mut App, spell: &str) -> Fight {
    let catalog = app.world().resource::<ElementCatalog>().clone();
    let book = app.world().resource::<SpellBook>().clone();
    let defender_coord = HexCoord::from_axial(3, 0);

    let caster = spawn(
        app,
        UnitId(1),
        Faction::Player,
        HexCoord::ORIGIN,
        20,
        lattice_casting(&book, &catalog, spell, "Fire", 2),
    );
    let defender = spawn(
        app,
        UnitId(2),
        Faction::Hostile,
        defender_coord,
        10,
        lattice_casting(&book, &catalog, "Ward", "Metal", 3),
    );
    publish_spatial_knowledge(app);

    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();

    Fight {
        caster,
        defender,
        defender_pos: TilePos::new(defender_coord, GROUND),
    }
}

fn push(app: &mut App, command: GameCommand) {
    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat::default(),
            command,
        });
}

fn take_events(app: &mut App) -> Vec<CombatEvent> {
    std::mem::take(&mut app.world_mut().resource_mut::<CapturedEvents>().0)
}

fn take_terrain_edits(app: &mut App) -> Vec<TerrainEdit> {
    std::mem::take(&mut app.world_mut().resource_mut::<CapturedTerrainEdits>().0)
}

/// Casts "Kindle" at `target` and yields the rest of the caster's turn.
fn kindle(app: &mut App, caster: UnitId, target: TilePos) {
    push(
        app,
        GameCommand::Cast {
            unit: caster,
            spell: "Kindle".to_owned(),
            target,
            facing: None,
            mana: None,
        },
    );
    push(app, GameCommand::EndTurn { unit: caster });
    app.update();
}

fn scry(app: &mut App, caster: UnitId, target: TilePos) {
    push(
        app,
        GameCommand::Cast {
            unit: caster,
            spell: "Scry".to_owned(),
            target,
            facing: None,
            mana: None,
        },
    );
    push(app, GameCommand::EndTurn { unit: caster });
    app.update();
}

fn cast_named(app: &mut App, caster: UnitId, spell: &str, target: TilePos) {
    push(
        app,
        GameCommand::Cast {
            unit: caster,
            spell: spell.to_owned(),
            target,
            facing: None,
            mana: None,
        },
    );
    push(app, GameCommand::EndTurn { unit: caster });
    app.update();
}

/// How many of a unit's cells are currently down.
#[expect(
    clippy::expect_used,
    reason = "test helper outside a #[test] fn; a unit that lost its lattice IS the \
              failure, and reporting zero would read as 'nothing was damaged'"
)]
fn disabled_count(app: &App, entity: Entity) -> usize {
    let entity = app.world().entity(entity);
    let spec = entity.get::<LatticeSpec>().expect("a lattice spec");
    let state = entity.get::<LatticeState>().expect("a lattice state");
    spec.cells()
        .filter(|&(coord, _)| state.is_disabled(coord))
        .count()
}

#[expect(
    clippy::expect_used,
    reason = "a fixture unit losing its lattice is the failure under test, not zero mana"
)]
fn lattice_mana(app: &App, entity: Entity) -> u32 {
    app.world()
        .entity(entity)
        .get::<LatticeState>()
        .expect("fixture caster has lattice state")
        .total_gem_mana()
}

#[expect(
    clippy::expect_used,
    reason = "a fixture unit losing its lattice is a malformed test fixture"
)]
fn disable_cells(app: &mut App, entity: Entity, cells: &[LatticeCoord]) {
    let mut target = app.world_mut().entity_mut(entity);
    let mut state = target
        .get_mut::<LatticeState>()
        .expect("fixture target has lattice state");
    hex_lattice::apply_disables(&mut state, cells);
}

#[expect(
    clippy::expect_used,
    reason = "a fixture unit losing its lattice is a malformed test fixture"
)]
fn disable_all_cells(app: &mut App, entity: Entity) -> Vec<LatticeCoord> {
    let cells = app
        .world()
        .entity(entity)
        .get::<LatticeSpec>()
        .expect("fixture target has lattice spec")
        .cells()
        .map(|(coord, _)| coord)
        .collect::<Vec<_>>();
    disable_cells(app, entity, &cells);
    cells
}

fn enter_combat(app: &mut App) {
    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();
}

fn mend_command(caster: UnitId, target: TilePos) -> GameCommand {
    GameCommand::Cast {
        unit: caster,
        spell: "Mend".to_owned(),
        target,
        facing: None,
        mana: None,
    }
}

#[expect(
    clippy::expect_used,
    reason = "a successful fixture cast must retain its canonical turn and lattice"
)]
fn assert_mend_opens_restoration(
    app: &mut App,
    caster_entity: Entity,
    caster: UnitId,
    target_pos: TilePos,
    target: UnitId,
) {
    let mana_before = lattice_mana(app, caster_entity);
    let _ = take_events(app);

    push(app, mend_command(caster, target_pos));
    app.update();

    assert_eq!(
        lattice_mana(app, caster_entity),
        mana_before - 1,
        "an accepted Mend pays its authored one mana"
    );
    assert!(
        app.world()
            .entity(caster_entity)
            .get::<Turn>()
            .expect("accepted caster retains its turn while the decision is open")
            .acted,
        "an accepted restoration spends the action"
    );
    assert_eq!(
        *app.world().resource::<PendingDecision>(),
        PendingDecision::ChooseRestores {
            decider: caster,
            target,
            count: 1,
        }
    );
    assert_eq!(
        take_events(app),
        vec![CombatEvent::Cast {
            caster,
            spell: "Mend".to_owned(),
            target: target_pos,
        }],
        "the paid cast opens the decision without applying restoration early"
    );
}

#[expect(
    clippy::expect_used,
    reason = "the acting caster must retain its canonical turn after a refused cast"
)]
fn assert_mend_refused_before_payment(
    app: &mut App,
    caster_entity: Entity,
    caster: UnitId,
    target: TilePos,
    refusal: CommandRefusal,
) {
    let command = mend_command(caster, target);
    let mana_before = lattice_mana(app, caster_entity);
    let _ = take_events(app);

    push(app, command.clone());
    app.update();

    assert_eq!(
        lattice_mana(app, caster_entity),
        mana_before,
        "a refused restoration must not spend mana"
    );
    assert!(
        !app.world()
            .entity(caster_entity)
            .get::<Turn>()
            .expect("refused caster retains its turn")
            .acted,
        "a refused restoration must not spend the action"
    );
    assert_eq!(
        *app.world().resource::<PendingDecision>(),
        PendingDecision::None,
        "a refused restoration must not open an exact-cell decision"
    );
    assert_eq!(
        take_events(app),
        vec![CombatEvent::CommandRefused { command, refusal }],
        "only the typed refusal may be published"
    );
}

fn raised_neighbor_arena(target: TilePos) -> SyntheticArena {
    let flat = SyntheticArena::flat_radius(12, GROUND);
    SyntheticArena::new(
        flat.surfaces()
            .iter()
            .copied()
            .filter(|surface| surface.position.coord != target.coord)
            .chain([SurfaceSpec::stone(target)]),
    )
}

fn spawn_mend_healer_and_keeper(app: &mut App) -> Entity {
    let catalog = app.world().resource::<ElementCatalog>().clone();
    let book = app.world().resource::<SpellBook>().clone();
    let healer = spawn(
        app,
        UnitId(1),
        Faction::Player,
        HexCoord::ORIGIN,
        30,
        lattice_casting(&book, &catalog, "Mend", "Fire", 2),
    );
    spawn(
        app,
        UnitId(3),
        Faction::Hostile,
        HexCoord::from_axial(0, 3),
        10,
        lattice_casting(&book, &catalog, "Ward", "Metal", 3),
    );
    healer
}

#[derive(Clone, Copy)]
enum MendTargetFixture {
    Empty,
    LatticelessAlly,
    LatticelessHostile,
    HealthyAlly,
    HealthyHostile,
    DamagedAlly,
    DamagedHostile,
    DownedAlly,
}

fn mend_fight(target_pos: TilePos, target: MendTargetFixture) -> (App, Entity, Option<Entity>) {
    let mut app = if target_pos.level == GROUND {
        test_app(2)
    } else {
        test_app_with_arena(2, raised_neighbor_arena(target_pos))
    };
    let healer = spawn_mend_healer_and_keeper(&mut app);
    let catalog = app.world().resource::<ElementCatalog>().clone();
    let book = app.world().resource::<SpellBook>().clone();
    let target_entity = match target {
        MendTargetFixture::Empty => None,
        MendTargetFixture::LatticelessAlly | MendTargetFixture::LatticelessHostile => {
            let faction = if matches!(target, MendTargetFixture::LatticelessAlly) {
                Faction::Player
            } else {
                Faction::Hostile
            };
            Some(spawn_latticeless(
                &mut app,
                UnitId(2),
                faction,
                target_pos,
                20,
            ))
        }
        MendTargetFixture::HealthyAlly | MendTargetFixture::HealthyHostile => {
            let faction = if matches!(target, MendTargetFixture::HealthyAlly) {
                Faction::Player
            } else {
                Faction::Hostile
            };
            Some(spawn_at(
                &mut app,
                UnitId(2),
                faction,
                target_pos,
                20,
                lattice_casting(&book, &catalog, "Ward", "Metal", 3),
            ))
        }
        MendTargetFixture::DamagedAlly | MendTargetFixture::DamagedHostile => {
            let faction = if matches!(target, MendTargetFixture::DamagedAlly) {
                Faction::Player
            } else {
                Faction::Hostile
            };
            let entity = spawn_at(
                &mut app,
                UnitId(2),
                faction,
                target_pos,
                20,
                lattice_casting(&book, &catalog, "Ward", "Metal", 3),
            );
            disable_cells(&mut app, entity, &[LatticeCoord::new(-1, 0)]);
            Some(entity)
        }
        MendTargetFixture::DownedAlly => {
            let entity = spawn_at(
                &mut app,
                UnitId(2),
                Faction::Player,
                target_pos,
                20,
                lattice_casting(&book, &catalog, "Ward", "Metal", 3),
            );
            let _ = disable_all_cells(&mut app, entity);
            app.world_mut().entity_mut(entity).insert(Downed);
            Some(entity)
        }
    };
    if matches!(target, MendTargetFixture::Empty) {
        publish_spatial_knowledge_with_surfaces(&mut app, [target_pos]);
    } else {
        publish_spatial_knowledge(&mut app);
    }
    enter_combat(&mut app);
    (app, healer, target_entity)
}

fn revealed_hostile_fight() -> (App, Entity, Entity, TilePos) {
    let mut app = test_app(2);
    let catalog = app.world().resource::<ElementCatalog>().clone();
    let book = app.world().resource::<SpellBook>().clone();
    spawn(
        &mut app,
        UnitId(1),
        Faction::Player,
        HexCoord::from_axial(-1, 0),
        30,
        lattice_casting(&book, &catalog, "Scry", "Fire", 2),
    );
    let healer = spawn(
        &mut app,
        UnitId(2),
        Faction::Player,
        HexCoord::ORIGIN,
        20,
        lattice_casting(&book, &catalog, "Mend", "Fire", 2),
    );
    let target_pos = TilePos::new(HexCoord::from_axial(1, 0), GROUND);
    let hostile = spawn_at(
        &mut app,
        UnitId(3),
        Faction::Hostile,
        target_pos,
        10,
        lattice_casting(&book, &catalog, "Ward", "Metal", 3),
    );
    disable_cells(&mut app, hostile, &[LatticeCoord::new(-1, 0)]);
    publish_spatial_knowledge(&mut app);
    enter_combat(&mut app);
    scry(&mut app, UnitId(1), target_pos);
    run_until_acting(&mut app, UnitId(2));
    (app, healer, hostile, target_pos)
}

/// How many more turns of burn a unit is carrying, summed across its fires.
///
/// Turns *remaining*, not records held: a one-turn burn that has already bitten is still
/// in the ledger until the next expiry sweep, and counting records would call that a fire
/// still burning. The ledger is the only store — the lattice holds hexes and mana, and
/// asking it about fire is what this suite used to do wrong.
#[expect(
    clippy::expect_used,
    reason = "test helper outside a #[test] fn; see disabled_count"
)]
fn burn_turns_left(app: &App, entity: Entity) -> u16 {
    let unit = *app
        .world()
        .entity(entity)
        .get::<UnitId>()
        .expect("a unit id");
    app.world()
        .resource::<PersistentEffects>()
        .on(unit)
        .filter(|(_, effect)| matches!(effect.payload, EffectPayload::Burn))
        .map(|(_, effect)| match effect.end {
            EffectEnd::AfterTurns(turns) => turns.saturating_sub(effect.ticks),
            _ => 0,
        })
        .sum()
}

/// Runs frames until `unit` is the one acting, yielding whoever holds the turn.
///
/// Turn handover takes more than one frame — the marker lands through `Commands` and
/// `advance_turn` waits for the acting unit to be spent — so a fixed update count would
/// be a guess that silently passes when the loop stalls. The bound is what turns a
/// stalled loop into a failure instead of a hang.
fn run_until_acting(app: &mut App, unit: UnitId) {
    for _ in 0..16 {
        let acting = app.world().resource::<TurnOrder>().current();
        if acting == Some(unit) {
            return;
        }
        push(
            app,
            GameCommand::EndTurn {
                unit: acting.unwrap_or(unit),
            },
        );
        app.update();
    }
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(unit),
        "the order never came round to this unit; the loop is stalled"
    );
}

// --- tests -------------------------------------------------------------------

#[test]
fn self_cast_refuses_an_off_self_anchor_before_payment() {
    let mut app = test_app(2);
    let catalog = app.world().resource::<ElementCatalog>().clone();
    let book = app.world().resource::<SpellBook>().clone();
    let caster = spawn(
        &mut app,
        UnitId(1),
        Faction::Player,
        HexCoord::ORIGIN,
        20,
        lattice_casting(&book, &catalog, "Self Ward", "Metal", 2),
    );
    let target = TilePos::new(HexCoord::from_axial(1, 0), GROUND);
    spawn_at(
        &mut app,
        UnitId(2),
        Faction::Hostile,
        target,
        10,
        lattice_casting(&book, &catalog, "Ward", "Metal", 3),
    );
    publish_spatial_knowledge(&mut app);
    enter_combat(&mut app);

    let command = GameCommand::Cast {
        unit: UnitId(1),
        spell: "Self Ward".to_owned(),
        target,
        facing: None,
        mana: None,
    };
    let refusal = CommandRefusal::TargetOutOfRange {
        spell: "Self Ward".to_owned(),
        target,
    };
    let mut reducer = hex_combat::authority_snapshot(app.world())
        .expect("the composed self-cast fixture freezes into pure authority");
    assert_eq!(
        reducer.apply(IssuedCommand {
            seat: PlayerSeat::default(),
            command: command.clone(),
        }),
        Err(refusal.clone()),
        "the reducer and ECS adapter must reject the same wire target"
    );
    let mana_before = lattice_mana(&app, caster);
    let _ = take_events(&mut app);
    push(&mut app, command.clone());
    app.update();

    assert_eq!(lattice_mana(&app, caster), mana_before);
    assert!(
        !app.world()
            .entity(caster)
            .get::<Turn>()
            .expect("the refused caster retains its turn")
            .acted
    );
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::CommandRefused { command, refusal }]
    );
}

#[test]
fn touch_restoration_accepts_self_and_every_flat_hex_direction() {
    let self_pos = TilePos::new(HexCoord::ORIGIN, GROUND);
    {
        let mut app = test_app(2);
        let healer = spawn_mend_healer_and_keeper(&mut app);
        disable_cells(&mut app, healer, &[LatticeCoord::new(-1, 0)]);
        publish_spatial_knowledge(&mut app);
        enter_combat(&mut app);

        assert_eq!(
            hex_combat::authority_snapshot(app.world())
                .expect("touch content freezes into the active combat authority")
                .content
                .spell("Mend")
                .expect("Mend remains in the frozen content projection")
                .targeting,
            FrozenTargeting::Touch,
            "the Bevy host must not collapse Touch into ranged zero"
        );

        assert_mend_opens_restoration(&mut app, healer, UnitId(1), self_pos, UnitId(1));
    }

    for neighbor in HexCoord::ORIGIN.neighbors() {
        let target_pos = TilePos::new(neighbor, GROUND);
        let (mut app, healer, _) = mend_fight(target_pos, MendTargetFixture::DamagedAlly);

        assert_mend_opens_restoration(&mut app, healer, UnitId(1), target_pos, UnitId(2));
    }
}

#[test]
fn touch_restoration_accepts_a_mutual_one_level_step() {
    let target_pos = TilePos::new(HexCoord::from_axial(1, 0), GROUND + 1);
    let (mut app, healer, _) = mend_fight(target_pos, MendTargetFixture::DamagedAlly);

    assert_mend_opens_restoration(&mut app, healer, UnitId(1), target_pos, UnitId(2));
}

#[test]
fn touch_restoration_refuses_empty_distant_and_two_level_targets_before_payment() {
    let empty = TilePos::new(HexCoord::from_axial(1, 0), GROUND);
    let (mut app, healer, _) = mend_fight(empty, MendTargetFixture::Empty);
    assert_mend_refused_before_payment(
        &mut app,
        healer,
        UnitId(1),
        empty,
        CommandRefusal::TargetUnoccupied { target: empty },
    );

    for target_pos in [
        TilePos::new(HexCoord::from_axial(2, 0), GROUND),
        TilePos::new(HexCoord::from_axial(1, 0), GROUND + 2),
    ] {
        let (mut app, healer, _) = mend_fight(target_pos, MendTargetFixture::DamagedAlly);
        assert_mend_refused_before_payment(
            &mut app,
            healer,
            UnitId(1),
            target_pos,
            CommandRefusal::TargetOutOfTouchReach { target: UnitId(2) },
        );
    }
}

#[test]
fn unobserved_touch_anchor_does_not_reveal_whether_it_is_occupied() {
    let target = TilePos::new(HexCoord::from_axial(1, 0), GROUND);
    for fixture in [MendTargetFixture::Empty, MendTargetFixture::DamagedAlly] {
        let (mut app, healer, _) = mend_fight(target, fixture);
        apply_observations(
            &mut app.world_mut().resource_mut::<FactionMapKnowledge>(),
            &SurfaceSnapshots::default(),
            &FactionObservations::default(),
        );

        assert_mend_refused_before_payment(
            &mut app,
            healer,
            UnitId(1),
            target,
            CommandRefusal::TargetUnobserved {
                spell: "Mend".to_owned(),
                target,
            },
        );
    }
}

#[test]
fn restoration_target_validation_precedes_payment_and_decision() {
    let target_pos = TilePos::new(HexCoord::from_axial(1, 0), GROUND);
    for (fixture, reason) in [
        (
            MendTargetFixture::LatticelessAlly,
            RestorationTargetRefusal::MissingLattice { target: UnitId(2) },
        ),
        (
            MendTargetFixture::HealthyAlly,
            RestorationTargetRefusal::FullyRestored { target: UnitId(2) },
        ),
        (
            MendTargetFixture::HealthyHostile,
            RestorationTargetRefusal::IncompleteHostileKnowledge { target: UnitId(2) },
        ),
        (
            MendTargetFixture::LatticelessHostile,
            RestorationTargetRefusal::IncompleteHostileKnowledge { target: UnitId(2) },
        ),
        (
            MendTargetFixture::DamagedHostile,
            RestorationTargetRefusal::IncompleteHostileKnowledge { target: UnitId(2) },
        ),
    ] {
        let (mut app, healer, _) = mend_fight(target_pos, fixture);
        assert_mend_refused_before_payment(
            &mut app,
            healer,
            UnitId(1),
            target_pos,
            CommandRefusal::RestorationTarget { reason },
        );
    }
}

#[test]
fn complete_hostile_reveal_permits_touch_restoration() {
    let (mut app, healer, _, target_pos) = revealed_hostile_fight();
    assert!(
        app.world()
            .resource::<FactionLatticeKnowledge>()
            .view(Faction::Player, UnitId(3))
            .is_some_and(hex_combat::LatticeKnowledge::is_complete),
        "Scry must establish the complete hostile knowledge restoration requires"
    );

    assert_mend_opens_restoration(&mut app, healer, UnitId(2), target_pos, UnitId(3));
}

#[test]
fn revealed_hostile_missing_live_lattice_is_refused_before_payment() {
    let (mut app, healer, hostile, target_pos) = revealed_hostile_fight();
    assert!(app
        .world()
        .resource::<FactionLatticeKnowledge>()
        .view(Faction::Player, UnitId(3))
        .is_some_and(hex_combat::LatticeKnowledge::is_complete));
    app.world_mut().resource_mut::<StripLatticeBeforeApply>().0 = Some(hostile);

    assert_mend_refused_before_payment(
        &mut app,
        healer,
        UnitId(2),
        target_pos,
        CommandRefusal::RestorationTarget {
            reason: RestorationTargetRefusal::MissingLattice { target: UnitId(3) },
        },
    );
}

#[test]
fn allied_downed_restoration_opens_an_exact_choice_and_revives() {
    let target_pos = TilePos::new(HexCoord::from_axial(1, 0), GROUND);
    let (mut app, healer, patient) = mend_fight(target_pos, MendTargetFixture::DownedAlly);
    let patient = patient.expect("the downed ally fixture is occupied");
    let disabled = app
        .world()
        .entity(patient)
        .get::<LatticeSpec>()
        .expect("the downed ally retains its lattice")
        .cells()
        .map(|(coord, _)| coord)
        .collect::<Vec<_>>();

    assert_mend_opens_restoration(&mut app, healer, UnitId(1), target_pos, UnitId(2));
    let restored = *disabled
        .first()
        .expect("the downed fixture disables at least one lattice cell");
    push(
        &mut app,
        GameCommand::ChooseRestores {
            unit: UnitId(1),
            target: UnitId(2),
            cells: vec![restored],
        },
    );
    app.update();

    let state = app
        .world()
        .entity(patient)
        .get::<LatticeState>()
        .expect("the revived ally retains its lattice");
    assert!(!state.is_disabled(restored));
    assert_eq!(disabled_count(&app, patient), disabled.len() - 1);
    assert!(!app.world().entity(patient).contains::<Downed>());
    assert_eq!(
        *app.world().resource::<PendingDecision>(),
        PendingDecision::None
    );
    let events = take_events(&mut app);
    assert_eq!(
        events.first(),
        Some(&CombatEvent::HexesRestored {
            caster: UnitId(1),
            target: UnitId(2),
            cells: vec![restored],
        })
    );
    assert_eq!(
        events.get(1),
        Some(&CombatEvent::Revived {
            unit: UnitId(2),
            reenters_round: 1,
        })
    );
    assert!(events
        .iter()
        .all(|event| !matches!(event, CombatEvent::CommandRefused { .. })));
    let authority = hex_combat::authority_snapshot(app.world())
        .expect("the live adapter must settle the revival into combat authority");
    assert_eq!(authority.pending_revivals.get(&UnitId(2)), Some(&1));
}

/// A cast that burns takes nothing down at the moment it lands.
///
/// This is the difference between `Burn` and `DisableHexes`, and the reason burn needed
/// a runtime at all: the cast starts a countdown on the target's lattice and records who
/// started it. If this ever asserts a disabled hex, burn has quietly become an ember.
#[test]
fn casting_a_burn_starts_a_countdown_rather_than_landing_damage() {
    let mut app = test_app(2);
    let fight = two_casters(&mut app);

    kindle(&mut app, UnitId(1), fight.defender_pos);

    assert_eq!(
        disabled_count(&app, fight.defender),
        0,
        "a burn must not disable anything on the turn it is cast"
    );
    assert_eq!(
        burn_turns_left(&app, fight.defender),
        2,
        "the countdown is booked in full, with none of the two turns elapsed"
    );

    let effects = app.world().resource::<PersistentEffects>();
    assert_eq!(effects.len(), 1, "and the attribution in the ledger");
    let (_, effect) = effects.iter().next().expect("the record just made");
    assert_eq!(effect.source, UnitId(1));
    assert_eq!(effect.target, UnitId(2));
    assert_eq!(effect.payload, EffectPayload::Burn);
    assert_eq!(effect.end, EffectEnd::AfterTurns(2));
    assert_eq!(
        take_events(&mut app),
        vec![
            CombatEvent::Cast {
                caster: UnitId(1),
                spell: "Kindle".to_owned(),
                target: fight.defender_pos,
            },
            CombatEvent::BurnApplied {
                source: UnitId(1),
                target: UnitId(2),
                turns: 2,
            },
            CombatEvent::TurnAdvanced {
                unit: UnitId(1),
                next: Some(UnitId(2)),
                round: 0,
            },
        ],
        "a successful cast is announced before the effect it booked"
    );
}

#[test]
fn reveal_exposes_a_complete_live_lattice_for_the_configured_rollovers() {
    let mut app = test_app(2);
    let fight = two_scriers(&mut app);

    scry(&mut app, UnitId(1), fight.defender_pos);

    let revealed: Vec<LatticeCoord> = app
        .world()
        .entity(fight.defender)
        .get::<LatticeSpec>()
        .expect("the defender has a lattice")
        .cells()
        .map(|(coord, _)| coord)
        .collect();
    let view = app
        .world()
        .resource::<FactionLatticeKnowledge>()
        .view(Faction::Player, UnitId(2))
        .expect("base visibility was published");
    assert_eq!(view.known_capacity(), Some(revealed.len()));
    assert_eq!(view.revealed_count(), revealed.len());
    assert_eq!(view.unknown_count(), Some(0));
    assert_eq!(
        take_events(&mut app),
        vec![
            CombatEvent::Cast {
                caster: UnitId(1),
                spell: "Scry".to_owned(),
                target: fight.defender_pos,
            },
            CombatEvent::Revealed {
                viewer: Faction::Player,
                subject: UnitId(2),
                cells: revealed,
                rounds: 1,
            },
            CombatEvent::TurnAdvanced {
                unit: UnitId(1),
                next: Some(UnitId(2)),
                round: 0,
            },
        ]
    );

    // The cast landed during round zero. Its target ends that partial round, leaving
    // the reveal alive throughout round one.
    push(&mut app, GameCommand::EndTurn { unit: UnitId(2) });
    app.update();
    assert_eq!(app.world().resource::<TurnOrder>().round, 1);
    assert_eq!(
        app.world()
            .resource::<FactionLatticeKnowledge>()
            .view(Faction::Player, UnitId(2))
            .and_then(|known| known.known_capacity()),
        Some(5),
        "one configured rollover leaves the next full round visible"
    );

    push(&mut app, GameCommand::EndTurn { unit: UnitId(1) });
    app.update();
    push(&mut app, GameCommand::EndTurn { unit: UnitId(2) });
    app.update();
    assert_eq!(app.world().resource::<TurnOrder>().round, 2);
    let expired = app
        .world()
        .resource::<FactionLatticeKnowledge>()
        .view(Faction::Player, UnitId(2))
        .expect("existence and faction remain known");
    assert!(expired.is_opaque());
    assert_eq!(expired.known_capacity(), None);
    assert_eq!(expired.unknown_count(), None);
}

#[test]
fn a_defence_reports_the_exact_damage_it_prevented() {
    let mut app = test_app(2);
    let fight = two_ember_casters(&mut app);

    {
        let catalog = app.world().resource::<ElementCatalog>().clone();
        let index = app.world().resource::<ContentIndex>().clone();
        let tables = index.tables(&catalog);
        let mut entity = app.world_mut().entity_mut(fight.defender);
        let spec = entity.get::<LatticeSpec>().expect("a spec").clone();
        let mut state = entity.get_mut::<LatticeState>().expect("a state");
        let plan = castable(&spec, &state, LatticeCoord::ORIGIN, &tables)
            .expect("the defender can afford its ward");
        assert!(apply_cast(&mut state, &plan, &tables));
    }
    publish_adapter_facts(&mut app);

    cast_named(&mut app, UnitId(1), "Ember", fight.defender_pos);
    assert!(
        !app.world().resource::<PendingDecision>().is_open(),
        "the fully absorbed hit opens no decision"
    );
    assert_eq!(disabled_count(&app, fight.defender), 0);
    assert!(
        app.world()
            .get::<hex_anim::Transformation>(fight.defender)
            .is_none(),
        "a fully absorbed spell should not make the defender recoil"
    );
    assert_eq!(
        take_events(&mut app),
        vec![
            CombatEvent::Cast {
                caster: UnitId(1),
                spell: "Ember".to_owned(),
                target: fight.defender_pos,
            },
            CombatEvent::DamagePrevented {
                source: UnitId(1),
                target: UnitId(2),
                amount: 1,
            },
            CombatEvent::TurnAdvanced {
                unit: UnitId(1),
                next: Some(UnitId(2)),
                round: 0,
            },
        ]
    );
}

#[test]
fn successful_direct_spell_damage_reuses_the_target_recoil() {
    let mut app = test_app(2);
    app.insert_resource(hex_assets::PlayerSettings {
        scale: 0.25,
        speed: 5.0,
    });
    let fight = two_ember_casters(&mut app);

    cast_named(&mut app, UnitId(1), "Ember", fight.defender_pos);

    assert!(
        app.world()
            .get::<hex_anim::Transformation>(fight.defender)
            .is_some(),
        "direct spell damage should visibly recoil its target"
    );
}

#[test]
fn casting_fails_closed_until_the_anchor_is_currently_observed() {
    let mut app = test_app(2);
    let fight = two_ember_casters(&mut app);
    let command = GameCommand::Cast {
        unit: UnitId(1),
        spell: "Ember".to_owned(),
        target: fight.defender_pos,
        facing: None,
        mana: None,
    };
    let before = app
        .world()
        .get::<LatticeState>(fight.caster)
        .expect("the caster has a lattice")
        .total_gem_mana();

    app.world_mut().remove_resource::<FactionMapKnowledge>();
    push(&mut app, command.clone());
    app.update();
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::CommandRefused {
            command: command.clone(),
            refusal: CommandRefusal::MissingCombatData {
                data: hex_combat::CombatData::SpatialKnowledge,
            },
        }]
    );

    app.insert_resource(FactionMapKnowledge::new());
    push(&mut app, command.clone());
    app.update();
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::CommandRefused {
            command: command.clone(),
            refusal: CommandRefusal::TargetUnobserved {
                spell: "Ember".to_owned(),
                target: fight.defender_pos,
            },
        }],
        "Unknown terrain must not be a cast anchor"
    );

    publish_spatial_knowledge(&mut app);
    let no_surfaces = SurfaceSnapshots::default();
    let no_observations = FactionObservations::default();
    apply_observations(
        &mut app.world_mut().resource_mut::<FactionMapKnowledge>(),
        &no_surfaces,
        &no_observations,
    );
    push(&mut app, command.clone());
    app.update();
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::CommandRefused {
            command: command.clone(),
            refusal: CommandRefusal::TargetUnobserved {
                spell: "Ember".to_owned(),
                target: fight.defender_pos,
            },
        }],
        "Remembered terrain must not be a cast anchor"
    );

    assert_eq!(
        app.world()
            .get::<LatticeState>(fight.caster)
            .expect("refusals preserve the caster lattice")
            .total_gem_mana(),
        before,
        "no failed observation check may spend mana"
    );
    assert!(
        !app.world()
            .get::<Turn>(fight.caster)
            .expect("the caster retains the turn")
            .acted,
        "no failed observation check may spend the action"
    );

    publish_spatial_knowledge(&mut app);
    push(&mut app, command);
    app.update();
    assert!(
        take_events(&mut app)
            .iter()
            .all(|event| !matches!(event, CombatEvent::CommandRefused { .. })),
        "the same anchor becomes legal when currently Observed"
    );
}

#[test]
fn an_observed_anchor_allows_area_spillover_into_unknown_space() {
    let mut app = test_app(2);
    let fight = two_wildfire_casters(&mut app);
    let player = app
        .world()
        .resource::<FactionMapKnowledge>()
        .faction(Faction::Player);
    assert_eq!(
        player.state(fight.defender_pos),
        hex_core::KnowledgeState::Observed
    );
    let hidden_neighbor = TilePos::new(
        fight.defender_pos.coord.neighbor(hex_core::Sextant::A),
        fight.defender_pos.level,
    );
    assert_eq!(
        player.state(hidden_neighbor),
        hex_core::KnowledgeState::Unknown,
        "the fixture needs genuinely hidden spillover space"
    );

    cast_named(&mut app, UnitId(1), "Wildfire", fight.defender_pos);

    let events = take_events(&mut app);
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, CombatEvent::CommandRefused { .. })),
        "only the anchor is an authorization boundary; hidden area spillover remains legal"
    );
    assert!(events.iter().any(|event| matches!(
        event,
        CombatEvent::BurnApplied {
            source: UnitId(1),
            target: UnitId(2),
            turns: 2,
        }
    )));
}

#[test]
fn a_blocked_direct_trajectory_refuses_before_payment_without_disclosing_the_voxel() {
    let mut app = test_app(2);
    let fight = trajectory_caster(&mut app, "Direct Ember");
    let blocker = TilePos::new(HexCoord::from_axial(1, 0), GROUND + 1);
    app.insert_resource(
        TerrainOccupancy::from_runs(
            HexCoord::ORIGIN
                .within_radius(12)
                .into_iter()
                .map(|coord| (TilePos::new(coord, GROUND), RunBottom(GROUND)))
                .chain([(blocker, RunBottom(blocker.level))]),
        )
        .expect("the exact wall fixture is valid"),
    );
    let command = GameCommand::Cast {
        unit: UnitId(1),
        spell: "Direct Ember".to_owned(),
        target: fight.defender_pos,
        facing: None,
        mana: None,
    };
    let mana_before = app
        .world()
        .get::<LatticeState>(fight.caster)
        .expect("the caster has a lattice")
        .total_gem_mana();

    push(&mut app, command.clone());
    app.update();

    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::CommandRefused {
            command,
            refusal: CommandRefusal::TrajectoryBlocked {
                spell: "Direct Ember".to_owned(),
            },
        }]
    );
    assert_eq!(
        app.world()
            .get::<LatticeState>(fight.caster)
            .expect("the refused caster keeps its lattice")
            .total_gem_mana(),
        mana_before
    );
    assert!(
        !app.world()
            .get::<Turn>(fight.caster)
            .expect("the refused caster keeps its turn")
            .acted
    );
}

#[test]
fn an_authored_arc_clears_the_same_wall_that_blocks_a_direct_cast() {
    let mut app = test_app(2);
    let fight = trajectory_caster(&mut app, "Arcing Ember");
    let blocker = TilePos::new(HexCoord::from_axial(1, 0), GROUND + 1);
    app.insert_resource(
        TerrainOccupancy::from_runs(
            HexCoord::ORIGIN
                .within_radius(12)
                .into_iter()
                .map(|coord| (TilePos::new(coord, GROUND), RunBottom(GROUND)))
                .chain([(blocker, RunBottom(blocker.level))]),
        )
        .expect("the exact wall fixture is valid"),
    );

    cast_named(&mut app, UnitId(1), "Arcing Ember", fight.defender_pos);

    let events = take_events(&mut app);
    assert!(
        events.iter().all(|event| !matches!(
            event,
            CombatEvent::CommandRefused {
                refusal: CommandRefusal::TrajectoryBlocked { .. },
                ..
            }
        )),
        "the authored rise should carry the trajectory over the exact wall: {events:?}"
    );
    assert!(events.iter().any(|event| matches!(
        event,
        CombatEvent::Cast {
            spell,
            ..
        } if spell == "Arcing Ember"
    )));
}

#[test]
fn terrain_creation_emits_exact_air_voxels_only_after_legality_and_payment() {
    let mut app = test_app(2);
    let (_fight, build_surface) = terrain_caster(&mut app, "Stone Shaper");
    let stone = app
        .world()
        .resource::<SubstanceTable>()
        .id("stone")
        .expect("the fixture defines stone");

    cast_named(&mut app, UnitId(1), "Stone Shaper", build_surface);

    assert_eq!(
        take_terrain_edits(&mut app),
        vec![TerrainEdit::Set {
            pos: build_surface.above(),
            substance: stone,
        }]
    );
    assert!(
        take_events(&mut app)
            .iter()
            .all(|event| !matches!(event, CombatEvent::CommandRefused { .. })),
        "an empty observed placement should commit"
    );
}

#[test]
fn an_earthen_wall_publishes_two_complete_voxels_above_the_selected_surface() {
    let mut app = test_app(2);
    let (_fight, build_surface) = terrain_caster(&mut app, "Earthen Wall");
    let stone = app
        .world()
        .resource::<SubstanceTable>()
        .id("stone")
        .expect("the fixture defines stone");

    cast_named(&mut app, UnitId(1), "Earthen Wall", build_surface);

    assert_eq!(
        take_terrain_edits(&mut app),
        vec![
            TerrainEdit::Set {
                pos: build_surface.above(),
                substance: stone,
            },
            TerrainEdit::Set {
                pos: build_surface.above().above(),
                substance: stone,
            },
        ]
    );
}

#[test]
fn settled_construction_replaces_stale_movement_authority_before_the_next_command() {
    let mut app = test_app(2);
    let (fight, build_surface) = terrain_caster(&mut app, "Stone Shaper");
    cast_named(&mut app, UnitId(1), "Stone Shaper", build_surface);
    assert_eq!(take_terrain_edits(&mut app).len(), 1);

    let old_tile = {
        let world = app.world_mut();
        let mut tiles = world.query_filtered::<(Entity, &TilePos), With<HexTile>>();
        tiles
            .iter(world)
            .find_map(|(entity, &position)| (position == build_surface).then_some(entity))
            .expect("the selected synthetic surface exists")
    };
    app.world_mut().despawn(old_tile);
    let raised = build_surface.above();
    app.world_mut().spawn((
        HexTile,
        raised.coord,
        raised,
        RunBottom(build_surface.level),
        // The raised fixture merges material levels 1 and 2 into one exact run.
        HexSpan::new(1.0, 3.0),
        STONE,
        Headroom(16),
    ));
    app.insert_resource(
        TerrainOccupancy::from_runs(
            HexCoord::ORIGIN
                .within_radius(12)
                .into_iter()
                .filter(|coord| *coord != build_surface.coord)
                .map(|coord| (TilePos::new(coord, GROUND), RunBottom(GROUND)))
                .chain([(raised, RunBottom(build_surface.level))]),
        )
        .expect("the settled raised run is exact"),
    );
    app.update();

    let state = hex_combat::authority_snapshot(app.world())
        .expect("combat authority survives the settled terrain refresh");
    let origin = app
        .world()
        .get::<StandsOn>(fight.defender)
        .expect("the next actor remains standing")
        .0
        .pos;
    let issued = |path| IssuedCommand {
        seat: PlayerSeat::default(),
        command: GameCommand::MoveAlong {
            unit: UnitId(2),
            path,
        },
    };

    let mut stale = state.clone();
    assert_eq!(
        stale.apply(issued(vec![origin, build_surface])),
        Err(CommandRefusal::InvalidPath),
        "the pre-edit floor route must be withdrawn"
    );
    let mut current = state;
    assert_eq!(
        current.apply(issued(vec![origin, raised])),
        Ok(()),
        "the newly published climb must be admitted"
    );
}

#[test]
fn hidden_air_material_and_unit_truth_have_the_same_acceptance_and_payment() {
    let mut clear_app = test_app(2);
    let (clear_fight, clear_surface) = terrain_caster(&mut clear_app, "Stone Shaper");
    let clear_mana_before = clear_app
        .world()
        .get::<LatticeState>(clear_fight.caster)
        .expect("the caster has a lattice")
        .total_gem_mana();
    let command = GameCommand::Cast {
        unit: UnitId(1),
        spell: "Stone Shaper".to_owned(),
        target: clear_surface,
        facing: None,
        mana: None,
    };
    push(&mut clear_app, command.clone());
    clear_app.update();
    let clear_events = take_events(&mut clear_app);
    let clear_mana_after = clear_app
        .world()
        .get::<LatticeState>(clear_fight.caster)
        .expect("the caster has a lattice")
        .total_gem_mana();
    assert_eq!(take_terrain_edits(&mut clear_app).len(), 1);

    let mut material_app = test_app(2);
    let (material_fight, material_surface) = terrain_caster(&mut material_app, "Stone Shaper");
    let material_mana_before = material_app
        .world()
        .get::<LatticeState>(material_fight.caster)
        .expect("the caster has a lattice")
        .total_gem_mana();
    let occupied = material_surface.above();
    material_app.insert_resource(
        TerrainOccupancy::from_runs(
            HexCoord::ORIGIN
                .within_radius(12)
                .into_iter()
                .map(|coord| (TilePos::new(coord, GROUND), RunBottom(GROUND)))
                .chain([(occupied, RunBottom(occupied.level))]),
        )
        .expect("the occupied placement fixture is exact"),
    );
    push(&mut material_app, command.clone());
    material_app.update();
    let material_events = take_events(&mut material_app);
    assert_eq!(
        material_events, clear_events,
        "hidden material must not change the visible command outcome"
    );
    assert!(take_terrain_edits(&mut material_app).is_empty());
    assert_eq!(
        material_mana_before
            - material_app
                .world()
                .get::<LatticeState>(material_fight.caster)
                .expect("the caster keeps its lattice")
                .total_gem_mana(),
        clear_mana_before - clear_mana_after,
        "hidden material must not change payment"
    );
    assert!(
        material_app
            .world()
            .get::<Turn>(material_fight.caster)
            .expect("the accepted caster keeps its turn record")
            .acted
    );

    let mut unit_app = test_app(2);
    let (unit_fight, unit_surface) = terrain_caster(&mut unit_app, "Stone Shaper");
    let old_standing = unit_app
        .world()
        .get::<StandsOn>(unit_fight.defender)
        .copied()
        .expect("the defender stands in the fixture");
    unit_app
        .world_mut()
        .entity_mut(unit_fight.defender)
        .insert(StandsOn(Standing {
            pos: unit_surface,
            span: old_standing.0.span,
        }));
    publish_adapter_facts(&mut unit_app);
    unit_app.update();
    let _settling_events = take_events(&mut unit_app);
    let unit_mana_before = unit_app
        .world()
        .get::<LatticeState>(unit_fight.caster)
        .expect("the caster has a lattice")
        .total_gem_mana();
    push(&mut unit_app, command);
    unit_app.update();
    let unit_events = take_events(&mut unit_app);
    assert_eq!(
        unit_events, clear_events,
        "a unit absent from faction knowledge must not change the visible command outcome"
    );
    assert!(take_terrain_edits(&mut unit_app).is_empty());
    assert_eq!(
        unit_mana_before
            - unit_app
                .world()
                .get::<LatticeState>(unit_fight.caster)
                .expect("the caster keeps its lattice")
                .total_gem_mana(),
        clear_mana_before - clear_mana_after,
        "a hidden unit must not change payment"
    );
}

#[test]
fn a_damage_cast_on_a_downed_unit_is_refused_before_payment() {
    let mut app = test_app(2);
    let fight = two_ember_casters(&mut app);
    publish_downed_fixture(&mut app, fight.defender, UnitId(2));
    let before = app
        .world()
        .get::<LatticeState>(fight.caster)
        .expect("the caster has a lattice")
        .total_gem_mana();
    let command = GameCommand::Cast {
        unit: UnitId(1),
        spell: "Ember".to_owned(),
        target: fight.defender_pos,
        facing: None,
        mana: None,
    };

    push(&mut app, command.clone());
    app.update();

    let after = app
        .world()
        .get::<LatticeState>(fight.caster)
        .expect("the caster kept its lattice")
        .total_gem_mana();
    assert_eq!(after, before, "a refused cast must not spend mana");
    assert!(
        !app.world()
            .get::<Turn>(fight.caster)
            .expect("the caster should retain its turn")
            .acted,
        "a refused cast must not spend the action"
    );
    assert!(app.world().resource::<PersistentEffects>().is_empty());
    assert!(!app.world().resource::<PendingDecision>().is_open());
    assert_eq!(
        take_events(&mut app),
        vec![
            CombatEvent::CommandRefused {
                command,
                refusal: CommandRefusal::TargetDowned { target: UnitId(2) },
            },
            CombatEvent::EncounterResolved {
                outcome: hex_combat::EncounterOutcome::Victory,
            },
        ]
    );
}

#[test]
fn a_non_damaging_reveal_can_still_inspect_a_downed_unit() {
    let mut app = test_app(2);
    let fight = two_scriers(&mut app);
    publish_downed_fixture(&mut app, fight.defender, UnitId(2));
    push(
        &mut app,
        GameCommand::Cast {
            unit: UnitId(1),
            spell: "Scry".to_owned(),
            target: fight.defender_pos,
            facing: None,
            mana: None,
        },
    );

    app.update();

    let capacity = app
        .world()
        .resource::<FactionLatticeKnowledge>()
        .view(Faction::Player, UnitId(2))
        .and_then(|known| known.known_capacity());
    assert_eq!(
        capacity,
        Some(5),
        "keeping a downed lattice queryable must still permit non-damaging effects"
    );
    assert!(
        take_events(&mut app)
            .iter()
            .all(|event| !matches!(event, CombatEvent::CommandRefused { .. })),
        "Reveal should not inherit the damage-only refusal"
    );
}

#[test]
fn exact_disables_announce_the_stable_name_of_a_broken_enchantment() {
    let mut app = test_app(2);
    let fight = two_casters(&mut app);
    {
        let catalog = app.world().resource::<ElementCatalog>().clone();
        let index = app.world().resource::<ContentIndex>().clone();
        let tables = index.tables(&catalog);
        let mut entity = app.world_mut().entity_mut(fight.defender);
        let spec = entity.get::<LatticeSpec>().expect("a spec").clone();
        let mut state = entity.get_mut::<LatticeState>().expect("a state");
        let plan = castable(&spec, &state, LatticeCoord::ORIGIN, &tables)
            .expect("the defender can afford its ward");
        assert!(apply_cast(&mut state, &plan, &tables));
    }

    *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseDisables {
        decider: UnitId(2),
        count: 1,
        source: UnitId(1),
    };
    publish_adapter_facts(&mut app);
    push(
        &mut app,
        GameCommand::ChooseDisables {
            unit: UnitId(2),
            cells: vec![LatticeCoord::new(1, 0)],
        },
    );
    app.update();

    assert_eq!(
        take_events(&mut app),
        vec![
            CombatEvent::HexesDisabled {
                source: UnitId(1),
                target: UnitId(2),
                cells: vec![LatticeCoord::new(1, 0)],
            },
            CombatEvent::EnchantmentBroken {
                unit: UnitId(2),
                spell: Some("Ward".to_owned()),
                burned_mana: 1,
                trigger: LatticeCoord::new(1, 0),
            },
        ],
        "the outcome stream uses the content name, never the session-local SpellId"
    );
}

/// The burn comes due at the start of the target's own turn, and takes a hex.
///
/// The tick point is the settled half of this ticket: personal, not global. A burn that
/// ticked on the round boundary would hit a unit that had just acted and one that had
/// not at the same moment, which is a different mechanic than the one the design words.
#[test]
fn a_burn_comes_due_at_the_start_of_its_targets_turn() {
    let mut app = test_app(2);
    let fight = two_casters(&mut app);

    kindle(&mut app, UnitId(1), fight.defender_pos);
    assert_eq!(
        disabled_count(&app, fight.defender),
        0,
        "precondition: nothing down yet"
    );

    // The caster yielded, so the next turn is the defender's — and that is when it hurts.
    run_until_acting(&mut app, UnitId(2));
    app.update();
    app.update();

    assert_eq!(
        disabled_count(&app, fight.defender),
        1,
        "the burn should take exactly one hex at the start of its target's turn"
    );
    assert_eq!(
        disabled_count(&app, fight.caster),
        0,
        "and nothing from anybody else"
    );
}

/// The burn's damage goes through `ChooseDisables`, so a future replay can preserve
/// the choice.
///
/// Burn ignoring armour is **not** burn ignoring the defender's choice. A runtime that
/// disabled hexes itself would satisfy every other test in this file — the hex would
/// still go down — so what is asserted here is the *route*: a decision was parked naming
/// the defender and the count, and a **command** was what answered it. A fight replays by
/// re-running its commands, and a choice made inside the applier would be re-derived
/// instead of replayed.
#[test]
fn a_due_burn_routes_through_the_defender_chooses_seam() {
    let mut app = test_app(2);
    let fight = two_casters(&mut app);
    kindle(&mut app, UnitId(1), fight.defender_pos);
    take_events(&mut app);
    run_until_acting(&mut app, UnitId(2));
    app.update();

    let seam = app.world().resource::<Seam>();
    assert_eq!(
        seam.parked.as_slice(),
        &[PendingDecision::ChooseDisables {
            decider: UnitId(2),
            count: 1,
            source: UnitId(1),
        }],
        "a due burn must park exactly one choice, naming the defender and who lit it"
    );
    assert!(
        seam.answered_by_command,
        "and the answer must arrive as a command, or the fight cannot be replayed"
    );
    assert!(
        !app.world().resource::<PendingDecision>().is_open(),
        "the answer should close the decision"
    );
    assert_eq!(disabled_count(&app, fight.defender), 1);
    assert_eq!(
        take_events(&mut app),
        vec![
            CombatEvent::BurnTicked {
                source: UnitId(1),
                target: UnitId(2),
                count: 1,
            },
            CombatEvent::DecisionOpened {
                decider: UnitId(2),
                source: UnitId(1),
                count: 1,
            },
            CombatEvent::HexesDisabled {
                source: UnitId(1),
                target: UnitId(2),
                cells: vec![LatticeCoord::new(-3, 0)],
            },
        ],
        "tick, opened decision, and exact answer must stay in causal order"
    );
}

/// A due burn waits for the seam rather than being lost to it.
///
/// The seam holds one decision at a time. A tick that skipped itself while another
/// decision was open would silently drop a turn of burn — and a tick that parked over
/// the open one would erase somebody else's damage. So the tick always happens on
/// schedule and queues what it found, and the queue drains when the seam frees up.
#[test]
fn a_due_burn_waits_for_an_occupied_seam_rather_than_being_dropped() {
    let mut app = test_app(2);
    let fight = two_casters(&mut app);
    kindle(&mut app, UnitId(1), fight.defender_pos);
    run_until_acting(&mut app, UnitId(2));

    // Occupy the seam with somebody else's decision, asking the caster for a hex.
    *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseDisables {
        decider: UnitId(1),
        count: 1,
        source: UnitId(2),
    };
    publish_adapter_facts(&mut app);
    for _ in 0..6 {
        app.update();
    }

    assert_eq!(
        disabled_count(&app, fight.caster),
        1,
        "the decision that was already open resolves first"
    );
    assert_eq!(
        disabled_count(&app, fight.defender),
        1,
        "and the burn that came due behind it still lands"
    );
    assert_eq!(
        burn_turns_left(&app, fight.defender),
        1,
        "having consumed exactly one of its turns"
    );
}

/// Armour that stops an ember does not stop a fire.
///
/// The precondition is the contrast and does the real work: the same lattice, asked what
/// a one-hex spell would do, answers zero. Burn then takes a hex anyway. A burn routed
/// through `resolve_incoming` by mistake would fail here and nowhere else — every other
/// test in this file passes with the subtraction wired in, because none of them have a
/// defence up.
#[test]
fn a_burn_ignores_the_armour_that_would_absorb_a_spell() {
    let mut app = test_app(2);
    let fight = two_casters(&mut app);

    // Raise the ward directly on the defender's lattice: `apply_cast` is the engine's
    // own entry point, and going through a turn to reach it would test the turn order.
    {
        let catalog = app.world().resource::<ElementCatalog>().clone();
        let index = app.world().resource::<ContentIndex>().clone();
        let tables = index.tables(&catalog);
        let mut entity = app.world_mut().entity_mut(fight.defender);
        let spec = entity.get::<LatticeSpec>().expect("a spec").clone();
        let mut state = entity.get_mut::<LatticeState>().expect("a state");
        let plan = castable(&spec, &state, LatticeCoord::ORIGIN, &tables)
            .expect("the defender can afford its own ward");
        assert!(
            apply_cast(&mut state, &plan, &tables),
            "the ward should apply"
        );
        assert_eq!(
            hex_lattice::resolve_incoming(&state, 1),
            0,
            "precondition: this defence absorbs a one-hex spell entirely"
        );
    }
    publish_adapter_facts(&mut app);

    kindle(&mut app, UnitId(1), fight.defender_pos);
    run_until_acting(&mut app, UnitId(2));
    app.update();
    app.update();
    app.update();

    assert_eq!(
        disabled_count(&app, fight.defender),
        1,
        "burn ignores armour, so the hex goes down anyway"
    );
}

/// A burn ticks once per turn, however many frames that turn lasts.
///
/// The tick is driven by the edge where a `Turn` component is newly granted. A tick
/// keyed on "the acting unit has a burn" would fire every frame and empty a lattice
/// in about a second.
#[test]
fn a_burn_ticks_once_per_turn_however_long_the_turn_lasts() {
    let mut app = test_app(4);
    let fight = two_casters(&mut app);

    kindle(&mut app, UnitId(1), fight.defender_pos);
    run_until_acting(&mut app, UnitId(2));
    // Sit on the defender's turn without ending it.
    for _ in 0..8 {
        app.update();
    }

    assert_eq!(
        disabled_count(&app, fight.defender),
        1,
        "one turn is one hex, no matter how many frames the turn takes"
    );
    assert_eq!(
        burn_turns_left(&app, fight.defender),
        3,
        "and the four-turn countdown should have advanced exactly once"
    );
}

/// A burn stops when its turns are spent, and the ledger forgets it.
///
/// A one-turn burn that ticked twice, or a record that outlived its countdown, would
/// both show up here — the second as an effect still in the ledger with nothing left to
/// tick, which is how a fire would become permanent.
#[test]
fn a_burn_expires_when_its_turns_are_spent() {
    let mut app = test_app(1);
    let fight = two_casters(&mut app);

    kindle(&mut app, UnitId(1), fight.defender_pos);
    run_until_acting(&mut app, UnitId(2));
    app.update();
    app.update();
    assert_eq!(
        disabled_count(&app, fight.defender),
        1,
        "precondition: the one turn it had"
    );

    // Round the order back to the defender and give it another turn.
    run_until_acting(&mut app, UnitId(1));
    run_until_acting(&mut app, UnitId(2));
    app.update();
    app.update();

    assert_eq!(
        disabled_count(&app, fight.defender),
        1,
        "a spent burn must not take a second hex"
    );
    assert_eq!(
        burn_turns_left(&app, fight.defender),
        0,
        "the countdown is empty"
    );
    assert!(
        app.world().resource::<PersistentEffects>().is_empty(),
        "and the ledger drops the record rather than keeping a fire that cannot burn"
    );
}

/// Leaving gameplay forgets every effect.
///
/// Unit ids restart each session, so a ledger carried across one names somebody else's
/// unit next launch — and unlike a stale command, nothing ever drains an effect: it
/// would tick on a stranger every turn, forever.
#[test]
fn the_ledger_is_cleared_on_leaving_gameplay() {
    let mut app = test_app(3);
    let fight = two_casters(&mut app);
    kindle(&mut app, UnitId(1), fight.defender_pos);
    assert_eq!(
        app.world().resource::<PersistentEffects>().len(),
        1,
        "precondition: something is running"
    );

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();

    assert!(
        app.world().resource::<PersistentEffects>().is_empty(),
        "a new session must not inherit the last one's fires"
    );
}

/// A fight ending drops undelivered ticks but not the fires themselves.
///
/// The two halves differ. A due hit has nobody left to answer it once the auto-policy
/// stops running, exactly like the open decision the funnel drops beside it — but nothing
/// in the design puts a fire out because the party walked away. A burn is measured in the
/// target's own turns, which mean the same thing in any fight, so every turn it is owed
/// survives. Only round-bounded effects cannot, and this asserts the turn-bounded half.
#[test]
fn a_fight_ending_keeps_the_fires_it_started() {
    let mut app = test_app(3);
    let fight = two_casters(&mut app);
    kindle(&mut app, UnitId(1), fight.defender_pos);

    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Exploring);
    app.update();

    assert_eq!(
        app.world().resource::<PersistentEffects>().len(),
        1,
        "the fire keeps burning; only what could not be delivered is dropped"
    );
    assert_eq!(
        burn_turns_left(&app, fight.defender),
        3,
        "with all three turns still owed — a fight ending does not spend them"
    );
}

/// A burn on a unit with no lattice must not park a decision nobody can answer.
///
/// This is the deadlock the seam makes possible, and it is worth the fixture. A unit
/// spawned from an archetype `lattices.ron` does not define has no lattice — `hex_units`
/// warns and spawns it inert — and it still joins the turn order. The seam holds one
/// decision at a time and has exactly one answerer, which needs the decider's lattice to
/// pick hexes from. Park a choice for a unit that has none and nothing ever clears it: no
/// unit acts, every later cast and strike is refused, and the only escape is walking far
/// enough away to end the fight.
///
/// Two guards are checked here, in the two places the fire can start. The cast refuses by
/// name so the caster is not charged for damage with nowhere to land; and the tick drops
/// the hit rather than parking it, because a ledger entry can outlive the components it
/// was made against.
#[test]
fn a_burn_on_a_lattice_less_unit_never_parks_an_unanswerable_decision() {
    let mut app = test_app(2);

    // The inert unit joins the frozen roster without a lattice.
    let inert_coord = HexCoord::new_cubic(-1, 1, 0);
    let inert_pos = TilePos::new(inert_coord, GROUND);
    let inert = app
        .world_mut()
        .spawn((
            Faction::Hostile,
            UnitId(3),
            StandsOn(Standing {
                pos: inert_pos,
                span: HexSpan::new(0.0, 1.0),
            }),
            Initiative(5),
        ))
        .id();
    app.world_mut()
        .resource_mut::<UnitRegistry>()
        .register(UnitId(3), inert);
    let fight = two_casters(&mut app);

    kindle(&mut app, UnitId(1), inert_pos);

    assert!(
        app.world().resource::<PersistentEffects>().is_empty(),
        "a fire on something that cannot burn must not reach the ledger at all"
    );

    // Now the other half: a fire booked against a lattice the unit later loses. The
    // ledger entry is legitimate when it is made, so only the seam can catch this one.
    // `kindle` yields the caster's turn, so the order has to come back round first.
    run_until_acting(&mut app, UnitId(1));
    kindle(&mut app, UnitId(1), fight.defender_pos);
    assert_eq!(
        app.world().resource::<PersistentEffects>().len(),
        1,
        "precondition: the defender did take the fire"
    );
    app.world_mut()
        .entity_mut(fight.defender)
        .remove::<LatticeState>();
    publish_adapter_facts(&mut app);

    run_until_acting(&mut app, UnitId(2));
    for _ in 0..8 {
        app.update();
    }

    assert!(
        !app.world().resource::<PendingDecision>().is_open(),
        "the tick must drop a hit nobody can answer rather than park the fight on it"
    );
}

/// Every newly granted turn burns, even when a downing handoff does not roll the round.
///
/// `TurnOrder::remove` wraps `current` to the front **without** counting a round when the
/// unit going down was last in the order, so the front unit gets a second turn inside a
/// round it has already had one in. That is still a real `Turn`, not a continuation of
/// the old one, so a personal effect advances again.
///
/// The wrap is forced directly rather than played out through a real downing: what is
/// under test is the turn edge, and driving it through combat would make the test
/// depend on the AI, on strike damage and on how many hexes a fixture lattice has.
#[test]
fn a_second_turn_from_a_downing_wrap_ticks_a_burn_again() {
    let mut app = test_app(4);
    let fight = two_casters(&mut app);

    // The caster sets *itself* alight, so the burn sits on the unit at the front of the
    // order — the one the wrap hands a second turn to.
    kindle(&mut app, UnitId(1), TilePos::new(HexCoord::ORIGIN, GROUND));
    assert_eq!(
        burn_turns_left(&app, fight.caster),
        4,
        "precondition: four turns booked, none spent"
    );

    run_until_acting(&mut app, UnitId(1));
    app.update();
    app.update();
    let after_first = burn_turns_left(&app, fight.caster);
    assert_eq!(after_first, 3, "precondition: its own turn spent one");
    let round = app.world().resource::<TurnOrder>().round;

    // Hand the turn on, then drop the unit holding it — the wrap that grants the front
    // unit a second turn without counting a round.
    run_until_acting(&mut app, UnitId(2));
    app.world_mut()
        .entity_mut(fight.defender)
        .insert(Downed)
        .remove::<Turn>();
    app.world_mut()
        .resource_mut::<TurnOrder>()
        .remove(UnitId(2));
    app.world_mut().entity_mut(fight.caster).insert(Turn {
        movement_left: 4,
        acted: false,
    });
    for _ in 0..8 {
        app.update();
    }

    assert_eq!(
        app.world().resource::<TurnOrder>().round,
        round,
        "precondition: the wrap does not count a round, which is what makes this a trap"
    );
    assert_eq!(
        burn_turns_left(&app, fight.caster),
        after_first.saturating_sub(1),
        "a newly granted Turn must spend another tick even before a round rollover"
    );
}
