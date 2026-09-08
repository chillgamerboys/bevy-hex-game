//! One-level Wisp and frozen Ember presentation, with ordinary terrain occlusion.
use super::encounter::EncounterEffect;
use super::{ArenaCamera, ViewState};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use hex_arena::{
    Actor, ArenaSession, AttackPhase, CreatureAbility, Projectile, ProjectileAppearance, Species,
};

#[derive(Component)]
#[require(NotShadowCaster)]
pub(super) struct WispPrism;

#[derive(Resource)]
pub(super) struct WispVisualAssets {
    column: Handle<Mesh>,
    sphere: Handle<Mesh>,
    body: Handle<StandardMaterial>,
    core: Handle<StandardMaterial>,
    glow: Handle<StandardMaterial>,
}

pub(super) fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut light = |color| {
        materials.add(StandardMaterial {
            base_color: color,
            unlit: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Blend,
            ..default()
        })
    };
    commands.insert_resource(WispVisualAssets {
        column: meshes.add(super::golem::native_hex_prism()),
        sphere: meshes.add(Sphere::new(1.0).mesh().uv(16, 10)),
        body: light(Color::srgba(1.0, 0.37, 0.045, 0.48)),
        core: light(Color::srgba(1.0, 0.96, 0.62, 0.98)),
        glow: light(Color::srgba(1.0, 0.44, 0.05, 0.045)),
    });
}

pub(super) fn spawn_body(
    body: &mut ChildSpawnerCommands,
    actor: &Actor,
    assets: &WispVisualAssets,
) {
    for prism in actor.body_hex_prisms() {
        body.spawn((
            WispPrism,
            Name::new("Wisp physical prism"),
            Mesh3d(assets.column.clone()),
            MeshMaterial3d(assets.body.clone()),
            Transform::from_translation(prism.offset + Vec3::Y * prism.height * 0.5)
                .with_scale(Vec3::new(1.0, prism.height, 1.0)),
        ));
    }
}

pub(super) fn glow(
    commands: &mut Commands,
    actor: &Actor,
    assets: &WispVisualAssets,
    gizmos: &mut Gizmos,
    accent: Color,
) {
    let progress = actor
        .attack_state()
        .filter(|attack| {
            attack.kind == CreatureAbility::WispEmber && attack.phase == AttackPhase::Windup
        })
        .map_or(0.0, |attack| attack.progress.clamp(0.0, 1.0));
    for (scale, material) in [
        (Vec3::splat(0.13 + progress * 0.05), &assets.core),
        (Vec3::new(1.03, 0.24, 1.13), &assets.glow),
    ] {
        commands.spawn((
            EncounterEffect,
            Name::new("Wisp glow"),
            Mesh3d(assets.sphere.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(actor.eye()).with_scale(scale),
        ));
    }
    for prism in actor.body_hex_prisms() {
        for height in [0.0, prism.height] {
            let center = actor.feet + prism.offset + Vec3::Y * height;
            for corner in 0..6u16 {
                let a = f32::from(corner) * std::f32::consts::TAU / 6.0;
                let b = f32::from(corner + 1) * std::f32::consts::TAU / 6.0;
                gizmos.line(
                    center + Vec3::new(a.sin(), 0.0, a.cos()),
                    center + Vec3::new(b.sin(), 0.0, b.cos()),
                    accent,
                );
            }
        }
    }
}

pub(super) fn ember(commands: &mut Commands, projectile: &Projectile, assets: &WispVisualAssets) {
    let radius = projectile.collision_radius();
    for (scale, material) in [(radius, &assets.core), (radius * 1.8, &assets.glow)] {
        commands.spawn((
            super::presentation::TransientEffect,
            Name::new("Frozen Wisp Ember"),
            Mesh3d(assets.sphere.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(projectile.position).with_scale(Vec3::splat(scale)),
        ));
    }
}

pub(super) fn dim_view(view: &str) -> bool {
    matches!(view, "encounter-wisp-dark" | "encounter-wisp-dark-rear")
}

pub(super) fn configure_capture_lighting(
    state: Res<ViewState>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut lights: Query<&mut DirectionalLight>,
    mut clear: ResMut<ClearColor>,
) {
    if state.capture.is_none() || !dim_view(&state.capture_view) {
        return;
    }
    ambient.brightness = 6.0;
    for mut light in &mut lights {
        light.illuminance = 0.0;
    }
    clear.0 = Color::srgb(0.008, 0.012, 0.02);
}

pub(super) fn phase_view(view: &str) -> bool {
    matches!(
        view.strip_suffix("-rear").unwrap_or(view),
        "encounter-wisp-body"
            | "encounter-wisp-dark"
            | "encounter-wisp-windup"
            | "encounter-wisp-ember"
            | "encounter-wisp-layers"
    )
}

pub(super) fn phase_actor<'a>(session: &'a ArenaSession, view: &str) -> Option<&'a Actor> {
    let view = view.strip_suffix("-rear").unwrap_or(view);
    session.actors.iter().find(|actor| {
        if actor.species != Species::Wisp || actor.hp <= 0.0 || !actor.flying {
            return false;
        }
        match view {
            "encounter-wisp-body" | "encounter-wisp-dark" => true,
            "encounter-wisp-layers" => {
                session.actors.len() == 24
                    && session
                        .actors
                        .iter()
                        .all(|a| a.species == Species::Wisp && a.hp > 0.0 && a.flying)
            }
            "encounter-wisp-windup" => actor.attack_state().is_some_and(|attack| {
                attack.kind == CreatureAbility::WispEmber
                    && attack.phase == AttackPhase::Windup
                    && (0.3..=0.7).contains(&attack.progress)
            }),
            "encounter-wisp-ember" => session.projectiles.iter().any(|p| {
                p.owner == actor.id
                    && p.appearance() == ProjectileAppearance::Ember
                    && p.source_ability() == Some(CreatureAbility::WispEmber)
                    && p.age > 0.0
                    && p.position.distance(actor.eye()) > 1.2
            }),
            _ => false,
        }
    })
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
    if state.capture_view == "encounter-wisp-layers" {
        for other in &session.actors {
            let half = other.body_dimensions() * 0.5;
            low = low.min(other.center() - half);
            high = high.max(other.center() + half);
        }
    } else if state.capture_view.starts_with("encounter-wisp-ember") {
        if let Some(projectile) = session.projectiles.iter().find(|p| {
            p.owner == actor.id
                && p.appearance() == ProjectileAppearance::Ember
                && p.source_ability() == Some(CreatureAbility::WispEmber)
                && p.age > 0.0
                && p.position.distance(actor.eye()) > 1.2
        }) {
            low = low.min(projectile.position - Vec3::splat(0.2));
            high = high.max(projectile.position + Vec3::splat(0.2));
        }
    }
    *camera = super::encounter::frame_bounds(low, high, state.capture_view.ends_with("-rear"));
}
