//! Spawns a spell's VFX when its [`CombatEvent::Cast`] fact is disclosed.

use bevy::prelude::*;
use bevy_hanabi::prelude::*;

use hex_anim::{LinearMovement, Transformation};
use hex_assets::{
    ElementCatalog, MotionArchetype, Spell, SpellAnimation, SpellAnimationFile, SpellBook, VfxStyle,
};
use hex_combat::CombatEvent;
use hex_core::{HexSpan, HexTile, KnowledgeState, TilePos};
use hex_perception::FactionMapKnowledge;
use hex_ui::element_color;
use hex_units::{Faction, StandsOn, UnitRegistry};

use super::assets::{
    beam_material_for, beam_mesh_handle, effect_handle_for, glow_texture_handle,
    SpellVfxAssetCache, VfxLayer, VfxPhase,
};

/// Countdown to despawn a spawned VFX entity, independent of any `Transformation` it
/// carries — `hex_anim` only removes a finished `Transformation` component, never
/// the entity it's attached to.
#[derive(Component)]
pub(crate) struct SpellVfxLifetime(pub Timer);

/// Marks a `Projectile` archetype's travelling emitter, so
/// [`super::lifetime::spawn_impact_burst_on_arrival`] knows which spell's impact
/// burst to play, and in what color, once its [`Transformation`] finishes.
///
/// `color` is carried rather than re-resolved on arrival: re-deriving it would need
/// this system's own `SpellBook`/`ElementCatalog` lookup duplicated in
/// `lifetime.rs` for a value already computed once here.
#[derive(Component)]
pub(super) struct SpellVfxProjectile {
    pub spell: String,
    pub color: Color,
}

/// Tiles as this system reads them: a surface's exact world height, for placing VFX
/// on the ground rather than at voxel-index zero. Mirrors `casting/preview.rs`'s own
/// `TileQuery`.
pub(super) type TileQuery<'w, 's> =
    Query<'w, 's, (&'static TilePos, &'static HexSpan), With<HexTile>>;

/// Drops every cached particle-effect handle whenever `SpellAnimationFile` changes —
/// on first load, on a hot-reloaded edit to `spell_animations.ron`, or on an edit made
/// in the VFX tuner — so a tuning change takes effect on the very next cast instead of
/// silently replaying whatever was cached before the edit.
pub(super) fn clear_stale_effect_cache(
    animations: Option<Res<SpellAnimationFile>>,
    mut cache: ResMut<SpellVfxAssetCache>,
) {
    let Some(animations) = animations else {
        return;
    };
    if animations.is_changed() {
        cache.clear_authored_content();
    }
}

/// Resolves the color a spell's VFX renders in: an authored override, or the tint of
/// its flavor element (the first gem its requirements name). Shared by the real
/// combat trigger and the VFX tuner so the two never compute color differently.
pub(crate) fn resolve_cast_color(
    spell_def: &Spell,
    animation: &SpellAnimation,
    elements: &ElementCatalog,
) -> Color {
    let flavor_element = spell_def
        .requirements
        .first()
        .map(|requirement| requirement.element.as_str());
    animation.color_override.map_or_else(
        || element_color(flavor_element.and_then(|name| elements.id(name)), elements),
        |[r, g, b, a]| Color::srgba(r, g, b, a),
    )
}

/// Reads [`CombatEvent::Cast`], resolves whether the player is authorized to see it,
/// and spawns the authored VFX for the spell.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors readouts/log.rs::ingest: resolving world position, color, and \
              disclosure for a combat event needs the registry, spatial positions, \
              catalogs, and both content resources independently"
)]
pub(super) fn trigger_spell_vfx(
    mut commands: Commands,
    mut events: MessageReader<CombatEvent>,
    registry: Res<UnitRegistry>,
    standing: Query<&StandsOn>,
    factions: Query<&Faction>,
    tiles: TileQuery,
    knowledge: Option<Res<FactionMapKnowledge>>,
    spells: Option<Res<SpellBook>>,
    animations: Option<Res<SpellAnimationFile>>,
    elements: Option<Res<ElementCatalog>>,
    mut cache: ResMut<SpellVfxAssetCache>,
    mut effects: ResMut<Assets<EffectAsset>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    asset_server: Res<AssetServer>,
) {
    let (Some(spells), Some(animations), Some(elements), Some(knowledge)) =
        (spells, animations, elements, knowledge)
    else {
        return;
    };

    for event in events.read() {
        let CombatEvent::Cast {
            caster,
            spell,
            target,
        } = event
        else {
            continue;
        };
        let Some(animation) = animations.animations.get(spell) else {
            continue;
        };

        // Fog-of-war gate, following the fail-closed idiom `readouts/log.rs` and
        // `terrain_health_bars.rs` both use. The target tile must be currently
        // observed; a hostile caster's own tile must be too, so a spell cast from
        // just outside sight does not flash a bolt into view from nowhere.
        let player_knowledge = knowledge.faction(Faction::Player);
        if player_knowledge.state(*target) != KnowledgeState::Observed {
            continue;
        }
        let Some(caster_entity) = registry.entity_of(*caster) else {
            continue;
        };
        let Ok(caster_standing) = standing.get(caster_entity) else {
            continue;
        };
        let Ok(&caster_faction) = factions.get(caster_entity) else {
            continue;
        };
        if caster_faction != Faction::Player
            && player_knowledge.state(caster_standing.0.pos) != KnowledgeState::Observed
        {
            continue;
        }

        let Some(spell_def) = spells.id(spell).and_then(|id| spells.spell(id)) else {
            continue;
        };
        let Some((_, target_span)) = tiles.iter().find(|(pos, _)| **pos == *target) else {
            continue;
        };

        let color = resolve_cast_color(spell_def, animation, &elements);
        let caster_world = caster_standing.0.world_position();
        let target_world = target.coord.to_world(target_span.top);

        spawn_cast_vfx(
            &mut commands,
            spell,
            animation,
            color,
            caster_world,
            target_world,
            &mut cache,
            &mut effects,
            &mut meshes,
            &mut materials,
            &mut images,
            &asset_server,
        );
    }
}

/// Spawns `spell`'s complete VFX for one cast from `caster_world` to `target_world`.
///
/// The one place that turns "a spell was cast here, aimed there" into entities —
/// shared by the real combat trigger above and `Screen::VfxTuner`
/// (`crate::screens::vfx_tuner`), which calls this directly on its two preview
/// dummies, bypassing combat, mana, and turn order entirely so a designer can
/// replay a cast on demand.
#[expect(
    clippy::too_many_arguments,
    reason = "spawning one cast's complete VFX needs the spell name, its authored \
              tuning, resolved color, both world positions, and every asset cache \
              independently"
)]
pub(crate) fn spawn_cast_vfx(
    commands: &mut Commands,
    spell: &str,
    animation: &SpellAnimation,
    color: Color,
    caster_world: Vec3,
    target_world: Vec3,
    cache: &mut SpellVfxAssetCache,
    effects: &mut Assets<EffectAsset>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    asset_server: &AssetServer,
) {
    // Flipbook styles are painted sheets on disk; everything else samples the
    // procedural soft-glow dot generated in `assets.rs`. One texture slot either way,
    // so nothing downstream has to branch on which kind of style it is drawing.
    let texture = animation.style.sprite_sheet().map_or_else(
        || glow_texture_handle(cache, images),
        |sheet| asset_server.load(sheet.path),
    );

    match animation.motion {
        MotionArchetype::InstantFlash { hold_seconds } => {
            spawn_style_layers(
                commands,
                spell,
                VfxPhase::Impact,
                animation,
                color,
                Transform::from_translation(target_world),
                hold_seconds,
                &texture,
                cache,
                effects,
            );
        }
        MotionArchetype::Arc {
            flash_seconds,
            impact_hold_seconds,
            thickness,
            displacement,
            subdivisions,
            branches,
        } => {
            if caster_world.distance(target_world) <= f32::EPSILON {
                return;
            }
            // Generated per cast and never cached: a bolt that replayed one
            // silhouette would read as a decal rather than lightning. The mesh dies
            // with the entity a fraction of a second later.
            let mesh = meshes.add(super::lightning::build_arc_mesh(
                caster_world,
                target_world,
                thickness,
                displacement,
                subdivisions,
                branches,
                &mut rand::thread_rng(),
            ));
            let material = beam_material_for(spell, color, cache, materials);
            commands.spawn((
                Name::new(format!("{spell} VFX (arc)")),
                Mesh3d(mesh),
                MeshMaterial3d(material),
                // The path is generated in world space, so the entity itself sits at
                // the origin untransformed — a jagged bolt has no meaningful local
                // pivot to rotate or scale about.
                Transform::IDENTITY,
                SpellVfxLifetime(Timer::from_seconds(flash_seconds, TimerMode::Once)),
            ));
            spawn_style_layers(
                commands,
                spell,
                VfxPhase::Impact,
                animation,
                color,
                Transform::from_translation(target_world),
                impact_hold_seconds,
                &texture,
                cache,
                effects,
            );
        }
        MotionArchetype::Beam {
            flash_seconds,
            impact_hold_seconds,
            thickness,
        } => {
            let distance = caster_world.distance(target_world);
            // Same degenerate case `LinearMovement`/`hex_combat::presentation`
            // guards for a zero-length leg: nothing to draw a line across.
            if distance <= f32::EPSILON {
                return;
            }
            let direction = (target_world - caster_world) / distance;
            // `looking_at` is undefined when the look direction is parallel to
            // `up`; fall back to a perpendicular axis for a near-vertical bolt.
            let up = if direction.cross(Vec3::Y).length_squared() < 1e-6 {
                Vec3::X
            } else {
                Vec3::Y
            };
            let midpoint = caster_world.midpoint(target_world);
            let mesh = beam_mesh_handle(cache, meshes);
            let material = beam_material_for(spell, color, cache, materials);
            commands.spawn((
                Name::new(format!("{spell} VFX (beam)")),
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::from_translation(midpoint)
                    .looking_at(target_world, up)
                    // The line's own thickness, not `animation.scale` — that one
                    // sizes the impact burst's particles, and sharing it made a
                    // thicker bolt also throw bigger sparks.
                    .with_scale(Vec3::new(thickness, thickness, distance)),
                SpellVfxLifetime(Timer::from_seconds(flash_seconds, TimerMode::Once)),
            ));
            spawn_style_layers(
                commands,
                spell,
                VfxPhase::Impact,
                animation,
                color,
                Transform::from_translation(target_world),
                impact_hold_seconds,
                &texture,
                cache,
                effects,
            );
        }
        MotionArchetype::Projectile {
            travel_seconds,
            impact_hold_seconds,
            ..
        } => {
            let distance = caster_world.distance(target_world);
            // A zero-length leg makes `LinearMovement` produce NaN (see its own
            // doc comment) — the same degenerate case `hex_combat::presentation`
            // guards for a lunge. A spell landing on the caster's own tile has
            // nothing to travel across, so this skips the VFX rather than the cast.
            if distance <= f32::EPSILON {
                return;
            }
            let handle = effect_handle_for(
                spell,
                VfxPhase::Travel,
                VfxLayer::Primary,
                animation,
                color,
                cache,
                effects,
            );
            let speed = distance / travel_seconds.max(f32::EPSILON);
            commands.spawn((
                Name::new(format!("{spell} VFX (travel)")),
                ParticleEffect::new(handle),
                EffectMaterial {
                    images: vec![texture.clone()],
                },
                Transform::from_translation(caster_world),
                Transformation::new(LinearMovement::new(caster_world, target_world, speed, 0.0)),
                SpellVfxProjectile {
                    spell: spell.to_owned(),
                    color,
                },
                SpellVfxLifetime(Timer::from_seconds(
                    travel_seconds + impact_hold_seconds + animation.particle_lifetime_seconds,
                    TimerMode::Once,
                )),
            ));
        }
    }
}

/// Spawns `spell`'s `phase` VFX at `transform`, held for `hold_seconds` beyond each
/// particle's own lifetime. [`VfxStyle::Flame`] is two entities spawned together —
/// the fire and its companion smoke — every other style is one; callers never branch
/// on style themselves.
#[expect(
    clippy::too_many_arguments,
    reason = "spawning a cast's VFX needs the spell name, phase, authored tuning, \
              resolved color, placement, the particle texture, and both asset \
              caches independently"
)]
pub(super) fn spawn_style_layers(
    commands: &mut Commands,
    spell: &str,
    phase: VfxPhase,
    animation: &SpellAnimation,
    color: Color,
    transform: Transform,
    hold_seconds: f32,
    texture: &Handle<Image>,
    cache: &mut SpellVfxAssetCache,
    effects: &mut Assets<EffectAsset>,
) {
    let layers: &[VfxLayer] = if animation.style == VfxStyle::Flame {
        &[VfxLayer::Primary, VfxLayer::Smoke]
    } else {
        &[VfxLayer::Primary]
    };
    for &layer in layers {
        let handle = effect_handle_for(spell, phase, layer, animation, color, cache, effects);
        // Smoke's own particle lifetime is stretched in `assets.rs`'s
        // `build_smoke_asset`, so its hold must stretch by the same factor or the
        // entity despawns mid-fade.
        let particle_lifetime = if layer == VfxLayer::Smoke {
            animation.particle_lifetime_seconds * 2.2
        } else {
            animation.particle_lifetime_seconds
        };
        commands.spawn((
            Name::new(format!("{spell} VFX ({layer:?})")),
            ParticleEffect::new(handle),
            EffectMaterial {
                images: vec![texture.clone()],
            },
            transform,
            SpellVfxLifetime(Timer::from_seconds(
                hold_seconds + particle_lifetime,
                TimerMode::Once,
            )),
        ));
    }
}
