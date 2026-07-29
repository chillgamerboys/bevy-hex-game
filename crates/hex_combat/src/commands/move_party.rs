//! Atomic validation and commitment of an exact whole-party move.

use std::collections::BTreeSet;

use bevy::prelude::*;
use hex_anim::Transformation;
use hex_core::{Busy, PartyMovementMode, PartyPath, Sextant, TilePos, UnitId};
use hex_units::{HexPathingLine, MovingTo, Standing};

use crate::{CombatData, CombatEvent, CommandRefusal, PartyMoveRefusal, UnitData};

use super::{ActorQuery, TileQuery, Verb};

struct PreparedPath {
    entity: Entity,
    steps: Vec<Standing>,
}

pub(super) fn apply(
    ctx: &mut Verb,
    commands: &mut Commands,
    tiles: &TileQuery,
    actors: &mut ActorQuery,
    issued_seat: hex_core::PlayerSeat,
    anchor: UnitId,
    paths: &[PartyPath],
) -> Result<(), CommandRefusal> {
    if ctx.in_combat || ctx.formation.mode != PartyMovementMode::Group {
        return Err(CommandRefusal::PartyMovementUnavailable);
    }
    let expected_anchor = ctx
        .formations
        .and_then(|catalog| catalog.get(&ctx.formation.preset))
        .and_then(|preset| ctx.formation.anchor_member(preset));
    if expected_anchor != Some(anchor) {
        return Err(party_refusal(PartyMoveRefusal::WrongAnchor));
    }
    let Some(table) = ctx.table else {
        return Err(CommandRefusal::MissingCombatData {
            data: CombatData::SubstanceTable,
        });
    };

    let mut named = BTreeSet::new();
    let mut destinations = BTreeSet::new();
    let mut prepared = Vec::with_capacity(paths.len());
    for path in paths {
        if !ctx.party.members.contains(&path.member) {
            return Err(party_refusal(PartyMoveRefusal::NotPartyMember {
                member: path.member,
            }));
        }
        if !named.insert(path.member) {
            return Err(party_refusal(PartyMoveRefusal::DuplicateMember {
                member: path.member,
            }));
        }
        let Some(entity) = ctx.registry.entity_of(path.member) else {
            return Err(CommandRefusal::MissingUnitData {
                unit: path.member,
                data: UnitData::EntityRecord,
            });
        };
        let (standing, body, busy, owner) = {
            let Ok((standing, body, _, busy, owner, _, _)) = actors.get_mut(entity) else {
                return Err(CommandRefusal::MissingUnitData {
                    unit: path.member,
                    data: UnitData::EntityRecord,
                });
            };
            let Some(standing) = standing else {
                return Err(CommandRefusal::MissingUnitData {
                    unit: path.member,
                    data: UnitData::Standing,
                });
            };
            let Some(body) = body else {
                return Err(CommandRefusal::MissingUnitData {
                    unit: path.member,
                    data: UnitData::Body,
                });
            };
            (standing.0, *body, busy, owner.copied().unwrap_or_default())
        };
        if owner.0 != issued_seat {
            return Err(CommandRefusal::WrongSeat {
                issued_by: issued_seat,
                owned_by: owner.0,
            });
        }
        if busy || ctx.committed.contains(&entity) {
            return Err(CommandRefusal::Busy);
        }
        if path.path.first() != Some(&standing.pos) {
            return Err(party_refusal(PartyMoveRefusal::InvalidStart {
                member: path.member,
            }));
        }
        let footing = hex_units::Footing::from_tiles(tiles.iter(), table, body);
        let Some(steps) = ground_path(&path.path, standing, &footing) else {
            return Err(party_refusal(PartyMoveRefusal::InvalidMemberPath {
                member: path.member,
            }));
        };
        let Some(destination) = steps.last().map(|standing| standing.pos) else {
            return Err(party_refusal(PartyMoveRefusal::InvalidMemberPath {
                member: path.member,
            }));
        };
        if !destinations.insert(destination) {
            return Err(party_refusal(PartyMoveRefusal::DuplicateDestination {
                destination,
            }));
        }
        prepared.push(PreparedPath { entity, steps });
    }

    if let Some(&member) = ctx
        .party
        .members
        .iter()
        .find(|member| !named.contains(member))
    {
        return Err(party_refusal(PartyMoveRefusal::MissingMember { member }));
    }

    for prepared_path in prepared {
        if prepared_path.steps.len() < 2 {
            continue;
        }
        let mut unit_commands = commands.entity(prepared_path.entity);
        if let Some(settings) = ctx.settings {
            let animation: Transformation =
                HexPathingLine::new(&prepared_path.steps, settings.speed).into();
            unit_commands.insert((
                animation,
                MovingTo::new(prepared_path.steps, settings.speed),
                Busy,
            ));
        } else {
            unit_commands.insert((MovingTo::new(prepared_path.steps, 0.0), Busy));
        }
        ctx.committed.push(prepared_path.entity);
    }

    if let Some(anchor_path) = paths.iter().find(|path| path.member == anchor) {
        if let [.., previous, last] = anchor_path.path.as_slice() {
            if let Some(facing) = facing_between(previous.coord, last.coord) {
                ctx.formation.facing = facing;
            }
        }
    }
    ctx.events.push(CombatEvent::PartyMoved {
        anchor,
        paths: paths.to_vec(),
    });
    Ok(())
}

fn ground_path(
    path: &[TilePos],
    from: Standing,
    footing: &hex_units::Footing,
) -> Option<Vec<Standing>> {
    if path.first() != Some(&from.pos) {
        return None;
    }
    let mut grounded = Vec::with_capacity(path.len());
    grounded.push(footing.at(from.pos)?);
    for pair in path.windows(2) {
        let [_, to] = pair else {
            return None;
        };
        let previous = grounded.last()?.pos;
        if !footing.admits_step(previous, *to) {
            return None;
        }
        grounded.push(footing.at(*to)?);
    }
    Some(grounded)
}

fn facing_between(from: hex_core::HexCoord, to: hex_core::HexCoord) -> Option<Sextant> {
    Sextant::ALL
        .into_iter()
        .find(|&facing| from.neighbor(facing) == to)
}

fn party_refusal(reason: PartyMoveRefusal) -> CommandRefusal {
    CommandRefusal::PartyMove { reason }
}
