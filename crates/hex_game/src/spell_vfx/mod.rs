//! World-space VFX for spell casts.
//!
//! Reads [`hex_combat::CombatEvent::Cast`] the same way `readouts::log` does, looks
//! the spell up in `hex_assets::SpellAnimationFile` (RON-authored, sibling to
//! `spells.ron`), and spawns a `bevy_hanabi` particle effect built from a closed
//! vocabulary of motion archetypes (see `hex_assets::spell_animation`). A spell with
//! no authored entry simply casts with no VFX.
//!
//! Extending this for a new targeting shape (`Sphere`, `Cone`, `Column`, `SelfCast`)
//! is a new [`hex_assets::MotionArchetype`] variant plus one match arm in
//! [`trigger::trigger_spell_vfx`] — the trigger/lifetime/asset-caching plumbing here
//! does not change.

use bevy::prelude::*;

use hex_combat::CombatSystems;
use hex_core::{AppSystems, PausableSystems, Screen};

mod assets;
mod lifetime;
mod lightning;
mod trigger;

pub(crate) use assets::SpellVfxAssetCache;
// `Screen::VfxTuner` calls these directly to replay a cast with no combat behind it;
// the real trigger below uses them through `trigger`'s module-internal paths.
pub(crate) use trigger::{resolve_cast_color, spawn_cast_vfx, SpellVfxLifetime};

/// Registers the spell-cast VFX trigger and its cleanup systems.
pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<SpellVfxAssetCache>();

    app.add_systems(
        Update,
        (
            trigger::clear_stale_effect_cache,
            trigger::trigger_spell_vfx,
        )
            .chain()
            .in_set(AppSystems::Update)
            .after(CombatSystems::Advance)
            .run_if(in_state(Screen::Gameplay)),
        // Not pausable: `CombatEvent`s age out after a couple of frames (see
        // `readouts/log.rs::ingest`, which reads the same message stream the same
        // way), so a pause landing on the resolution frame must not eat the trigger.
        // Chained so a same-frame tuning edit is cleared before that frame's casts
        // are triggered, rather than one frame later.
    );
    // Cache invalidation is not gameplay-only. The VFX tuner edits `SpellAnimationFile`
    // directly and then replays the cast on the same screen; without this running
    // there, every cached effect handle survives the edit and Play silently replays
    // the values from before it — the tuner would appear to do nothing at all.
    app.add_systems(
        Update,
        trigger::clear_stale_effect_cache
            .in_set(AppSystems::Update)
            .run_if(in_state(Screen::VfxTuner)),
    );
    // Likewise the lifetime countdown: it is what despawns a finished effect. Gated
    // to gameplay it left the tuner accumulating dead emitters forever, and a
    // projectile there would never hand off to its impact burst.
    app.add_systems(
        Update,
        (
            lifetime::spawn_impact_burst_on_arrival.after(hex_anim::AnimationSystems::Drive),
            lifetime::tick_spell_vfx_lifetime,
        )
            .in_set(AppSystems::Update)
            .in_set(PausableSystems)
            .run_if(in_state(Screen::Gameplay)),
    );
    app.add_systems(
        Update,
        (
            lifetime::spawn_impact_burst_on_arrival.after(hex_anim::AnimationSystems::Drive),
            lifetime::tick_spell_vfx_lifetime,
        )
            .in_set(AppSystems::Update)
            .run_if(in_state(Screen::VfxTuner)),
    );
}
