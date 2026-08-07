//! Bevy host for the renderer-free combat authority.
//!
//! This module freezes published ECS/world contracts once combat starts. It never
//! reaches into a map generator or renderer: exact surfaces come from `HexTile`
//! components, traversal from `Footing`, observation from `FactionMapKnowledge`, and
//! combatants from stable unit components.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use hex_assets::{
    CastingAxis, CombatSettings, ContentIndex, Effect, ElementCatalog, SpellBook, SubstanceTable,
    TargetShape, TargetingReach,
};
use hex_combat_core::{
    ArenaSnapshot, CombatLattice, CombatState, CombatUnit, CombatUnitProjection, ElementNames,
    FrozenCasting, FrozenCombatContent, FrozenEffect, FrozenRequirement, FrozenSpell,
    FrozenTargeting, RulesProfile,
};
use hex_core::{
    Busy, ControlOwner, Faction, GameCommand, Headroom, HexSpan, HexTile, KnowledgeState,
    PendingDecision, SubstanceId, TerrainSystems, TilePos, Turn, UnitId,
};
use hex_lattice::{LatticeSpec, LatticeState, LatticeStats};
use hex_perception::FactionMapKnowledge;
use hex_units::{Body, Downed, Footing, StandsOn, TerrainOccupancy, TerrainOccupancySystems};

use crate::{Initiative, TurnOrder};

/// Live renderer-free state. ECS components are projections of this resource.
#[derive(Resource, Debug)]
pub(crate) struct CombatAuthority {
    pub(crate) state: CombatState,
    published_events: usize,
    published_round: u32,
    adapter_pending: bool,
}

impl CombatAuthority {
    /// Whether this command is reduced entirely inside `hex_combat_core`.
    pub(crate) fn handles(&self, command: &GameCommand) -> bool {
        matches!(
            command,
            GameCommand::MoveAlong { .. }
                | GameCommand::Strike { .. }
                | GameCommand::EndTurn { .. }
                | GameCommand::Channel { .. }
                | GameCommand::ChooseDisables { .. }
        )
    }

    /// Marks one content-dependent adapter transition for exact adoption after
    /// deferred ECS writes settle.
    pub(crate) fn mark_adapter_pending(&mut self) {
        self.adapter_pending = true;
    }

    pub(crate) fn adapter_pending(&self) -> bool {
        self.adapter_pending
    }

    pub(crate) fn finish_adapter_adoption(&mut self) {
        self.adapter_pending = false;
        self.published_round = self.state.round;
    }

    /// Drains authority events exactly once into the Bevy presentation stream.
    pub(crate) fn drain_events(&mut self, output: &mut Vec<crate::CombatEvent>) {
        assert!(
            self.published_events <= self.state.events.len(),
            "combat authority event transcript shrank after publication"
        );
        if let Some(unpublished) = self.state.events.get(self.published_events..) {
            output.extend(unpublished.iter().cloned());
        }
        self.published_events = self.state.events.len();
    }

    /// Publishes each authority-owned round edge exactly once.
    pub(crate) fn drain_rounds(&mut self, output: &mut Vec<hex_core::RoundElapsed>) {
        output.extend((self.published_round..self.state.round).map(|_| hex_core::RoundElapsed));
        self.published_round = self.state.round;
    }
}

/// Loud initialization failure retained for diagnostics and headless assertions.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub(crate) struct CombatAuthorityFailure(pub(crate) String);

/// Returns a read-only clone of the renderer-free authority for contract tests and
/// diagnostics. Callers cannot mutate combat through this observation seam.
pub(crate) fn snapshot(world: &World) -> Result<CombatState, String> {
    if let Some(authority) = world.get_resource::<CombatAuthority>() {
        return Ok(authority.state.clone());
    }
    Err(world.get_resource::<CombatAuthorityFailure>().map_or_else(
        || "combat authority is not active".to_owned(),
        |failure| failure.0.clone(),
    ))
}

/// Marks a complete ECS projection for validated adoption at the next settled
/// boundary.
///
/// Content adapters and contract fixtures call this after publishing domain facts
/// that are intentionally resolved outside the pure command reducer. The authority
/// still validates the exact roster, identities, order, and mutable projection before
/// accepting it.
pub(crate) fn publish_adapter_facts(world: &mut World) -> Result<(), String> {
    if let Some(mut authority) = world.get_resource_mut::<CombatAuthority>() {
        authority.mark_adapter_pending();
        return Ok(());
    }
    Err(world.get_resource::<CombatAuthorityFailure>().map_or_else(
        || "combat authority is not active".to_owned(),
        |failure| failure.0.clone(),
    ))
}

type TileFacts<'w, 's> = Query<
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

type UnitFacts<'w, 's> = Query<
    'w,
    's,
    (
        &'static UnitId,
        Option<&'static ControlOwner>,
        &'static Faction,
        &'static StandsOn,
        Option<&'static Body>,
        Option<&'static Turn>,
        Has<Busy>,
        Has<Downed>,
        Option<&'static Initiative>,
        Option<&'static LatticeSpec>,
        Option<&'static LatticeState>,
        Option<&'static LatticeStats>,
    ),
>;

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(
        OnEnter(hex_core::Mode::Combat),
        initialize
            .after(crate::turns::begin_combat)
            .after(hex_units::MovementSystems::HaltOnCombat),
    )
    .add_systems(OnExit(hex_core::Mode::Combat), clear)
    .add_systems(
        Update,
        refresh_arena_after_terrain_publication
            .after(TerrainOccupancySystems::Publish)
            .before(TerrainSystems::ReconcileActors)
            .run_if(in_state(hex_core::Mode::Combat)),
    )
    .add_systems(
        Update,
        refresh_arena_after_terrain_publication
            .after(TerrainOccupancySystems::Publish)
            .after(hex_core::PerceptionSystems::PublishKnowledge)
            .before(crate::CombatSystems::Act)
            .run_if(in_state(hex_core::Mode::Combat)),
    )
    .add_systems(
        Update,
        reconcile_domain_movement
            .after(hex_units::MovementSystems::Reconcile)
            .before(crate::CombatSystems::Apply)
            .run_if(in_state(hex_core::Mode::Combat)),
    )
    .add_systems(
        PostUpdate,
        assert_equivalent_projections.run_if(in_state(hex_core::Mode::Combat)),
    );
}

/// Replaces the movement authority's frozen arena after settled terrain edits.
///
/// Unit/order state remains authoritative and continuous; only the published surface,
/// traversal, and observation graph is rebuilt. This runs at the occupancy publication
/// boundary, before any new command can be reduced against stale terrain.
#[expect(
    clippy::too_many_arguments,
    reason = "arena refresh consumes the same independent published facts as combat entry"
)]
fn refresh_arena_after_terrain_publication(
    mut commands: Commands,
    occupancy: Option<Res<TerrainOccupancy>>,
    mut authority: Option<ResMut<CombatAuthority>>,
    substances: Option<Res<SubstanceTable>>,
    blockers: Option<Res<hex_core::TraversalBlockers>>,
    spatial: Option<Res<FactionMapKnowledge>>,
    tiles: TileFacts,
    units: UnitFacts,
) {
    let Some(occupancy) = occupancy else {
        return;
    };
    if !occupancy.is_changed() {
        return;
    }
    let Some(authority) = authority.as_deref_mut() else {
        return;
    };
    let Some(substances) = substances.as_deref() else {
        let reason = "cannot refresh combat arena: SubstanceTable is unavailable".to_owned();
        commands.remove_resource::<CombatAuthority>();
        commands.insert_resource(CombatAuthorityFailure(reason));
        return;
    };
    match build_arena(
        substances,
        blockers.as_deref(),
        spatial.as_deref(),
        &tiles,
        &units,
    ) {
        Ok(arena) => authority.state.arena = arena,
        Err(reason) => {
            error!("combat arena refresh failed after terrain publication: {reason}");
            commands.remove_resource::<CombatAuthority>();
            commands.insert_resource(CombatAuthorityFailure(reason));
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the host freezes independent published resources and queries at one explicit boundary"
)]
fn initialize(
    mut commands: Commands,
    settings: Option<Res<CombatSettings>>,
    elements: Option<Res<ElementCatalog>>,
    spells: Option<Res<SpellBook>>,
    content: Option<Res<ContentIndex>>,
    substances: Option<Res<SubstanceTable>>,
    blockers: Option<Res<hex_core::TraversalBlockers>>,
    spatial: Option<Res<FactionMapKnowledge>>,
    tiles: TileFacts,
    units: UnitFacts,
    order: Res<TurnOrder>,
    pending: Res<PendingDecision>,
    revivals: Res<crate::turns::PendingRevivals>,
) {
    commands.remove_resource::<CombatAuthority>();
    commands.remove_resource::<CombatAuthorityFailure>();
    let result = freeze(
        settings.as_deref(),
        elements.as_deref(),
        spells.as_deref(),
        content.as_deref(),
        substances.as_deref(),
        blockers.as_deref(),
        spatial.as_deref(),
        &tiles,
        &units,
        &order,
        &pending,
        &revivals,
    );
    match result {
        Ok(state) => {
            commands.insert_resource(CombatAuthority {
                state,
                published_events: 0,
                published_round: 0,
                adapter_pending: false,
            });
        }
        Err(reason) => {
            error!("combat authority initialization failed: {reason}");
            commands.insert_resource(CombatAuthorityFailure(reason));
        }
    }
}

fn reconcile_domain_movement(
    mut authority: Option<ResMut<CombatAuthority>>,
    positions: Query<(&UnitId, &StandsOn)>,
) {
    let Some(authority) = authority.as_deref_mut() else {
        return;
    };
    for (unit, standing) in &positions {
        let moving = authority
            .state
            .units
            .get(unit)
            .is_some_and(|actor| actor.motion.is_some());
        if moving {
            let result = authority.state.reach_movement(*unit, standing.0.pos);
            if result.is_err() {
                error!(
                    "ECS movement projection left the committed authority route for \
                     {unit:?} at {:?}: {result:?}",
                    standing.0.pos,
                );
                assert!(
                    result.is_ok(),
                    "movement projection left its committed authority route"
                );
                break;
            }
        }
    }
}

type ProjectionFacts<'w, 's> = Query<
    'w,
    's,
    (
        &'static UnitId,
        &'static StandsOn,
        Option<&'static Turn>,
        Has<Busy>,
        Has<Downed>,
        Option<&'static LatticeState>,
    ),
>;

fn assert_equivalent_projections(
    mut authority: Option<ResMut<CombatAuthority>>,
    order: Res<TurnOrder>,
    pending: Res<PendingDecision>,
    revivals: Res<crate::turns::PendingRevivals>,
    units: ProjectionFacts,
    mut events: MessageWriter<crate::CombatEvent>,
    mut rounds: MessageWriter<hex_core::RoundElapsed>,
) {
    let Some(authority) = authority.as_deref_mut() else {
        return;
    };
    if authority.adapter_pending {
        let projection = units
            .iter()
            .map(
                |(id, standing, turn, busy, downed, lattice)| CombatUnitProjection {
                    id: *id,
                    position: standing.0.pos,
                    turn: turn.copied(),
                    busy,
                    downed,
                    lattice: lattice.cloned(),
                },
            )
            .collect::<Vec<_>>();
        let result = authority.state.adopt_projection(
            order.order().to_vec(),
            order.current(),
            order.round,
            pending.clone(),
            revivals.snapshot(),
            projection,
        );
        if result.is_err() {
            error!("content adapter published an invalid combat projection: {result:?}");
            assert!(
                result.is_ok(),
                "content adapter published an invalid combat projection"
            );
        }
        authority.finish_adapter_adoption();
    }
    authority.state.settle_outcome();
    let state = &authority.state;
    assert_eq!(state.order, order.order(), "initiative order projection");
    assert_eq!(state.current(), order.current(), "current turn projection");
    assert_eq!(state.round, order.round, "round projection");
    assert_eq!(state.pending, *pending, "pending decision projection");
    for (id, standing, turn, busy, downed, lattice) in &units {
        let actor = state.units.get(id);
        assert!(
            actor.is_some(),
            "ECS projected unit {id:?} outside the authority roster"
        );
        let Some(actor) = actor else {
            return;
        };
        assert_eq!(actor.position, standing.0.pos, "{id:?} exact position");
        assert_eq!(actor.turn.as_ref(), turn, "{id:?} turn projection");
        assert_eq!(actor.busy, busy, "{id:?} busy projection");
        assert_eq!(actor.downed, downed, "{id:?} downed projection");
        assert_eq!(
            actor.lattice.as_ref().map(|value| &value.state),
            lattice,
            "{id:?} lattice projection"
        );
    }
    let mut published = Vec::new();
    authority.drain_events(&mut published);
    events.write_batch(published);
    let mut elapsed = Vec::new();
    authority.drain_rounds(&mut elapsed);
    rounds.write_batch(elapsed);
}

fn clear(mut commands: Commands) {
    commands.remove_resource::<CombatAuthority>();
    commands.remove_resource::<CombatAuthorityFailure>();
}

#[expect(
    clippy::too_many_arguments,
    reason = "freezing one authority snapshot deliberately names every published input"
)]
fn freeze(
    settings: Option<&CombatSettings>,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
    content: Option<&ContentIndex>,
    substances: Option<&SubstanceTable>,
    blockers: Option<&hex_core::TraversalBlockers>,
    spatial: Option<&FactionMapKnowledge>,
    tiles: &TileFacts,
    units: &UnitFacts,
    order: &TurnOrder,
    pending: &PendingDecision,
    revivals: &crate::turns::PendingRevivals,
) -> Result<CombatState, String> {
    let settings = settings.ok_or("CombatSettings is unavailable")?;
    let substances = substances.ok_or("SubstanceTable is unavailable")?;
    let mut roster = Vec::new();
    for (
        id,
        owner,
        faction,
        standing,
        _body,
        turn,
        busy,
        downed,
        initiative,
        spec,
        lattice,
        lattice_stats,
    ) in units.iter()
    {
        let lattice = match (spec, lattice, lattice_stats) {
            (Some(spec), Some(state), Some(stats)) => Some(CombatLattice {
                spec: spec.clone(),
                state: state.clone(),
                stats: stats.clone(),
            }),
            (None, None, None) => None,
            _ => {
                return Err(format!(
                    "unit {id:?} exposes a partial lattice projection at combat entry"
                ));
            }
        };
        roster.push(CombatUnit {
            id: *id,
            seat: owner.copied().unwrap_or_default().0,
            faction: *faction,
            position: standing.0.pos,
            initiative: initiative.map_or(settings.default_initiative, |value| value.0),
            turn: turn.copied(),
            busy,
            motion: None,
            downed,
            lattice,
        });
    }
    if roster.is_empty() {
        return Err("combat roster is empty".to_owned());
    }

    let arena = build_arena(substances, blockers, spatial, tiles, units)?;

    let mut element_names = BTreeMap::new();
    if let Some(elements) = elements {
        for raw in 0..elements.len() {
            let raw = u16::try_from(raw).map_err(|_error| "element catalog exceeds u16 ids")?;
            let id = hex_core::ElementId(raw);
            if let Some(name) = elements.name(id) {
                element_names.insert(id, name.to_owned());
            }
        }
    }

    let rules = RulesProfile::new("runtime", settings.movement_per_turn)?
        .with_strike_disable_count(settings.strike_disables)
        .with_cast_policy(
            settings.levels_per_bonus_range,
            settings.divination_rounds_per_tier,
        );
    let spell_names = spells
        .into_iter()
        .flat_map(SpellBook::iter)
        .map(|(id, name, _)| (id, name.to_owned()));
    let names = ElementNames::new(element_names).with_spells(spell_names);
    let frozen_content = match (elements, spells, content) {
        (Some(elements), Some(spells), Some(content)) => freeze_content(elements, spells, content)?,
        _ => FrozenCombatContent::default(),
    };
    let state = CombatState::start_with_content_and_session(
        rules,
        arena,
        names,
        frozen_content,
        roster,
        pending.clone(),
        revivals.snapshot(),
    )?;
    if state.order != order.order()
        || state.current() != order.current()
        || state.round != order.round
    {
        return Err(format!(
            "authority initiative projection disagrees: core={:?}/{:?}/{} ecs={:?}/{:?}/{}",
            state.order,
            state.current(),
            state.round,
            order.order(),
            order.current(),
            order.round
        ));
    }
    Ok(state)
}

fn build_arena(
    substances: &SubstanceTable,
    blockers: Option<&hex_core::TraversalBlockers>,
    spatial: Option<&FactionMapKnowledge>,
    tiles: &TileFacts,
    units: &UnitFacts,
) -> Result<ArenaSnapshot, String> {
    let mut bodies = Vec::new();
    let mut unit_bodies = BTreeMap::new();
    for (id, _, _, _, body, ..) in units.iter() {
        if let Some(body) = body.copied() {
            if !bodies.contains(&body) {
                bodies.push(body);
            }
        }
        unit_bodies.insert(*id, body.copied());
    }
    let all_tiles = tiles.iter().collect::<Vec<_>>();
    let surfaces = all_tiles
        .iter()
        .map(|(position, ..)| **position)
        .collect::<BTreeSet<_>>();
    let mut links = BTreeSet::new();
    let mut links_by_body = Vec::new();
    for body in bodies {
        let footing = Footing::from_tiles(all_tiles.iter().copied(), substances, body, blockers);
        let mut body_links = BTreeSet::new();
        for from in footing.standings() {
            for neighbor in from.pos.coord.neighbors() {
                for to in footing.steps_from(from, neighbor) {
                    links.insert((from.pos, to.pos));
                    body_links.insert((from.pos, to.pos));
                }
            }
        }
        links_by_body.push((body, body_links));
    }
    let mut arena = ArenaSnapshot::new(surfaces, links)?;
    for (&unit, body) in &unit_bodies {
        let links = body
            .and_then(|body| {
                links_by_body
                    .iter()
                    .find(|(candidate, _)| *candidate == body)
            })
            .map_or_else(BTreeSet::new, |(_, links)| links.clone());
        arena = arena.with_unit_links(unit, links)?;
    }
    if let Some(spatial) = spatial {
        for faction in [Faction::Player, Faction::Hostile] {
            let observed = spatial
                .faction(faction)
                .surfaces()
                .filter_map(|(position, known)| {
                    (known.state() == KnowledgeState::Observed).then_some(position)
                });
            arena = arena.with_observation(faction, observed);
        }
    }

    Ok(arena)
}

fn freeze_content(
    elements: &ElementCatalog,
    spells: &SpellBook,
    content: &ContentIndex,
) -> Result<FrozenCombatContent, String> {
    let frozen_spells = spells
        .iter()
        .filter_map(|(id, name, spell)| {
            let targeting = match (&spell.targeting.shape, spell.targeting.reach) {
                (TargetShape::SelfCast, _) => FrozenTargeting::SelfOnly,
                (TargetShape::Single, TargetingReach::Touch) => FrozenTargeting::Touch,
                (TargetShape::Single, TargetingReach::Ranged) => FrozenTargeting::ExactSurface {
                    range: u32::from(spell.targeting.range),
                },
                _ => return None,
            };
            let effects = spell
                .effects
                .iter()
                .map(|effect| match *effect {
                    Effect::DisableHexes {
                        count,
                        targeted: false,
                    } => Some(FrozenEffect::DisableHexes {
                        count: u16::from(count),
                    }),
                    Effect::Burn { turns } => Some(FrozenEffect::Burn { turns }),
                    Effect::RestoreHexes { count } => Some(FrozenEffect::RestoreHexes {
                        count: u16::from(count),
                    }),
                    Effect::Reveal { tier } => Some(FrozenEffect::Reveal {
                        tier: u32::from(tier),
                    }),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            let requirements = content
                .requirements(id)?
                .iter()
                .map(|&(element, mana)| FrozenRequirement { element, mana })
                .collect();
            let casting = match content.casting(id)? {
                CastingAxis::Evocation => FrozenCasting::Evocation,
                CastingAxis::Enchantment { defense } => FrozenCasting::Enchantment { defense },
            };
            Some(FrozenSpell {
                id,
                name: name.to_owned(),
                requirements,
                casting,
                targeting,
                effects,
            })
        })
        .collect::<Vec<_>>();
    let fusions = (0..elements.len())
        .filter_map(|raw| {
            let raw = u16::try_from(raw).ok()?;
            let id = hex_core::ElementId(raw);
            elements.recipe(id).map(|recipe| {
                (
                    id,
                    recipe
                        .iter()
                        .map(|&(element, mana)| FrozenRequirement { element, mana })
                        .collect(),
                )
            })
        })
        .collect::<Vec<_>>();
    FrozenCombatContent::new(frozen_spells, fusions)
}
