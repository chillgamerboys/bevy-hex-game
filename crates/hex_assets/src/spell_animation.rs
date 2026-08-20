//! Spell VFX presentation config, loaded from `assets/config/spell_animations.ron`.
//!
//! Deliberately **not** a field on [`crate::spells::Spell`]: that type's
//! `SpellBook::source_fingerprint` is a semantic hash of gameplay-affecting content,
//! and animation tuning has no gameplay effect. Keeping it a sibling file, keyed by
//! spell name, means editing a burst's particle count never perturbs a hash anything
//! downstream treats as "did the spell's rules change".
//!
//! Unlike [`crate::terrain_damage`], this file's only cross-reference is its own
//! top-level key — a spell name — checked against [`crate::spells::SpellBook`] at
//! lookup time by the presentation system that consumes it. There is no id to
//! resolve and cache: `hex_game`'s trigger system looks a cast's spell name up in
//! this map directly, the same way it already looks the name up in `SpellBook`. A
//! name with no matching spell simply plays no animation — it costs a visual, never
//! a cast — so unlike `spells.rs`/`terrain_damage.rs` this never blocks or rejects a
//! load; it only warns.
//!
//! # Closed schema, no scripting
//!
//! [`MotionArchetype`] is a fixed enum, mirroring the discipline `spells.rs`
//! documents for [`crate::Effect`]: extension is one variant plus one match arm in
//! the trigger system (`hex_game::spell_vfx`), never new logic here.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

use crate::spells::SpellBook;
use crate::{LoadSettings, CONFIG_EXTENSIONS};

/// A closed vocabulary of VFX motion shapes.
///
/// `Single`/`Direct` spells only need these three. A future `Sphere`/`Cone`/`Line`/
/// `Column` shape's animation is a new variant (a `Burst` over a resolved volume, a
/// `Sweep` along a cone, an `Aura` for `SelfCast`) plus one match arm in
/// `hex_game::spell_vfx::trigger` — not a rewrite.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum MotionArchetype {
    /// A small emitter travels from the caster's position to the anchor over
    /// `travel_seconds` (driven by `hex_anim::LinearMovement`), optionally trailing
    /// particles continuously, then plays a one-shot impact burst at the anchor and
    /// holds for `impact_hold_seconds`. Reads as a visible flight — a thrown or
    /// launched projectile, not an instant strike. No shipped spell uses this yet;
    /// it stays a real, fully wired archetype for a future spell that genuinely
    /// travels (a thrown dagger, an arrow).
    Projectile {
        /// Seconds the emitter spends travelling from caster to target.
        travel_seconds: f32,
        /// Whether particles spawn continuously across the travel, not just at the
        /// start and the impact.
        trail: bool,
        /// Seconds the impact burst is held after arrival.
        impact_hold_seconds: f32,
    },
    /// A line snaps into existence between caster and target with no travel time —
    /// "a line appears instantly" — held for `flash_seconds`, with an impact burst
    /// at the target end held for `impact_hold_seconds`. Lightning Bolt's shape:
    /// instant strike, not a thrown object.
    Beam {
        /// Seconds the line itself is visible before it disappears.
        flash_seconds: f32,
        /// Seconds the impact burst is held at the target end.
        impact_hold_seconds: f32,
        /// The line's cross-section width, in world units.
        ///
        /// Deliberately separate from [`SpellAnimation::scale`]: the line and the
        /// impact burst's particles are different objects, and tying them to one
        /// number means thickening the bolt also inflates every spark it throws.
        thickness: f32,
    },
    /// A jagged electric arc snaps between caster and target, with an impact burst
    /// at the target end.
    ///
    /// Unlike [`Self::Beam`]'s straight line, the path is generated per cast by
    /// recursive midpoint displacement: take the segment, displace its midpoint
    /// perpendicular by a random amount, recurse on each half with the displacement
    /// halved. Good lightning is procedural rather than painted — a fixed texture
    /// reads as fake precisely because real lightning never strikes the same shape
    /// twice, and re-rolling the path every cast is what sells it.
    Arc {
        /// Seconds the arc itself is visible before it disappears.
        flash_seconds: f32,
        /// Seconds the impact burst is held at the target end.
        impact_hold_seconds: f32,
        /// The arc's cross-section width, in world units.
        thickness: f32,
        /// How far, in world units, the first midpoint may be displaced. Each
        /// recursion level halves this, so it sets the overall crookedness.
        displacement: f32,
        /// How many times the path is subdivided. Each level doubles the segment
        /// count, so the path has `2^subdivisions` segments.
        subdivisions: u32,
        /// How many short forked branches split off the main path.
        branches: u32,
    },
    /// The full effect appears immediately at the anchor, with no caster-relative
    /// travel, and burns out over `hold_seconds`.
    InstantFlash {
        /// Seconds the effect is held at the anchor before it despawns.
        hold_seconds: f32,
    },
}

/// The most times an [`MotionArchetype::Arc`] path may be subdivided.
///
/// A safety limit, not a taste one: segment count is `2^subdivisions`, so this is
/// what keeps a typo'd value from generating a mesh with billions of vertices.
pub const MAX_ARC_SUBDIVISIONS: u32 = 10;

/// The most forked branches an [`MotionArchetype::Arc`] may spawn, for the same
/// reason as [`MAX_ARC_SUBDIVISIONS`].
pub const MAX_ARC_BRANCHES: u32 = 64;

/// A closed vocabulary of particle *looks*, orthogonal to [`MotionArchetype`] (which
/// only decides *where* an effect appears). Reusable across every spell that shares
/// an element's flavor rather than authored per spell.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum VfxStyle {
    /// A tight, single-color burst — sparks, arcane light, electricity.
    Spark,
    /// Multi-color fire (a white-hot core cooling through yellow and orange to red)
    /// plus a second, slower, darker rising smoke layer, generated in code. Any
    /// fire-flavored spell can request this look rather than each authoring its own
    /// gradient.
    Flame,
    /// Hand-authored fire, played as an 8x8 flipbook: a dense, bright, nearly opaque
    /// ball that stays contained. Alpha-blended, because the whole point of this
    /// sheet is that you cannot see through the fire.
    ///
    /// Painted frames beat a generated gradient for fire specifically — real flame
    /// has internal structure that a color ramp over a soft dot cannot produce.
    FireballConcentrated,
    /// The same flipbook treatment with the thin, wispy sheet: sparser, more
    /// tendril-like fire. Additively blended, so the wisps glow and overlap into
    /// brightness rather than stacking as flat opaque smears.
    FireballWispy,
}

/// One frame grid of a flipbook sheet: how the sprite atlas is cut up.
///
/// Both shipped sheets are 8x8, but this is carried per style rather than assumed,
/// so a differently-cut sheet is a data change rather than a code change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpriteSheet {
    /// Asset path, relative to the assets root.
    pub path: &'static str,
    /// Frame columns across the sheet.
    pub columns: u32,
    /// Frame rows down the sheet.
    pub rows: u32,
}

impl SpriteSheet {
    /// Total frames in the sheet.
    #[must_use]
    pub const fn frames(&self) -> u32 {
        self.columns * self.rows
    }
}

impl VfxStyle {
    /// The flipbook sheet this style plays, or `None` for the styles generated
    /// entirely in code.
    ///
    /// Paths are baked in rather than authored so [`VfxStyle`] stays a `Copy` closed
    /// vocabulary that the tuner can cycle through — the same discipline the rest of
    /// this schema follows. A new sheet is a new variant here plus its file.
    #[must_use]
    pub const fn sprite_sheet(self) -> Option<SpriteSheet> {
        match self {
            Self::Spark | Self::Flame => None,
            Self::FireballConcentrated => Some(SpriteSheet {
                path: "textures/flipbooks/FireBall01-flipbooks/FireBall01_8x8.tga",
                columns: 8,
                rows: 8,
            }),
            Self::FireballWispy => Some(SpriteSheet {
                path: "textures/flipbooks/FireBall04-flipbooks/FireBall04_8x8.tga",
                columns: 8,
                rows: 8,
            }),
        }
    }
}

/// One spell's authored VFX. Every field is a tunable parameter, never behavior —
/// the same discipline [`crate::Effect`] documents.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpellAnimation {
    /// The motion archetype driving where the effect appears.
    pub motion: MotionArchetype,
    /// The particle look. `Flame`'s palette and smoke layer are baked in and not
    /// further tunable here — see [`VfxStyle`].
    pub style: VfxStyle,
    /// Overrides the spell's flavor-element tint (resolved by the presentation
    /// system from the spell's first requirement's element — this crate cannot see
    /// `hex_ui`, which owns element color). `None` reuses that tint. Ignored by
    /// `VfxStyle::Flame`, whose palette is fixed.
    pub color_override: Option<[f32; 4]>,
    /// How many particles the effect spawns.
    pub particle_count: u32,
    /// World units/second particles leave the emitter at.
    pub particle_speed: f32,
    /// Seconds an individual particle lives.
    pub particle_lifetime_seconds: f32,
    /// Individual particle size in world units. Distinct from `spread`: a burst
    /// reads as "many small particles" only when this stays well under `spread`.
    ///
    /// Particles only — a `Beam`'s line has its own
    /// [`MotionArchetype::Beam::thickness`], so thickening a bolt does not also
    /// inflate the sparks it throws.
    pub scale: f32,
    /// Radius, in world units, of the ball particles spawn inside and fly outward
    /// from — how much space the *burst* fills, independent of how big any one
    /// particle is.
    pub spread: f32,
}

/// The smallest duration the tuner may drive a timing down to, in seconds.
///
/// `SpellAnimation::validate` only requires "greater than 0", which neither a
/// stepper nor a typed-in value can express — both have to stop at some epsilon.
/// This is that stop, one frame at 100fps.
///
/// There is deliberately **no** matching maximum. An earlier revision capped every
/// timing, scale, and spread on the theory that a large number was an authoring
/// mistake; in practice the caps were reached while tuning legitimate effects and
/// only got in the way. Taste is the author's call, so validation now rejects only
/// what cannot be rendered at all — NaN, infinity, and non-positive sizes.
pub const MIN_DURATION_SECONDS: f32 = 0.001;

/// The largest particle count content may author for one effect.
///
/// The one surviving ceiling, and a memory-safety limit rather than a taste one:
/// this number is the GPU buffer capacity passed to `EffectAsset::new`, so a typo'd
/// ten million would try to allocate an effect buffer far past anything a cast
/// needs. Set high enough that no plausible authored burst reaches it.
pub const MAX_PARTICLE_COUNT: u32 = 100_000;

/// The largest particle (or beam thickness) scale content may author, in world
/// units.
///
/// The smallest particle scale, beam thickness, or burst spread the tuner may drive
/// down to, in world units. A floor only — sizes have no authored ceiling.
pub const MIN_SIZE: f32 = 0.001;

impl SpellAnimation {
    fn validate(&self, spell: &str) -> Result<(), String> {
        // Rejects only what cannot be rendered — NaN, infinity, and non-positive
        // sizes. There is no upper bound: an effect that fills the screen is a
        // legitimate authoring choice, and the caps this used to impose were hit
        // while tuning real spells rather than catching real mistakes.
        let positive = |field: &str, value: f32| -> Result<(), String> {
            if !value.is_finite() || value <= 0.0 {
                return Err(format!(
                    "spell_animations.ron entry '{spell}' has {field} {value}; it must be a finite number greater than 0"
                ));
            }
            Ok(())
        };
        match self.motion {
            MotionArchetype::Projectile {
                travel_seconds,
                impact_hold_seconds,
                trail: _,
            } => {
                positive("travel_seconds", travel_seconds)?;
                positive("impact_hold_seconds", impact_hold_seconds)?;
            }
            MotionArchetype::Beam {
                flash_seconds,
                impact_hold_seconds,
                thickness,
            } => {
                positive("flash_seconds", flash_seconds)?;
                positive("impact_hold_seconds", impact_hold_seconds)?;
                positive("thickness", thickness)?;
            }
            MotionArchetype::Arc {
                flash_seconds,
                impact_hold_seconds,
                thickness,
                displacement,
                subdivisions,
                branches,
            } => {
                positive("flash_seconds", flash_seconds)?;
                positive("impact_hold_seconds", impact_hold_seconds)?;
                positive("thickness", thickness)?;
                positive("displacement", displacement)?;
                if subdivisions > MAX_ARC_SUBDIVISIONS {
                    return Err(format!(
                        "spell_animations.ron entry '{spell}' has subdivisions {subdivisions}; the maximum is {MAX_ARC_SUBDIVISIONS} (segment count is 2^subdivisions)"
                    ));
                }
                if branches > MAX_ARC_BRANCHES {
                    return Err(format!(
                        "spell_animations.ron entry '{spell}' has branches {branches}; the maximum is {MAX_ARC_BRANCHES}"
                    ));
                }
            }
            MotionArchetype::InstantFlash { hold_seconds } => {
                positive("hold_seconds", hold_seconds)?;
            }
        }
        if self.particle_count == 0 {
            return Err(format!(
                "spell_animations.ron entry '{spell}' has particle_count 0; it must spawn at least one particle"
            ));
        }
        if self.particle_count > MAX_PARTICLE_COUNT {
            return Err(format!(
                "spell_animations.ron entry '{spell}' has particle_count {}; the maximum is {MAX_PARTICLE_COUNT}",
                self.particle_count
            ));
        }
        if !self.particle_speed.is_finite() || self.particle_speed < 0.0 {
            return Err(format!(
                "spell_animations.ron entry '{spell}' has particle_speed {}; it must be a finite number at least 0",
                self.particle_speed
            ));
        }
        positive("particle_lifetime_seconds", self.particle_lifetime_seconds)?;
        positive("scale", self.scale)?;
        positive("spread", self.spread)?;
        if let Some(color) = self.color_override {
            if color.iter().any(|channel| !channel.is_finite()) {
                return Err(format!(
                    "spell_animations.ron entry '{spell}' has a non-finite color_override channel"
                ));
            }
        }
        Ok(())
    }
}

/// The raw file: spell name -> [`SpellAnimation`].
///
/// `Serialize` is plain-derived (only `Deserialize` needs the hand-written
/// validating impl below) — it's what lets a live-tuned copy of this resource be
/// written back out to `spell_animations.ron` (see `hex_game`'s dev-only VFX tuning
/// panel).
#[derive(Asset, Resource, Reflect, Debug, Clone, PartialEq, Serialize)]
#[reflect(Resource)]
pub struct SpellAnimationFile {
    /// Authored animations, by spell name.
    pub animations: HashMap<String, SpellAnimation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedSpellAnimationFile {
    animations: HashMap<String, SpellAnimation>,
}

impl<'de> Deserialize<'de> for SpellAnimationFile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedSpellAnimationFile::deserialize(deserializer)?;
        for (spell, animation) in &raw.animations {
            animation.validate(spell).map_err(D::Error::custom)?;
        }
        Ok(Self {
            animations: raw.animations,
        })
    }
}

/// Registers the spell animation content for loading.
///
/// Registers every nested type, not just the container: the dev-only world
/// inspector (`hex_dev`) only descends into and edits a type's fields once it is
/// itself registered, so a designer tuning `SpellAnimationFile` live needs
/// `SpellAnimation`/`MotionArchetype`/`VfxStyle` reflected too, not just their
/// parent.
pub fn plugin(app: &mut App) {
    app.register_type::<SpellAnimationFile>()
        .register_type::<SpellAnimation>()
        .register_type::<MotionArchetype>()
        .register_type::<VfxStyle>();
    app.load_settings::<SpellAnimationFile>("config/spell_animations.ron", CONFIG_EXTENSIONS);
    app.add_systems(Update, warn_on_dangling_animation_references);
}

/// A spell name in `spell_animations.ron` with no matching entry in the live
/// [`SpellBook`] costs a missing visual, never a broken cast — so this warns rather
/// than blocking the load the way an unknown element/substance in `spells.rs` does.
fn warn_on_dangling_animation_references(
    animations: Option<Res<SpellAnimationFile>>,
    spells: Option<Res<SpellBook>>,
) {
    let (Some(animations), Some(spells)) = (animations, spells) else {
        return;
    };
    if !animations.is_changed() && !spells.is_changed() {
        return;
    }
    for name in animations.animations.keys() {
        if spells.id(name).is_none() {
            warn!("spell_animations.ron references unknown spell '{name}'");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flash() -> SpellAnimation {
        SpellAnimation {
            motion: MotionArchetype::InstantFlash { hold_seconds: 0.4 },
            style: VfxStyle::Spark,
            color_override: None,
            particle_count: 10,
            particle_speed: 1.0,
            particle_lifetime_seconds: 0.5,
            scale: 0.06,
            spread: 0.2,
        }
    }

    #[test]
    fn a_well_formed_entry_parses() {
        let ron = r#"(
            animations: {
                "Ember": (
                    motion: InstantFlash(hold_seconds: 0.4),
                    style: Flame,
                    color_override: None,
                    particle_count: 10,
                    particle_speed: 1.0,
                    particle_lifetime_seconds: 0.5,
                    scale: 0.06,
                    spread: 0.2,
                ),
            },
        )"#;
        let file: SpellAnimationFile =
            ron::from_str(ron).expect("well-formed content should parse");
        assert_eq!(file.animations.len(), 1);
    }

    #[test]
    fn a_beam_entry_parses() {
        let ron = r#"(
            animations: {
                "Lightning Bolt": (
                    motion: Beam(flash_seconds: 0.1, impact_hold_seconds: 0.25, thickness: 0.12),
                    style: Spark,
                    color_override: Some((1.0, 0.95, 0.55, 1.0)),
                    particle_count: 40,
                    particle_speed: 2.0,
                    particle_lifetime_seconds: 0.25,
                    scale: 0.05,
                    spread: 0.15,
                ),
            },
        )"#;
        let file: SpellAnimationFile =
            ron::from_str(ron).expect("well-formed content should parse");
        assert_eq!(file.animations.len(), 1);
    }

    #[test]
    fn an_unknown_field_is_rejected() {
        let ron = r#"(
            animations: {
                "Ember": (
                    motion: InstantFlash(hold_seconds: 0.4),
                    style: Flame,
                    color_override: None,
                    particle_count: 10,
                    particle_speed: 1.0,
                    particle_lifetime_seconds: 0.5,
                    scale: 0.06,
                    spread: 0.2,
                    unexpected: true,
                ),
            },
        )"#;
        ron::from_str::<SpellAnimationFile>(ron).expect_err("an unknown field must not parse");
    }

    #[test]
    fn a_zero_duration_is_rejected() {
        let mut animation = flash();
        animation.motion = MotionArchetype::InstantFlash { hold_seconds: 0.0 };
        assert!(animation.validate("Ember").is_err());
    }

    #[test]
    fn a_non_finite_duration_is_rejected() {
        let mut animation = flash();
        animation.motion = MotionArchetype::InstantFlash {
            hold_seconds: f32::NAN,
        };
        assert!(animation.validate("Ember").is_err());
    }

    /// The caps this schema used to impose were reached while tuning real effects
    /// rather than catching real mistakes, so a deliberately huge value is now
    /// simply the author's choice.
    #[test]
    fn a_large_but_finite_value_is_accepted_everywhere() {
        let mut animation = flash();
        animation.motion = MotionArchetype::InstantFlash {
            hold_seconds: 600.0,
        };
        animation.scale = 40.0;
        animation.spread = 250.0;
        animation.particle_speed = 5_000.0;
        assert!(animation.validate("Ember").is_ok());
    }

    #[test]
    fn an_infinite_duration_is_still_rejected() {
        let mut animation = flash();
        animation.motion = MotionArchetype::InstantFlash {
            hold_seconds: f32::INFINITY,
        };
        assert!(animation.validate("Ember").is_err());
    }

    #[test]
    fn zero_particles_is_rejected() {
        let mut animation = flash();
        animation.particle_count = 0;
        assert!(animation.validate("Ember").is_err());
    }

    #[test]
    fn a_negative_particle_speed_is_rejected() {
        let mut animation = flash();
        animation.particle_speed = -1.0;
        assert!(animation.validate("Ember").is_err());
    }

    #[test]
    fn a_zero_scale_is_rejected() {
        let mut animation = flash();
        animation.scale = 0.0;
        assert!(animation.validate("Ember").is_err());
    }

    #[test]
    fn a_zero_spread_is_rejected() {
        let mut animation = flash();
        animation.spread = 0.0;
        assert!(animation.validate("Ember").is_err());
    }

    #[test]
    fn a_beam_with_a_zero_thickness_is_rejected() {
        let mut animation = flash();
        animation.motion = MotionArchetype::Beam {
            flash_seconds: 0.1,
            impact_hold_seconds: 0.2,
            thickness: 0.0,
        };
        assert!(animation.validate("Lightning Bolt").is_err());
    }

    #[test]
    fn a_valid_entry_is_accepted() {
        assert!(flash().validate("Ember").is_ok());
    }
}
