//! Shared rolling-ground helpers for authored-vegetation recipes.
//!
//! This module owns no recipe feature placement. Deep Forest and Prairie retain
//! separate topology, density, and validation while sharing deterministic ground
//! semantics and presentation framing.

use std::collections::{BTreeMap, BTreeSet};

use hex_core::{HexCoord, MapViewHint, TilePos};

use super::seed::SeedStream;
use super::volume::{LevelInterval, SolidMass, SolidMaterialRole, VolumeColumn, VolumeElement};
use super::world::{WorldIssueCode, WorldValidationIssue};

const MOUND_COUNT: u64 = 5;

pub(super) fn rolling_levels(
    mask: &BTreeSet<HexCoord>,
    base_level: i32,
    max_relief: i32,
    stream: Option<SeedStream<'_>>,
    recipe: &'static str,
) -> Result<BTreeMap<HexCoord, i32>, Vec<WorldValidationIssue>> {
    let mut candidates = mask.iter().copied().collect::<Vec<_>>();
    candidates.sort_unstable();
    if candidates.len() < usize::try_from(MOUND_COUNT).unwrap_or_default() {
        return Err(vec![recipe_issue(
            recipe,
            "footprint cannot fit its rolling-ground centres",
        )]);
    }
    let mut centres = Vec::new();
    if let Some(stream) = stream {
        let count = u64::try_from(candidates.len()).unwrap_or(u64::MAX);
        for index in 0..MOUND_COUNT {
            let mut cursor = usize::try_from(stream.sample(index) % count).unwrap_or_default();
            for _ in 0..candidates.len() {
                let Some(candidate) = candidates.get(cursor).copied() else {
                    break;
                };
                if !centres.contains(&candidate) {
                    centres.push(candidate);
                    break;
                }
                cursor = cursor.saturating_add(1) % candidates.len();
            }
        }
    } else {
        let denominator = usize::try_from(MOUND_COUNT).unwrap_or(1).saturating_add(1);
        for index in 1..=usize::try_from(MOUND_COUNT).unwrap_or_default() {
            let cursor = candidates.len().saturating_mul(index) / denominator;
            if let Some(candidate) = candidates.get(cursor.min(candidates.len() - 1)).copied() {
                centres.push(candidate);
            }
        }
    }
    if centres.len() != usize::try_from(MOUND_COUNT).unwrap_or_default() {
        return Err(vec![recipe_issue(
            recipe,
            "could not select five distinct rolling-ground centres",
        )]);
    }
    Ok(mask
        .iter()
        .copied()
        .map(|coord| {
            let height = centres
                .iter()
                .enumerate()
                .map(|(index, centre)| {
                    let amplitude = if index == 0 {
                        max_relief
                    } else {
                        max_relief.saturating_sub(1).max(1)
                    };
                    let distance = i32::try_from(centre.distance(coord)).unwrap_or(i32::MAX);
                    amplitude.saturating_sub(distance / 2).max(0)
                })
                .max()
                .unwrap_or_default();
            (coord, base_level.saturating_add(height))
        })
        .collect())
}

pub(super) fn actor_anchors(
    ordinary: &BTreeMap<HexCoord, TilePos>,
    recipe: &'static str,
) -> Result<(TilePos, TilePos), Vec<WorldValidationIssue>> {
    let party = ordinary
        .values()
        .copied()
        .min_by_key(|position| (position.coord.x(), position.coord.y(), *position))
        .ok_or_else(|| vec![recipe_issue(recipe, "has no ordinary party landing")])?;
    let hostile = ordinary
        .values()
        .copied()
        .max_by_key(|position| (position.coord.x(), position.coord.y(), *position))
        .ok_or_else(|| vec![recipe_issue(recipe, "has no ordinary hostile landing")])?;
    Ok((party, hostile))
}

pub(super) fn grassland_column(surface: i32, trail: bool) -> VolumeColumn {
    VolumeColumn {
        elements: vec![
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(0, 1),
                material: SolidMaterialRole::Bedrock,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(1, surface.saturating_sub(3)),
                material: SolidMaterialRole::Stone,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(surface.saturating_sub(3), surface),
                material: SolidMaterialRole::Dirt,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(surface, surface.saturating_add(1)),
                material: if trail {
                    SolidMaterialRole::Gravel
                } else {
                    SolidMaterialRole::Grass
                },
                cutaway_for: None,
            }),
        ],
    }
}

pub(super) fn view_hint(
    radius: u32,
    base_level: i32,
    relief: i32,
    level_height: f32,
    recipe: &'static str,
) -> Result<MapViewHint, Vec<WorldValidationIssue>> {
    let radius = u16::try_from(radius)
        .map(f32::from)
        .map_err(|error| vec![recipe_issue(recipe, format!("radius exceeds u16: {error}"))])?;
    let focus_level = i16::try_from(base_level.saturating_add(relief / 2))
        .map(f32::from)
        .map_err(|error| {
            vec![recipe_issue(
                recipe,
                format!("focus level exceeds i16: {error}"),
            )]
        })?;
    let focus_y = focus_level * level_height;
    let hint = MapViewHint::new(
        (
            radius.mul_add(1.25, 4.0),
            focus_y + radius.mul_add(0.85, 8.0),
            radius.mul_add(1.35, 4.0),
        ),
        (0.0, focus_y, 0.0),
    );
    hint.is_valid()
        .then_some(hint)
        .ok_or_else(|| vec![recipe_issue(recipe, "camera hint is invalid")])
}

fn recipe_issue(recipe: &'static str, detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(
        WorldIssueCode::Recipe(recipe),
        format!("{recipe} {}", detail.into()),
    )
}
