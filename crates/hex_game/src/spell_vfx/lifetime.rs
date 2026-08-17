//! Cleanup and hand-off for spawned spell VFX entities.

use bevy::prelude::*;
use bevy_hanabi::prelude::*;

use hex_anim::Transformation;
use hex_assets::{MotionArchetype, SpellAnimationFile};

use super::assets::{glow_texture_handle, SpellVfxAssetCache, VfxPhase};
use super::trigger::{spawn_style_layers, SpellVfxLifetime, SpellVfxProjectile};

/// Despawns a spell VFX entity once its authored duration elapses.
///
/// `hex_anim` only ever removes a finished [`Transformation`] component, never the
/// entity it's attached to, so every spawned VFX entity needs this to actually leave
/// the world.
pub(super) fn tick_spell_vfx_lifetime(
    mut commands: Commands,
    time: Res<Time>,
    mut lifetimes: Query<(Entity, &mut SpellVfxLifetime)>,
) {
    for (entity, mut lifetime) in &mut lifetimes {
        lifetime.0.tick(time.delta());
        if lifetime.0.is_finished() {
            commands.entity(entity).despawn();
        }
    }
}

/// Spawns a `Projectile` archetype's impact burst the exact frame `hex_anim` removes
/// the travelling entity's [`Transformation`] — the documented way to detect "this
/// animation just finished" (see `hex_anim::AnimationSystems::Drive`'s own doc
/// comment on observing a finished animation's deferred removal).
pub(super) fn spawn_impact_burst_on_arrival(
    mut commands: Commands,
    mut removed: RemovedComponents<Transformation>,
    projectiles: Query<(&Transform, &SpellVfxProjectile)>,
    animations: Option<Res<SpellAnimationFile>>,
    mut cache: ResMut<SpellVfxAssetCache>,
    mut effects: ResMut<Assets<EffectAsset>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(animations) = animations else {
        return;
    };
    for entity in removed.read() {
        let Ok((transform, projectile)) = projectiles.get(entity) else {
            continue;
        };
        let Some(animation) = animations.animations.get(&projectile.spell) else {
            continue;
        };
        let MotionArchetype::Projectile {
            impact_hold_seconds,
            ..
        } = animation.motion
        else {
            continue;
        };
        let glow = glow_texture_handle(&mut cache, &mut images);
        spawn_style_layers(
            &mut commands,
            &projectile.spell,
            VfxPhase::Impact,
            animation,
            projectile.color,
            Transform::from_translation(transform.translation),
            impact_hold_seconds,
            &glow,
            &mut cache,
            &mut effects,
        );
    }
}
