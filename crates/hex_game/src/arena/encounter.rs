//! Bounded presentation and ordinary-input capture adapter for encounter snapshots.

use super::{ArenaCamera, ViewState};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use hex_arena::{
    ActorIntent, ArenaSession, ArenaTuning, AttackPhase, CreatureAbility, Species, Spell,
};
use hex_core::arena::{ArenaMap, ArenaTerrainView, ArenaVoxelGeometry};

/// Synthetic visit duration; a complete three-party loop takes 1.8 seconds.
pub const STRESS_VISIT_TICKS: u32 = 72;

/// Explicit synthetic workload only: distant visible teleports must not exhaust
/// a party's home leash. Normal search clocks, sensing and combat remain intact.
/// Call before the reset/admission tick so the private party leash is consistent.
pub fn configure_encounter_stress_tuning(
    capture: bool,
    view: &str,
    tuning: &mut ArenaTuning,
) -> Result<bool, String> {
    if !capture || !stress_view(view) {
        return Ok(false);
    }
    let mut candidate = tuning.clone();
    candidate.encounters.ground_leash = 150.0;
    candidate.encounters.shadow_leash = 150.0;
    candidate.encounters.dragon_leash = 150.0;
    candidate.validate()?;
    *tuning = candidate;
    Ok(true)
}

fn stress_home_leashes(tuning: &ArenaTuning) -> [f32; 3] {
    [
        tuning.encounters.ground_leash,
        tuning.encounters.shadow_leash,
        tuning.encounters.dragon_leash,
    ]
}

pub(super) fn stress_view(view: &str) -> bool {
    view == "encounter-stress"
}

#[derive(serde::Serialize)]
pub(super) struct StressStimulus {
    representative: u8,
    human_feet: [f32; 3],
    cast_requested: bool,
    visit: u32,
    visit_anchor: Option<[f32; 3]>,
    pose_valid: bool,
    terrain_revision: u64,
    parties_before_tick: Vec<StressParty>,
    home_leashes: [f32; 3],
}

#[derive(serde::Serialize)]
struct StressParty {
    id: u16,
    phase: String,
    knowledge_source: String,
    knowledge_tick: Option<u64>,
}

#[derive(serde::Serialize)]
pub(super) struct StressTick {
    pub frame: u32,
    pub tick: u64,
    pub active_parties: usize,
    pub living_enemies: usize,
    pub terrain_publication: bool,
    pub damage_outcome: bool,
    pub destroyed_voxels: usize,
    pub cpu_ms: f64,
    pub stimulus: StressStimulus,
}

/// Explicit synthetic capture fixture, never used by native play or visual approval.
pub(super) fn prepare_stress_tick(world: &mut World) -> Option<StressStimulus> {
    let state = world.resource::<ViewState>();
    if state.capture.is_none() || !stress_view(&state.capture_view) {
        return None;
    }
    let home_leashes = stress_home_leashes(world.resource::<ArenaTuning>());
    if home_leashes
        .iter()
        .any(|leash| (*leash - 150.0).abs() > f32::EPSILON)
    {
        error!(
            "Synthetic encounter stress requires validated 150-unit home leashes before admission"
        );
        return None;
    }
    let step = state.capture_stress_steps;
    let initialize = !state.capture_stress_initialized;
    let visit = step / STRESS_VISIT_TICKS;
    if world.resource::<ArenaSession>().parties().is_empty() {
        return None;
    }
    if initialize {
        for actor in &mut world.resource_mut::<ArenaSession>().actors {
            actor.max_hp = 100_000.0;
            actor.hp = actor.max_hp;
        }
        world.resource_mut::<ViewState>().capture_stress_initialized = true;
    }
    let (
        representative,
        feet,
        aim,
        visit_anchor,
        pose_valid,
        terrain_revision,
        parties_before_tick,
    ) = {
        let session = world.resource::<ArenaSession>();
        let terrain = world.resource::<ArenaTerrainView>();
        let geometry = *world.resource::<ArenaVoxelGeometry>();
        let representatives = session
            .parties()
            .iter()
            .filter_map(|party| {
                session
                    .actors
                    .iter()
                    .find(|actor| actor.party == Some(party.id))
            })
            .collect::<Vec<_>>();
        if representatives.is_empty() {
            return None;
        }
        let index = usize::try_from(visit).ok()? % representatives.len();
        let target = *representatives.get(index)?;
        let human = session.actors.first()?;
        let home = session
            .parties()
            .iter()
            .find(|party| Some(party.id) == target.party)?
            .home;
        // Reuse this party's area across visits too: rebuilding a forward area
        // around its pursuing representative slowly drags the group off leash.
        let previous_anchor = world
            .resource::<ViewState>()
            .capture_stress_ticks
            .iter()
            .rev()
            .find(|row| row.stimulus.representative == target.id)
            .and_then(|row| row.stimulus.visit_anchor)
            .map(Vec3::from_array);
        let placement = stress_target_pose(
            session,
            human.id,
            target.id,
            previous_anchor,
            home,
            terrain,
            geometry,
        );
        let feet = placement.unwrap_or(human.feet);
        let aim = (target.center() - (feet + human.eye() - human.feet)).normalize_or(Vec3::NEG_Z);
        let knowledge = session.party_knowledge();
        let parties = session
            .parties()
            .iter()
            .map(|party| {
                let observed = knowledge.iter().find(|known| known.id == party.id);
                StressParty {
                    id: party.id,
                    phase: format!("{:?}", party.phase),
                    knowledge_source: observed.map_or("none", |known| known.source).into(),
                    knowledge_tick: observed.and_then(|known| known.tick),
                }
            })
            .collect();
        (
            target.id,
            feet,
            aim,
            placement.or(previous_anchor),
            placement.is_some(),
            terrain.revision,
            parties,
        )
    };
    if let Some(human) = world.resource_mut::<ArenaSession>().actors.first_mut() {
        human.feet = feet;
        human.aim = aim;
    }
    let cast_requested = pose_valid && step.is_multiple_of(240);
    world.resource_mut::<hex_arena::ArenaInput>().human = ActorIntent {
        aim,
        selected: Some(Spell::AreaBlast),
        cast_pressed: cast_requested,
        cast_released: cast_requested,
        ..default()
    };
    Some(StressStimulus {
        representative,
        human_feet: feet.to_array(),
        cast_requested,
        visit,
        visit_anchor: visit_anchor.map(|point| point.to_array()),
        pose_valid,
        terrain_revision,
        parties_before_tick,
        home_leashes,
    })
}

/// Revalidate a synthetic visit anchor against current geometry without pulling
/// its party progressively away from home. This never changes combat state.
#[must_use]
pub fn stress_target_pose(
    session: &ArenaSession,
    actor: u8,
    representative: u8,
    previous: Option<Vec3>,
    home: Vec3,
    terrain: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) -> Option<Vec3> {
    let near_home = |feet: &Vec3| feet.distance(home) <= 10.0;
    if let Some(feet) = session
        .synthetic_combat_target_pose(actor, representative, previous, terrain, geometry)
        .filter(near_home)
    {
        return Some(feet);
    }
    // A destroyed/occluded anchor may need another small home-area placement.
    // Each query already bounds surfaces and admits complete dry body + sight.
    for offset in [
        Vec3::ZERO,
        Vec3::X * 4.0,
        Vec3::NEG_X * 4.0,
        Vec3::Z * 4.0,
        Vec3::NEG_Z * 4.0,
    ] {
        if let Some(feet) = session
            .visible_supported_actor_pose(actor, representative, home + offset, terrain, geometry)
            .filter(near_home)
        {
            return Some(feet);
        }
    }
    None
}

const FORT_APPROACH: [(i32, i32); 7] = [
    (9, -4),
    (8, -3),
    (7, -3),
    (6, -3),
    (5, -3),
    (4, -3),
    (3, -3),
];

pub(super) fn fort_approach_complete(step: usize) -> bool {
    step >= FORT_APPROACH.len()
}

pub(super) fn composition_view(view: &str, map: ArenaMap) -> bool {
    map == ArenaMap::Fort
        && (matches!(view, "encounter-first" | "encounter-third")
            || view.starts_with("encounter-body"))
}

/// Replay the accepted Fort gate/keep detour with ordinary movement. Directly
/// chasing a creature cuts through the keep at this fixed seed.
/// Cell centers match the frozen FORT contract in tests/arena_routes.rs.
pub(super) fn fort_capture_waypoint(
    view: &str,
    terrain: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    session: &ArenaSession,
    step: &mut usize,
) -> Option<Vec3> {
    if terrain.selection.map != ArenaMap::Fort
        || !view.starts_with("encounter-")
        || view == "encounter-landmark"
        || stress_view(view)
    {
        return None;
    }
    let human = session.actors.first()?;
    while let Some(&(q, r)) = FORT_APPROACH.get(*step) {
        let pos = hex_core::TilePos::new(hex_core::HexCoord::from_axial(q, r), 15);
        let waypoint = pos.coord.to_world(geometry.top(pos));
        if human.feet.with_y(0.0).distance(waypoint.with_y(0.0)) >= 0.16
            || (human.feet.y - waypoint.y).abs() >= 0.1
        {
            return Some(waypoint);
        }
        *step += 1;
    }
    None
}

pub(super) fn capture_intent(
    frame: u32,
    session: &ArenaSession,
    view: &str,
    waypoint: Option<Vec3>,
    terrain: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &ArenaTuning,
) -> ActorIntent {
    let Some(human) = session.actors.first() else {
        return ActorIntent::default();
    };
    if let Some(waypoint) = waypoint {
        return ActorIntent {
            aim: (waypoint - human.feet)
                .with_y(0.0)
                .normalize_or(Vec3::NEG_Z),
            movement: Vec2::Y,
            run: true,
            ..default()
        };
    }
    let Some(enemy) = session
        .actors
        .iter()
        .filter(|a| a.team != human.team && a.hp > 0.0)
        .min_by(|a, b| {
            a.feet
                .distance_squared(human.feet)
                .total_cmp(&b.feet.distance_squared(human.feet))
        })
    else {
        return ActorIntent::default();
    };
    if view
        .strip_suffix("-rear")
        .unwrap_or(view)
        .starts_with("encounter-golem-swipe")
        && enemy.species == Species::Golem
    {
        return golem_swipe_input(session, human, enemy, terrain, geometry, tuning);
    }
    let target = if enemy.species == hex_arena::Species::Worm {
        enemy.eye()
    } else {
        enemy.center()
    };
    let offset = target - human.eye();
    let distance = offset.with_y(0.0).length();
    let direction = offset.normalize_or(Vec3::NEG_Z);
    let close = view.contains("swipe") || view.contains("windup") || view.contains("breath");
    let preferred = if close { 1.6 } else { 4.0 };
    let movement = if distance > preferred {
        Vec2::Y
    } else {
        Vec2::ZERO
    };
    // Only Barrier needs projectile pressure. Firing at the Shaman instead
    // encourages defensive Shield and can obstruct the Fireball/Aura proof.
    let attack = view.contains("barrier");
    let cycle = frame.saturating_add(44) % 150;
    ActorIntent {
        movement,
        aim: direction,
        jump: movement != Vec2::ZERO && frame.is_multiple_of(45),
        run: distance > 8.0,
        selected: Some(Spell::Fireball),
        cast_pressed: attack && cycle == 45,
        cast_held: attack && (45..76).contains(&cycle),
        cast_released: attack && cycle == 76,
    }
}

// This review script supplies only ordinary player inputs. A real Shield creates
// the obstruction; Golem's unchanged movement/attack policy must choose its swipe.
fn golem_swipe_input(
    session: &ArenaSession,
    human: &hex_arena::Actor,
    golem: &hex_arena::Actor,
    terrain: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &ArenaTuning,
) -> ActorIntent {
    let away = (human.feet - golem.feet).with_y(0.0).normalize_or(Vec3::X);
    let distance = human.feet.with_y(0.0).distance(golem.feet.with_y(0.0));
    let front = golem.body_dimensions().x.max(golem.body_dimensions().z) * 0.5;
    // Leave enough room for seed flight and normal .18s emergence before the
    // advancing body reaches the wall. Current preview clips all actual bodies.
    let ground = golem.feet + away * (front + 2.0);
    let offset = ground - human.eye();
    let horizontal = offset.with_y(0.0).length();
    let speed = tuning.launch_speed(0.0);
    let gravity = tuning.projectile_gravity;
    let square = speed * speed;
    let discriminant =
        square * square - gravity * (gravity * horizontal * horizontal + 2.0 * offset.y * square);
    let aim = if horizontal > 0.1 && discriminant >= 0.0 {
        let tangent = (square - discriminant.sqrt()) / (gravity * horizontal);
        (offset.with_y(0.0).normalize_or(-away) + Vec3::Y * tangent).normalize_or(-away)
    } else {
        -away
    };
    let mut input = ActorIntent {
        aim,
        selected: Some(Spell::Shield),
        ..default()
    };
    let ready = human
        .cooldowns
        .first()
        .is_some_and(|cooldown| *cooldown <= 0.0);
    if ready && human.selected == Spell::Shield && human.charge().is_none() {
        // Selection/aim is installed by a previous normal input tick. This
        // forecast uses that exact existing aim, never a fabricated collision map.
        let preview = hex_arena::preview(session, terrain, &geometry, tuning);
        let side = away.cross(Vec3::Y);
        let blocks_lane = preview.wall_voxels.iter().any(|surface| {
            let point = geometry.center(*surface);
            let relative = (point - golem.feet).with_y(0.0);
            let forward = relative.dot(away);
            forward > front + 0.5
                && forward < front + 3.5
                && relative.dot(side).abs() < 1.0
                && point.y > golem.feet.y + 0.4
                && point.y < golem.feet.y + golem.body_dimensions().y
        });
        if preview.valid && blocks_lane {
            input.aim = human.aim;
            input.cast_pressed = true;
            input.cast_released = true;
            return input;
        }
    }
    // Stay outside Slam and below long-beam range. The public pose query only
    // admits a nearby dry, clear step destination; actual displacement still runs
    // through the ordinary WASD controller and collision sweep.
    let travel = if distance < 9.0 {
        away
    } else if distance > 10.0 {
        -away
    } else {
        Vec3::ZERO
    };
    if travel.length_squared() > 0.0 {
        let desired = human.feet + travel * 0.6;
        let safe = session
            .visible_supported_actor_pose(human.id, golem.id, desired, terrain, geometry)
            .is_some_and(|pose| {
                pose.with_y(0.0).distance(desired.with_y(0.0)) < 0.9
                    && (pose.y - human.feet.y).abs() < 0.1
            });
        if safe {
            let forward = input.aim.with_y(0.0).normalize_or(-away);
            input.movement = Vec2::new(travel.dot(forward.cross(Vec3::Y)), travel.dot(forward));
            input.run = true;
        }
    }
    input
}

#[cfg(all(test, feature = "test-support"))]
#[path = "golem_swipe_capture_tests.rs"]
mod golem_swipe_capture_tests;

// Use the same snapshot match for readiness and framing. An unrelated enemy
// attack must not steal the camera from the effect the capture is reviewing.
fn phase_owner(
    view: &str,
    actors: impl IntoIterator<
        Item = (
            u8,
            Option<hex_arena::ChargeState>,
            Option<hex_arena::AttackSnapshot>,
        ),
    >,
    barriers: &[hex_arena::BarrierSnapshot],
    auras: &[hex_arena::AuraSnapshot],
) -> Option<u8> {
    let view = view.strip_suffix("-rear").unwrap_or(view);
    if view == "encounter-barrier" {
        return barriers.first().map(|barrier| barrier.owner);
    }
    if view == "encounter-aura" {
        return auras.first().map(|aura| aura.owner);
    }
    actors.into_iter().find_map(|(id, charge, attack)| {
        if id == 0 {
            return None;
        }
        let matches = if view == "encounter-fireball" {
            charge.is_some_and(|charge| charge.spell == Spell::Fireball && charge.elapsed >= 0.25)
        } else {
            attack.is_some_and(|attack| match view {
                "encounter-windup" => {
                    attack.phase == AttackPhase::Windup && attack.progress >= 0.25
                }
                "encounter-breath" => {
                    attack.kind == CreatureAbility::FireCone && attack.phase == AttackPhase::Active
                }
                "encounter-swipe" => {
                    attack.kind == CreatureAbility::Swipe
                        && attack.phase == AttackPhase::Windup
                        && attack.progress >= 0.25
                }
                _ => false,
            })
        };
        matches.then_some(id)
    })
}

fn phase_actor<'a>(session: &'a ArenaSession, view: &str) -> Option<&'a hex_arena::Actor> {
    let owner = phase_owner(
        view,
        session
            .actors
            .iter()
            .filter(|actor| actor.hp > 0.0)
            .map(|actor| (actor.id, actor.charge(), actor.attack_state())),
        session.barriers(),
        session.auras(),
    )?;
    session.actors.iter().find(|actor| actor.id == owner)
}

pub(super) fn phase_ready(session: &ArenaSession, view: &str) -> bool {
    phase_actor(session, view).is_some()
        || super::golem::phase_actor(session, view).is_some()
        || super::wisp::phase_actor(session, view).is_some()
        || super::worm::phase_actor(session, view).is_some()
}

pub(super) fn phase_view(view: &str) -> bool {
    super::golem::phase_view(view)
        || super::wisp::phase_view(view)
        || super::worm::phase_view(view)
        || super::worm_capture::phase_view(view)
        || matches!(
            view.strip_suffix("-rear").unwrap_or(view),
            "encounter-windup"
                | "encounter-breath"
                | "encounter-swipe"
                | "encounter-barrier"
                | "encounter-aura"
                | "encounter-fireball"
        )
}

fn close_camera(target: Vec3, rotation: Quat, view: &str) -> Transform {
    let mut offset = if view.starts_with("encounter-body") {
        rotation * Vec3::new(3.8, 1.8, -4.5)
    } else {
        Vec3::new(5.0, 4.0, 7.0)
    };
    if view.ends_with("-rear") {
        offset.x = -offset.x;
        offset.z = -offset.z;
    }
    Transform::from_translation(target + offset).looking_at(target, Vec3::Y)
}

pub(super) fn overview(
    geometry: ArenaVoxelGeometry,
    view: &ArenaTerrainView,
    rear: bool,
) -> Transform {
    let radius = i32::try_from(geometry.radius).unwrap_or(33);
    let mut minimum = Vec3::splat(f32::INFINITY);
    let mut maximum = Vec3::splat(f32::NEG_INFINITY);
    for (q, r) in [
        (radius, 0),
        (radius, -radius),
        (0, -radius),
        (-radius, 0),
        (-radius, radius),
        (0, radius),
    ] {
        let point = hex_core::HexCoord::from_axial(q, r).to_world(0.0);
        minimum = minimum.min(point - Vec3::new(1.1, 0.0, 1.1));
        maximum = maximum.max(point + Vec3::new(1.1, 0.0, 1.1));
    }
    for span in view.columns.values().flatten() {
        minimum.y = minimum
            .y
            .min(geometry.top(span.bottom) - geometry.level_height);
        maximum.y = maximum
            .y
            .max(geometry.top(hex_core::TilePos::new(span.bottom.coord, span.top_level)));
    }
    for span in &view.static_spans {
        maximum.y = maximum
            .y
            .max(geometry.top(hex_core::TilePos::new(span.bottom.coord, span.top_level)));
    }
    frame_bounds(minimum, maximum, rear)
}

pub(super) fn frame_bounds(minimum: Vec3, maximum: Vec3, rear: bool) -> Transform {
    let center = (minimum + maximum) * 0.5;
    let forward = Vec3::new(
        if rear { 0.65 } else { -0.65 },
        -0.85,
        if rear { 0.75 } else { -0.75 },
    )
    .normalize();
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward);
    let tangent = (75.0_f32.to_radians() * 0.5).tan();
    let mut distance: f32 = 1.0;
    for x in [minimum.x, maximum.x] {
        for y in [minimum.y, maximum.y] {
            for z in [minimum.z, maximum.z] {
                let offset = Vec3::new(x, y, z) - center;
                distance = distance.max(offset.dot(up).abs() / tangent - offset.dot(forward));
                distance = distance
                    .max(offset.dot(right).abs() / (tangent * 16.0 / 9.0) - offset.dot(forward));
            }
        }
    }
    Transform::from_translation(center - forward * (distance * 1.12 + 3.0))
        .looking_to(forward, Vec3::Y)
}

/// Conservative composition admission: a living subject must occupy a useful
/// part of the real camera frustum and have an unobstructed terrain ray.
pub(super) fn visible_subjects(session: &ArenaSession, camera: &Transform, view: &str) -> Vec<u8> {
    let Some(human) = session.actors.first() else {
        return Vec::new();
    };
    session
        .actors
        .iter()
        .filter(|actor| {
            if actor.team == human.team
                || actor.hp <= 0.0
                || (view.starts_with("encounter-body") && actor.id != 1)
            {
                return false;
            }
            let (point, visible_height) = if actor.species == Species::Worm {
                if !actor.worm().is_some_and(|worm| worm.exposed) {
                    return false;
                }
                (
                    actor.eye(),
                    actor
                        .body_hex_prisms()
                        .next()
                        .map_or(0.0, |part| part.height),
                )
            } else {
                (actor.center(), actor.body_dimensions().y)
            };
            let local = camera.rotation.inverse() * (point - camera.translation);
            let depth = -local.z;
            let tangent = (75.0_f32.to_radians() * 0.5).tan();
            depth > 0.1
                && depth < 80.0
                && local.y.abs() < depth * tangent * 0.85
                && local.x.abs() < depth * tangent * (16.0 / 9.0) * 0.85
                && visible_height / depth > 0.02
                && session
                    .camera_position(camera.translation, point)
                    .distance(point)
                    < 0.08
        })
        .map(|actor| actor.id)
        .collect()
}

fn phase_camera(session: &ArenaSession, view: &str) -> Option<Transform> {
    if view.strip_suffix("-rear").unwrap_or(view) == "encounter-barrier" {
        let barrier = session.barriers().first()?;
        let normal = barrier.normal.with_y(0.0).normalize_or(Vec3::Z);
        let side = normal.cross(Vec3::Y);
        let horizontal =
            (normal * 7.0 + side * 2.5) * if view.ends_with("-rear") { -1.0 } else { 1.0 };
        return Some(
            Transform::from_translation(barrier.center + horizontal + Vec3::Y * 4.0)
                .looking_at(barrier.center, Vec3::Y),
        );
    }
    let actor = phase_actor(session, view)?;
    if let Some(attack) = actor.attack_state().filter(|attack| {
        matches!(
            attack.kind,
            CreatureAbility::FireCone | CreatureAbility::Bite | CreatureAbility::Swipe
        )
    }) {
        let direction = attack.direction.normalize_or(Vec3::NEG_Z);
        let target = attack.origin + direction * attack.range * 0.4;
        let side = direction
            .with_y(0.0)
            .normalize_or(Vec3::NEG_Z)
            .cross(Vec3::Y);
        let points = [
            attack.origin,
            target,
            attack.origin + direction * attack.range * 0.8,
        ];
        // Pick an opaque-world-visible review side of the actual attack lane;
        // this changes only the external review camera, never bodies or effects.
        return [-1.0, 1.0]
            .into_iter()
            .map(|sign| {
                Transform::from_translation(
                    target + side * sign * 7.0 + Vec3::Y * 5.0 - direction * 2.0,
                )
                .looking_at(target, Vec3::Y)
            })
            .max_by_key(|camera| {
                points
                    .iter()
                    .filter(|point| {
                        session
                            .camera_position(camera.translation, **point)
                            .distance(**point)
                            < 0.08
                    })
                    .count()
            });
    }
    let target = if view.strip_suffix("-rear").unwrap_or(view) == "encounter-aura" {
        session.auras().first()?.center
    } else {
        actor.feet + Vec3::Y * actor.body_dimensions().y * 0.5
    };
    Some(close_camera(target, actor.body_rotation(), view))
}

pub(super) fn camera(
    session: Res<ArenaSession>,
    state: Res<ViewState>,
    view: Res<ArenaTerrainView>,
    geometry: Res<ArenaVoxelGeometry>,
    mut cameras: Query<&mut Transform, With<ArenaCamera>>,
) {
    if !state.external_camera() {
        return;
    }
    let Ok(mut camera) = cameras.single_mut() else {
        return;
    };
    if matches!(state.capture_view.as_str(), "overview" | "rear") {
        *camera = overview(*geometry, &view, state.capture_view == "rear");
    } else if state.capture_view == "encounter-landmark" {
        if let Some(anchor) = state
            .capture_focus
            .as_ref()
            .and_then(|name| view.anchors.get(name))
        {
            *camera = Transform::from_translation(*anchor + Vec3::new(12.0, 15.0, 16.0))
                .looking_at(*anchor + Vec3::Y, Vec3::Y);
        }
    } else if let Some(pose) = phase_camera(&session, &state.capture_view) {
        *camera = pose;
    } else if state.capture_view.starts_with("encounter-") {
        let actor = if stress_view(&state.capture_view) {
            session.actors.first()
        } else if state.capture_view.starts_with("encounter-body") {
            session.actors.get(1)
        } else {
            phase_actor(&session, &state.capture_view).or_else(|| session.actors.get(1))
        };
        if let Some(actor) = actor {
            let target = actor.feet + Vec3::Y * actor.body_dimensions().y * 0.5;
            *camera = close_camera(target, actor.body_rotation(), &state.capture_view);
        }
    }
}

#[derive(Resource)]
pub(super) struct VisualAssets {
    block: Handle<Mesh>,
    sphere: Handle<Mesh>,
    cone: Handle<Mesh>,
    barrier: Handle<StandardMaterial>,
    aura: Handle<StandardMaterial>,
    breath: Handle<StandardMaterial>,
}
// These meshes visualize effects; they must not project opaque geometry shadows.
#[derive(Component)]
#[require(NotShadowCaster)]
pub(super) struct EncounterEffect;

pub(super) fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut material = |color: Color| {
        materials.add(StandardMaterial {
            base_color: color,
            unlit: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Blend,
            ..default()
        })
    };
    commands.insert_resource(VisualAssets {
        block: meshes.add(Cuboid::from_size(Vec3::ONE)),
        sphere: meshes.add(Sphere::new(1.0).mesh().uv(20, 12)),
        cone: meshes.add(Cone::new(1.0, 1.0)),
        barrier: material(Color::srgba(0.12, 0.78, 1.0, 0.22)),
        aura: material(Color::srgba(0.30, 0.95, 0.51, 0.015)),
        breath: material(Color::srgba(1.0, 0.31, 0.025, 0.19)),
    });
}

pub(super) fn effects(
    mut commands: Commands,
    session: Res<ArenaSession>,
    tuning: Res<ArenaTuning>,
    assets: Res<VisualAssets>,
    golem_assets: Res<super::golem::GolemVisualAssets>,
    wisp_assets: Res<super::wisp::WispVisualAssets>,
    worm_assets: Res<super::worm::WormVisualAssets>,
    previous: Query<Entity, With<EncounterEffect>>,
    mut gizmos: Gizmos,
) {
    for entity in &previous {
        commands.entity(entity).despawn();
    }
    for barrier in session.barriers() {
        let normal = barrier.normal.normalize_or(Vec3::Z);
        let rotation = Quat::from_rotation_arc(Vec3::Z, normal);
        commands.spawn((
            EncounterEffect,
            Mesh3d(assets.block.clone()),
            MeshMaterial3d(assets.barrier.clone()),
            Transform::from_translation(barrier.center)
                .with_rotation(rotation)
                .with_scale(Vec3::new(barrier.width, barrier.height, 0.055)),
        ));
        let right = rotation * Vec3::X * barrier.width * 0.5;
        let up = Vec3::Y * barrier.height * 0.5;
        let corners = [
            barrier.center - right - up,
            barrier.center + right - up,
            barrier.center + right + up,
            barrier.center - right + up,
        ];
        for (a, b) in corners.into_iter().zip(corners.into_iter().cycle().skip(1)) {
            gizmos.line(a, b, Color::srgb(0.35, 0.91, 1.0));
        }
        let start = barrier.center - right + up + Vec3::Y * 0.06;
        gizmos.line(
            start,
            start + right * 2.0 * (barrier.hp / barrier.max_hp.max(0.01)).clamp(0.0, 1.0),
            Color::srgb(0.45, 1.0, 0.94),
        );
    }
    for aura in session.auras() {
        commands.spawn((
            EncounterEffect,
            Mesh3d(assets.sphere.clone()),
            MeshMaterial3d(assets.aura.clone()),
            Transform::from_translation(aura.center).with_scale(Vec3::splat(aura.radius)),
        ));
        let mut last = aura.center + Vec3::X * aura.radius;
        for step in 1..=48u16 {
            let angle = f32::from(step) * std::f32::consts::TAU / 48.0;
            let next = aura.center + Vec3::new(angle.cos(), 0.02, angle.sin()) * aura.radius;
            gizmos.line(last, next, Color::srgba(0.35, 0.98, 0.58, 0.8));
            last = next;
        }
    }
    for actor in session.actors.iter().filter(|a| a.hp > 0.0) {
        if actor.species == hex_arena::Species::Worm {
            super::worm::windup(&mut commands, actor, &worm_assets);
            continue;
        }
        if actor.species == hex_arena::Species::Wisp {
            let accent = if super::spectator::active(&session) {
                super::spectator::team_color(&session, actor.team)
            } else {
                Color::srgb(1.0, 0.72, 0.25)
            };
            super::wisp::glow(&mut commands, actor, &wisp_assets, &mut gizmos, accent);
            continue;
        }
        if actor.species == hex_arena::Species::Golem {
            let accent = if super::spectator::active(&session) {
                super::spectator::team_color(&session, actor.team)
            } else {
                Color::srgb(1.0, 0.66, 0.24)
            };
            super::golem::face_and_laser(
                &mut commands,
                actor,
                super::golem::MouthPose {
                    origin: actor.eye(),
                    direction: actor.aim,
                },
                &golem_assets,
                &mut gizmos,
                accent,
            );
            continue;
        }
        if session.human_actor_id() != Some(actor.id) {
            if let Some(charge) = actor.charge() {
                let progress = (charge.elapsed / tuning.charge_seconds).clamp(0.0, 1.0);
                let origin = actor.eye() + actor.aim.normalize_or(Vec3::NEG_Z) * 0.32;
                commands.spawn((
                    EncounterEffect,
                    Mesh3d(assets.sphere.clone()),
                    MeshMaterial3d(if charge.spell == Spell::Shield {
                        assets.barrier.clone()
                    } else {
                        assets.breath.clone()
                    }),
                    Transform::from_translation(origin)
                        .with_scale(Vec3::splat(0.07 + progress * 0.12)),
                ));
            }
        }
        let Some(attack) = actor.attack_state() else {
            continue;
        };
        let direction = attack.direction.normalize_or(Vec3::NEG_Z);
        let color = if attack.phase == AttackPhase::Windup {
            Color::srgb(1.0, 0.72, 0.20)
        } else {
            Color::srgb(1.0, 0.32, 0.06)
        };
        gizmos.sphere(Isometry3d::from_translation(attack.origin), 0.075, color);
        if attack.kind == CreatureAbility::FireCone && attack.phase == AttackPhase::Active {
            let radius = attack.range * attack.half_angle.tan();
            commands.spawn((
                EncounterEffect,
                Mesh3d(assets.cone.clone()),
                MeshMaterial3d(assets.breath.clone()),
                Transform::from_translation(attack.origin + direction * attack.range * 0.5)
                    .with_rotation(Quat::from_rotation_arc(Vec3::NEG_Y, direction))
                    .with_scale(Vec3::new(radius, attack.range, radius)),
            ));
        }
        if matches!(
            attack.kind,
            CreatureAbility::Swipe | CreatureAbility::Bite | CreatureAbility::FireCone
        ) {
            let side = direction.cross(Vec3::Y).normalize_or(Vec3::X);
            let reach = attack.range;
            let half = attack.half_angle;
            let mut previous = attack.origin + (direction * half.cos() - side * half.sin()) * reach;
            gizmos.line(attack.origin, previous, color);
            for step in 1..=16u16 {
                let angle = -half + 2.0 * half * f32::from(step) / 16.0;
                let point = attack.origin + (direction * angle.cos() + side * angle.sin()) * reach;
                gizmos.line(previous, point, color);
                previous = point;
            }
            gizmos.line(attack.origin, previous, color);
        }
        if attack.phase == AttackPhase::Windup {
            let start = attack.origin + Vec3::Y * 0.16 - Vec3::X * 0.22;
            gizmos.line(
                start,
                start + Vec3::X * 0.44 * attack.progress.clamp(0.0, 1.0),
                color,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonphysical_spell_meshes_cannot_cast_shadows_and_aura_fill_stays_subtle() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        assert!(
            world.run_system_once(setup).is_ok(),
            "encounter visual setup"
        );
        for entity in [
            world.spawn(EncounterEffect).id(),
            world
                .spawn(super::super::presentation::TransientEffect)
                .id(),
        ] {
            assert!(
                world.get::<NotShadowCaster>(entity).is_some(),
                "effect markers must require non-shadow-casting on every spawned visual"
            );
        }
        let assets = world.resource::<VisualAssets>();
        let materials = world.resource::<Assets<StandardMaterial>>();
        for handle in [&assets.barrier, &assets.aura, &assets.breath] {
            assert!(
                materials.get(handle).is_some_and(|material| {
                    material.unlit
                        && material.cull_mode.is_none()
                        && material.alpha_mode == AlphaMode::Blend
                }),
                "translucent encounter volumes must remain unlit and visible from both sides"
            );
        }
        assert!(
            materials.get(&assets.aura).is_some_and(|material| {
                let alpha = material.base_color.alpha();
                alpha > 0.0 && alpha <= 0.02
            }),
            "the radius shell must remain a subtle tint behind the readable ring"
        );
    }

    #[test]
    fn rear_reviews_preserve_phase_waits_and_reverse_only_the_camera_azimuth() {
        let target = Vec3::new(4.0, 2.0, -5.0);
        for view in ["encounter-barrier", "encounter-aura", "encounter-body"] {
            let rear = format!("{view}-rear");
            assert_eq!(phase_view(view), phase_view(&rear));
            assert_eq!(phase_view(view), view != "encounter-body");
            let front_pose = close_camera(target, Quat::from_rotation_y(0.7), view);
            let rear_pose = close_camera(target, Quat::from_rotation_y(0.7), &rear);
            let front_offset = front_pose.translation - target;
            let rear_offset = rear_pose.translation - target;
            assert!((front_offset.x + rear_offset.x).abs() < 0.001);
            assert!((front_offset.z + rear_offset.z).abs() < 0.001);
            assert!((front_offset.y - rear_offset.y).abs() < 0.001);
            for pose in [front_pose, rear_pose] {
                assert!(
                    Vec3::from(pose.forward()).dot((target - pose.translation).normalize()) > 0.999
                );
            }
        }
    }
    #[test]
    fn phase_camera_follows_the_requested_owner_despite_an_unrelated_swipe() {
        use hex_arena::{AttackSnapshot, AuraSnapshot, BarrierSnapshot, ChargeState};
        let actors = [
            (
                2,
                None,
                Some(AttackSnapshot {
                    kind: CreatureAbility::Swipe,
                    phase: AttackPhase::Windup,
                    origin: Vec3::ZERO,
                    direction: Vec3::X,
                    range: 1.7,
                    half_angle: 0.7,
                    progress: 0.5,
                }),
            ),
            (
                1,
                Some(ChargeState {
                    spell: Spell::Fireball,
                    elapsed: 0.3,
                }),
                None,
            ),
            (3, None, None),
        ];
        let barriers = [BarrierSnapshot {
            id: 1,
            owner: 3,
            center: Vec3::Z,
            normal: Vec3::X,
            width: 3.5,
            height: 1.6,
            hp: 60.0,
            max_hp: 60.0,
            remaining: 4.0,
            lifetime: 4.0,
        }];
        let auras = [AuraSnapshot {
            owner: 1,
            center: Vec3::ZERO,
            radius: 6.0,
            remaining: 5.0,
            lifetime: 5.0,
        }];
        for suffix in ["", "-rear"] {
            for (view, expected) in [
                ("encounter-fireball", 1),
                ("encounter-aura", 1),
                ("encounter-barrier", 3),
                ("encounter-swipe", 2),
            ] {
                assert_eq!(
                    phase_owner(&format!("{view}{suffix}"), actors, &barriers, &auras),
                    Some(expected)
                );
            }
        }
        assert_eq!(
            phase_owner("encounter-breath", actors, &barriers, &auras),
            None
        );
        assert_eq!(phase_owner("encounter-aura", actors, &barriers, &[]), None);
        assert_eq!(phase_owner("encounter-barrier", actors, &[], &auras), None);
    }

    #[test]
    fn bounds_framing_contains_all_corners_at_both_azimuths() {
        for rear in [false, true] {
            let minimum = Vec3::new(-59.0, -0.4, -52.0);
            let maximum = Vec3::new(59.0, 35.0, 52.0);
            let camera = frame_bounds(minimum, maximum, rear);
            let tangent = (75.0_f32.to_radians() * 0.5).tan();
            for x in [minimum.x, maximum.x] {
                for y in [minimum.y, maximum.y] {
                    for z in [minimum.z, maximum.z] {
                        let local =
                            camera.rotation.inverse() * (Vec3::new(x, y, z) - camera.translation);
                        assert!(-local.z > 0.035 && -local.z < 480.0);
                        assert!(local.y.abs() < -local.z * tangent);
                        assert!(local.x.abs() < -local.z * tangent * 16.0 / 9.0);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod stress_tuning_tests {
    use super::*;

    #[test]
    fn extended_leashes_require_explicit_capture_and_change_no_other_tuning() {
        let original = ArenaTuning::default();
        for (capture, view) in [
            (false, "encounter-stress"),
            (true, "first"),
            (true, "observer-performance"),
        ] {
            let mut actual = original.clone();
            assert!(
                !configure_encounter_stress_tuning(capture, view, &mut actual)
                    .expect("valid original")
            );
            assert_eq!(
                serde_json::to_value(&actual).expect("tuning"),
                serde_json::to_value(&original).expect("original")
            );
        }
        let mut actual = original.clone();
        assert!(
            configure_encounter_stress_tuning(true, "encounter-stress", &mut actual)
                .expect("validated stress tuning")
        );
        assert!(actual.validate().is_ok());
        assert_eq!(
            stress_home_leashes(&actual).map(f32::to_bits),
            [150.0_f32.to_bits(); 3]
        );
        actual.encounters.ground_leash = original.encounters.ground_leash;
        actual.encounters.shadow_leash = original.encounters.shadow_leash;
        actual.encounters.dragon_leash = original.encounters.dragon_leash;
        assert_eq!(
            serde_json::to_value(&actual).expect("restored"),
            serde_json::to_value(&original).expect("original")
        );
    }
}
