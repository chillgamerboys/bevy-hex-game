//! Deterministic whole-party formation planning.
//!
//! This module turns an anchor route plus authored slot assignments into exact
//! per-member paths. It owns planning only: the combat command applier revalidates
//! every path atomically against the live world before presentation starts.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use hex_core::{FormationPreset, HexCoord, PartyFormation, PartyPath, Sextant, TilePos, UnitId};

use crate::{Footing, OccupancyBlock, Reach, Standing, UnitOccupancy};

/// One party member's live movement facts for a formation plan.
#[derive(Debug, Clone)]
pub struct FormationMember {
    /// Stable member identity.
    pub unit: UnitId,
    /// Current exact surface.
    pub standing: Standing,
    /// Surfaces and transitions admitted by this member's body.
    ///
    /// Members with the same traversal profile should share this index. A party
    /// move reads it many times but never mutates it, so rebuilding the same map
    /// projection once per member only adds allocation and query work.
    pub footing: Arc<Footing>,
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
    /// Member routes would share an endpoint or swap through each other.
    Occupied(OccupancyBlock),
}

/// Selects the deterministic anchor for one complete party subset.
///
/// The authored global anchor wins when it belongs to the subset. Otherwise slots are
/// considered in authored order, preserving a stable anchor for every seat without
/// rewriting the shared formation assignment.
#[must_use]
pub fn formation_subset_anchor(
    preset: &FormationPreset,
    formation: &PartyFormation,
    members: &[UnitId],
) -> Option<UnitId> {
    let global = formation.anchor_member(preset);
    if global.is_some_and(|unit| members.contains(&unit)) {
        return global;
    }
    preset.slots.iter().find_map(|slot| {
        formation.assignments.iter().find_map(|(&unit, &assigned)| {
            (assigned == slot.offset && members.contains(&unit)).then_some(unit)
        })
    })
}

/// Builds an atomic formation plan, returning the first member that cannot be placed.
pub fn plan_formation_move(
    preset: &FormationPreset,
    formation: &PartyFormation,
    anchor_path: &[Standing],
    members: Vec<FormationMember>,
) -> Result<FormationPlan, FormationPlanError> {
    plan_formation_move_with_occupancy(
        preset,
        formation,
        anchor_path,
        members,
        &UnitOccupancy::default(),
    )
}

/// Builds a formation plan while excluding bodies outside the moving party.
pub fn plan_formation_move_with_occupancy(
    preset: &FormationPreset,
    formation: &PartyFormation,
    anchor_path: &[Standing],
    members: Vec<FormationMember>,
    occupancy: &UnitOccupancy,
) -> Result<FormationPlan, FormationPlanError> {
    let Some(anchor_slot) = preset.anchor() else {
        return Err(FormationPlanError::InvalidFormation);
    };
    let anchor = formation
        .assignments
        .iter()
        .find_map(|(&unit, &slot)| (slot == anchor_slot).then_some(unit))
        .ok_or(FormationPlanError::InvalidFormation)?;
    plan_formation_subset_move_with_occupancy(
        preset,
        formation,
        anchor,
        anchor_path,
        members,
        occupancy,
    )
}

/// Builds an atomic route for one seat-owned party subset around an explicit anchor.
///
/// The member list is the complete subset the caller intends to move. Authored slot
/// offsets remain shared across the whole party, but the selected seat's anchor replaces
/// the preset's global anchor for route order and translation.
pub fn plan_formation_subset_move_with_occupancy(
    preset: &FormationPreset,
    formation: &PartyFormation,
    anchor: UnitId,
    anchor_path: &[Standing],
    members: Vec<FormationMember>,
    occupancy: &UnitOccupancy,
) -> Result<FormationPlan, FormationPlanError> {
    if preset.anchor().is_none() || !formation.assignments.contains_key(&anchor) {
        return Err(FormationPlanError::InvalidFormation);
    }
    let member_ids = members.iter().map(|member| member.unit).collect::<Vec<_>>();
    if formation_subset_anchor(preset, formation, &member_ids) != Some(anchor) {
        return Err(FormationPlanError::InvalidFormation);
    }
    let mut members: BTreeMap<_, _> = members
        .into_iter()
        .map(|member| (member.unit, member))
        .collect();
    if !members.contains_key(&anchor) {
        return Err(FormationPlanError::NoSafeSlot(anchor));
    }
    let anchor_offset = formation
        .assignments
        .get(&anchor)
        .copied()
        .ok_or(FormationPlanError::InvalidFormation)?;
    let mut ordered = Vec::with_capacity(members.len());
    ordered.push(anchor);
    for slot in &preset.slots {
        let occupant = formation
            .assignments
            .iter()
            .find_map(|(&unit, &assigned)| (assigned == slot.offset).then_some(unit));
        if let Some(unit) = occupant {
            if unit != anchor && members.contains_key(&unit) && !ordered.contains(&unit) {
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
            let relative_slot =
                HexCoord::from_axial(slot.x() - anchor_offset.x(), slot.y() - anchor_offset.y());
            let ideal_coord = translated(anchor_step.pos.coord, rotated(relative_slot, facing));

            let chosen = if unit == anchor {
                member.footing.at(anchor_step.pos).and_then(|destination| {
                    let reach = Reach::until_with_occupancy(
                        from,
                        &member.footing,
                        destination.pos,
                        occupancy,
                        unit,
                    );
                    reach
                        .path_to(destination.pos)
                        .map(|path| (destination, path))
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
                    occupancy,
                    unit,
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
    UnitOccupancy::validate_group_routes(&planned_paths).map_err(FormationPlanError::Occupied)?;
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
    occupancy: &UnitOccupancy,
    unit: UnitId,
) -> Option<(Standing, Vec<Standing>)> {
    let mut ideals = footing.at_coord(ideal).to_vec();
    ideals.sort_by_key(|standing| (standing.pos.level.abs_diff(ideal_level), standing.pos));
    let mut candidates = ideals;

    for &position in recent {
        if let Some(candidate) = footing.at(position) {
            candidates.push(candidate);
        }
    }

    let fallback_candidates = || {
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
    };

    // Candidate priority is a gameplay contract: ideal slot, recent anchor trail,
    // then nearest compression fallback. Search toward the first admissible
    // candidate. If it is reachable, breadth-first discovery gives the exact path a
    // full projection would; if it is not, the search necessarily exhausts the
    // connected component and therefore answers every lower-priority candidate too.
    if let Some(first) = candidates
        .iter()
        .find(|candidate| !used.contains(&candidate.pos))
    {
        let reach = Reach::until_with_occupancy(from, footing, first.pos, occupancy, unit);
        if let Some(chosen) = candidates
            .into_iter()
            .find_map(|candidate| usable_route(candidate, used, &reach))
        {
            return Some(chosen);
        }
        return fallback_candidates()
            .into_iter()
            .find_map(|candidate| usable_route(candidate, used, &reach));
    }

    let candidates = fallback_candidates();
    let first = candidates
        .iter()
        .find(|candidate| !used.contains(&candidate.pos))?;
    let reach = Reach::until_with_occupancy(from, footing, first.pos, occupancy, unit);
    candidates
        .into_iter()
        .find_map(|candidate| usable_route(candidate, used, &reach))
}

fn usable_route(
    candidate: Standing,
    used: &BTreeSet<TilePos>,
    reach: &Reach,
) -> Option<(Standing, Vec<Standing>)> {
    if used.contains(&candidate.pos) {
        return None;
    }
    reach.path_to(candidate.pos).map(|path| (candidate, path))
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
    use std::time::{Duration, Instant};

    use bevy::platform::collections::HashMap;
    use hex_assets::{
        ArtPalette, PaletteSwatch, SrgbColor, Substance, SubstanceFile, SubstanceTable, SwatchId,
    };
    use hex_core::{FormationSlot, Headroom, HexSpan, SubstanceId, TraversalProfile, MAX_HEADROOM};

    use super::*;
    use crate::route;

    const STONE: SubstanceId = SubstanceId(10);
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

    fn upper_tile(coord: HexCoord) -> (TilePos, HexSpan, SubstanceId, Headroom) {
        (
            TilePos::new(coord, 1),
            HexSpan::new(1.0, 2.0),
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

    fn six_member_wedge() -> FormationPreset {
        FormationPreset {
            name: "Six Member Wedge".to_owned(),
            slots: [
                (HexCoord::ORIGIN, true),
                (HexCoord::from_axial(-1, 1), false),
                (HexCoord::from_axial(0, -1), false),
                (HexCoord::from_axial(-1, 0), false),
                (HexCoord::from_axial(0, 1), false),
                (HexCoord::from_axial(1, -1), false),
            ]
            .into_iter()
            .map(|(offset, anchor)| FormationSlot { offset, anchor })
            .collect(),
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum FormationTerrain {
        Open,
        Stacked,
        Narrow,
        Blocked,
    }

    fn six_member_case(
        terrain: FormationTerrain,
        distance: i32,
    ) -> (
        FormationPreset,
        PartyFormation,
        Vec<Standing>,
        Vec<FormationMember>,
    ) {
        let table = table();
        let preset = six_member_wedge();
        let coords: Vec<_> = match terrain {
            FormationTerrain::Open | FormationTerrain::Stacked => (-3..=distance + 3)
                .flat_map(|q| (-3..=3).map(move |r| HexCoord::from_axial(q, r)))
                .collect(),
            FormationTerrain::Narrow => {
                let mut coords: BTreeSet<_> =
                    (0..=distance).map(|q| HexCoord::from_axial(q, 0)).collect();
                for slot in &preset.slots {
                    let _inserted = coords.insert(slot.offset);
                    let _inserted =
                        coords.insert(translated(HexCoord::from_axial(distance, 0), slot.offset));
                }
                coords.into_iter().collect()
            }
            FormationTerrain::Blocked => {
                let wall = distance / 2;
                (-3..=distance + 3)
                    .flat_map(|q| (-4..=4).map(move |r| HexCoord::from_axial(q, r)))
                    .filter(|coord| coord.x() != wall || coord.y().abs() > 3)
                    .collect()
            }
        };
        let mut tiles = Vec::with_capacity(coords.len().saturating_mul(
            if matches!(terrain, FormationTerrain::Stacked) {
                2
            } else {
                1
            },
        ));
        for coord in coords {
            tiles.push(tile(coord));
            if matches!(terrain, FormationTerrain::Stacked) {
                tiles.push(upper_tile(coord));
            }
        }
        let shared_footing = Arc::new(footing(&tiles, &table));
        let ids: Vec<_> = (0..6).map(UnitId).collect();
        let mut formation = PartyFormation::default();
        formation.select_preset(&preset, &ids);
        let level = if matches!(terrain, FormationTerrain::Stacked) {
            1
        } else {
            0
        };
        let from = shared_footing
            .at(TilePos::new(HexCoord::ORIGIN, level))
            .expect("the formation case should contain its origin");
        let to = shared_footing
            .at(TilePos::new(HexCoord::from_axial(distance, 0), level))
            .expect("the formation case should contain its destination");
        let anchor_path =
            route(from, to, &shared_footing).expect("the formation terrain should connect");
        let members = preset
            .slots
            .iter()
            .zip(ids)
            .map(|(slot, unit)| FormationMember {
                unit,
                standing: shared_footing
                    .at(TilePos::new(slot.offset, level))
                    .expect("every authored starting slot should be standable"),
                footing: Arc::clone(&shared_footing),
            })
            .collect();
        (preset, formation, anchor_path, members)
    }

    #[test]
    fn subset_planning_ignores_other_seats_authored_slots() {
        let (preset, formation, _global_path, members) = six_member_case(FormationTerrain::Open, 3);
        let subset_ids = [UnitId(1), UnitId(3)];
        let anchor = formation_subset_anchor(&preset, &formation, &subset_ids)
            .expect("the subset should receive its first authored slot as anchor");
        let anchor_member = members
            .iter()
            .find(|member| member.unit == anchor)
            .expect("the fixture should contain the subset anchor");
        let destination = TilePos::new(
            translated(anchor_member.standing.pos.coord, HexCoord::from_axial(3, 0)),
            anchor_member.standing.pos.level,
        );
        let anchor_destination = anchor_member
            .footing
            .at(destination)
            .expect("the open fixture should contain the translated destination");
        let anchor_path = route(
            anchor_member.standing,
            anchor_destination,
            &anchor_member.footing,
        )
        .expect("the open fixture should connect the subset route");
        let subset = members
            .into_iter()
            .filter(|member| subset_ids.contains(&member.unit))
            .collect::<Vec<_>>();

        let plan = plan_formation_subset_move_with_occupancy(
            &preset,
            &formation,
            anchor,
            &anchor_path,
            subset,
            &UnitOccupancy::default(),
        )
        .expect("foreign formation occupants must not enter the seat-owned plan");

        assert_eq!(plan.paths.first().map(|path| path.member), Some(anchor));
        assert_eq!(
            plan.paths
                .iter()
                .map(|path| path.member)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(subset_ids)
        );
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
        let shared_footing = Arc::new(footing(&tiles, &table));
        let anchor_from = shared_footing
            .at(TilePos::new(HexCoord::ORIGIN, 0))
            .expect("anchor start should be standable");
        let anchor_to = shared_footing
            .at(TilePos::new(HexCoord::from_axial(4, 0), 0))
            .expect("anchor finish should be standable");
        let anchor_path =
            route(anchor_from, anchor_to, &shared_footing).expect("the bridge should connect");
        let members = [
            (ids[0], HexCoord::ORIGIN),
            (ids[1], HexCoord::from_axial(-1, 1)),
            (ids[2], HexCoord::from_axial(0, -1)),
        ]
        .into_iter()
        .map(|(unit, coord)| FormationMember {
            unit,
            standing: shared_footing
                .at(TilePos::new(coord, 0))
                .expect("member start should be standable"),
            footing: Arc::clone(&shared_footing),
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
    fn six_member_open_routes_are_deterministic_at_scale() {
        for distance in [10, 50, 100] {
            let (preset, formation, anchor_path, members) =
                six_member_case(FormationTerrain::Open, distance);
            let expected = plan_formation_move(&preset, &formation, &anchor_path, members.clone())
                .expect("six members should traverse open terrain");

            assert_eq!(expected.paths.len(), 6);
            assert_eq!(
                expected
                    .paths
                    .iter()
                    .filter_map(|path| path.path.last())
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .len(),
                6,
                "a {distance}-step plan ended with overlapping members"
            );
            for _ in 0..4 {
                assert_eq!(
                    plan_formation_move(&preset, &formation, &anchor_path, members.clone()),
                    Ok(expected.clone()),
                    "the {distance}-step route was not deterministic"
                );
            }
        }
    }

    #[test]
    #[ignore = "manual release-mode six-member formation acceptance matrix"]
    fn six_member_formation_benchmark_matrix() {
        for terrain in [
            FormationTerrain::Open,
            FormationTerrain::Stacked,
            FormationTerrain::Narrow,
            FormationTerrain::Blocked,
        ] {
            for distance in [10, 50, 100] {
                let (preset, formation, anchor_path, members) = six_member_case(terrain, distance);
                let expected =
                    plan_formation_move(&preset, &formation, &anchor_path, members.clone())
                        .expect("benchmark terrain should admit a complete plan");
                let mut samples = Vec::with_capacity(100);
                for _ in 0..100 {
                    let started = Instant::now();
                    let actual =
                        plan_formation_move(&preset, &formation, &anchor_path, members.clone())
                            .expect("the repeated benchmark plan should remain valid");
                    samples.push(started.elapsed());
                    assert_eq!(actual, expected);
                }
                samples.sort_unstable();
                let p95 = samples
                    .get(94)
                    .copied()
                    .expect("the benchmark records exactly 100 samples");
                let worst = samples
                    .get(99)
                    .copied()
                    .expect("the benchmark records exactly 100 samples");
                eprintln!(
                    "six-member {terrain:?} formation distance={distance}: \
                     p95={p95:?}, worst={worst:?}"
                );

                let (p95_budget, worst_budget) = if cfg!(debug_assertions) {
                    (Duration::from_millis(100), Duration::from_millis(250))
                } else {
                    (Duration::from_micros(16_700), Duration::from_millis(50))
                };
                assert!(
                    p95 < p95_budget && worst < worst_budget,
                    "{terrain:?} distance {distance} exceeded formation budgets: \
                     p95={p95:?}, worst={worst:?}"
                );
            }
        }
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
        let anchor_footing = Arc::new(footing(&full_tiles, &table));
        let stranded_footing = Arc::new(footing(&stranded_tiles, &table));
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
                standing: anchor_footing
                    .at(TilePos::new(HexCoord::ORIGIN, 0))
                    .expect("anchor starts"),
                footing: Arc::clone(&anchor_footing),
            },
            FormationMember {
                unit: ids[1],
                standing: anchor_footing
                    .at(TilePos::new(HexCoord::from_axial(-1, 1), 0))
                    .expect("rear starts"),
                footing: Arc::clone(&anchor_footing),
            },
            FormationMember {
                unit: ids[2],
                standing: stranded_footing
                    .at(TilePos::new(HexCoord::from_axial(0, -1), 0))
                    .expect("stranded member starts"),
                footing: Arc::clone(&stranded_footing),
            },
        ];

        assert_eq!(
            plan_formation_move(&preset, &formation, &anchor_path, members),
            Err(FormationPlanError::NoSafeSlot(ids[2]))
        );
    }
}
