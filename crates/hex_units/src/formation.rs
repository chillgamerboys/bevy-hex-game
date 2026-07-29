//! Deterministic whole-party formation planning.
//!
//! This module turns an anchor route plus authored slot assignments into exact
//! per-member paths. It owns planning only: the combat command applier revalidates
//! every path atomically against the live world before presentation starts.

use std::collections::{BTreeMap, BTreeSet};

use hex_core::{FormationPreset, HexCoord, PartyFormation, PartyPath, Sextant, TilePos, UnitId};

use crate::{route, Footing, Standing};

/// One party member's live movement facts for a formation plan.
pub struct FormationMember {
    /// Stable member identity.
    pub unit: UnitId,
    /// Current exact surface.
    pub standing: Standing,
    /// Surfaces and transitions admitted by this member's body.
    pub footing: Footing,
}

/// A complete formation plan and the direction it finishes facing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormationPlan {
    /// Exact replayable paths in stable party order.
    pub paths: Vec<PartyPath>,
    /// Facing derived from the final anchor segment.
    pub facing: Sextant,
}

/// Why a complete formation route could not be planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormationPlanError {
    /// Runtime preset or assignment state lacked a validated invariant.
    InvalidFormation,
    /// A member could not reach an admissible unique compressed position.
    NoSafeSlot(UnitId),
}

/// Builds an atomic formation plan, returning the first member that cannot be placed.
pub fn plan_formation_move(
    preset: &FormationPreset,
    formation: &PartyFormation,
    anchor_path: &[Standing],
    members: Vec<FormationMember>,
) -> Result<FormationPlan, FormationPlanError> {
    let Some(anchor_slot) = preset.anchor() else {
        return Err(FormationPlanError::InvalidFormation);
    };
    let anchor = formation
        .assignments
        .iter()
        .find_map(|(&unit, &slot)| (slot == anchor_slot).then_some(unit))
        .ok_or(FormationPlanError::InvalidFormation)?;
    let mut members: BTreeMap<_, _> = members
        .into_iter()
        .map(|member| (member.unit, member))
        .collect();
    let mut ordered = Vec::with_capacity(members.len());
    ordered.push(anchor);
    for slot in &preset.slots {
        let occupant = formation
            .assignments
            .iter()
            .find_map(|(&unit, &assigned)| (assigned == slot.offset).then_some(unit));
        if let Some(unit) = occupant {
            if unit != anchor && !ordered.contains(&unit) {
                ordered.push(unit);
            }
        }
    }
    for unit in members.keys().copied() {
        if !ordered.contains(&unit) {
            ordered.push(unit);
        }
    }

    let mut current = BTreeMap::new();
    let mut paths = BTreeMap::new();
    for &unit in &ordered {
        let member = members
            .get(&unit)
            .ok_or(FormationPlanError::NoSafeSlot(unit))?;
        current.insert(unit, member.standing);
        paths.insert(unit, vec![member.standing.pos]);
    }
    let mut facing = formation.facing;
    let max_spread = u32::try_from(ordered.len().saturating_sub(1)).unwrap_or(u32::MAX);
    let mut recent = Vec::new();
    if let Some(first) = anchor_path.first() {
        recent.push(first.pos);
    }

    for segment in anchor_path.windows(2) {
        let [previous_anchor, anchor_step] = segment else {
            return Err(FormationPlanError::InvalidFormation);
        };
        facing =
            sextant_between(previous_anchor.pos.coord, anchor_step.pos.coord).unwrap_or(facing);
        recent.insert(0, anchor_step.pos);
        recent.truncate(usize::try_from(max_spread.saturating_add(1)).unwrap_or(usize::MAX));
        let mut used = BTreeSet::new();

        for &unit in &ordered {
            let member = members
                .get_mut(&unit)
                .ok_or(FormationPlanError::NoSafeSlot(unit))?;
            let from = current
                .get(&unit)
                .copied()
                .ok_or(FormationPlanError::InvalidFormation)?;
            let slot = formation
                .assignments
                .get(&unit)
                .copied()
                .ok_or(FormationPlanError::InvalidFormation)?;
            let ideal_coord = translated(anchor_step.pos.coord, rotated(slot, facing));

            let chosen = if unit == anchor {
                member.footing.at(anchor_step.pos).and_then(|destination| {
                    route(from, destination, &member.footing).map(|path| (destination, path))
                })
            } else {
                choose_destination(
                    from,
                    ideal_coord,
                    anchor_step.pos.level,
                    anchor_step.pos.coord,
                    max_spread,
                    &recent,
                    &used,
                    &member.footing,
                )
            };
            let Some((destination, route)) = chosen else {
                return Err(FormationPlanError::NoSafeSlot(unit));
            };
            used.insert(destination.pos);
            current.insert(unit, destination);
            let Some(member_path) = paths.get_mut(&unit) else {
                return Err(FormationPlanError::InvalidFormation);
            };
            member_path.extend(route.into_iter().skip(1).map(|standing| standing.pos));
        }
    }

    let mut planned_paths = Vec::with_capacity(ordered.len());
    for member in ordered {
        let Some(path) = paths.remove(&member) else {
            return Err(FormationPlanError::InvalidFormation);
        };
        planned_paths.push(PartyPath { member, path });
    }
    Ok(FormationPlan {
        paths: planned_paths,
        facing,
    })
}

fn choose_destination(
    from: Standing,
    ideal: HexCoord,
    ideal_level: i32,
    anchor: HexCoord,
    max_spread: u32,
    recent: &[TilePos],
    used: &BTreeSet<TilePos>,
    footing: &Footing,
) -> Option<(Standing, Vec<Standing>)> {
    let mut ideals = footing.at_coord(ideal).to_vec();
    ideals.sort_by_key(|standing| (standing.pos.level.abs_diff(ideal_level), standing.pos));
    for candidate in ideals {
        if let Some(found) = usable_route(from, candidate, used, footing) {
            return Some(found);
        }
    }

    for &position in recent {
        if let Some(candidate) = footing.at(position) {
            if let Some(found) = usable_route(from, candidate, used, footing) {
                return Some(found);
            }
        }
    }

    let mut nearby = footing.standings();
    nearby.retain(|standing| standing.pos.coord.distance(anchor) <= max_spread);
    nearby.sort_by_key(|standing| {
        (
            standing.pos.coord.distance(ideal),
            standing.pos.level.abs_diff(ideal_level),
            standing.pos,
        )
    });
    nearby
        .into_iter()
        .find_map(|candidate| usable_route(from, candidate, used, footing))
}

fn usable_route(
    from: Standing,
    candidate: Standing,
    used: &BTreeSet<TilePos>,
    footing: &Footing,
) -> Option<(Standing, Vec<Standing>)> {
    if used.contains(&candidate.pos) {
        return None;
    }
    route(from, candidate, footing).map(|path| (candidate, path))
}

fn translated(origin: HexCoord, offset: HexCoord) -> HexCoord {
    HexCoord::from_axial(origin.x() + offset.x(), origin.y() + offset.y())
}

/// Rotates an authored axial offset from [`Sextant::A`] into `facing`.
#[must_use]
pub fn rotated(offset: HexCoord, facing: Sextant) -> HexCoord {
    let [x, y, z] = offset.to_cubic_array();
    let (x, y) = match facing {
        Sextant::A => (x, y),
        Sextant::B => (-y, -z),
        Sextant::C => (z, x),
        Sextant::D => (-x, -y),
        Sextant::E => (y, z),
        Sextant::F => (-z, -x),
    };
    HexCoord::from_axial(x, y)
}

fn sextant_between(from: HexCoord, to: HexCoord) -> Option<Sextant> {
    Sextant::ALL
        .into_iter()
        .find(|&facing| from.neighbor(facing) == to)
}

#[cfg(test)]
mod tests {
    use bevy::platform::collections::HashMap;
    use hex_assets::{
        ArtPalette, PaletteSwatch, SrgbColor, Substance, SubstanceFile, SubstanceTable, SwatchId,
    };
    use hex_core::{FormationSlot, Headroom, HexSpan, SubstanceId, TraversalProfile, MAX_HEADROOM};

    use super::*;

    const STONE: SubstanceId = SubstanceId(1);
    const BODY: crate::Body = crate::Body::new(TraversalProfile::WALKER);

    fn table() -> SubstanceTable {
        let swatch = SwatchId::new("terrain/stone").expect("the fixture swatch id should be valid");
        let palette = ArtPalette::new(BTreeMap::from([(
            swatch.clone(),
            PaletteSwatch::new(
                "Stone",
                SrgbColor::new(0.5, 0.5, 0.5).expect("the fixture color should be valid"),
                BTreeSet::from(["test".to_owned()]),
            )
            .expect("the fixture swatch should be valid"),
        )]))
        .expect("the fixture palette should be valid");
        let mut substances = HashMap::default();
        substances.insert("air".to_owned(), Substance::invisible(false, false));
        substances.insert(
            "stone".to_owned(),
            Substance::from_swatch(swatch, true, true),
        );
        SubstanceTable::from_file(&SubstanceFile { substances }, &palette)
            .expect("the fixture substance should resolve")
    }

    fn tile(coord: HexCoord) -> (TilePos, HexSpan, SubstanceId, Headroom) {
        (
            TilePos::new(coord, 0),
            HexSpan::new(0.0, 1.0),
            STONE,
            Headroom(MAX_HEADROOM),
        )
    }

    fn footing(
        tiles: &[(TilePos, HexSpan, SubstanceId, Headroom)],
        table: &SubstanceTable,
    ) -> Footing {
        Footing::from_tiles(
            tiles
                .iter()
                .map(|(pos, span, substance, headroom)| (pos, span, substance, headroom)),
            table,
            BODY,
            None,
        )
    }

    fn wedge() -> FormationPreset {
        FormationPreset {
            name: "Wedge".to_owned(),
            slots: vec![
                FormationSlot {
                    offset: HexCoord::ORIGIN,
                    anchor: true,
                },
                FormationSlot {
                    offset: HexCoord::from_axial(-1, 1),
                    anchor: false,
                },
                FormationSlot {
                    offset: HexCoord::from_axial(0, -1),
                    anchor: false,
                },
            ],
        }
    }

    #[test]
    fn every_full_rotation_is_congruent() {
        let offsets = [
            HexCoord::ORIGIN,
            HexCoord::from_axial(1, 0),
            HexCoord::from_axial(1, -1),
            HexCoord::from_axial(0, -1),
            HexCoord::from_axial(-1, 1),
            HexCoord::from_axial(0, 1),
        ];
        for facing in Sextant::ALL {
            let rotated: BTreeSet<_> = offsets
                .iter()
                .copied()
                .map(|offset| super::rotated(offset, facing))
                .collect();
            assert_eq!(rotated.len(), offsets.len());
            let mut rotated_distances: Vec<_> = rotated
                .iter()
                .map(|offset| offset.distance(HexCoord::ORIGIN))
                .collect();
            rotated_distances.sort_unstable();
            let mut original_distances: Vec<_> = offsets
                .iter()
                .map(|offset| offset.distance(HexCoord::ORIGIN))
                .collect();
            original_distances.sort_unstable();
            assert_eq!(rotated_distances, original_distances);
        }
        for offset in offsets {
            let turned = Sextant::ALL
                .into_iter()
                .fold(offset, |offset, _| super::rotated(offset, Sextant::B));
            assert_eq!(turned, offset);
        }
    }

    #[test]
    fn a_party_compresses_through_a_one_wide_bridge_and_reforms() {
        let table = table();
        let mut coords = vec![HexCoord::from_axial(-1, 1), HexCoord::from_axial(0, -1)];
        coords.extend((0..=4).map(|q| HexCoord::from_axial(q, 0)));
        coords.extend([HexCoord::from_axial(3, 1), HexCoord::from_axial(4, -1)]);
        let tiles: Vec<_> = coords.into_iter().map(tile).collect();
        let preset = wedge();
        let ids = [UnitId(0), UnitId(1), UnitId(2)];
        let mut formation = PartyFormation::default();
        formation.select_preset(&preset, &ids);
        let anchor_footing = footing(&tiles, &table);
        let anchor_from = anchor_footing
            .at(TilePos::new(HexCoord::ORIGIN, 0))
            .expect("anchor start should be standable");
        let anchor_to = anchor_footing
            .at(TilePos::new(HexCoord::from_axial(4, 0), 0))
            .expect("anchor finish should be standable");
        let anchor_path =
            route(anchor_from, anchor_to, &anchor_footing).expect("the bridge should connect");
        let members = [
            (ids[0], HexCoord::ORIGIN),
            (ids[1], HexCoord::from_axial(-1, 1)),
            (ids[2], HexCoord::from_axial(0, -1)),
        ]
        .into_iter()
        .map(|(unit, coord)| FormationMember {
            unit,
            standing: footing(&tiles, &table)
                .at(TilePos::new(coord, 0))
                .expect("member start should be standable"),
            footing: footing(&tiles, &table),
        })
        .collect();

        let plan = plan_formation_move(&preset, &formation, &anchor_path, members)
            .expect("the party should compress and reform");
        let destinations: Vec<_> = plan
            .paths
            .iter()
            .map(|path| *path.path.last().expect("every path has a start"))
            .collect();
        assert_eq!(
            destinations,
            vec![
                TilePos::new(HexCoord::from_axial(4, 0), 0),
                TilePos::new(HexCoord::from_axial(3, 1), 0),
                TilePos::new(HexCoord::from_axial(4, -1), 0),
            ]
        );
        assert_eq!(
            destinations.iter().copied().collect::<BTreeSet<_>>().len(),
            ids.len()
        );
    }

    #[test]
    fn one_stranded_member_rejects_the_complete_plan() {
        let table = table();
        let full_tiles: Vec<_> = [
            HexCoord::from_axial(-1, 1),
            HexCoord::from_axial(0, -1),
            HexCoord::from_axial(0, 0),
            HexCoord::from_axial(1, 0),
            HexCoord::from_axial(2, 0),
            HexCoord::from_axial(3, 0),
            HexCoord::from_axial(4, 0),
            HexCoord::from_axial(3, 1),
            HexCoord::from_axial(4, -1),
        ]
        .into_iter()
        .map(tile)
        .collect();
        let stranded_tiles = vec![tile(HexCoord::from_axial(0, -1))];
        let preset = wedge();
        let ids = [UnitId(0), UnitId(1), UnitId(2)];
        let mut formation = PartyFormation::default();
        formation.select_preset(&preset, &ids);
        let anchor_footing = footing(&full_tiles, &table);
        let anchor_path = route(
            anchor_footing
                .at(TilePos::new(HexCoord::ORIGIN, 0))
                .expect("anchor start should be standable"),
            anchor_footing
                .at(TilePos::new(HexCoord::from_axial(4, 0), 0))
                .expect("anchor finish should be standable"),
            &anchor_footing,
        )
        .expect("the bridge should connect");
        let members = vec![
            FormationMember {
                unit: ids[0],
                standing: footing(&full_tiles, &table)
                    .at(TilePos::new(HexCoord::ORIGIN, 0))
                    .expect("anchor starts"),
                footing: footing(&full_tiles, &table),
            },
            FormationMember {
                unit: ids[1],
                standing: footing(&full_tiles, &table)
                    .at(TilePos::new(HexCoord::from_axial(-1, 1), 0))
                    .expect("rear starts"),
                footing: footing(&full_tiles, &table),
            },
            FormationMember {
                unit: ids[2],
                standing: footing(&stranded_tiles, &table)
                    .at(TilePos::new(HexCoord::from_axial(0, -1), 0))
                    .expect("stranded member starts"),
                footing: footing(&stranded_tiles, &table),
            },
        ];

        assert_eq!(
            plan_formation_move(&preset, &formation, &anchor_path, members),
            Err(FormationPlanError::NoSafeSlot(ids[2]))
        );
    }
}
