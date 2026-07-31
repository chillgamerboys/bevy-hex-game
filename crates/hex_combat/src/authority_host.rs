//! Bevy host for the renderer-free combat authority.
//!
//! This module freezes published ECS/world contracts once combat starts. It never
//! reaches into a map generator or renderer: exact surfaces come from `HexTile`
//! components, traversal from `Footing`, observation from `FactionMapKnowledge`, and
//! combatants from stable unit components.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use hex_assets::{CombatSettings, ElementCatalog, SubstanceTable};
use hex_combat_core::{
    ArenaSnapshot, CombatLattice, CombatState, CombatUnit, CommandRefusal, ElementNames,
    RulesProfile,
};
use hex_core::{
    Busy, ControlOwner, Faction, GameCommand, Headroom, HexSpan, HexTile, IssuedCommand,
    KnowledgeState, PendingDecision, SubstanceId, TilePos, Turn, UnitId,
};
use hex_lattice::{LatticeSpec, LatticeState, LatticeStats};
use hex_perception::FactionMapKnowledge;
use hex_units::{Body, Downed, Footing, StandsOn};

use crate::{Initiative, TurnOrder};

/// Live renderer-free state. ECS components are projections of this resource.
#[derive(Resource, Debug)]
pub(crate) struct CombatAuthority {
    pub(crate) state: CombatState,
    shadow_complete: bool,
}

impl CombatAuthority {
    pub(crate) fn shadow_complete(&self) -> bool {
        self.shadow_complete
    }

    pub(crate) fn disable_shadow(&mut self, reason: &str) {
        if self.shadow_complete {
            warn!("combat authority shadow comparison disabled: {reason}");
            self.shadow_complete = false;
        }
    }

    /// Applies reducer-covered commands for migration-time field comparison.
    ///
    /// Content-resolved casting and restoration are cut over in the consolidation
    /// lane. Encountering one disables later comparisons for this combat rather than
    /// pretending that an incomplete shadow is authoritative.
    pub(crate) fn apply_shadow(
        &mut self,
        issued: &IssuedCommand,
    ) -> Option<Result<(), CommandRefusal>> {
        if !self.shadow_complete {
            return None;
        }
        let covered = matches!(
            issued.command,
            GameCommand::MoveAlong { .. }
                | GameCommand::Strike { .. }
                | GameCommand::EndTurn { .. }
                | GameCommand::Channel { .. }
                | GameCommand::ChooseDisables { .. }
        );
        if !covered {
            self.shadow_complete = false;
            return None;
        }
        Some(self.state.apply(issued.clone()))
    }
}

/// Loud initialization failure retained for diagnostics and headless assertions.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub(crate) struct CombatAuthorityFailure(pub(crate) String);

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

#[expect(
    clippy::too_many_arguments,
    reason = "the host freezes independent published resources and queries at one explicit boundary"
)]
fn initialize(
    mut commands: Commands,
    settings: Option<Res<CombatSettings>>,
    elements: Option<Res<ElementCatalog>>,
    substances: Option<Res<SubstanceTable>>,
    blockers: Option<Res<hex_core::TraversalBlockers>>,
    spatial: Option<Res<FactionMapKnowledge>>,
    tiles: TileFacts,
    units: UnitFacts,
    order: Res<TurnOrder>,
) {
    commands.remove_resource::<CombatAuthority>();
    commands.remove_resource::<CombatAuthorityFailure>();
    let result = freeze(
        settings.as_deref(),
        elements.as_deref(),
        substances.as_deref(),
        blockers.as_deref(),
        spatial.as_deref(),
        &tiles,
        &units,
        &order,
    );
    match result {
        Ok(state) => {
            commands.insert_resource(CombatAuthority {
                state,
                shadow_complete: true,
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
    if !authority.shadow_complete {
        return;
    }
    for (unit, standing) in &positions {
        let moving = authority
            .state
            .units
            .get(unit)
            .is_some_and(|actor| actor.motion.is_some());
        if moving {
            if let Err(error) = authority.state.reach_movement(*unit, standing.0.pos) {
                error!(
                    "ECS movement projection left the committed authority route for \
                     {unit:?} at {:?}: {error:?}",
                    standing.0.pos
                );
                authority.disable_shadow("movement projection left its committed route");
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
    units: ProjectionFacts,
) {
    let Some(authority) = authority.as_deref_mut() else {
        return;
    };
    if !authority.shadow_complete {
        return;
    }
    let state = &authority.state;
    assert_eq!(state.order, order.order(), "initiative order projection");
    assert_eq!(state.current(), order.current(), "current turn projection");
    assert_eq!(state.round, order.round, "round projection");
    assert_eq!(state.pending, *pending, "pending decision projection");
    for (id, standing, turn, busy, downed, lattice) in &units {
        let Some(actor) = state.units.get(id) else {
            error!("ECS projected unknown combat unit {id:?}");
            authority.disable_shadow("ECS projected a unit absent from the authority roster");
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
    substances: Option<&SubstanceTable>,
    blockers: Option<&hex_core::TraversalBlockers>,
    spatial: Option<&FactionMapKnowledge>,
    tiles: &TileFacts,
    units: &UnitFacts,
    order: &TurnOrder,
) -> Result<CombatState, String> {
    let settings = settings.ok_or("CombatSettings is unavailable")?;
    let substances = substances.ok_or("SubstanceTable is unavailable")?;
    let mut bodies = Vec::new();
    let mut roster = Vec::new();
    for (
        id,
        owner,
        faction,
        standing,
        body,
        turn,
        busy,
        downed,
        initiative,
        spec,
        lattice,
        lattice_stats,
    ) in units.iter()
    {
        if let Some(body) = body.copied() {
            if !bodies.contains(&body) {
                bodies.push(body);
            }
        }
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

    let all_tiles = tiles.iter().collect::<Vec<_>>();
    let surfaces = all_tiles
        .iter()
        .map(|(position, ..)| **position)
        .collect::<BTreeSet<_>>();
    let mut links = BTreeSet::new();
    for body in bodies {
        let footing = Footing::from_tiles(all_tiles.iter().copied(), substances, body, blockers);
        for from in footing.standings() {
            for neighbor in from.pos.coord.neighbors() {
                for to in footing.steps_from(from, neighbor) {
                    links.insert((from.pos, to.pos));
                }
            }
        }
    }
    let mut arena = ArenaSnapshot::new(surfaces, links)?;
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
        .with_strike_disable_count(settings.strike_disables);
    let state = CombatState::start(rules, arena, ElementNames::new(element_names), roster)?;
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
