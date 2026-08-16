//! Gameplay-owned authority adapter for complete Campaign checkpoints.
//!
//! This module consumes only public terrain projections. It never asks `hex_map` how a
//! world was generated, and it keeps the complete host checkpoint distinct from the
//! disclosure-limited replicas sent to clients.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use hex_assets::{ContentIndex, ElementCatalog, FormationCatalog, LatticeLibrary, SubstanceTable};
use hex_combat::{PersistentEffects, PersistentEffectsCheckpointError};
use hex_core::{
    Busy, EffectEnd, EffectId, Faction, GameplaySetup, GameplaySetupFailure, Headroom, HexSpan,
    HexTile, PartyFormation, PersistentEffect, SubstanceId, TilePos, TraversalBlockers,
    TraversalProfile, UnitId,
};
use hex_lattice::{LatticeState, LatticeStateError};
use hex_multiplayer::{
    BoundError, BoundedText, BoundedVec, CampaignEffectCheckpointV2, CampaignEffectLedgerV2,
    CampaignUnitCheckpointV2, MAX_CAMPAIGN_EFFECTS, MAX_IDENTITY_BYTES, MAX_SESSION_UNITS,
};
use hex_units::{
    Archetype, AuthoredObjectOccupancy, Body, Downed, Footing, MovingTo, Party, StandsOn,
};

/// Gameplay-owned portion of a complete host Campaign checkpoint.
///
/// L3 composes this with the independently exported world checkpoint and session
/// metadata. The type carries no camera, selection, entity, seat, identity, or transport
/// state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignGameplayCheckpointV2 {
    /// Every authority actor in stable unit order.
    pub units: BoundedVec<CampaignUnitCheckpointV2, MAX_SESSION_UNITS>,
    /// Exact authority-private persistent-effect ledger.
    pub effects: CampaignEffectLedgerV2,
    /// Exact player-party formation.
    pub formation: PartyFormation,
}

/// A gameplay checkpoint waiting for ordinary scenario actors to be spawned.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct PendingCampaignGameplayCheckpointV2(pub CampaignGameplayCheckpointV2);

/// Typed result of the most recent gameplay checkpoint restore attempt.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct CampaignGameplayRestoreResultV2 {
    /// Applied identity or exact refusal.
    pub outcome: CampaignGameplayRestoreOutcomeV2,
}

/// Whether a gameplay checkpoint was applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CampaignGameplayRestoreOutcomeV2 {
    /// Every actor, effect, and formation assignment was replaced atomically.
    Applied {
        /// Number of restored actors.
        unit_count: usize,
        /// Number of restored effects.
        effect_count: usize,
    },
    /// The candidate failed before any gameplay mutation.
    Refused(CampaignGameplayCheckpointError),
}

/// Why authoritative gameplay state cannot be exported or restored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CampaignGameplayCheckpointError {
    /// A required gameplay resource is absent.
    MissingResource(&'static str),
    /// No actors exist in the checkpoint or runtime.
    EmptyRoster,
    /// Unit records are not in strict id order.
    NonCanonicalUnits,
    /// Two actors occupy the same exact surface.
    DuplicatePosition(TilePos),
    /// A bounded checkpoint field is invalid.
    Bound(BoundError),
    /// Runtime and checkpoint actor identities differ.
    RosterMismatch,
    /// A spawned scenario actor is missing a required gameplay component.
    IncompleteRuntimeUnit(UnitId),
    /// A unit's immutable faction differs from the spawned scenario.
    FactionMismatch(UnitId),
    /// A unit's shipped archetype differs from the spawned scenario.
    ArchetypeMismatch(UnitId),
    /// Saved and spawned lattice presence differs.
    LatticePresenceMismatch(UnitId),
    /// A saved lattice is unavailable in accepted shipped content.
    MissingLatticeArchetype(UnitId),
    /// A saved lattice violates its immutable inscription or current rules.
    InvalidLattice(UnitId, LatticeStateError),
    /// The downed flag disagrees with complete lattice disablement.
    InvalidDownedState(UnitId),
    /// An actor is not on valid public footing.
    InvalidFooting(UnitId),
    /// An actor still has a command or domain movement in flight.
    UnitInFlight(UnitId),
    /// The runtime party resource differs from player-faction actors.
    RuntimePartyMismatch,
    /// The formation does not assign every and only player-faction actor.
    FormationMembership,
    /// The formation names an unavailable preset.
    FormationPreset,
    /// The formation repeats or names a slot outside its preset.
    FormationSlot,
    /// The authority effect ledger is not checkpoint-safe or canonical.
    Effects(PersistentEffectsCheckpointError),
    /// An effect references an actor absent from the checkpoint.
    DanglingEffectUnit(EffectId),
    /// An effect has an impossible lifetime at a quiescent exploration boundary.
    InvalidEffectLifetime(EffectId),
    /// An enchantment-bound effect names no live target enchantment.
    DanglingEffectEnchantment(EffectId),
}

impl std::fmt::Display for CampaignGameplayCheckpointError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingResource(name) => write!(formatter, "Campaign gameplay requires {name}"),
            Self::EmptyRoster => formatter.write_str("Campaign gameplay has no actors"),
            Self::NonCanonicalUnits => {
                formatter.write_str("Campaign actors are not in canonical order")
            }
            Self::DuplicatePosition(_) => {
                formatter.write_str("Campaign actors repeat an exact position")
            }
            Self::Bound(error) => write!(formatter, "Campaign gameplay field is invalid: {error}"),
            Self::RosterMismatch => {
                formatter.write_str("Campaign roster no longer matches the scenario")
            }
            Self::IncompleteRuntimeUnit(unit) => {
                write!(formatter, "Campaign runtime unit {} is incomplete", unit.0)
            }
            Self::FactionMismatch(unit) => {
                write!(formatter, "Campaign faction changed for unit {}", unit.0)
            }
            Self::ArchetypeMismatch(unit) => {
                write!(formatter, "Campaign archetype changed for unit {}", unit.0)
            }
            Self::LatticePresenceMismatch(unit) => {
                write!(
                    formatter,
                    "Campaign lattice presence changed for unit {}",
                    unit.0
                )
            }
            Self::MissingLatticeArchetype(unit) => {
                write!(
                    formatter,
                    "Campaign lattice content is missing for unit {}",
                    unit.0
                )
            }
            Self::InvalidLattice(unit, error) => {
                write!(
                    formatter,
                    "Campaign lattice for unit {} is invalid: {error}",
                    unit.0
                )
            }
            Self::InvalidDownedState(unit) => {
                write!(
                    formatter,
                    "Campaign downed state is invalid for unit {}",
                    unit.0
                )
            }
            Self::InvalidFooting(unit) => {
                write!(formatter, "Campaign footing is invalid for unit {}", unit.0)
            }
            Self::UnitInFlight(unit) => {
                write!(
                    formatter,
                    "Campaign unit {} is still moving or busy",
                    unit.0
                )
            }
            Self::RuntimePartyMismatch => {
                formatter.write_str("Campaign runtime party does not match player actors")
            }
            Self::FormationMembership => {
                formatter.write_str("Campaign formation does not match the player party")
            }
            Self::FormationPreset => {
                formatter.write_str("Campaign formation preset is unavailable")
            }
            Self::FormationSlot => formatter.write_str("Campaign formation slot is invalid"),
            Self::Effects(error) => write!(formatter, "Campaign effects are invalid: {error}"),
            Self::DanglingEffectUnit(id) => {
                write!(
                    formatter,
                    "Campaign effect {} references an absent actor",
                    id.0
                )
            }
            Self::InvalidEffectLifetime(id) => {
                write!(
                    formatter,
                    "Campaign effect {} has an invalid lifetime",
                    id.0
                )
            }
            Self::DanglingEffectEnchantment(id) => write!(
                formatter,
                "Campaign effect {} references an absent enchantment",
                id.0
            ),
        }
    }
}

impl std::error::Error for CampaignGameplayCheckpointError {}

impl From<BoundError> for CampaignGameplayCheckpointError {
    fn from(error: BoundError) -> Self {
        Self::Bound(error)
    }
}

impl From<PersistentEffectsCheckpointError> for CampaignGameplayCheckpointError {
    fn from(error: PersistentEffectsCheckpointError) -> Self {
        Self::Effects(error)
    }
}

#[derive(Debug, Clone)]
struct RuntimeUnit {
    entity: Entity,
    id: UnitId,
    faction: Faction,
    archetype: String,
    has_lattice: bool,
    has_standing: bool,
    has_transform: bool,
}

#[derive(Debug, Clone)]
struct PreparedUnit {
    entity: Entity,
    standing: hex_units::Standing,
    lattice: Option<LatticeState>,
    downed: bool,
    display_name: String,
}

#[derive(Debug)]
struct PreparedGameplayRestore {
    units: Vec<PreparedUnit>,
    effects: Vec<(EffectId, PersistentEffect)>,
    next_effect_id: u64,
    formation: PartyFormation,
}

/// Registers the gameplay-owned restore adapter.
pub fn plugin(app: &mut App) {
    app.add_systems(
        OnEnter(hex_core::Screen::Gameplay),
        restore_pending_campaign_gameplay.in_set(GameplaySetup::Restore),
    );
}

/// Exports complete authoritative actor state from public gameplay facts.
pub fn export_campaign_gameplay_checkpoint(
    world: &mut World,
) -> Result<CampaignGameplayCheckpointV2, CampaignGameplayCheckpointError> {
    let mut query = world.query::<(
        &UnitId,
        &Faction,
        &Archetype,
        &StandsOn,
        Option<&LatticeState>,
        Option<&Name>,
        Has<Downed>,
        Has<Busy>,
        Has<MovingTo>,
    )>();
    let mut units = query
        .iter(world)
        .map(
            |(id, faction, archetype, standing, lattice, name, downed, busy, moving)| {
                if busy || moving {
                    return Err(CampaignGameplayCheckpointError::UnitInFlight(*id));
                }
                Ok(CampaignUnitCheckpointV2 {
                    unit: *id,
                    faction: *faction,
                    archetype_identity: BoundedText::<MAX_IDENTITY_BYTES>::new(
                        archetype.0.clone(),
                    )?,
                    position: standing.0.pos,
                    lattice: lattice.cloned(),
                    downed,
                    display_name: BoundedText::<MAX_IDENTITY_BYTES>::new(
                        name.map_or_else(|| format!("Unit {}", id.0), |name| name.to_string()),
                    )?,
                })
            },
        )
        .collect::<Result<Vec<_>, CampaignGameplayCheckpointError>>()?;
    units.sort_by_key(|unit| unit.unit);
    if units.is_empty() {
        return Err(CampaignGameplayCheckpointError::EmptyRoster);
    }

    let effects = world.get_resource::<PersistentEffects>().ok_or(
        CampaignGameplayCheckpointError::MissingResource("PersistentEffects"),
    )?;
    let (next_id, effects) = effects.authority_checkpoint()?;
    let effects = CampaignEffectLedgerV2 {
        next_id,
        effects: BoundedVec::<CampaignEffectCheckpointV2, MAX_CAMPAIGN_EFFECTS>::new(
            effects
                .into_iter()
                .map(|(id, effect)| CampaignEffectCheckpointV2 { id, effect })
                .collect(),
        )?,
    };
    let formation = world.get_resource::<PartyFormation>().cloned().ok_or(
        CampaignGameplayCheckpointError::MissingResource("PartyFormation"),
    )?;
    let checkpoint = CampaignGameplayCheckpointV2 {
        units: BoundedVec::new(units)?,
        effects,
        formation,
    };
    prepare_campaign_gameplay_restore(world, &checkpoint)?;
    Ok(checkpoint)
}

/// Queues a complete gameplay candidate for the ordinary Gameplay restore phase.
pub fn queue_campaign_gameplay_restore(
    world: &mut World,
    checkpoint: CampaignGameplayCheckpointV2,
) {
    world.insert_resource(PendingCampaignGameplayCheckpointV2(checkpoint));
}

fn restore_pending_campaign_gameplay(world: &mut World) {
    let Some(pending) = world.remove_resource::<PendingCampaignGameplayCheckpointV2>() else {
        return;
    };
    let outcome = match prepare_campaign_gameplay_restore(world, &pending.0) {
        Ok(prepared) => {
            let unit_count = prepared.units.len();
            let effect_count = prepared.effects.len();
            apply_campaign_gameplay_restore(world, prepared);
            CampaignGameplayRestoreOutcomeV2::Applied {
                unit_count,
                effect_count,
            }
        }
        Err(error) => {
            world.insert_resource(GameplaySetupFailure::new(format!(
                "Campaign gameplay could not be restored: {error}."
            )));
            CampaignGameplayRestoreOutcomeV2::Refused(error)
        }
    };
    world.insert_resource(CampaignGameplayRestoreResultV2 { outcome });
}

fn runtime_units(world: &mut World) -> Vec<RuntimeUnit> {
    let mut query = world.query::<(
        Entity,
        &UnitId,
        &Faction,
        &Archetype,
        Option<&LatticeState>,
        Option<&StandsOn>,
        Option<&Transform>,
    )>();
    query
        .iter(world)
        .map(
            |(entity, id, faction, archetype, lattice, standing, transform)| RuntimeUnit {
                entity,
                id: *id,
                faction: *faction,
                archetype: archetype.0.clone(),
                has_lattice: lattice.is_some(),
                has_standing: standing.is_some(),
                has_transform: transform.is_some(),
            },
        )
        .collect()
}

fn public_footing(world: &mut World) -> Result<Footing, CampaignGameplayCheckpointError> {
    let mut tiles =
        world.query_filtered::<(&TilePos, &HexSpan, &SubstanceId, &Headroom), With<HexTile>>();
    let tiles = tiles
        .iter(world)
        .map(|(position, span, substance, headroom)| (*position, *span, *substance, *headroom))
        .collect::<Vec<_>>();
    let substances = world.get_resource::<SubstanceTable>().ok_or(
        CampaignGameplayCheckpointError::MissingResource("SubstanceTable"),
    )?;
    let blockers = world.get_resource::<TraversalBlockers>();
    let authored_objects = world.get_resource::<AuthoredObjectOccupancy>();
    let body = Body::new(TraversalProfile::WALKER);
    let projected = || {
        tiles
            .iter()
            .map(|(position, span, substance, headroom)| (position, span, substance, headroom))
    };
    Ok(match authored_objects {
        Some(authored_objects) => Footing::from_tiles_with_object_occupancy(
            projected(),
            substances,
            body,
            blockers,
            authored_objects,
        ),
        None => Footing::from_tiles(projected(), substances, body, blockers),
    })
}

fn prepare_campaign_gameplay_restore(
    world: &mut World,
    checkpoint: &CampaignGameplayCheckpointV2,
) -> Result<PreparedGameplayRestore, CampaignGameplayCheckpointError> {
    if checkpoint.units.is_empty() {
        return Err(CampaignGameplayCheckpointError::EmptyRoster);
    }
    let runtime = runtime_units(world);
    if runtime.is_empty() {
        return Err(CampaignGameplayCheckpointError::EmptyRoster);
    }
    let runtime_by_id = runtime
        .iter()
        .map(|unit| (unit.id, unit))
        .collect::<BTreeMap<_, _>>();
    if runtime_by_id.len() != runtime.len() {
        return Err(CampaignGameplayCheckpointError::RosterMismatch);
    }
    if world.get_resource::<PersistentEffects>().is_none() {
        return Err(CampaignGameplayCheckpointError::MissingResource(
            "PersistentEffects",
        ));
    }
    if world.get_resource::<PartyFormation>().is_none() {
        return Err(CampaignGameplayCheckpointError::MissingResource(
            "PartyFormation",
        ));
    }

    let mut previous = None;
    let mut ids = BTreeSet::new();
    let mut positions = BTreeSet::new();
    let mut players = BTreeSet::new();
    for unit in checkpoint.units.as_slice() {
        if previous.is_some_and(|previous| previous >= unit.unit) {
            return Err(CampaignGameplayCheckpointError::NonCanonicalUnits);
        }
        previous = Some(unit.unit);
        ids.insert(unit.unit);
        if !positions.insert(unit.position) {
            return Err(CampaignGameplayCheckpointError::DuplicatePosition(
                unit.position,
            ));
        }
        if unit.faction == Faction::Player {
            players.insert(unit.unit);
        }
    }
    if ids != runtime_by_id.keys().copied().collect() {
        return Err(CampaignGameplayCheckpointError::RosterMismatch);
    }

    validate_party_and_formation(world, checkpoint, &players)?;
    let footing = public_footing(world)?;
    let content = world.get_resource::<ContentIndex>().ok_or(
        CampaignGameplayCheckpointError::MissingResource("ContentIndex"),
    )?;
    let elements = world.get_resource::<ElementCatalog>().ok_or(
        CampaignGameplayCheckpointError::MissingResource("ElementCatalog"),
    )?;
    let lattices = world.get_resource::<LatticeLibrary>().ok_or(
        CampaignGameplayCheckpointError::MissingResource("LatticeLibrary"),
    )?;
    let tables = content.tables(elements);
    let mut prepared = Vec::with_capacity(checkpoint.units.len());
    for saved in checkpoint.units.as_slice() {
        let runtime = runtime_by_id
            .get(&saved.unit)
            .copied()
            .ok_or(CampaignGameplayCheckpointError::RosterMismatch)?;
        if !runtime.has_standing || !runtime.has_transform {
            return Err(CampaignGameplayCheckpointError::IncompleteRuntimeUnit(
                saved.unit,
            ));
        }
        if saved.faction != runtime.faction {
            return Err(CampaignGameplayCheckpointError::FactionMismatch(saved.unit));
        }
        if saved.archetype_identity.as_str() != runtime.archetype {
            return Err(CampaignGameplayCheckpointError::ArchetypeMismatch(
                saved.unit,
            ));
        }
        if saved.lattice.is_some() != runtime.has_lattice {
            return Err(CampaignGameplayCheckpointError::LatticePresenceMismatch(
                saved.unit,
            ));
        }
        if let Some(state) = saved.lattice.as_ref() {
            let definition = lattices.get(&runtime.archetype).ok_or(
                CampaignGameplayCheckpointError::MissingLatticeArchetype(saved.unit),
            )?;
            state
                .validate_against(&definition.spec, &definition.stats, &tables)
                .map_err(|error| {
                    CampaignGameplayCheckpointError::InvalidLattice(saved.unit, error)
                })?;
            let fully_disabled = definition.spec.capacity() > 0
                && definition
                    .spec
                    .cells()
                    .all(|(coord, _)| state.is_disabled(coord));
            if saved.downed != fully_disabled {
                return Err(CampaignGameplayCheckpointError::InvalidDownedState(
                    saved.unit,
                ));
            }
        } else if saved.downed {
            return Err(CampaignGameplayCheckpointError::InvalidDownedState(
                saved.unit,
            ));
        }
        let standing = footing
            .at(saved.position)
            .ok_or(CampaignGameplayCheckpointError::InvalidFooting(saved.unit))?;
        prepared.push(PreparedUnit {
            entity: runtime.entity,
            standing,
            lattice: saved.lattice.clone(),
            downed: saved.downed,
            display_name: saved.display_name.as_str().to_owned(),
        });
    }

    let effects = checkpoint
        .effects
        .effects
        .as_slice()
        .iter()
        .map(|entry| (entry.id, entry.effect))
        .collect::<Vec<_>>();
    PersistentEffects::validate_authority_checkpoint(checkpoint.effects.next_id, &effects)?;
    validate_effects(&effects, checkpoint, &ids)?;

    Ok(PreparedGameplayRestore {
        units: prepared,
        effects,
        next_effect_id: checkpoint.effects.next_id,
        formation: checkpoint.formation.clone(),
    })
}

fn validate_party_and_formation(
    world: &World,
    checkpoint: &CampaignGameplayCheckpointV2,
    players: &BTreeSet<UnitId>,
) -> Result<(), CampaignGameplayCheckpointError> {
    if players.is_empty() {
        return Err(CampaignGameplayCheckpointError::FormationMembership);
    }
    let party = world
        .get_resource::<Party>()
        .ok_or(CampaignGameplayCheckpointError::MissingResource("Party"))?;
    let party_set = party.members.iter().copied().collect::<BTreeSet<_>>();
    if party_set.len() != party.members.len() || &party_set != players {
        return Err(CampaignGameplayCheckpointError::RuntimePartyMismatch);
    }
    let assigned = checkpoint
        .formation
        .assignments
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    if &assigned != players {
        return Err(CampaignGameplayCheckpointError::FormationMembership);
    }
    let formations = world.get_resource::<FormationCatalog>().ok_or(
        CampaignGameplayCheckpointError::MissingResource("FormationCatalog"),
    )?;
    let preset = formations
        .get(&checkpoint.formation.preset)
        .ok_or(CampaignGameplayCheckpointError::FormationPreset)?;
    let authored = preset
        .slots
        .iter()
        .map(|slot| slot.offset)
        .collect::<BTreeSet<_>>();
    let assigned_slots = checkpoint
        .formation
        .assignments
        .values()
        .copied()
        .collect::<BTreeSet<_>>();
    if assigned_slots.len() != checkpoint.formation.assignments.len()
        || assigned_slots.iter().any(|slot| !authored.contains(slot))
    {
        return Err(CampaignGameplayCheckpointError::FormationSlot);
    }
    Ok(())
}

fn validate_effects(
    effects: &[(EffectId, PersistentEffect)],
    checkpoint: &CampaignGameplayCheckpointV2,
    units: &BTreeSet<UnitId>,
) -> Result<(), CampaignGameplayCheckpointError> {
    let lattices = checkpoint
        .units
        .as_slice()
        .iter()
        .map(|unit| (unit.unit, unit.lattice.as_ref()))
        .collect::<BTreeMap<_, _>>();
    for (id, effect) in effects {
        if !units.contains(&effect.source) || !units.contains(&effect.target) {
            return Err(CampaignGameplayCheckpointError::DanglingEffectUnit(*id));
        }
        match effect.end {
            EffectEnd::AfterTurns(turns) if turns > 0 && effect.ticks < turns => {}
            EffectEnd::WithEnchantment(enchantment)
                if lattices
                    .get(&effect.target)
                    .copied()
                    .flatten()
                    .is_some_and(|state| state.enchantment(enchantment).is_some()) => {}
            EffectEnd::AfterRounds(_) | EffectEnd::AfterTurns(_) => {
                return Err(CampaignGameplayCheckpointError::InvalidEffectLifetime(*id));
            }
            EffectEnd::WithEnchantment(_) => {
                return Err(CampaignGameplayCheckpointError::DanglingEffectEnchantment(
                    *id,
                ));
            }
        }
    }
    Ok(())
}

#[expect(
    clippy::expect_used,
    reason = "the immutable roster, component presence, and effect ledger were preflighted in the same exclusive system"
)]
fn apply_campaign_gameplay_restore(world: &mut World, prepared: PreparedGameplayRestore) {
    for unit in prepared.units {
        let mut entity = world.entity_mut(unit.entity);
        entity
            .get_mut::<StandsOn>()
            .expect("preflighted unit must retain standing")
            .0 = unit.standing;
        entity
            .get_mut::<Transform>()
            .expect("preflighted unit must retain transform")
            .translation = unit.standing.world_position();
        if let Some(saved) = unit.lattice {
            *entity
                .get_mut::<LatticeState>()
                .expect("preflighted lattice presence must remain stable") = saved;
        }
        if unit.downed {
            entity.insert(Downed);
        } else {
            entity.remove::<Downed>();
        }
        entity.insert(Name::new(unit.display_name));
    }
    world
        .resource_mut::<PersistentEffects>()
        .replace_authority_checkpoint(prepared.next_effect_id, &prepared.effects)
        .expect("preflighted effect ledger must remain valid");
    *world.resource_mut::<PartyFormation>() = prepared.formation;
}
