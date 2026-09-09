//! Bounded Golem model and effects, projected only from gameplay-owned geometry.

use super::encounter::EncounterEffect;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use hex_arena::{Actor, AttackPhase, CreatureAbility};

#[derive(Component)]
pub(super) struct GolemPrism;

// Native circumradius-one pointy hex; same corner convention as HexCoord and
// hex_map's arena terrain renderer. This is render geometry, never a collider.
const CORNERS: [Vec3; 6] = [
    Vec3::new(0.0, 0.0, 1.0),
    Vec3::new(0.866_025_4, 0.0, 0.5),
    Vec3::new(0.866_025_4, 0.0, -0.5),
    Vec3::new(0.0, 0.0, -1.0),
    Vec3::new(-0.866_025_4, 0.0, -0.5),
    Vec3::new(-0.866_025_4, 0.0, 0.5),
];

#[derive(Resource)]
pub(super) struct GolemVisualAssets {
    column: Handle<Mesh>,
    block: Handle<Mesh>,
    sphere: Handle<Mesh>,
    beam: Handle<Mesh>,
    stone: [Handle<StandardMaterial>; 3],
    face: Handle<StandardMaterial>,
    core: Handle<StandardMaterial>,
    glow: Handle<StandardMaterial>,
    laser_core: Handle<StandardMaterial>,
}

/// Local presentation input, not a proposed second gameplay authority.
pub(super) struct MouthPose {
    pub origin: Vec3,
    pub direction: Vec3,
}

pub(super) fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let stone = [0.34, 0.39, 0.43].map(|tone| {
        materials.add(StandardMaterial {
            base_color: Color::srgb(tone * 0.88, tone * 0.94, tone),
            perceptual_roughness: 0.96,
            ..default()
        })
    });
    let face = materials.add(StandardMaterial {
        base_color: Color::srgb(0.045, 0.055, 0.07),
        perceptual_roughness: 0.9,
        ..default()
    });
    let mut light = |base_color| {
        materials.add(StandardMaterial {
            base_color,
            unlit: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Blend,
            ..default()
        })
    };
    let core = light(Color::srgba(1.0, 0.92, 0.60, 0.96));
    let glow = light(Color::srgba(1.0, 0.40, 0.06, 0.18));
    let laser_core = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.99, 0.92),
        unlit: true,
        cull_mode: None,
        ..default()
    });
    commands.insert_resource(GolemVisualAssets {
        column: meshes.add(native_hex_prism()),
        block: meshes.add(Cuboid::from_size(Vec3::ONE)),
        sphere: meshes.add(Sphere::new(1.0).mesh().uv(16, 10)),
        beam: meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(12)),
        stone,
        face,
        core,
        glow,
        laser_core,
    });
}

/// Invoke only for the explicit Species::Golem branch when its ActorModel spawns.
/// The parent uses authoritative body_rotation() (identity for Golem).
pub(super) fn spawn_body(
    body: &mut ChildSpawnerCommands,
    actor: &Actor,
    assets: &GolemVisualAssets,
) {
    for (index, prism) in actor.body_hex_prisms().enumerate() {
        body.spawn((
            GolemPrism,
            Name::new(format!("Golem physical prism {index}")),
            Mesh3d(assets.column.clone()),
            MeshMaterial3d(
                assets
                    .stone
                    .get(index % assets.stone.len())
                    .cloned()
                    .unwrap_or_default(),
            ),
            Transform::from_translation(prism.offset + Vec3::Y * prism.height * 0.5)
                .with_scale(Vec3::new(1.0, prism.height, 1.0)),
        ));
    }
}

/// Called from encounter::effects only for a living Golem. It shares the existing
/// EncounterEffect cleanup/NotShadowCaster invariant and cached visual assets.
/// The caller supplies the gameplay-owned idle mouth pose.
pub(super) fn face_and_laser(
    commands: &mut Commands,
    actor: &Actor,
    idle_mouth: MouthPose,
    assets: &GolemVisualAssets,
    gizmos: &mut Gizmos,
    accent: Color,
) {
    let beam = actor.beam();
    // The cast snapshot supplies the authoritative moving direction and mouth under knockback.
    let (mouth, direction) = beam
        .as_ref()
        .map_or((idle_mouth.origin, idle_mouth.direction), |beam| {
            (beam.origin, beam.direction)
        });
    let direction = direction.normalize_or(Vec3::NEG_Z);
    let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, direction);
    let attack = actor
        .attack_state()
        .filter(|attack| attack.kind == CreatureAbility::GolemLaser);
    let progress = attack.map_or(0.0, |attack| match attack.phase {
        AttackPhase::Windup => attack.progress.clamp(0.0, 1.0),
        AttackPhase::Active => 1.0,
        AttackPhase::Recovery => (1.0 - attack.progress).clamp(0.0, 1.0),
    });

    // A wide plate at the physical mouth is buried in the seven-hex notches.
    // Mount only decorative eyes beyond the body's front support plane, leaving
    // the center open. The charge core and beam retain their physical mouth.
    let face = decorative_mount(actor, mouth, direction);
    for side in [-1.0, 1.0] {
        commands.spawn((
            EncounterEffect,
            Name::new("Golem independently aimed eye surround"),
            Mesh3d(assets.block.clone()),
            MeshMaterial3d(assets.face.clone()),
            Transform::from_translation(face + rotation * Vec3::new(side * 0.40, 0.09, 0.0))
                .with_rotation(rotation)
                .with_scale(Vec3::new(0.24, 0.19, 0.06)),
        ));
        commands.spawn((
            EncounterEffect,
            Mesh3d(assets.block.clone()),
            MeshMaterial3d(assets.core.clone()),
            Transform::from_translation(face + rotation * Vec3::new(side * 0.40, 0.09, -0.045))
                .with_rotation(rotation)
                .with_scale(Vec3::new(0.15, 0.075, 0.025)),
        ));
    }
    let core_center = mouth + direction * 0.10 - Vec3::Y * 0.06;
    commands.spawn((
        EncounterEffect,
        Name::new("Golem growing charge core"),
        Mesh3d(assets.sphere.clone()),
        MeshMaterial3d(assets.core.clone()),
        Transform::from_translation(core_center).with_scale(Vec3::splat(0.055 + progress * 0.16)),
    ));
    if progress > 0.0 {
        commands.spawn((
            EncounterEffect,
            Mesh3d(assets.sphere.clone()),
            MeshMaterial3d(assets.glow.clone()),
            Transform::from_translation(core_center)
                .with_scale(Vec3::splat(0.09 + progress * 0.28)),
        ));
    }
    // Team affiliation is an accent; the stone mass stays recognizably stone.
    gizmos.line(
        face + rotation * Vec3::new(-0.54, 0.22, -0.025),
        face + rotation * Vec3::new(0.54, 0.22, -0.025),
        accent,
    );
    body_seams(actor, gizmos, accent);

    if let Some(slam) = actor.attack_state().filter(|attack| {
        attack.kind == CreatureAbility::GolemSlam && attack.phase == AttackPhase::Windup
    }) {
        // Show the complete spherical damage extent throughout its warning.
        // Gameplay emits the ordinary session effect on the actual damaging
        // pulse; the existing explosion renderer owns its Active shockwave.
        gizmos.sphere(
            Isometry3d::from_translation(slam.origin),
            slam.range,
            Color::srgba(1.0, 0.69, 0.25, 0.35 + slam.progress.clamp(0.0, 1.0) * 0.45),
        );
    }

    if let Some(swipe) = actor.attack_state().filter(|attack| {
        attack.kind == CreatureAbility::GolemSwipe && attack.phase != AttackPhase::Recovery
    }) {
        let forward = swipe.direction.with_y(0.0).normalize_or(Vec3::NEG_Z);
        let side = Vec3::Y.cross(forward);
        let sweep = (swipe.progress.clamp(0.0, 1.0) - 0.5) * 1.4;
        let tip = swipe.origin + forward * (0.4 + swipe.range * 0.65) + side * sweep;
        let color = if swipe.phase == AttackPhase::Active {
            Color::srgb(1.0, 0.95, 0.72)
        } else {
            Color::srgb(1.0, 0.55, 0.12)
        };
        for offset in [-0.3, 0.0, 0.3] {
            gizmos.line(tip + side * (offset - 0.3) + Vec3::Y * 0.3,
                tip + side * (offset + 0.3) - Vec3::Y * 0.3, color);
        }
    }

    let Some((beam, attack)) = beam.zip(attack) else {
        return;
    };
    match attack.phase {
        AttackPhase::Windup => {
            // No expanding cone or fake range: this is the exact current ray.
            let color = if attack.progress >= 0.75 {
                Color::srgb(1.0, 0.95, 0.72)
            } else {
                Color::srgb(1.0, 0.55, 0.12)
            };
            gizmos.line(beam.origin, beam.end, color);
            if attack.progress >= 0.75 {
                gizmos.sphere(Isometry3d::from_translation(beam.origin), 0.24, color);
            }
        }
        AttackPhase::Active => {
            laser_segments(commands, beam, assets);
        }
        AttackPhase::Recovery => {}
    }
}

/// Presentation-only support of the published vertices; never a mouth, collider,
/// attack ray or body pose. Eye plaques remain in front of every stone prism.
pub(super) fn decorative_mount(actor: &Actor, mouth: Vec3, direction: Vec3) -> Vec3 {
    let advance = actor
        .body_hex_prisms()
        .flat_map(|prism| {
            CORNERS.into_iter().flat_map(move |corner| {
                let base = actor.feet + prism.offset + corner;
                [base, base + Vec3::Y * prism.height]
            })
        })
        .map(|point| (point - mouth).dot(direction))
        .fold(0.0_f32, f32::max);
    mouth + direction * (advance + 0.07)
}

fn laser_segments(
    commands: &mut Commands,
    beam: hex_arena::BeamSnapshot,
    assets: &GolemVisualAssets,
) {
    // The outer mesh has exactly the published damaging radius. Its brighter
    // inner core improves contrast without inventing a wider attack.
    for (radius, material) in [
        (beam.radius, &assets.glow),
        (beam.radius * 0.72, &assets.laser_core),
    ] {
        if let Some(transform) = segment_pose(beam.origin, beam.end, radius) {
            commands.spawn((
                EncounterEffect,
                Name::new("Golem authoritative active laser segment"),
                Mesh3d(assets.beam.clone()),
                MeshMaterial3d(material.clone()),
                transform,
            ));
        }
    }
}

fn body_seams(actor: &Actor, gizmos: &mut Gizmos, accent: Color) {
    for (index, prism) in actor.body_hex_prisms().enumerate() {
        // Five stone courses; these lines never resize the physical columns.
        for level in 1..=5u16 {
            let center =
                actor.feet + prism.offset + Vec3::Y * prism.height * f32::from(level) / 5.0;
            let color = Color::srgba(0.075, 0.09, 0.11, 0.5);
            for (a, b) in CORNERS.into_iter().zip(CORNERS.into_iter().cycle().skip(1)) {
                gizmos.line(center + a, center + b, color);
            }
        }
        if index == 0 {
            let center = actor.feet + prism.offset + Vec3::Y * (prism.height + 0.005);
            for (a, b) in CORNERS.into_iter().zip(CORNERS.into_iter().cycle().skip(1)) {
                gizmos.line(center + a * 0.6, center + b * 0.6, accent);
            }
        }
    }
}

fn segment_pose(origin: Vec3, end: Vec3, radius: f32) -> Option<Transform> {
    let delta = end - origin;
    let length = delta.length();
    if !length.is_finite() || length <= 0.0001 || !radius.is_finite() || radius <= 0.0 {
        return None;
    }
    Some(
        Transform::from_translation((origin + end) * 0.5)
            .with_rotation(Quat::from_rotation_arc(Vec3::Y, delta / length))
            .with_scale(Vec3::new(radius, length, radius)),
    )
}

pub(super) fn native_hex_prism() -> Mesh {
    let mut positions = Vec::<[f32; 3]>::with_capacity(72);
    let mut normals = Vec::<[f32; 3]>::with_capacity(72);
    let mut triangle = |a: Vec3, b: Vec3, c: Vec3| {
        let normal = (b - a).cross(c - a).normalize();
        positions.extend([a.to_array(), b.to_array(), c.to_array()]);
        normals.extend([normal.to_array(); 3]);
    };
    let up = Vec3::Y * 0.5;
    for (a, b) in CORNERS.into_iter().zip(CORNERS.into_iter().cycle().skip(1)) {
        triangle(a - up, b - up, b + up);
        triangle(a - up, b + up, a + up);
        triangle(up, a + up, b + up);
        triangle(-up, b - up, a - up);
    }
    let uvs = vec![[0.0; 2]; positions.len()];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
}

pub(super) fn phase_view(view: &str) -> bool {
    matches!(
        view.strip_suffix("-rear").unwrap_or(view),
        "encounter-golem-charge"
            | "encounter-golem-charge-late"
            | "encounter-golem-beam"
            | "encounter-golem-slam-windup"
            | "encounter-golem-slam"
    )
}

fn phase_matches(
    view: &str,
    attack: Option<hex_arena::AttackSnapshot>,
    beam: Option<hex_arena::BeamSnapshot>,
) -> bool {
    let Some(attack) = attack else {
        return false;
    };
    match view.strip_suffix("-rear").unwrap_or(view) {
        "encounter-golem-charge" => {
            attack.kind == CreatureAbility::GolemLaser
                && attack.phase == AttackPhase::Windup
                && (0.25..=0.55).contains(&attack.progress)
                && beam.is_some()
        }
        "encounter-golem-charge-late" => {
            attack.kind == CreatureAbility::GolemLaser
                && attack.phase == AttackPhase::Windup
                && attack.progress >= 0.75
                && beam.is_some()
        }
        "encounter-golem-beam" => {
            attack.kind == CreatureAbility::GolemLaser
                && attack.phase == AttackPhase::Active
                && beam.is_some()
        }
        "encounter-golem-slam-windup" => {
            attack.kind == CreatureAbility::GolemSlam
                && attack.phase == AttackPhase::Windup
                && attack.progress >= 0.25
        }
        "encounter-golem-slam" => {
            attack.kind == CreatureAbility::GolemSlam && attack.phase == AttackPhase::Active
        }
        _ => false,
    }
}

pub(super) fn phase_actor<'a>(
    session: &'a hex_arena::ArenaSession,
    view: &str,
) -> Option<&'a Actor> {
    session.actors.iter().find(|actor| {
        actor.species == hex_arena::Species::Golem
            && actor.hp > 0.0
            && phase_matches(view, actor.attack_state(), actor.beam())
            && (view != "encounter-golem-slam"
                || actor.attack_state().is_some_and(|attack| {
                    // Age>0 means the next ArenaTick has published the pulse's queued terrain edits.
                    session.effects.iter().any(|effect| {
                        effect.kind == hex_arena::Spell::AreaBlast
                            && effect.age > 0.0
                            && effect.age <= 0.10
                            && effect.center.distance(attack.origin) < 0.5
                            && (effect.radius - attack.range).abs() < 0.01
                    })
                }))
    })
}

/// Explicit static-review camera only. Run after the ordinary observer camera.
/// The human and observer camera paths retain their live controls unchanged.
pub(super) fn capture_camera(
    session: Res<hex_arena::ArenaSession>,
    state: Res<super::ViewState>,
    mut cameras: Query<&mut Transform, With<super::ArenaCamera>>,
) {
    if state.capture.is_none() {
        return;
    }
    let body_view = state.capture_view.starts_with("encounter-body");
    let actor = if body_view {
        session
            .actors
            .iter()
            .find(|actor| actor.species == hex_arena::Species::Golem && actor.hp > 0.0)
    } else {
        phase_actor(&session, &state.capture_view)
    };
    let Some(actor) = actor else {
        return;
    };
    let Ok(mut camera) = cameras.single_mut() else {
        return;
    };
    let half = actor.body_dimensions() * 0.5;
    let mut low = actor.center() - half;
    let mut high = actor.center() + half;
    if let Some(beam) = actor.beam().filter(|_| phase_view(&state.capture_view)) {
        // Review the actual finite beam segment; never extend or re-query it.
        low = low.min(beam.origin).min(beam.end);
        high = high.max(beam.origin).max(beam.end);
    }
    if state.capture_view.contains("slam") {
        if let Some(attack) = actor.attack_state() {
            low = low.min(attack.origin - Vec3::splat(attack.range));
            high = high.max(attack.origin + Vec3::splat(attack.range));
        }
    }
    *camera = super::encounter::frame_bounds(low, high, state.capture_view.ends_with("-rear"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::light::NotShadowCaster;
    use hex_arena::{AttackSnapshot, BeamSnapshot};

    #[test]
    fn laser_mesh_endpoints_radius_and_shadow_policy_follow_the_snapshot() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>();
        app.world_mut()
            .run_system_once(setup)
            .expect("cached visual assets");
        let beam = BeamSnapshot {
            origin: Vec3::new(1.0, 2.0, 3.0),
            direction: Vec3::new(3.0, 0.0, 4.0).normalize(),
            end: Vec3::new(4.0, 2.0, 7.0),
            radius: 0.12,
            tracking: true,
        };
        app.world_mut()
            .run_system_once(
                move |mut commands: Commands, assets: Res<GolemVisualAssets>| {
                    laser_segments(&mut commands, beam, &assets);
                },
            )
            .expect("beam projection");
        let segments = app
            .world_mut()
            .query_filtered::<(
                &Transform,
                &MeshMaterial3d<StandardMaterial>,
                Has<NotShadowCaster>,
            ), With<EncounterEffect>>()
            .iter(app.world())
            .map(|(pose, material, no_shadow)| (*pose, material.0.clone(), no_shadow))
            .collect::<Vec<_>>();
        assert_eq!(segments.len(), 2);
        let materials = app.world().resource::<Assets<StandardMaterial>>();
        let core = &app.world().resource::<GolemVisualAssets>().laser_core;
        assert!(segments.iter().any(|(pose, material, _)| {
            material == core && (pose.scale.x - beam.radius * 0.72).abs() < 0.0001
        }));
        for (pose, material, no_shadow) in segments {
            assert!(no_shadow);
            assert!(pose
                .transform_point(Vec3::NEG_Y * 0.5)
                .abs_diff_eq(beam.origin, 0.0001));
            assert!(pose
                .transform_point(Vec3::Y * 0.5)
                .abs_diff_eq(beam.end, 0.0001));
            assert!(pose.scale.x <= beam.radius && pose.scale.z <= beam.radius);
            let is_core = &material == core;
            let material = materials.get(&material).expect("cached beam material");
            assert!(material.unlit && material.cull_mode.is_none());
            if is_core {
                assert_eq!(material.alpha_mode, AlphaMode::Opaque);
                let color = material.base_color.to_srgba();
                assert!(color.red >= 0.99 && color.green >= 0.98 && color.blue >= 0.9);
            } else {
                assert_eq!(material.alpha_mode, AlphaMode::Blend);
            }
        }
        assert!(segment_pose(beam.origin, beam.origin, beam.radius).is_none());
        assert!(segment_pose(beam.origin, beam.end, f32::NAN).is_none());
    }

    #[test]
    fn phase_guards_reject_unrelated_attacks_and_distinguish_charge_late_charge_and_pulse() {
        let mut attack = AttackSnapshot {
            kind: CreatureAbility::GolemLaser,
            phase: AttackPhase::Windup,
            origin: Vec3::ZERO,
            direction: Vec3::Z,
            range: 20.0,
            half_angle: 0.0,
            progress: 0.4,
        };
        let mut beam = BeamSnapshot {
            origin: Vec3::ZERO,
            direction: Vec3::Z,
            end: Vec3::Z * 20.0,
            radius: 0.12,
            tracking: true,
        };
        assert!(phase_matches(
            "encounter-golem-charge",
            Some(attack),
            Some(beam)
        ));
        assert!(!phase_matches(
            "encounter-golem-charge-late",
            Some(attack),
            Some(beam)
        ));
        beam.tracking = false;
        attack.progress = 0.9;
        assert!(phase_matches(
            "encounter-golem-charge-late",
            Some(attack),
            Some(beam)
        ));
        assert!(!phase_matches(
            "encounter-golem-charge",
            Some(attack),
            Some(beam)
        ));
        attack.phase = AttackPhase::Active;
        for view in ["encounter-golem-beam", "encounter-golem-beam-rear"] {
            assert!(phase_view(view));
            assert!(phase_matches(view, Some(attack), Some(beam)));
        }
        attack.kind = CreatureAbility::Swipe;
        assert!(!phase_matches(
            "encounter-golem-beam",
            Some(attack),
            Some(beam)
        ));
        attack.kind = CreatureAbility::GolemSlam;
        assert!(phase_matches("encounter-golem-slam", Some(attack), None));
        attack.phase = AttackPhase::Recovery;
        assert!(!phase_matches("encounter-golem-slam", Some(attack), None));
    }
}
