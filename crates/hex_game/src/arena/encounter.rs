//! Bounded presentation and ordinary-input capture adapter for encounter snapshots.

use super::{ArenaCamera, ViewState};
use bevy::prelude::*;
use hex_arena::{ActorIntent, ArenaSession, ArenaTuning, AttackPhase, CreatureAbility, Spell};
use hex_core::arena::{ArenaTerrainView, ArenaVoxelGeometry};

pub(super) fn capture_intent(frame: u32, session: &ArenaSession, view: &str) -> ActorIntent {
    let Some(human) = session.actors.first() else {
        return ActorIntent::default();
    };
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
    let offset = enemy.feet + Vec3::Y * enemy.body_dimensions().y * 0.5 - human.eye();
    let distance = offset.with_y(0.0).length();
    let direction = offset.normalize_or(Vec3::NEG_Z);
    let close = view.contains("swipe") || view.contains("windup") || view.contains("breath");
    let preferred = if close { 1.6 } else { 4.0 };
    let movement = if distance > preferred {
        Vec2::Y
    } else {
        Vec2::ZERO
    };
    let attack = view.contains("barrier") || view.contains("aura") || view.contains("fireball");
    let cycle = frame % 150;
    ActorIntent {
        movement,
        aim: direction,
        jump: movement != Vec2::ZERO && frame.is_multiple_of(45),
        run: distance > 8.0,
        selected: Some(Spell::Fireball),
        cast_pressed: attack && cycle == 45,
        cast_held: attack && (45..76).contains(&cycle),
        cast_released: attack && cycle == 76,
        ..default()
    }
}

pub(super) fn phase_ready(session: &ArenaSession, view: &str) -> bool {
    if view == "encounter-barrier" {
        return !session.barriers().is_empty();
    }
    if view == "encounter-aura" {
        return !session.auras().is_empty();
    }
    session
        .actors
        .iter()
        .filter(|actor| actor.id != 0 && actor.hp > 0.0)
        .any(|actor| {
            if view == "encounter-fireball" {
                return actor.charge().is_some_and(|charge| {
                    charge.spell == Spell::Fireball && charge.elapsed >= 0.25
                });
            }
            actor.attack_state().is_some_and(|attack| match view {
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
        })
}

pub(super) fn phase_view(view: &str) -> bool {
    matches!(
        view,
        "encounter-windup"
            | "encounter-breath"
            | "encounter-swipe"
            | "encounter-barrier"
            | "encounter-aura"
            | "encounter-fireball"
    )
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
    } else if state.capture_view.starts_with("encounter-") {
        let actor = session
            .actors
            .iter()
            .find(|actor| actor.attack_state().is_some())
            .or_else(|| session.actors.get(1));
        if let Some(actor) = actor {
            let target = actor.feet + Vec3::Y * actor.body_dimensions().y * 0.5;
            *camera = Transform::from_translation(target + Vec3::new(5.0, 4.0, 7.0))
                .looking_at(target, Vec3::Y);
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
#[derive(Component)]
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
        aura: material(Color::srgba(0.30, 0.95, 0.51, 0.055)),
        breath: material(Color::srgba(1.0, 0.31, 0.025, 0.19)),
    });
}

pub(super) fn effects(
    mut commands: Commands,
    session: Res<ArenaSession>,
    tuning: Res<ArenaTuning>,
    assets: Res<VisualAssets>,
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
        if actor.id != 0 {
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
