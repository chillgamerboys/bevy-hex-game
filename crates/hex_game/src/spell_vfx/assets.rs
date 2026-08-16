//! Builds and caches the render assets each spell's VFX plays: `bevy_hanabi`
//! particle effects for every style, a shared soft-glow particle texture, and a
//! shared unit beam mesh for `Beam` motion.
//!
//! An asset is built once per cache key and reused on every later cast. Unlike the
//! mesh/texture/material caches, [`SpellVfxAssetCache::handles`] is cleared whenever
//! `SpellAnimationFile` changes (see `super::trigger::clear_stale_effect_cache`), so
//! a tuning edit — live in the dev world inspector, or a hand edit picked up by
//! `spell_animations.ron`'s hot reload — takes effect on the very next cast rather
//! than requiring a restart.

use bevy::asset::RenderAssetUsages;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_hanabi::prelude::*;
use hex_assets::{MotionArchetype, SpellAnimation, SpriteSheet, VfxStyle};

/// Which half of a cast a built effect renders.
///
/// `InstantFlash` only ever needs [`Self::Impact`] — it has no travel leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum VfxPhase {
    /// The emitter travelling from caster to target (`Projectile` only).
    Travel,
    /// A one-shot burst: `Projectile`'s or `Beam`'s arrival, or the whole of
    /// `InstantFlash`.
    Impact,
}

/// Which particle population within one phase this asset renders.
///
/// Only [`VfxStyle::Flame`] uses [`Self::Smoke`] — every other style is a single
/// population.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum VfxLayer {
    /// The style's main particles (fire, sparks, ...).
    Primary,
    /// `Flame`'s secondary rising smoke.
    Smoke,
}

/// Built effect handles, cached by `(spell name, phase, layer)` so repeated casts of
/// the same spell reuse one GPU-side effect instead of rebuilding it every time.
#[derive(Resource, Default)]
pub(crate) struct SpellVfxAssetCache {
    handles: HashMap<(String, VfxPhase, VfxLayer), Handle<EffectAsset>>,
    /// A single shared unit cuboid, scaled per cast to a `Beam`'s exact length and
    /// thickness — one mesh asset serves every beam-motion spell. Never invalidated:
    /// geometry, not authored tuning.
    beam_mesh: Option<Handle<Mesh>>,
    /// Per-spell emissive materials for the shared beam mesh.
    beam_materials: HashMap<String, Handle<StandardMaterial>>,
    /// A single shared soft radial-falloff texture every particle style samples for
    /// its shape. Never invalidated: it has no tunable parameters.
    glow_texture: Option<Handle<Image>>,
}

impl SpellVfxAssetCache {
    /// Drops every cached particle-effect handle and beam material, so the next cast
    /// of each spell rebuilds from the current `SpellAnimationFile` instead of
    /// replaying stale tuning. Called whenever that resource changes.
    ///
    /// The beam mesh and glow texture are deliberately untouched — neither has any
    /// field a designer can tune, so there is nothing in them to go stale.
    pub(crate) fn clear_authored_content(&mut self) {
        self.handles.clear();
        self.beam_materials.clear();
    }
}

/// Returns the cached handle for `spell`'s `(phase, layer)`, building and caching it
/// on first use.
pub(super) fn effect_handle_for(
    spell: &str,
    phase: VfxPhase,
    layer: VfxLayer,
    animation: &SpellAnimation,
    color: Color,
    cache: &mut SpellVfxAssetCache,
    effects: &mut Assets<EffectAsset>,
) -> Handle<EffectAsset> {
    let key = (spell.to_owned(), phase, layer);
    if let Some(handle) = cache.handles.get(&key) {
        return handle.clone();
    }
    let handle = effects.add(build_effect_asset(layer, animation, color));
    cache.handles.insert(key, handle.clone());
    handle
}

/// Returns the shared unit beam mesh, building it on first use.
pub(super) fn beam_mesh_handle(
    cache: &mut SpellVfxAssetCache,
    meshes: &mut Assets<Mesh>,
) -> Handle<Mesh> {
    if let Some(handle) = &cache.beam_mesh {
        return handle.clone();
    }
    let handle = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    cache.beam_mesh = Some(handle.clone());
    handle
}

/// Returns `spell`'s cached beam material, building it on first use. Unlit and fully
/// opaque so the strike reads as a clean glow — "BAM" — rather than a translucent
/// blur, and independent of scene lighting/shadows.
pub(super) fn beam_material_for(
    spell: &str,
    color: Color,
    cache: &mut SpellVfxAssetCache,
    materials: &mut Assets<StandardMaterial>,
) -> Handle<StandardMaterial> {
    if let Some(handle) = cache.beam_materials.get(spell) {
        return handle.clone();
    }
    let handle = materials.add(StandardMaterial {
        base_color: color,
        emissive: color.to_linear() * 2.0,
        unlit: true,
        // Both the beam cuboid and the arc's cross ribbon are drawn from one set of
        // triangles with a single winding, so back-face culling made them vanish
        // entirely when the camera orbited past them — visible from one side, gone
        // from the other. A flat ribbon has no inside to cull.
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    cache
        .beam_materials
        .insert(spell.to_owned(), handle.clone());
    handle
}

/// Returns the shared soft-glow particle texture, building it on first use.
///
/// Every particle style samples this same texture — it defines the *shape* of one
/// particle (a soft round falloff, not a hard-edged sprite square); color still comes
/// from each style's own [`ColorOverLifetimeModifier`] gradient. Generated in code
/// rather than shipped as an image file: it has exactly one visual property (radial
/// softness) and generating it removes any question of asset licensing.
pub(super) fn glow_texture_handle(
    cache: &mut SpellVfxAssetCache,
    images: &mut Assets<Image>,
) -> Handle<Image> {
    if let Some(handle) = &cache.glow_texture {
        return handle.clone();
    }
    let handle = images.add(build_glow_texture());
    cache.glow_texture = Some(handle.clone());
    handle
}

/// Side length, in texels, of the generated glow texture. Small on purpose — this is
/// a soft blob, not a detailed sprite, and every particle in the game reuses one copy.
const GLOW_TEXTURE_SIZE: u32 = 32;

#[expect(
    clippy::cast_precision_loss,
    reason = "GLOW_TEXTURE_SIZE is a small compile-time constant (32), far inside \
              f32's exact integer range"
)]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "falloff is computed and clamped into [0.0, 1.0] immediately above the \
              cast, so scaling by 255.0 always lands in u8 range"
)]
fn build_glow_texture() -> Image {
    let size = GLOW_TEXTURE_SIZE;
    let center = (size as f32 - 1.0) / 2.0;
    let radius = center;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let dist = (dx * dx + dy * dy).sqrt();
            let t = (dist / radius).min(1.0);
            // Smooth center-bright, edge-feathered falloff — a soft glow dot rather
            // than a hard-edged disc.
            let falloff = (1.0 - t * t).max(0.0).powf(1.5);
            let byte = (falloff * 255.0).round() as u8;
            data.extend_from_slice(&[byte, byte, byte, byte]);
        }
    }
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    )
}

fn build_effect_asset(layer: VfxLayer, animation: &SpellAnimation, color: Color) -> EffectAsset {
    match (animation.style, layer) {
        (VfxStyle::Flame, VfxLayer::Smoke) => build_smoke_asset(animation),
        (VfxStyle::Flame, VfxLayer::Primary) => build_flame_asset(animation),
        (VfxStyle::Spark, _) => build_spark_asset(animation, color),
        (style @ (VfxStyle::FireballConcentrated | VfxStyle::FireballWispy), _) => {
            // A style with no sheet can never reach here, but the schema keeps the
            // sheet optional, so fall back to the generated flame rather than
            // failing to build an effect at all.
            style.sprite_sheet().map_or_else(
                || build_flame_asset(animation),
                |sheet| build_flipbook_asset(animation, sheet, style),
            )
        }
    }
}

/// Plays a hand-authored fire sheet frame by frame across each particle's lifetime.
///
/// The sheet supplies both shape and color, so unlike the generated styles there is
/// no palette gradient here — the gradient only fades the particle out, and the
/// texture is sampled with [`ImageSampleMapping::Modulate`] so its own RGB survives
/// instead of being reduced to an opacity mask.
fn build_flipbook_asset(
    animation: &SpellAnimation,
    sheet: SpriteSheet,
    style: VfxStyle,
) -> EffectAsset {
    let writer = ExprWriter::new();
    let (init_pos, init_vel, init_age, init_lifetime) = common_inits(
        &writer,
        animation.spread,
        animation.particle_speed,
        animation.particle_lifetime_seconds,
    );
    let rise = AccelModifier::new(writer.lit(Vec3::Y * 0.4).expr());

    // Walk the sheet once over the particle's life: frame = age/lifetime * frames,
    // wrapped so a particle that outlives its lifetime by a hair cannot index past
    // the last frame.
    let frames = f32_from_count(sheet.frames());
    let sprite_index = writer
        .attr(Attribute::AGE)
        .div(writer.attr(Attribute::LIFETIME))
        .mul(writer.lit(frames))
        .cast(ScalarType::Int)
        .rem(writer.lit(i32_from_count(sheet.frames())))
        .expr();
    let update_sprite_index = SetAttributeModifier::new(Attribute::SPRITE_INDEX, sprite_index);

    let mut color_gradient = bevy_hanabi::Gradient::new();
    color_gradient.add_key(0.0, Vec4::ONE);
    color_gradient.add_key(0.7, Vec4::ONE);
    color_gradient.add_key(1.0, Vec4::new(1.0, 1.0, 1.0, 0.0));

    let mut size_gradient = bevy_hanabi::Gradient::new();
    size_gradient.add_key(0.0, Vec3::ONE * animation.scale * 0.8);
    size_gradient.add_key(0.35, Vec3::ONE * animation.scale);
    size_gradient.add_key(1.0, Vec3::ONE * animation.scale * 0.85);

    let spawner = SpawnerSettings::once(f32_from_count(animation.particle_count).into());
    let (texture_slot, module) = finish_with_texture_slot(writer);

    // The concentrated sheet is meant to be nearly opaque, so it blends normally;
    // the wispy one is thin tendrils that should glow where they overlap.
    let alpha_mode = match style {
        VfxStyle::FireballWispy => bevy_hanabi::AlphaMode::Add,
        _ => bevy_hanabi::AlphaMode::Blend,
    };

    EffectAsset::new(animation.particle_count.max(64), spawner, module)
        .with_alpha_mode(alpha_mode)
        .init(init_pos)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .update(rise)
        .update(update_sprite_index)
        .render(face_camera())
        .render(ParticleTextureModifier {
            texture_slot,
            sample_mapping: ImageSampleMapping::Modulate,
        })
        .render(FlipbookModifier {
            sprite_grid_size: UVec2::new(sheet.columns, sheet.rows),
        })
        .render(ColorOverLifetimeModifier::new(color_gradient))
        .render(SizeOverLifetimeModifier {
            gradient: size_gradient,
            screen_space_size: false,
        })
}

#[expect(
    clippy::cast_possible_wrap,
    reason = "frame counts come from an 8x8-scale sprite grid, far inside i32 range"
)]
const fn i32_from_count(count: u32) -> i32 {
    count as i32
}

/// The shared init modifiers every style builds on: particles spawn in a ball of
/// `spread` radius around the emitter's origin, fly outward at `speed`, and live for
/// `lifetime`.
fn common_inits(
    writer: &ExprWriter,
    spawn_radius: f32,
    speed: f32,
    lifetime: f32,
) -> (
    SetPositionSphereModifier,
    SetVelocitySphereModifier,
    SetAttributeModifier,
    SetAttributeModifier,
) {
    let init_pos = SetPositionSphereModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        radius: writer.lit(spawn_radius).expr(),
        dimension: ShapeDimension::Volume,
    };
    let init_vel = SetVelocitySphereModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        speed: writer.lit(speed).expr(),
    };
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, writer.lit(lifetime).expr());
    (init_pos, init_vel, init_age, init_lifetime)
}

/// Consumes an [`ExprWriter`] and returns the texture-slot expression every style's
/// [`ParticleTextureModifier`] samples, alongside the finished [`Module`]. Every
/// style shares texture slot 0, bound at spawn time to the shared glow texture (see
/// [`glow_texture_handle`]) via the spawned entity's `EffectMaterial`.
fn finish_with_texture_slot(writer: ExprWriter) -> (ExprHandle, Module) {
    let texture_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("glow");
    (texture_slot, module)
}

/// Turns every particle to face the camera.
///
/// **Not optional, and the reason particles once looked like flat 2D discs.** A
/// `bevy_hanabi` particle is a quad, and the vertex shader only ever writes its
/// orientation axes from an [`OrientModifier`]'s generated code. With no such
/// modifier the axes keep their defaults, so every quad sits in one fixed world
/// plane: head-on the burst looks correct, but orbiting the camera reveals a cloud
/// of discs turning edge-on. Billboarding is what makes them read as volume.
///
/// [`OrientMode::FaceCameraPosition`] rather than the cheaper
/// `ParallelCameraDepthPlane` because these bursts throw particles wide of the view
/// axis, where pointing each quad at the camera's actual position holds up and
/// screen-parallel quads visibly shear.
fn face_camera() -> OrientModifier {
    OrientModifier::new(OrientMode::FaceCameraPosition)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "particle_count is capped (spell_animation.rs), well inside f32's \
              exact integer range"
)]
fn once_or_rate(
    phase_is_trailing_travel: bool,
    particle_count: u32,
    travel_seconds: f32,
) -> SpawnerSettings {
    let count = particle_count as f32;
    if phase_is_trailing_travel {
        SpawnerSettings::rate((count / travel_seconds.max(f32::EPSILON)).into())
    } else {
        SpawnerSettings::once(count.into())
    }
}

/// A tight, single-color burst: sparks, arcane light, electricity. Used for
/// `VfxStyle::Spark` in every phase. Additive blending (`AlphaMode::Add`) is what
/// makes overlapping particles brighten each other into a glow rather than just
/// stacking flat color — the standard treatment for energy/magic effects.
fn build_spark_asset(animation: &SpellAnimation, color: Color) -> EffectAsset {
    let writer = ExprWriter::new();
    let trailing = matches!(
        animation.motion,
        MotionArchetype::Projectile { trail: true, .. }
    );
    let travel_seconds = match animation.motion {
        MotionArchetype::Projectile { travel_seconds, .. } => travel_seconds,
        _ => 1.0,
    };

    let (init_pos, init_vel, init_age, init_lifetime) = common_inits(
        &writer,
        animation.spread,
        animation.particle_speed,
        animation.particle_lifetime_seconds,
    );

    let linear = color.to_linear();
    let rgba = Vec4::new(linear.red, linear.green, linear.blue, linear.alpha);
    let mut color_gradient = bevy_hanabi::Gradient::new();
    color_gradient.add_key(0.0, rgba);
    color_gradient.add_key(1.0, rgba.with_w(0.0));

    let mut size_gradient = bevy_hanabi::Gradient::new();
    size_gradient.add_key(0.0, Vec3::ONE * animation.scale);
    size_gradient.add_key(1.0, Vec3::ONE * animation.scale * 0.3);

    let spawner = once_or_rate(trailing, animation.particle_count, travel_seconds);
    let (texture_slot, module) = finish_with_texture_slot(writer);

    EffectAsset::new(animation.particle_count.max(64), spawner, module)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Add)
        .init(init_pos)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .render(face_camera())
        .render(ParticleTextureModifier {
            texture_slot,
            sample_mapping: ImageSampleMapping::ModulateOpacityFromR,
        })
        .render(ColorOverLifetimeModifier::new(color_gradient))
        .render(SizeOverLifetimeModifier {
            gradient: size_gradient,
            screen_space_size: false,
        })
}

/// A white-hot core cooling through yellow and orange to red, drifting gently
/// upward. Reusable by any fire-flavored spell — `Ember` today, `Fireball` and
/// `Flamethrower` later — via `VfxStyle::Flame`; none of it is per-spell tunable
/// beyond `particle_count`/`particle_speed`/`particle_lifetime_seconds`/`scale`/
/// `spread`. Additive blending sells the "burning" glow.
fn build_flame_asset(animation: &SpellAnimation) -> EffectAsset {
    let writer = ExprWriter::new();
    let (init_pos, init_vel, init_age, init_lifetime) = common_inits(
        &writer,
        animation.spread,
        animation.particle_speed,
        animation.particle_lifetime_seconds,
    );
    // Flames lick upward rather than just drifting on their spawn velocity.
    let rise = AccelModifier::new(writer.lit(Vec3::Y * 0.6).expr());

    let mut color_gradient = bevy_hanabi::Gradient::new();
    color_gradient.add_key(0.0, Vec4::new(1.0, 0.95, 0.7, 1.0));
    color_gradient.add_key(0.25, Vec4::new(1.0, 0.75, 0.15, 1.0));
    color_gradient.add_key(0.55, Vec4::new(0.95, 0.35, 0.05, 0.85));
    color_gradient.add_key(1.0, Vec4::new(0.4, 0.05, 0.02, 0.0));

    let mut size_gradient = bevy_hanabi::Gradient::new();
    size_gradient.add_key(0.0, Vec3::ONE * animation.scale * 0.7);
    size_gradient.add_key(0.4, Vec3::ONE * animation.scale);
    size_gradient.add_key(1.0, Vec3::ONE * animation.scale * 0.35);

    let spawner = SpawnerSettings::once(f32_from_count(animation.particle_count).into());
    let (texture_slot, module) = finish_with_texture_slot(writer);

    EffectAsset::new(animation.particle_count.max(64), spawner, module)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Add)
        .init(init_pos)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .update(rise)
        .render(face_camera())
        .render(ParticleTextureModifier {
            texture_slot,
            sample_mapping: ImageSampleMapping::ModulateOpacityFromR,
        })
        .render(ColorOverLifetimeModifier::new(color_gradient))
        .render(SizeOverLifetimeModifier {
            gradient: size_gradient,
            screen_space_size: false,
        })
}

/// `Flame`'s companion layer: fewer, larger, slower, longer-lived, darker particles
/// that rise and dissipate — a proper flame effect needs smoke, not just fire. Kept
/// on ordinary alpha blending (not additive): smoke is opaque haze, not light, and
/// additive smoke would look like a glowing fog instead.
fn build_smoke_asset(animation: &SpellAnimation) -> EffectAsset {
    let writer = ExprWriter::new();
    let smoke_count = (animation.particle_count / 4).max(8);
    let (init_pos, init_vel, init_age, init_lifetime) = common_inits(
        &writer,
        animation.spread * 1.2,
        animation.particle_speed * 0.4,
        animation.particle_lifetime_seconds * 2.2,
    );
    let rise = AccelModifier::new(writer.lit(Vec3::Y * 0.9).expr());
    let drag = LinearDragModifier::new(writer.lit(1.5).expr());

    let mut color_gradient = bevy_hanabi::Gradient::new();
    color_gradient.add_key(0.0, Vec4::new(0.35, 0.33, 0.3, 0.0));
    color_gradient.add_key(0.25, Vec4::new(0.32, 0.3, 0.28, 0.32));
    color_gradient.add_key(1.0, Vec4::new(0.12, 0.11, 0.1, 0.0));

    let mut size_gradient = bevy_hanabi::Gradient::new();
    size_gradient.add_key(0.0, Vec3::ONE * animation.scale * 0.6);
    size_gradient.add_key(1.0, Vec3::ONE * animation.scale * 1.8);

    let spawner = SpawnerSettings::once(f32_from_count(smoke_count).into());
    let (texture_slot, module) = finish_with_texture_slot(writer);

    EffectAsset::new(smoke_count.max(32), spawner, module)
        .init(init_pos)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .update(rise)
        .update(drag)
        .render(face_camera())
        .render(ParticleTextureModifier {
            texture_slot,
            sample_mapping: ImageSampleMapping::ModulateOpacityFromR,
        })
        .render(ColorOverLifetimeModifier::new(color_gradient))
        .render(SizeOverLifetimeModifier {
            gradient: size_gradient,
            screen_space_size: false,
        })
}

#[expect(
    clippy::cast_precision_loss,
    reason = "particle_count is capped (spell_animation.rs), well inside f32's \
              exact integer range"
)]
fn f32_from_count(count: u32) -> f32 {
    count as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spark_flash() -> SpellAnimation {
        SpellAnimation {
            motion: MotionArchetype::InstantFlash { hold_seconds: 0.4 },
            style: VfxStyle::Spark,
            color_override: None,
            particle_count: 40,
            particle_speed: 1.4,
            particle_lifetime_seconds: 0.5,
            scale: 0.12,
            spread: 0.18,
        }
    }

    fn spark_projectile() -> SpellAnimation {
        SpellAnimation {
            motion: MotionArchetype::Projectile {
                travel_seconds: 0.12,
                trail: true,
                impact_hold_seconds: 0.25,
            },
            style: VfxStyle::Spark,
            color_override: Some([1.0, 0.95, 0.55, 1.0]),
            particle_count: 60,
            particle_speed: 6.0,
            particle_lifetime_seconds: 0.2,
            scale: 0.1,
            spread: 0.15,
        }
    }

    fn beam() -> SpellAnimation {
        SpellAnimation {
            motion: MotionArchetype::Beam {
                flash_seconds: 0.08,
                impact_hold_seconds: 0.22,
                thickness: 0.12,
            },
            style: VfxStyle::Spark,
            color_override: Some([1.0, 0.95, 0.55, 1.0]),
            particle_count: 50,
            particle_speed: 2.2,
            particle_lifetime_seconds: 0.18,
            scale: 0.12,
            spread: 0.18,
        }
    }

    fn flame() -> SpellAnimation {
        SpellAnimation {
            motion: MotionArchetype::InstantFlash { hold_seconds: 0.45 },
            style: VfxStyle::Flame,
            color_override: None,
            particle_count: 70,
            particle_speed: 0.7,
            particle_lifetime_seconds: 0.45,
            scale: 0.16,
            spread: 0.22,
        }
    }

    /// Every shipped archetype/style/phase combination must build without
    /// panicking — this is the surface most likely to break on a `bevy_hanabi` API
    /// change or a bad modifier field.
    #[test]
    fn every_shipped_archetype_style_and_phase_builds_without_panicking() {
        build_effect_asset(VfxLayer::Primary, &spark_flash(), Color::WHITE);
        build_effect_asset(VfxLayer::Primary, &spark_projectile(), Color::WHITE);
        build_effect_asset(VfxLayer::Primary, &beam(), Color::WHITE);
        build_effect_asset(VfxLayer::Primary, &flame(), Color::WHITE);
        build_effect_asset(VfxLayer::Smoke, &flame(), Color::WHITE);
    }

    /// The regression guard for the "particles look like flat 2D discs when I
    /// orbit" bug: without an `OrientModifier` the vertex shader never writes the
    /// quad's orientation axes, so every style must carry one.
    #[test]
    fn every_style_billboards_its_particles() {
        for (layer, animation) in [
            (VfxLayer::Primary, spark_flash()),
            (VfxLayer::Primary, beam()),
            (VfxLayer::Primary, flame()),
            (VfxLayer::Smoke, flame()),
        ] {
            let asset = build_effect_asset(layer, &animation, Color::WHITE);
            assert!(
                asset
                    .render_modifiers()
                    .any(|modifier| modifier.as_any().is::<OrientModifier>()),
                "{layer:?}/{:?} must orient its particles toward the camera",
                animation.style
            );
        }
    }

    /// `Smoke` is meaningless for `Spark` — `build_effect_asset` must not panic if
    /// asked for it anyway, since `VfxLayer` and `VfxStyle` are independent enums
    /// with no type-level link between them.
    #[test]
    fn a_smoke_layer_request_for_a_non_flame_style_still_builds() {
        build_effect_asset(VfxLayer::Smoke, &spark_flash(), Color::WHITE);
    }

    #[test]
    fn a_non_trailing_projectile_still_builds() {
        let mut animation = spark_projectile();
        animation.motion = MotionArchetype::Projectile {
            travel_seconds: 0.12,
            trail: false,
            impact_hold_seconds: 0.25,
        };
        build_effect_asset(VfxLayer::Primary, &animation, Color::WHITE);
    }

    #[test]
    fn the_glow_texture_is_built_once_and_reused() {
        let mut cache = SpellVfxAssetCache::default();
        let mut images = Assets::<Image>::default();

        let first = glow_texture_handle(&mut cache, &mut images);
        let second = glow_texture_handle(&mut cache, &mut images);
        assert_eq!(first.id(), second.id());
    }

    #[test]
    fn the_glow_texture_is_radially_soft_not_a_hard_edged_square() {
        let image = build_glow_texture();
        let size = usize::try_from(GLOW_TEXTURE_SIZE).expect("texture size fits usize");
        let stride = size * 4;
        let data = image.data.as_ref().expect("generated texture has data");
        let center_byte = *data
            .get((size / 2) * stride + (size / 2) * 4)
            .expect("center texel is in bounds");
        let corner_byte = *data
            .first()
            .expect("generated texture has at least one texel");
        assert!(
            center_byte > 200,
            "the center should be near-opaque, was {center_byte}"
        );
        assert!(
            corner_byte < 20,
            "the far corner should have faded almost to nothing, was {corner_byte}"
        );
    }

    #[test]
    fn repeated_lookups_for_the_same_spell_phase_and_layer_reuse_one_handle() {
        let mut cache = SpellVfxAssetCache::default();
        let mut effects = Assets::<EffectAsset>::default();
        let animation = spark_flash();

        let first = effect_handle_for(
            "Ember",
            VfxPhase::Impact,
            VfxLayer::Primary,
            &animation,
            Color::WHITE,
            &mut cache,
            &mut effects,
        );
        let second = effect_handle_for(
            "Ember",
            VfxPhase::Impact,
            VfxLayer::Primary,
            &animation,
            Color::WHITE,
            &mut cache,
            &mut effects,
        );
        assert_eq!(first.id(), second.id());
    }

    #[test]
    fn clearing_authored_content_forces_a_rebuilt_handle() {
        let mut cache = SpellVfxAssetCache::default();
        let mut effects = Assets::<EffectAsset>::default();
        let animation = spark_flash();

        let first = effect_handle_for(
            "Ember",
            VfxPhase::Impact,
            VfxLayer::Primary,
            &animation,
            Color::WHITE,
            &mut cache,
            &mut effects,
        );
        cache.clear_authored_content();
        let second = effect_handle_for(
            "Ember",
            VfxPhase::Impact,
            VfxLayer::Primary,
            &animation,
            Color::WHITE,
            &mut cache,
            &mut effects,
        );
        assert_ne!(
            first.id(),
            second.id(),
            "a tuning change must not keep serving the pre-edit effect"
        );
    }

    #[test]
    fn flame_and_smoke_layers_of_the_same_spell_get_different_handles() {
        let mut cache = SpellVfxAssetCache::default();
        let mut effects = Assets::<EffectAsset>::default();
        let animation = flame();

        let fire = effect_handle_for(
            "Ember",
            VfxPhase::Impact,
            VfxLayer::Primary,
            &animation,
            Color::WHITE,
            &mut cache,
            &mut effects,
        );
        let smoke = effect_handle_for(
            "Ember",
            VfxPhase::Impact,
            VfxLayer::Smoke,
            &animation,
            Color::WHITE,
            &mut cache,
            &mut effects,
        );
        assert_ne!(fire.id(), smoke.id());
    }

    #[test]
    fn different_spells_never_share_a_cached_handle() {
        let mut cache = SpellVfxAssetCache::default();
        let mut effects = Assets::<EffectAsset>::default();

        let ember = effect_handle_for(
            "Ember",
            VfxPhase::Impact,
            VfxLayer::Primary,
            &flame(),
            Color::WHITE,
            &mut cache,
            &mut effects,
        );
        let lightning = effect_handle_for(
            "Lightning Bolt",
            VfxPhase::Impact,
            VfxLayer::Primary,
            &beam(),
            Color::WHITE,
            &mut cache,
            &mut effects,
        );
        assert_ne!(ember.id(), lightning.id());
    }

    #[test]
    fn the_beam_mesh_is_shared_across_every_beam_spell() {
        let mut cache = SpellVfxAssetCache::default();
        let mut meshes = Assets::<Mesh>::default();

        let first = beam_mesh_handle(&mut cache, &mut meshes);
        let second = beam_mesh_handle(&mut cache, &mut meshes);
        assert_eq!(first.id(), second.id());
    }

    /// The bolt is a flat ribbon: culled back faces made it disappear completely
    /// when the camera orbited to the other side.
    #[test]
    fn beam_and_arc_geometry_renders_from_both_sides() {
        let mut cache = SpellVfxAssetCache::default();
        let mut materials = Assets::<StandardMaterial>::default();

        let handle = beam_material_for("Lightning Bolt", Color::WHITE, &mut cache, &mut materials);
        let material = materials
            .get(&handle)
            .expect("the material was just inserted");
        assert!(material.double_sided);
        assert_eq!(material.cull_mode, None);
    }

    #[test]
    fn beam_materials_are_cached_per_spell() {
        let mut cache = SpellVfxAssetCache::default();
        let mut materials = Assets::<StandardMaterial>::default();

        let lightning_first =
            beam_material_for("Lightning Bolt", Color::WHITE, &mut cache, &mut materials);
        let lightning_second =
            beam_material_for("Lightning Bolt", Color::WHITE, &mut cache, &mut materials);
        assert_eq!(lightning_first.id(), lightning_second.id());

        let other_spell = beam_material_for(
            "Some Other Beam Spell",
            Color::WHITE,
            &mut cache,
            &mut materials,
        );
        assert_ne!(lightning_first.id(), other_spell.id());
    }
}
