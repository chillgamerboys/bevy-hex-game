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

#[derive(Clone, serde::Serialize)]
struct ExposedSurface {
    position: TilePos,
    top_center: [f32; 3],
    camera: [f32; 3],
    revision: u64,
    frame: u32,
}

#[derive(Default, Resource)]
pub(super) struct Evidence {
    conversion: Option<Conversion>,
    reset_key_frame: Option<u32>,
    restored_revision: Option<u64>,
    accepted_outcomes: u64,
    rejected_outcomes: u64,
    exposed_surface: Option<ExposedSurface>,
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
            "exposed_surface":self.exposed_surface,
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
    geometry: Res<ArenaVoxelGeometry>,
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
    let surface = if state.capture_view == "encounter-worm-converted-earth" {
        exposed_surface(
            conversion,
            &session,
            &view,
            *geometry,
            &materials,
            state.frames,
        )
    } else {
        None
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
        surface.is_some()
            && reset.generation == conversion.generation
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
        evidence.exposed_surface = surface;
        if state.capture_view == "encounter-worm-reset" {
            evidence.restored_revision = Some(view.revision);
        }
        state.capture_event_frame = Some(state.frames);
        state.accumulator = 0.0;
    }
}

/// A conservative composition query, never a terrain mutation. Preserve the first
/// correlated outcome and wait until one of its real top faces is unobstructed.
fn exposed_surface(
    conversion: &Conversion,
    session: &ArenaSession,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    materials: &ArenaMaterials,
    frame: u32,
) -> Option<ExposedSurface> {
    conversion.changed.iter().find_map(|change| {
        if view.voxels.get(&change.position) != Some(&materials.dirt)
            || view
                .voxels
                .keys()
                .any(|p| p.coord == change.position.coord && p.level > change.position.level)
            || view.static_spans.iter().any(|span| {
                span.bottom.coord == change.position.coord
                    && span.blocks_sight
                    && span.top_level > change.position.level
            })
        {
            return None;
        }
        let top = geometry
            .center(change.position)
            .with_y(geometry.top(change.position));
        let camera = top + Vec3::Y * 9.0;
        if session
            .camera_position(top + Vec3::Y * 0.2, camera)
            .distance(camera)
            > 0.08
        {
            return None;
        }
        let obscured = session
            .actors
            .iter()
            .filter(|actor| actor.hp > 0.0)
            .any(|actor| {
                let parts = actor.body_hex_prisms().collect::<Vec<_>>();
                let bounds = if parts.is_empty() {
                    let half = actor.body_dimensions() * 0.5;
                    let rotated = actor.body_rotation();
                    let extent = (rotated * Vec3::X).abs() * half.x
                        + (rotated * Vec3::Y).abs() * half.y
                        + (rotated * Vec3::Z).abs() * half.z;
                    vec![(actor.center() - extent, actor.center() + extent)]
                } else {
                    parts
                        .iter()
                        .map(|part| {
                            let bottom = actor.feet + part.offset;
                            let width = Vec3::new(0.866_025_4, 0.0, 1.0);
                            (bottom - width, bottom + width + Vec3::Y * part.height)
                        })
                        .collect()
                };
                bounds.iter().any(|(low, high)| {
                    // Native surface accents extend 0.019 above the physical prism.
                    // Wait for decorative geometry too; do not alter live models.
                    high.y + 0.03 > top.y + 0.01
                        && low.y < camera.y
                        && low.x < top.x + 0.9
                        && high.x > top.x - 0.9
                        && low.z < top.z + 1.05
                        && high.z > top.z - 1.05
                })
            });
        (!obscured).then_some(ExposedSurface {
            position: change.position,
            top_center: top.to_array(),
            camera: camera.to_array(),
            revision: view.revision,
            frame,
        })
    })
}

pub(super) fn camera(
    state: Res<ViewState>,
    evidence: Res<Evidence>,
    mut cameras: Query<&mut Transform, With<ArenaCamera>>,
) {
    if state.capture.is_none() || state.capture_view != "encounter-worm-converted-earth" {
        return;
    }
    let Some(surface) = &evidence.exposed_surface else {
        return;
    };
    if let Ok(mut camera) = cameras.single_mut() {
        *camera = Transform::from_translation(Vec3::from_array(surface.camera))
            .looking_at(Vec3::from_array(surface.top_center), Vec3::Z);
    }
}
