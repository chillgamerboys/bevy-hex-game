//! Dynamic native Worm segments and frozen Boulder visuals, without terrain authority.

use super::encounter::EncounterEffect;
use super::{ArenaCamera, ViewState};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use hex_arena::{
    Actor, ArenaSession, AttackPhase, CreatureAbility, Projectile, ProjectileAppearance, Species,
    WormPhase,
};

/// Physical model mesh; counted separately from decorative pieces in render receipts.
#[derive(Component)]
pub(super) struct WormPrism;

#[derive(Component)]
pub(super) struct WormWindup;

#[derive(Component)]
pub(super) struct BoulderVisual;

#[derive(Clone, Copy)]
enum PartKind {
    Segment(usize),
    Face,
}

// Dynamic parts own visibility so updates do not depend on a renderer plugin
// adding it indirectly to Mesh3d (minimal application fixtures omit that plugin).
#[derive(Component)]
#[require(Visibility)]
pub(super) struct WormPart {
    actor: u8,
    kind: PartKind,
}

impl WormPart {
    pub(super) fn actor_id(&self) -> u8 {
        self.actor
    }

    pub(super) fn segment_index(&self) -> Option<usize> {
        match self.kind {
            PartKind::Segment(index) => Some(index),
            PartKind::Face => None,
        }
    }
}

#[derive(Resource)]
pub(super) struct WormVisualAssets {
    column: Handle<Mesh>,
    block: Handle<Mesh>,
    boulder: Handle<Mesh>,
    direction: Handle<Mesh>,
    body: [Handle<StandardMaterial>; 3],
    head: Handle<StandardMaterial>,
    head_warning: Handle<StandardMaterial>,
    mouth: Handle<StandardMaterial>,
    eye: Handle<StandardMaterial>,
    stone: Handle<StandardMaterial>,
    warning: Handle<StandardMaterial>,
}

pub(super) fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut matte = |color| {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: 0.94,
            ..default()
        })
    };
    let body = [
        Color::srgb(0.43, 0.22, 0.16),
        Color::srgb(0.52, 0.29, 0.20),
        Color::srgb(0.60, 0.36, 0.24),
    ]
    .map(&mut matte);
    let head = matte(Color::srgb(0.69, 0.43, 0.27));
    let mouth = matte(Color::srgb(0.065, 0.040, 0.034));
    let eye = matte(Color::srgb(0.96, 0.84, 0.52));
    let stone = matte(Color::srgb(0.54, 0.51, 0.47));
    let head_warning = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.86, 0.39),
        unlit: true,
        ..default()
    });
    let warning = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.77, 0.38),
        unlit: true,
        ..default()
    });
    commands.insert_resource(WormVisualAssets {
        column: meshes.add(super::golem::native_hex_prism()),
        block: meshes.add(Cuboid::from_size(Vec3::ONE)),
        boulder: meshes.add(Sphere::new(1.0).mesh().uv(10, 6)),
        direction: meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(8)),
        body,
        head,
        head_warning,
        mouth,
        eye,
        stone,
        warning,
    });
}

/// The actor parent uses feet and the authoritative identity body rotation.
/// Accent is the caller's ordinary player/species or spectator-team material.
pub(super) fn spawn_body(
    body: &mut ChildSpawnerCommands,
    actor: &Actor,
    assets: &WormVisualAssets,
    accent: Handle<StandardMaterial>,
) {
    for (index, _) in actor.body_hex_prisms().enumerate() {
        let Some(transform) = part_transform(actor, PartKind::Segment(index)) else {
            continue;
        };
        let material = if index == 0 {
            head_material(actor, assets)
        } else {
            assets
                .body
                .get(index % assets.body.len())
                .cloned()
                .unwrap_or_default()
        };
        body.spawn((
            WormPrism,
            WormPart {
                actor: actor.id,
                kind: PartKind::Segment(index),
            },
            Name::new(format!("Worm physical segment {index}")),
            Mesh3d(assets.column.clone()),
            MeshMaterial3d(material),
            transform,
        ))
        .with_children(|segment| {
            // A small surface stripe carries affiliation; no distant outline or marker.
            segment.spawn((
                NotShadowCaster,
                Mesh3d(assets.block.clone()),
                MeshMaterial3d(accent.clone()),
                Transform::from_xyz(0.0, 0.525, 0.0).with_scale(Vec3::new(0.64, 0.045, 0.17)),
            ));
        });
    }
    let Some(transform) = part_transform(actor, PartKind::Face) else {
        return;
    };
    body.spawn((
        WormPart {
            actor: actor.id,
            kind: PartKind::Face,
        },
        Name::new("Worm head decoration"),
        transform,
        Visibility::Inherited,
    ))
    .with_children(|face| {
        face.spawn((
            NotShadowCaster,
            Mesh3d(assets.block.clone()),
            MeshMaterial3d(assets.mouth.clone()),
            Transform::from_xyz(0.0, -0.055, 0.0).with_scale(Vec3::new(0.28, 0.07, 0.035)),
        ));
        for side in [-1.0, 1.0] {
            face.spawn((
                NotShadowCaster,
                Mesh3d(assets.block.clone()),
                MeshMaterial3d(assets.eye.clone()),
                Transform::from_xyz(side * 0.22, 0.055, 0.0)
                    .with_scale(Vec3::new(0.12, 0.09, 0.035)),
            ));
            face.spawn((
                NotShadowCaster,
                Mesh3d(assets.block.clone()),
                MeshMaterial3d(assets.mouth.clone()),
                Transform::from_xyz(side * 0.22, 0.055, -0.025)
                    .with_scale(Vec3::new(0.044, 0.057, 0.018)),
            ));
        }
    });
}

fn part_transform(actor: &Actor, kind: PartKind) -> Option<Transform> {
    if actor.species != Species::Worm {
        return None;
    }
    match kind {
        PartKind::Segment(index) => actor.body_hex_prisms().nth(index).map(|part| {
            Transform::from_translation(part.offset + Vec3::Y * (part.height * 0.5))
                .with_scale(Vec3::new(1.0, part.height, 1.0))
        }),
        PartKind::Face => {
            let head = actor.body_hex_prisms().next()?;
            let direction = Vec3::new(actor.aim.x, 0.0, actor.aim.z).normalize_or(Vec3::NEG_Z);
            // Decorative support plane of this head's six published native corners.
            // It does not move the actual eye, body or projectile launch origin.
            let support = (0..6u16)
                .map(|corner| {
                    let angle = f32::from(corner) * std::f32::consts::TAU / 6.0;
                    Vec3::new(angle.sin(), 0.0, angle.cos()).dot(direction)
                })
                .fold(0.0, f32::max);
            Some(
                Transform::from_translation(
                    head.offset + Vec3::Y * (head.height * 0.5) + direction * (support + 0.035),
                )
                .with_rotation(Quat::from_rotation_arc(Vec3::NEG_Z, direction)),
            )
        }
    }
}

/// Update bounded child transforms, including the head's actual emergence.
/// Earth hiding is ordinary depth/opacity; phase never controls body visibility.
pub(super) fn update_parts(
    session: Res<ArenaSession>,
    assets: Res<WormVisualAssets>,
    mut parts: Query<(
        &WormPart,
        &mut Transform,
        &mut Visibility,
        Option<&mut MeshMaterial3d<StandardMaterial>>,
    )>,
) {
    for (part, mut transform, mut visibility, material) in &mut parts {
        let actor = session.actors.iter().find(|actor| actor.id == part.actor);
        let current = actor.and_then(|actor| part_transform(actor, part.kind));
        if let Some(current) = current {
            *transform = current;
            *visibility = Visibility::Inherited;
            if matches!(part.kind, PartKind::Segment(0)) {
                if let (Some(actor), Some(mut material)) = (actor, material) {
                    material.0 = head_material(actor, &assets);
                }
            }
        } else {
            // Only stale/missing actor parts are hidden while their parent is removed.
            *visibility = Visibility::Hidden;
        }
    }
}

fn head_material(actor: &Actor, assets: &WormVisualAssets) -> Handle<StandardMaterial> {
    if actor.hp > 0.0
        && actor.attack_state().is_some_and(|attack| {
            attack.kind == CreatureAbility::WormBoulder && attack.phase == AttackPhase::Windup
        })
    {
        assets.head_warning.clone()
    } else {
        assets.head.clone()
    }
}

/// A short direction cue, not a straight prediction or a spherical damage extent.
/// Only the real admitted Windup creates it; opaque meshes retain ordinary depth tests.
pub(super) fn windup(commands: &mut Commands, actor: &Actor, assets: &WormVisualAssets) {
    let Some(attack) = actor.attack_state().filter(|attack| {
        attack.kind == CreatureAbility::WormBoulder && attack.phase == AttackPhase::Windup
    }) else {
        return;
    };
    let direction = attack.direction.normalize_or(Vec3::NEG_Z);
    let length = attack.range.clamp(0.0, 2.5);
    let progress = attack.progress.clamp(0.0, 1.0);
    commands.spawn((
        EncounterEffect,
        WormWindup,
        Name::new("Worm admitted Boulder direction cue"),
        Mesh3d(assets.direction.clone()),
        MeshMaterial3d(assets.warning.clone()),
        Transform::from_translation(attack.origin + direction * (length * 0.5))
            .with_rotation(Quat::from_rotation_arc(Vec3::Y, direction))
            .with_scale(Vec3::new(
                0.018 + progress * 0.01,
                length,
                0.018 + progress * 0.01,
            )),
    ));
}

/// Appearance, radius and pose belong to the released projectile, even after owner death.
pub(super) fn boulder(commands: &mut Commands, projectile: &Projectile, assets: &WormVisualAssets) {
    commands.spawn((
        super::presentation::TransientEffect,
        BoulderVisual,
        Name::new("Frozen Worm Boulder"),
        Mesh3d(assets.boulder.clone()),
        MeshMaterial3d(assets.stone.clone()),
        Transform::from_translation(projectile.position)
            .with_rotation(Quat::from_euler(
                EulerRot::XYZ,
                projectile.age * 2.0,
                projectile.age * 1.3,
                0.0,
            ))
            .with_scale(Vec3::splat(projectile.collision_radius())),
    ));
}

pub(super) fn phase_view(view: &str) -> bool {
    matches!(
        view.strip_suffix("-rear").unwrap_or(view),
        "encounter-worm-body"
            | "encounter-worm-buried"
            | "encounter-worm-emerging"
            | "encounter-worm-windup"
            | "encounter-worm-boulder"
    )
}

/// Conservative composition criterion using published complete-body bounds.
/// This is intentionally not another projectile collision query.
fn released_boulder<'a>(session: &'a ArenaSession, actor: &Actor) -> Option<&'a Projectile> {
    let half = actor.body_dimensions() * 0.5;
    session.projectiles.iter().find(|projectile| {
        if projectile.owner != actor.id
            || projectile.appearance() != ProjectileAppearance::Boulder
            || projectile.source_ability() != Some(CreatureAbility::WormBoulder)
            || projectile.age <= 0.0
        {
            return false;
        }
        let from_center = (projectile.position - actor.center()).abs();
        from_center
            .cmpgt(half + Vec3::splat(projectile.collision_radius()))
            .any()
    })
}

pub(super) fn phase_actor<'a>(session: &'a ArenaSession, view: &str) -> Option<&'a Actor> {
    let view = view.strip_suffix("-rear").unwrap_or(view);
    session.actors.iter().find(|actor| {
        if actor.species != Species::Worm {
            return false;
        }
        if view == "encounter-worm-boulder" {
            return released_boulder(session, actor).is_some();
        }
        if actor.hp <= 0.0 {
            return false;
        }
        let Some(worm) = actor.worm() else {
            return false;
        };
        match view {
            "encounter-worm-body" => matches!(actor.body_hex_prisms().count(), 4 | 6),
            "encounter-worm-buried" => worm.phase == WormPhase::Travel && !worm.exposed,
            "encounter-worm-emerging" => {
                let mut parts = actor.body_hex_prisms();
                let head_y = parts.next().map_or(0.0, |part| part.offset.y);
                let tail_y = parts.map(|part| part.offset.y).fold(head_y, f32::min);
                worm.phase == WormPhase::Emerging
                    && (0.1..0.35).contains(&worm.head_clearance)
                    && head_y - tail_y > 0.5
            }
            "encounter-worm-windup" => {
                worm.exposed
                    && actor.attack_state().is_some_and(|attack| {
                        attack.kind == CreatureAbility::WormBoulder
                            && attack.phase == AttackPhase::Windup
                            && (0.3..=0.7).contains(&attack.progress)
                    })
            }
            _ => false,
        }
    })
}

#[derive(serde::Serialize)]
pub(super) struct HeadEarth {
    position: hex_core::TilePos,
    substance: hex_core::SubstanceId,
}

/// Capture evidence uses published material at the actual head center. Clearance
/// is deliberately zero, not negative, when gameplay finds the head in earth.
pub(super) fn head_earth(
    actor: &Actor,
    view: &hex_core::arena::ArenaTerrainView,
    geometry: hex_core::arena::ArenaVoxelGeometry,
) -> Option<HeadEarth> {
    if actor.species != Species::Worm {
        return None;
    }
    let position = geometry.voxel_at(actor.eye())?;
    let substance = *view.voxels.get(&position)?;
    (!substance.is_air()).then_some(HeadEarth {
        position,
        substance,
    })
}

pub(super) fn terrain_phase_ready(
    session: &ArenaSession,
    name: &str,
    view: &hex_core::arena::ArenaTerrainView,
    geometry: hex_core::arena::ArenaVoxelGeometry,
) -> bool {
    name.strip_suffix("-rear").unwrap_or(name) != "encounter-worm-buried"
        || phase_actor(session, name)
            .is_some_and(|actor| head_earth(actor, view, geometry).is_some())
}

pub(super) fn capture_camera(
    session: Res<ArenaSession>,
    state: Res<ViewState>,
    mut cameras: Query<&mut Transform, With<ArenaCamera>>,
) {
    if state.capture.is_none() {
        return;
    }
    let Some(actor) = phase_actor(&session, &state.capture_view) else {
        return;
    };
    let Ok(mut camera) = cameras.single_mut() else {
        return;
    };
    let half = actor.body_dimensions() * 0.5;
    let mut low = actor.center() - half;
    let mut high = actor.center() + half;
    if state.capture_view.starts_with("encounter-worm-boulder") {
        if let Some(projectile) = released_boulder(&session, actor) {
            low = low.min(projectile.position - Vec3::splat(projectile.collision_radius()));
            high = high.max(projectile.position + Vec3::splat(projectile.collision_radius()));
        }
    }
    *camera = super::encounter::frame_bounds(low, high, state.capture_view.ends_with("-rear"));
}
