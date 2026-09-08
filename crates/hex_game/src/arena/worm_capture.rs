//! Capture-only observation of public burrow outcomes and the ordinary R-key adapter.

use super::{ArenaCamera, ViewState};
use bevy::prelude::*;
use hex_arena::{ArenaSession, Species};
use hex_core::arena::{
    ArenaBurrowOutcome, ArenaBurrowResult, ArenaMaterials, ArenaReset, ArenaTerrainView,
    ArenaVoxelGeometry,
};
use hex_core::{DamagedVoxels, SubstanceId, TilePos};

#[derive(Clone, serde::Serialize)]
struct ChangedCell {
    position: TilePos,
    center: [f32; 3],
    before: SubstanceId,
    health_before: [u8; 2],
    health_after: [u8; 2],
}

#[derive(Clone, serde::Serialize)]
struct Conversion {
    generation: u64,
    actor: u8,
    sequence: u64,
    frame: u32,
    tick: u64,
    revision: u64,
    changed: Vec<ChangedCell>,
}

#[derive(Default, Resource)]
pub(super) struct Evidence {
    conversion: Option<Conversion>,
    reset_key_frame: Option<u32>,
    restored_revision: Option<u64>,
    accepted_outcomes: u64,
    rejected_outcomes: u64,
}

impl Evidence {
    pub(super) fn receipt(
        &self,
        view: &ArenaTerrainView,
        reset: &ArenaReset,
        materials: &ArenaMaterials,
        damaged: &DamagedVoxels,
    ) -> serde_json::Value {
        let current_cells = self.conversion.as_ref().map(|conversion| conversion.changed.iter().map(|change| {
            serde_json::json!({"position":change.position,"material":view.voxels.get(&change.position),
                "published_health":damaged.get(change.position).map(|health|[health.remaining,health.maximum])})
        }).collect::<Vec<_>>()).unwrap_or_default();
        serde_json::json!({
            "conversion": self.conversion, "current_cells": current_cells, "dirt":materials.dirt,
            "current_generation": reset.generation, "current_revision": view.revision,
            "accepted_outcomes":self.accepted_outcomes,"rejected_outcomes":self.rejected_outcomes,
            "reset_key":self.reset_key_frame.map(|frame|serde_json::json!({"key":"R","frame":frame})),
            "restored_revision":self.restored_revision,
            "boundary":"Capture-only public outcome/material/health observation; no terrain edits, actor poses or enemy HUD indicators are injected."
        })
    }
}

pub(super) fn phase_view(view: &str) -> bool {
    matches!(
        view,
        "encounter-worm-converted-earth" | "encounter-worm-reset"
    )
}

pub(super) fn observe(
    mut outcomes: MessageReader<ArenaBurrowOutcome>,
    session: Res<ArenaSession>,
    state: Res<ViewState>,
    view: Res<ArenaTerrainView>,
    geometry: Res<ArenaVoxelGeometry>,
    reset: Res<ArenaReset>,
    materials: Res<ArenaMaterials>,
    mut evidence: ResMut<Evidence>,
) {
    if state.capture.is_none() {
        outcomes.clear();
        return;
    }
    for outcome in outcomes.read() {
        if outcome.generation != reset.generation
            || !session
                .actors
                .iter()
                .any(|actor| actor.id == outcome.actor && actor.species == Species::Worm)
        {
            continue;
        }
        let changed = match &outcome.result {
            ArenaBurrowResult::Accepted { changed } => {
                evidence.accepted_outcomes += 1;
                changed
            }
            ArenaBurrowResult::Rejected { .. } => {
                evidence.rejected_outcomes += 1;
                continue;
            }
        };
        if evidence.conversion.is_some()
            || changed.is_empty()
            || !changed
                .iter()
                .all(|change| view.voxels.get(&change.position) == Some(&materials.dirt))
        {
            continue;
        }
        evidence.conversion = Some(Conversion {
            generation: outcome.generation,
            actor: outcome.actor,
            sequence: outcome.sequence,
            frame: state.frames,
            tick: session.tick,
            revision: view.revision,
            changed: changed
                .iter()
                .map(|change| ChangedCell {
                    position: change.position,
                    center: geometry.center(change.position).to_array(),
                    before: change.before,
                    health_before: [change.health_before.remaining, change.health_before.maximum],
                    health_after: [change.health_after.remaining, change.health_after.maximum],
                })
                .collect(),
        });
    }
}

/// Press the same R key consumed by input(), only after an actual conversion has
/// had four app frames. InputPlugin's next frame clears this synthetic key state.
pub(super) fn inject_reset_key(
    state: Res<ViewState>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut evidence: ResMut<Evidence>,
) {
    if state.capture.is_none() || state.capture_view != "encounter-worm-reset" {
        return;
    }
    if evidence.reset_key_frame.is_some() {
        keys.reset(KeyCode::KeyR);
        return;
    }
    if state.started
        && evidence
            .conversion
            .as_ref()
            .is_some_and(|conversion| state.frames >= conversion.frame.saturating_add(4))
    {
        keys.press(KeyCode::KeyR);
        evidence.reset_key_frame = Some(state.frames.saturating_add(1));
    }
}

pub(super) fn progress(
    mut state: ResMut<ViewState>,
    session: Res<ArenaSession>,
    view: Res<ArenaTerrainView>,
    reset: Res<ArenaReset>,
    materials: Res<ArenaMaterials>,
    damaged: Res<DamagedVoxels>,
    mut evidence: ResMut<Evidence>,
) {
    if state.capture.is_none()
        || !phase_view(&state.capture_view)
        || state.capture_event_frame.is_some()
    {
        return;
    }
    let Some(conversion) = &evidence.conversion else {
        return;
    };
    let reached = if state.capture_view == "encounter-worm-reset" {
        evidence.reset_key_frame.is_some()
            && !state.started
            && state.paused
            && reset.generation == conversion.generation.saturating_add(1)
            && view.revision != conversion.revision
            && session
                .actors
                .iter()
                .any(|actor| actor.species == Species::Worm)
            && conversion.changed.iter().all(|change| {
                view.voxels.get(&change.position) == Some(&change.before)
                    && damaged.get(change.position).is_none()
            })
    } else {
        reset.generation == conversion.generation
            && conversion.changed.iter().all(|change| {
                let sparse = damaged
                    .get(change.position)
                    .map(|health| [health.remaining, health.maximum]);
                view.voxels.get(&change.position) == Some(&materials.dirt)
                    && if change.health_after.first() == change.health_after.get(1) {
                        sparse.is_none()
                    } else {
                        sparse == Some(change.health_after)
                    }
            })
    };
    if reached {
        if state.capture_view == "encounter-worm-reset" {
            evidence.restored_revision = Some(view.revision);
        }
        state.capture_event_frame = Some(state.frames);
        state.accumulator = 0.0;
    }
}

pub(super) fn camera(
    state: Res<ViewState>,
    evidence: Res<Evidence>,
    mut cameras: Query<&mut Transform, With<ArenaCamera>>,
) {
    if state.capture.is_none() || state.capture_view != "encounter-worm-converted-earth" {
        return;
    }
    let Some(conversion) = &evidence.conversion else {
        return;
    };
    let bounds = conversion
        .changed
        .iter()
        .fold(None::<(Vec3, Vec3)>, |bounds, change| {
            let center = Vec3::from_array(change.center);
            let extent = Vec3::new(1.0, 0.2, 1.0);
            Some(
                bounds.map_or((center - extent, center + extent), |(low, high)| {
                    (low.min(center - extent), high.max(center + extent))
                }),
            )
        });
    if let (Some((low, high)), Ok(mut camera)) = (bounds, cameras.single_mut()) {
        *camera = super::encounter::frame_bounds(low, high, false);
    }
}
