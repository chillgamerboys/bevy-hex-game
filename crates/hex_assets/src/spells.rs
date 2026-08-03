//! Spells, loaded from `assets/config/spells.ron`.
//!
//! A spell is defined by its **requirements** (an element multiset drawn from adjacent
//! gems, whose length is the tier — at most six, a full ring), its **casting axis**
//! (evocation vs. enchantment), its **mana axis** (fixed vs. variable), whether it is
//! **co-castable**, its **targeting**, and a list of **effects** from a closed enum.
//!
//! # "Ritual" is derived, not stored
//!
//! The design names two independent axes and observes that "ritual" is the corner
//! where both hold: variable mana *and* co-castable. Rather than store a third flag
//! that can disagree with the two it summarises, [`Spell::is_ritual`] derives it.
//!
//! # Effects are a closed enum, never a script
//!
//! [`Effect`] is a fixed vocabulary of primitives (audit §8). A closed enum can be
//! bounds-checked at parse and gives downstream admission an exhaustive checklist —
//! the whole reason there is no scripting engine. Extension is one variant plus one
//! match arm. Delivered effects are applied downstream (hex_combat, when casting
//! lands); this crate also retains schema variants whose runtime lifecycle is deferred,
//! and shipped content must not advertise those variants meanwhile.
//!
//! Element and substance references are by **name**; resolving them against the
//! element and substance tables is [`ContentIndex`](crate::ContentIndex)'s job.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use hex_core::{HexCoord, Level, Screen, SpellId};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

use crate::fingerprint::FingerprintEncoder;
use crate::{LoadSettings, CONFIG_EXTENSIONS};

/// One gem a spell requires: a distinct adjacent source of `element` contributing
/// `mana`. Mirrors `hex_lattice::Requirement` so the future `SpellTable` mapping is
/// direct.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GemRequirement {
    /// The element the adjacent gem (or live fusion output) must provide, by name.
    pub element: String,
    /// How much mana that gem contributes to the cast.
    pub mana: u16,
}

/// How a spell spends the mana it draws. Mirrors `hex_lattice::Casting`.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CastingAxis {
    /// Drains and consumes the mana; its cost is throughput, recovered by channelling.
    Evocation,
    /// Ties the drawn mana up for as long as the enchantment lasts, lost if it breaks.
    Enchantment {
        /// Flat reduction applied to incoming disable counts while active. Zero for a
        /// non-defensive enchantment.
        defense: u16,
    },
}

/// Whether a spell draws a fixed amount of mana or a variable amount for a varied
/// effect.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManaAxis {
    /// A binary spell: it fires at full strength or not at all.
    Fixed,
    /// A variable spell: it scales with the mana it is given.
    Variable,
}

/// One authored voxel of a [`TargetShape::Path`], relative to the anchor and written
/// in the **unrotated frame** — the one where the facing is
/// [`Sextant::A`](hex_core::Sextant::A).
///
/// A path is the escape hatch for a shape the parameterised vocabulary cannot say,
/// and so the only shape whose vertical extent an author controls voxel by voxel.
/// `hex_units::volumes::rotated` turns it into the cast's facing; the `level` is not
/// affected by that turn, because the rotation is about the vertical axis.
#[derive(Reflect, Debug, Default, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoxelOffset {
    /// Horizontal displacement from the anchor, before rotation.
    pub coord: HexCoord,
    /// Vertical displacement from the anchor.
    pub level: Level,
}

/// The shape a spell's targeting covers. Pure data; `hex_units::volumes` resolves it
/// to exact voxels at cast time.
///
/// The parameters are the shape's own extent and are unrelated to
/// [`TargetingSpec::range`], which bounds how far away the anchor may be.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetShape {
    /// Cast on the caster itself; `range` is 0.
    SelfCast,
    /// A single target voxel.
    Single,
    /// A grid-space ball around the anchor, reaching as far up and down as it does
    /// sideways.
    Sphere {
        /// Radius in grid space: hexes out, and levels up or down. `0` is the anchor
        /// voxel alone.
        radius: u8,
    },
    /// The anchor voxel and the voxels stacked above it.
    Column {
        /// How many voxels tall, **counting the anchor**. A conjured wall is `2`,
        /// because the canonical walker is two voxels tall and climbs one.
        height: u8,
    },
    /// Out from the caster along the facing, at the caster's level. The caster's own
    /// voxel is never included.
    Line {
        /// How many hexes out.
        length: u8,
        /// Half-thickness in hexes; `0` is a single file.
        width: u8,
    },
    /// Widening out from the caster along the facing, at the caster's level.
    Cone {
        /// How many hexes out.
        length: u8,
        /// How many 60-degree sectors open to **each** side of the facing: `0` is a
        /// bare ray, `1` the familiar 120-degree cone, `3` a full disc.
        spread: u8,
    },
    /// An authored voxel list, rotated into the facing and hung on the anchor.
    Path {
        /// The offsets from the anchor, in the unrotated frame.
        offsets: Vec<VoxelOffset>,
    },
}

impl TargetShape {
    /// Whether this shape can resolve to more than one distinct voxel.
    ///
    /// This is a content-admission property rather than concrete geometry: it keeps
    /// lattice and Creator gates aligned without either crate resolving a live cast.
    #[must_use]
    pub fn can_cover_multiple_voxels(&self) -> bool {
        match self {
            Self::SelfCast | Self::Single => false,
            Self::Sphere { radius } => *radius > 0,
            Self::Column { height } => *height > 1,
            Self::Line { length, width } => *length > 1 || (*length == 1 && *width > 0),
            Self::Cone { length, spread } => *length > 1 || (*length == 1 && *spread > 0),
            Self::Path { offsets } => offsets
                .first()
                .is_some_and(|first| offsets.iter().skip(1).any(|candidate| candidate != first)),
        }
    }
}

/// Where a spell can be cast, reusing `hex_units::targeting`'s height-advantage
/// geometry at cast time. Pure data here.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TargetingSpec {
    /// Base range in hexes, before any high-ground bonus.
    pub range: u8,
    /// The shape the spell covers.
    pub shape: TargetShape,
    /// How material occupancy between caster and target affects the cast.
    pub trajectory: Trajectory,
}

/// How a spell travels from its caster to the selected anchor.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trajectory {
    /// A straight exact-voxel segment, blocked by touched intervening material.
    Direct,
    /// A two-segment arch through a deterministic apex.
    Arc {
        /// Exact levels the apex rises above the higher endpoint.
        rise: u8,
    },
    /// Deliberately ignores material obstruction.
    None,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedTargetingSpec {
    range: u8,
    shape: TargetShape,
    #[serde(default, deserialize_with = "present_value")]
    trajectory: Option<Trajectory>,
    // Read-only migration seam for creator saves and external content authored before
    // the trajectory vocabulary. Serialization always writes `trajectory`, so this is
    // not a second live authority.
    #[serde(default, deserialize_with = "present_value")]
    needs_los: Option<bool>,
}

fn present_value<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

impl<'de> Deserialize<'de> for TargetingSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedTargetingSpec::deserialize(deserializer)?;
        let trajectory = match (raw.trajectory, raw.needs_los) {
            (Some(trajectory), None) => trajectory,
            (None, Some(true)) => Trajectory::Direct,
            (None, Some(false)) => Trajectory::None,
            (Some(_), Some(_)) => {
                return Err(D::Error::custom(
                    "targeting cannot define both trajectory and legacy needs_los",
                ));
            }
            (None, None) => {
                return Err(D::Error::custom(
                    "targeting must define trajectory (or legacy needs_los)",
                ));
            }
        };
        Ok(Self {
            range: raw.range,
            shape: raw.shape,
            trajectory,
        })
    }
}

/// One primitive effect a spell applies when it resolves. A closed vocabulary
/// (audit §8) — extension is one variant here plus one match arm where effects are
/// applied.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    /// Disable a number of the target's hexes. `targeted` chooses specific hexes
    /// rather than a flat count.
    DisableHexes {
        /// How many hexes to disable.
        count: u8,
        /// Whether the caster picks which hexes, rather than an arbitrary count.
        targeted: bool,
    },
    /// Set the target alight, disabling one hex at the start of each of its own turns.
    ///
    /// Burn **ignores armour** — fire's identity is beating defences rather than
    /// overpowering them — but still goes through the defender's choice of which hexes.
    Burn {
        /// How many of the target's own turns it burns for.
        ///
        /// Each of those turns costs the target one hex, so `turns: 3` is three hexes
        /// spread over three of its turns rather than three at once — which is the whole
        /// difference between burn and a direct hit, and why it can be survived by
        /// finishing the fight quickly.
        ///
        /// **Named `turns` rather than `amount` deliberately.** It was `amount` and
        /// documented as locked mana; nothing ever implemented that reading, and a
        /// designer could not tell from the schema whether `2` meant mana, hexes per
        /// tick, or duration. Saves do not exist yet, so the rename is free now and
        /// content-compatibility debt later.
        turns: u16,
    },
    /// Restore a number of the target's disabled hexes.
    RestoreHexes {
        /// How many hexes to restore.
        count: u8,
    },
    /// Reduce incoming disable counts by a flat amount (a one-shot ward, distinct from
    /// an enchantment's persistent [`CastingAxis::Enchantment`] defense).
    ModifyIncomingDisables {
        /// The flat reduction.
        amount: u16,
    },
    /// Reveal the target's complete live lattice for a tier-scaled duration.
    Reveal {
        /// The divination tier, multiplied by the configured rounds per tier.
        tier: u8,
    },
    /// Light an area, lifting fog around the caster.
    Illuminate {
        /// Radius lit, in hexes.
        radius: u8,
    },
    /// Replace terrain at the target with a named substance.
    SetTerrain {
        /// The substance to place, by name (resolved against the substance table).
        substance: String,
    },
    /// Conjure a wall of a named substance.
    SpawnWall {
        /// The substance the wall is made of, by name.
        substance: String,
    },
    /// Announce elemental damage over the resolved canonical terrain volume.
    ///
    /// Gameplay names the element and power; the world-owned terrain resolver alone
    /// decides which materials resist, take damage, or are destroyed.
    Impact {
        /// Element by stable authored name, resolved through [`ContentIndex`](crate::ContentIndex).
        element: String,
        /// Exact voxel damage announced to the world-owned resolver.
        power: u8,
    },
    /// Legacy creator-save spelling for terrain removal.
    ///
    /// Retained only so schema-v1 libraries still deserialize and can surface a
    /// precise validation issue. Current content admission always rejects it.
    ClearTerrain,
    /// Push the target a number of hexes away.
    Displace {
        /// How many hexes to push.
        distance: u8,
    },
}

impl Effect {
    /// The substance name this effect references, if any — the cross-file reference
    /// [`ContentIndex`](crate::ContentIndex) must resolve.
    #[must_use]
    pub fn substance(&self) -> Option<&str> {
        match self {
            Self::SetTerrain { substance } | Self::SpawnWall { substance } => Some(substance),
            _ => None,
        }
    }

    /// The element name this effect references, if any — the cross-file reference
    /// [`ContentIndex`](crate::ContentIndex) must resolve.
    #[must_use]
    pub fn element(&self) -> Option<&str> {
        match self {
            Self::Impact { element, .. } => Some(element),
            _ => None,
        }
    }
}

/// A single spell definition, before element/substance names are resolved.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spell {
    /// The adjacent gems this spell draws on; its length is the tier (≤ 6).
    pub requirements: Vec<GemRequirement>,
    /// How the spell spends its mana.
    pub casting: CastingAxis,
    /// Whether the spell draws fixed or variable mana.
    pub mana: ManaAxis,
    /// Whether this spell can be cast alongside another in the same action.
    pub co_castable: bool,
    /// Where the spell can be cast.
    pub targeting: TargetingSpec,
    /// What the spell does when it resolves.
    pub effects: Vec<Effect>,
}

impl Spell {
    /// The spell's tier: how many adjacent gems it requires.
    ///
    /// Saturates at [`u8::MAX`], but [`SpellFile::validate`] caps tier at six, so a
    /// spell that parsed is always in range.
    #[must_use]
    pub fn tier(&self) -> u8 {
        u8::try_from(self.requirements.len()).unwrap_or(u8::MAX)
    }

    /// Whether this spell is a **ritual** — the design's name for the corner where a
    /// spell is both variable-mana and co-castable. Derived, never stored.
    #[must_use]
    pub fn is_ritual(&self) -> bool {
        matches!(self.mana, ManaAxis::Variable) && self.co_castable
    }
}

/// The largest tier a spell can have: a full ring of six adjacent gems.
const MAX_TIER: usize = 6;

/// The largest extent a shape may name, in hexes or levels.
///
/// Not a balance decision — a guard rail. A resolved volume is a materialised
/// `Vec<TilePos>`, so a radius of `200` typed in place of `2` is a prism of forty-odd
/// million voxels, allocated inside a frame. The resolvers deliberately do not clamp,
/// so this is where an implausible number has to be caught.
const MAX_SHAPE_EXTENT: u8 = 16;

/// Largest authored anchor range accepted by exact trajectory enumeration.
///
/// A technical allocation/work guardrail, not a balance ruling.
pub const MAX_TARGET_RANGE: u8 = 16;

/// Largest authored arc rise accepted by exact trajectory enumeration.
///
/// A technical traversal guardrail, not a balance ruling.
pub const MAX_ARC_RISE: u8 = 16;

/// The largest number of voxels an authored path may list.
const MAX_PATH_VOXELS: usize = 64;

/// Sectors of cone spread beyond which the cone is already a full disc.
const MAX_CONE_SPREAD: u8 = 3;

/// The widest line content may author, until a wide line's near end is decided.
///
/// `width` is a half-thickness around the spine, and the spine starts one hex ahead
/// of the caster — so at `width` 2 the first spine hex's disc reaches back past the
/// caster and covers *every* neighbour, including the one directly behind. A line
/// that burns the ally standing behind you is not what the word means, and the fix is
/// a decision about the near cap rather than a bug in the thickening: either the rear
/// arc is subtracted, or the shape is renamed for what it is. Width 1 is safe — its
/// near cap reaches the caster's voxel and no further, and that voxel is already
/// excluded — so content is held there until [HEX-19b] settles the wider case.
///
/// [HEX-19b]: https://linear.app/hex-game/issue/HEX-19/terrain-magic
const MAX_LINE_WIDTH: u8 = 1;

/// The raw file, before names are turned into ids.
///
/// `Deserialize` is hand-written (via `UnvalidatedSpellFile`) so tier bounds, mana
/// amounts and effect fields are checked at parse: an invalid `spells.ron` fails to
/// load and the previous valid [`SpellBook`] stays active.
#[derive(Asset, Resource, Reflect, Debug, Clone)]
#[reflect(Resource)]
pub struct SpellFile {
    /// Spells by name.
    pub spells: HashMap<String, Spell>,
}

/// The same shape as [`SpellFile`] with a derived `Deserialize` and no validation.
#[derive(Deserialize)]
struct UnvalidatedSpellFile {
    spells: HashMap<String, Spell>,
}

impl<'de> Deserialize<'de> for SpellFile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = UnvalidatedSpellFile::deserialize(deserializer)?;
        let file = Self { spells: raw.spells };
        file.validate().map_err(serde::de::Error::custom)?;
        Ok(file)
    }
}

impl SpellFile {
    /// Checks every intra-file invariant. Cross-file references (an element in
    /// `requirements`, a substance in an effect) are [`ContentIndex`](crate::ContentIndex)'s
    /// job — a single file cannot see the others.
    pub fn validate(&self) -> Result<(), String> {
        for (name, spell) in &self.spells {
            let tier = spell.requirements.len();
            if tier == 0 {
                return Err(format!(
                    "spell '{name}' requires no gems; tier must be at least 1"
                ));
            }
            if tier > MAX_TIER {
                return Err(format!(
                    "spell '{name}' has tier {tier}; the maximum is {MAX_TIER} (a full ring)"
                ));
            }
            for requirement in &spell.requirements {
                if requirement.mana == 0 {
                    return Err(format!(
                        "spell '{name}' draws 0 mana from a '{}' gem",
                        requirement.element
                    ));
                }
            }
            // SelfCast means "the caster's own hex"; a nonzero range on it would
            // parse cleanly and then mean nothing to whichever system resolves
            // targeting later — reject the contradiction while it is still text.
            if matches!(spell.targeting.shape, TargetShape::SelfCast) && spell.targeting.range != 0
            {
                return Err(format!(
                    "spell '{name}' is SelfCast but has range {}; self-casts have range 0",
                    spell.targeting.range
                ));
            }
            if spell.targeting.range > MAX_TARGET_RANGE {
                return Err(format!(
                    "spell '{name}' targeting range is {}; the technical maximum is {MAX_TARGET_RANGE}",
                    spell.targeting.range
                ));
            }
            validate_shape(name, &spell.targeting.shape)?;
            if matches!(spell.targeting.trajectory, Trajectory::Arc { rise: 0 }) {
                return Err(format!(
                    "spell '{name}' trajectory Arc.rise must be at least 1"
                ));
            }
            if let Trajectory::Arc { rise } = spell.targeting.trajectory {
                if rise > MAX_ARC_RISE {
                    return Err(format!(
                        "spell '{name}' trajectory Arc.rise is {rise}; the technical maximum is {MAX_ARC_RISE}"
                    ));
                }
            }
            validate_effects(name, spell)?;
        }
        Ok(())
    }
}

/// Checks a shape's extents are present and plausible.
///
/// The match is total on purpose: a new [`TargetShape`] variant should not compile
/// until someone has decided what an implausible one of it looks like.
fn validate_shape(name: &str, shape: &TargetShape) -> Result<(), String> {
    let zero = |field: &str| format!("spell '{name}' shape {field} must be at least 1");
    let over = |field: &str, value: u32, limit: u32| {
        format!("spell '{name}' shape {field} is {value}; the maximum is {limit}")
    };
    let extent = u32::from(MAX_SHAPE_EXTENT);
    match shape {
        TargetShape::SelfCast | TargetShape::Single => {}
        TargetShape::Sphere { radius } => {
            if u32::from(*radius) > extent {
                return Err(over("Sphere.radius", u32::from(*radius), extent));
            }
        }
        TargetShape::Column { height } => {
            if *height == 0 {
                return Err(zero("Column.height"));
            }
            if u32::from(*height) > extent {
                return Err(over("Column.height", u32::from(*height), extent));
            }
        }
        TargetShape::Line { length, width } => {
            if *length == 0 {
                return Err(zero("Line.length"));
            }
            if u32::from(*length) > extent {
                return Err(over("Line.length", u32::from(*length), extent));
            }
            if *width > MAX_LINE_WIDTH {
                return Err(over(
                    "Line.width",
                    u32::from(*width),
                    u32::from(MAX_LINE_WIDTH),
                ));
            }
        }
        TargetShape::Cone { length, spread } => {
            if *length == 0 {
                return Err(zero("Cone.length"));
            }
            if u32::from(*length) > extent {
                return Err(over("Cone.length", u32::from(*length), extent));
            }
            if *spread > MAX_CONE_SPREAD {
                return Err(over(
                    "Cone.spread",
                    u32::from(*spread),
                    u32::from(MAX_CONE_SPREAD),
                ));
            }
        }
        TargetShape::Path { offsets } => {
            if offsets.is_empty() {
                return Err(format!("spell '{name}' has a Path shape with no voxels"));
            }
            if offsets.len() > MAX_PATH_VOXELS {
                return Err(format!(
                    "spell '{name}' Path lists {} voxels; the maximum is {MAX_PATH_VOXELS}",
                    offsets.len()
                ));
            }
            for offset in offsets {
                let out = HexCoord::ORIGIN.distance(offset.coord);
                if out > extent {
                    return Err(over("Path offset", out, extent));
                }
                if offset.level.unsigned_abs() > extent {
                    return Err(over(
                        "Path offset level",
                        offset.level.unsigned_abs(),
                        extent,
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Checks a spell's effects have sane fields and that the spell does *something*.
fn validate_effects(name: &str, spell: &Spell) -> Result<(), String> {
    let mut exact_cell_decisions = 0_u8;
    let creation_effects = spell
        .effects
        .iter()
        .filter(|effect| matches!(effect, Effect::SetTerrain { .. } | Effect::SpawnWall { .. }))
        .count();
    if creation_effects > 0 && creation_effects != spell.effects.len() {
        return Err(format!(
            "spell '{name}' mixes terrain creation with another effect; the initial \
             creation slice requires one placement volume and no second effect volume"
        ));
    }
    if creation_effects > 1 {
        return Err(format!(
            "spell '{name}' has multiple terrain-creation effects; one cast may publish \
             one material over one placement volume"
        ));
    }
    if creation_effects > 0
        && !matches!(
            spell.targeting.shape,
            TargetShape::Single | TargetShape::Column { .. }
        )
    {
        return Err(format!(
            "spell '{name}' creates terrain with a shape that is not a fully authorized \
             vertical volume; use Single or Column"
        ));
    }
    if creation_effects > 0 && !matches!(spell.casting, CastingAxis::Evocation) {
        return Err(format!(
            "spell '{name}' creates permanent terrain but is not an Evocation"
        ));
    }
    for effect in &spell.effects {
        let zero = |field: &str| format!("spell '{name}' effect {field} must be at least 1");
        match effect {
            Effect::DisableHexes { count, targeted } => {
                if *count == 0 {
                    return Err(zero("DisableHexes.count"));
                }
                if !targeted {
                    exact_cell_decisions = exact_cell_decisions.saturating_add(1);
                }
            }
            Effect::Burn { turns } if *turns == 0 => return Err(zero("Burn.turns")),
            Effect::RestoreHexes { count } if *count == 0 => {
                return Err(zero("RestoreHexes.count"));
            }
            Effect::RestoreHexes { .. } => {
                exact_cell_decisions = exact_cell_decisions.saturating_add(1);
            }
            Effect::ModifyIncomingDisables { amount } if *amount == 0 => {
                return Err(zero("ModifyIncomingDisables.amount"));
            }
            Effect::Reveal { tier } if *tier == 0 => return Err(zero("Reveal.tier")),
            Effect::Illuminate { radius } if *radius == 0 => return Err(zero("Illuminate.radius")),
            Effect::Impact { element, power } => {
                if element.trim().is_empty() {
                    return Err(format!("spell '{name}' Impact names a blank element"));
                }
                if *power == 0 {
                    return Err(zero("Impact.power"));
                }
            }
            Effect::Displace { distance } if *distance == 0 => {
                return Err(zero("Displace.distance"));
            }
            Effect::SetTerrain { substance } | Effect::SpawnWall { substance }
                if substance.is_empty() =>
            {
                return Err(format!("spell '{name}' names an empty substance"));
            }
            Effect::ClearTerrain => {
                return Err(format!(
                    "spell '{name}' uses legacy ClearTerrain, which is decode-only and no longer supported"
                ));
            }
            _ => {}
        }
    }
    if exact_cell_decisions > 1 {
        return Err(format!(
            "spell '{name}' has multiple exact-cell decision effects; only one damage \
             or restoration choice can be pending at a time"
        ));
    }

    // A spell must do something: at least one effect, or a defensive enchantment whose
    // whole point is the disable reduction it carries in its casting axis.
    let is_defensive = matches!(spell.casting, CastingAxis::Enchantment { defense } if defense > 0);
    if spell.effects.is_empty() && !is_defensive {
        return Err(format!(
            "spell '{name}' has no effects and no defensive enchantment — it would do nothing"
        ));
    }
    Ok(())
}

/// Spells indexed by the [`SpellId`] assigned from sorted names.
#[derive(Resource, Reflect, Debug, Clone, Default)]
#[reflect(Resource)]
pub struct SpellBook {
    /// Names indexed by id; `by_id[i]` is the name of `SpellId(i)`.
    by_id: Vec<String>,
    #[reflect(ignore)]
    by_name: HashMap<String, SpellId>,
    #[reflect(ignore)]
    spells: HashMap<SpellId, Spell>,
    /// Canonical semantics of the `SpellFile` this book was built from.
    #[reflect(ignore)]
    source_fingerprint: u64,
}

impl SpellBook {
    /// The id a name maps to, or [`None`] if there is no such spell.
    #[must_use]
    pub fn id(&self, name: &str) -> Option<SpellId> {
        self.by_name.get(name).copied()
    }

    /// The name of a spell, for logs and content resolution.
    #[must_use]
    pub fn name(&self, id: SpellId) -> Option<&str> {
        self.by_id.get(id.0 as usize).map(String::as_str)
    }

    /// The definition of a spell, or [`None`] if the id is not in the book.
    #[must_use]
    pub fn spell(&self, id: SpellId) -> Option<&Spell> {
        self.spells.get(&id)
    }

    /// Every spell, in id order — for content resolution and the dev-feature dump.
    pub fn iter(&self) -> impl Iterator<Item = (SpellId, &str, &Spell)> + '_ {
        self.by_id.iter().enumerate().filter_map(|(index, name)| {
            let id = SpellId(u16::try_from(index).unwrap_or(u16::MAX));
            self.spells.get(&id).map(|spell| (id, name.as_str(), spell))
        })
    }

    /// How many spells the book holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Whether the book is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Whether this book was built from the current authored spell semantics.
    #[must_use]
    pub fn matches_source(&self, file: &SpellFile) -> bool {
        self.source_fingerprint == spell_file_fingerprint(file)
    }

    pub(crate) const fn source_fingerprint(&self) -> u64 {
        self.source_fingerprint
    }

    /// Builds a book from a loaded file, assigning ids from sorted names.
    #[must_use]
    pub fn from_file(file: &SpellFile) -> Self {
        let mut by_id: Vec<String> = file.spells.keys().cloned().collect();
        by_id.sort();

        let mut by_name: HashMap<String, SpellId> = HashMap::default();
        let mut spells: HashMap<SpellId, Spell> = HashMap::default();
        for (index, name) in by_id.iter().enumerate() {
            let id = SpellId(u16::try_from(index).unwrap_or(u16::MAX));
            by_name.insert(name.clone(), id);
            if let Some(spell) = file.spells.get(name) {
                spells.insert(id, spell.clone());
            }
        }

        Self {
            by_id,
            by_name,
            spells,
            source_fingerprint: spell_file_fingerprint(file),
        }
    }
}

fn spell_file_fingerprint(file: &SpellFile) -> u64 {
    let mut encoder = FingerprintEncoder::new(b"hex-spell-file-v1");
    let mut entries: Vec<_> = file.spells.iter().collect();
    entries.sort_by_key(|(name, _)| *name);
    encoder.usize(entries.len());
    for (name, spell) in entries {
        encoder.string(name);
        encoder.usize(spell.requirements.len());
        for requirement in &spell.requirements {
            encoder.string(&requirement.element);
            encoder.u16(requirement.mana);
        }
        match spell.casting {
            CastingAxis::Evocation => encoder.u8(0),
            CastingAxis::Enchantment { defense } => {
                encoder.u8(1);
                encoder.u16(defense);
            }
        }
        encoder.u8(match spell.mana {
            ManaAxis::Fixed => 0,
            ManaAxis::Variable => 1,
        });
        encoder.bool(spell.co_castable);
        encoder.u8(spell.targeting.range);
        fingerprint_shape(&mut encoder, &spell.targeting.shape);
        match spell.targeting.trajectory {
            Trajectory::Direct => encoder.u8(0),
            Trajectory::Arc { rise } => {
                encoder.u8(1);
                encoder.u8(rise);
            }
            Trajectory::None => encoder.u8(2),
        }
        encoder.usize(spell.effects.len());
        for effect in &spell.effects {
            fingerprint_effect(&mut encoder, effect);
        }
    }
    encoder.finish()
}

fn fingerprint_shape(encoder: &mut FingerprintEncoder, shape: &TargetShape) {
    match shape {
        TargetShape::SelfCast => encoder.u8(0),
        TargetShape::Single => encoder.u8(1),
        TargetShape::Sphere { radius } => {
            encoder.u8(2);
            encoder.u8(*radius);
        }
        TargetShape::Column { height } => {
            encoder.u8(3);
            encoder.u8(*height);
        }
        TargetShape::Line { length, width } => {
            encoder.u8(4);
            encoder.u8(*length);
            encoder.u8(*width);
        }
        TargetShape::Cone { length, spread } => {
            encoder.u8(5);
            encoder.u8(*length);
            encoder.u8(*spread);
        }
        TargetShape::Path { offsets } => {
            encoder.u8(6);
            encoder.usize(offsets.len());
            for offset in offsets {
                encoder.i32(offset.coord.x());
                encoder.i32(offset.coord.y());
                encoder.i32(offset.level);
            }
        }
    }
}

fn fingerprint_effect(encoder: &mut FingerprintEncoder, effect: &Effect) {
    match effect {
        Effect::DisableHexes { count, targeted } => {
            encoder.u8(0);
            encoder.u8(*count);
            encoder.bool(*targeted);
        }
        Effect::Burn { turns } => {
            encoder.u8(1);
            encoder.u16(*turns);
        }
        Effect::RestoreHexes { count } => {
            encoder.u8(2);
            encoder.u8(*count);
        }
        Effect::ModifyIncomingDisables { amount } => {
            encoder.u8(3);
            encoder.u16(*amount);
        }
        Effect::Reveal { tier } => {
            encoder.u8(4);
            encoder.u8(*tier);
        }
        Effect::Illuminate { radius } => {
            encoder.u8(5);
            encoder.u8(*radius);
        }
        Effect::SetTerrain { substance } => {
            encoder.u8(6);
            encoder.string(substance);
        }
        Effect::SpawnWall { substance } => {
            encoder.u8(8);
            encoder.string(substance);
        }
        Effect::ClearTerrain => {
            // Preserve the schema-v1 discriminant for decode-only creator records.
            encoder.u8(7);
        }
        Effect::Displace { distance } => {
            encoder.u8(9);
            encoder.u8(*distance);
        }
        Effect::Impact { element, power } => {
            encoder.u8(10);
            encoder.string(element);
            encoder.u8(*power);
        }
    }
}

/// Registers the spell book for loading.
pub fn plugin(app: &mut App) {
    app.register_type::<SpellBook>();
    app.load_settings::<SpellFile>("config/spells.ron", CONFIG_EXTENSIONS);
    register_book_builder(app);
}

/// Rebuilds the book when the file loads or hot-reloads, but never during gameplay.
fn register_book_builder(app: &mut App) {
    app.add_systems(
        Update,
        build_spellbook.run_if(not(in_state(Screen::Gameplay))),
    );
}

/// Turns the loaded file into the indexed book, and rebuilds it on hot-reload.
fn build_spellbook(
    mut commands: Commands,
    file: Option<Res<SpellFile>>,
    book: Option<Res<SpellBook>>,
) {
    let Some(file) = file else { return };
    if !file.is_changed() && book.is_some() {
        return;
    }
    commands.insert_resource(SpellBook::from_file(&file));
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::reflect::ReflectRef;
    use hex_test_app::HeadlessAppBuilder;

    fn targeting() -> TargetingSpec {
        TargetingSpec {
            range: 3,
            shape: TargetShape::Single,
            trajectory: Trajectory::Direct,
        }
    }

    fn gem(element: &str, mana: u16) -> GemRequirement {
        GemRequirement {
            element: element.to_owned(),
            mana,
        }
    }

    fn ember() -> Spell {
        Spell {
            requirements: vec![gem("Fire", 1)],
            casting: CastingAxis::Evocation,
            mana: ManaAxis::Fixed,
            co_castable: false,
            targeting: targeting(),
            effects: vec![Effect::DisableHexes {
                count: 1,
                targeted: false,
            }],
        }
    }

    fn test_file() -> SpellFile {
        let mut spells = HashMap::default();
        spells.insert("Ember".to_owned(), ember());
        spells.insert(
            "Flamethrower".to_owned(),
            Spell {
                requirements: vec![gem("Fire", 1), gem("Fire", 1)],
                casting: CastingAxis::Evocation,
                mana: ManaAxis::Variable,
                co_castable: true,
                targeting: TargetingSpec {
                    range: 2,
                    shape: TargetShape::Line {
                        length: 2,
                        width: 0,
                    },
                    trajectory: Trajectory::Direct,
                },
                effects: vec![Effect::Burn { turns: 2 }],
            },
        );
        SpellFile { spells }
    }

    fn shipped_file() -> SpellFile {
        ron::from_str(include_str!("../../../assets/config/spells.ron"))
            .expect("the shipped spell file should parse and validate")
    }

    #[test]
    fn shipped_spells_parse() {
        let book = SpellBook::from_file(&shipped_file());
        assert!(book.id("Ember").is_some());
        assert!(book.id("Fireball").is_some());
    }

    /// Every effect intentionally exercised by the shipped roster stays represented.
    /// Decode-only `ClearTerrain` plus deferred ward, illumination, and displacement
    /// effects deliberately remain absent until spells with delivered mechanics own them.
    #[test]
    fn shipped_spells_cover_every_advertised_effect_variant() {
        let file = shipped_file();
        let mut seen = std::collections::HashSet::new();
        for spell in file.spells.values() {
            for effect in &spell.effects {
                seen.insert(std::mem::discriminant(effect));
            }
        }
        // Every spell-authored effect variant intentionally present in current content.
        // Destruction is `TerrainImpact`, not a spell-side clear instruction.
        let all = [
            Effect::DisableHexes {
                count: 1,
                targeted: false,
            },
            Effect::Burn { turns: 1 },
            Effect::RestoreHexes { count: 1 },
            Effect::Reveal { tier: 1 },
            Effect::SetTerrain {
                substance: "stone".to_owned(),
            },
            Effect::SpawnWall {
                substance: "stone".to_owned(),
            },
            Effect::Impact {
                element: "Fire".to_owned(),
                power: 2,
            },
        ];
        for effect in &all {
            assert!(
                seen.contains(&std::mem::discriminant(effect)),
                "shipped spells never use {effect:?}"
            );
        }
        for deferred in [
            Effect::ModifyIncomingDisables { amount: 1 },
            Effect::Illuminate { radius: 1 },
            Effect::Displace { distance: 1 },
            Effect::ClearTerrain,
        ] {
            assert!(
                !seen.contains(&std::mem::discriminant(&deferred)),
                "deferred effect leaked into shipped content: {deferred:?}"
            );
        }
    }

    #[test]
    fn shipped_renewal_only_restores_hexes() {
        let file = shipped_file();
        let renewal = file
            .spells
            .get("Renewal")
            .expect("the shipped spell roster defines Renewal");

        assert_eq!(
            renewal.effects,
            vec![Effect::RestoreHexes { count: 2 }],
            "Renewal must not advertise the deferred one-shot ward effect",
        );
    }

    #[test]
    fn shipped_fireball_announces_fire_impact_without_deferred_displacement() {
        let file = shipped_file();
        let fireball = file
            .spells
            .get("Fireball")
            .expect("the shipped spell roster defines Fireball");

        assert!(fireball.effects.contains(&Effect::Impact {
            element: "Fire".to_owned(),
            power: 2,
        }));
        assert!(
            !fireball
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::Displace { .. })),
            "Fireball must not advertise deferred forced movement"
        );
    }

    #[test]
    fn impact_rejects_blank_elements_and_zero_power_before_indexing() {
        let mut blank = test_file();
        blank
            .spells
            .get_mut("Ember")
            .expect("the fixture contains Ember")
            .effects = vec![Effect::Impact {
            element: "   ".to_owned(),
            power: 2,
        }];
        let error = blank
            .validate()
            .expect_err("blank impact elements must fail spell validation");
        assert!(error.contains("blank element"), "{error}");

        let mut powerless = test_file();
        powerless
            .spells
            .get_mut("Ember")
            .expect("the fixture contains Ember")
            .effects = vec![Effect::Impact {
            element: "Fire".to_owned(),
            power: 0,
        }];
        let error = powerless
            .validate()
            .expect_err("zero-power impacts must fail spell validation");
        assert!(error.contains("Impact.power must be at least 1"), "{error}");
    }

    #[test]
    fn impact_round_trips_reflects_and_has_complete_fingerprint_fields() {
        let impact = Effect::Impact {
            element: "Fire".to_owned(),
            power: 2,
        };
        let encoded = ron::to_string(&impact).expect("Impact should serialize");
        let decoded: Effect = ron::from_str(&encoded).expect("Impact should deserialize");
        assert_eq!(decoded, impact);

        let ReflectRef::Enum(reflected) = impact.reflect_ref() else {
            panic!("Effect reflection must remain enum-shaped");
        };
        assert_eq!(reflected.variant_name(), "Impact");
        assert_eq!(
            reflected
                .field("element")
                .and_then(|field| field.try_downcast_ref::<String>())
                .map(String::as_str),
            Some("Fire")
        );
        assert_eq!(
            reflected
                .field("power")
                .and_then(|field| field.try_downcast_ref::<u8>()),
            Some(&2),
            "derived reflection must retain both Impact fields"
        );

        let fingerprint = |element: &str, power: u8| {
            let mut file = test_file();
            file.spells
                .get_mut("Ember")
                .expect("the fixture contains Ember")
                .effects = vec![Effect::Impact {
                element: element.to_owned(),
                power,
            }];
            SpellBook::from_file(&file).source_fingerprint()
        };
        assert_eq!(fingerprint("Fire", 2), fingerprint("Fire", 2));
        assert_ne!(fingerprint("Fire", 2), fingerprint("Water", 2));
        assert_ne!(fingerprint("Fire", 2), fingerprint("Fire", 3));
    }

    #[test]
    fn ids_do_not_depend_on_file_order() {
        let first = SpellBook::from_file(&test_file());
        let second = SpellBook::from_file(&test_file());
        for name in ["Ember", "Flamethrower"] {
            assert_eq!(
                first.id(name),
                second.id(name),
                "{name} moved between builds"
            );
        }
    }

    #[test]
    fn ritual_is_variable_and_co_castable() {
        let book = SpellBook::from_file(&test_file());
        let flamethrower = book
            .id("Flamethrower")
            .expect("test file defines Flamethrower");
        let ember = book.id("Ember").expect("test file defines Ember");
        assert!(book.spell(flamethrower).expect("present").is_ritual());
        assert!(
            !book.spell(ember).expect("present").is_ritual(),
            "Ember is a binary evocation"
        );
    }

    #[test]
    fn tier_is_the_requirement_count() {
        assert_eq!(ember().tier(), 1);
    }

    #[test]
    fn validate_rejects_over_tier_spells() {
        let mut file = test_file();
        let mut over = ember();
        over.requirements = std::iter::repeat_with(|| gem("Fire", 1)).take(7).collect();
        file.spells.insert("Inferno".to_owned(), over);
        assert!(file.validate().is_err(), "tier 7 exceeds the six-gem ring");
    }

    #[test]
    fn validate_rejects_a_ranged_self_cast() {
        let mut file = test_file();
        let mut confused = ember();
        confused.targeting.shape = TargetShape::SelfCast;
        confused.targeting.range = 2;
        file.spells.insert("Navel Gaze".to_owned(), confused);
        assert!(
            file.validate().is_err(),
            "a self-cast with range 2 is a contradiction in the file"
        );
    }

    #[test]
    fn validate_rejects_a_zero_rise_arc_instead_of_aliasing_direct() {
        let mut file = test_file();
        file.spells
            .get_mut("Ember")
            .expect("the fixture contains Ember")
            .targeting
            .trajectory = Trajectory::Arc { rise: 0 };
        let error = file.validate().expect_err("a zero-rise arc is ambiguous");
        assert!(error.contains("Arc.rise must be at least 1"), "{error}");
    }

    #[test]
    fn trajectory_range_and_rise_guardrails_reject_pathological_content() {
        let mut distant = test_file();
        distant
            .spells
            .get_mut("Ember")
            .expect("the fixture contains Ember")
            .targeting
            .range = MAX_TARGET_RANGE + 1;
        let error = distant
            .validate()
            .expect_err("trajectory range above the technical bound must fail");
        assert!(error.contains("technical maximum"), "{error}");

        let mut towering = test_file();
        towering
            .spells
            .get_mut("Ember")
            .expect("the fixture contains Ember")
            .targeting
            .trajectory = Trajectory::Arc {
            rise: MAX_ARC_RISE + 1,
        };
        let error = towering
            .validate()
            .expect_err("arc rise above the technical bound must fail");
        assert!(error.contains("technical maximum"), "{error}");
    }

    #[test]
    fn invalid_trajectory_reload_keeps_the_last_valid_spellbook() {
        let valid = test_file();
        let mut builder = HeadlessAppBuilder::new();
        builder.app_mut().add_systems(Update, build_spellbook);
        builder.app_mut().insert_resource(valid.clone());
        let mut app = builder.build();
        app.update();
        assert!(app.world().resource::<SpellBook>().matches_source(&valid));

        let mut invalid = ember();
        invalid.targeting.range = u8::MAX;
        let encoded = format!(
            "(spells:{{\"Ember\":{}}})",
            ron::to_string(&invalid).expect("the raw invalid spell serializes")
        );
        assert!(
            ron::from_str::<SpellFile>(&encoded).is_err(),
            "the asset loader must reject the candidate before resource replacement"
        );
        app.update();
        assert!(
            app.world().resource::<SpellBook>().matches_source(&valid),
            "a rejected reload must leave the accepted book live"
        );
    }

    #[test]
    fn legacy_boolean_targeting_migrates_on_read_but_never_serializes_back() {
        let direct: TargetingSpec =
            ron::from_str("(range: 3, shape: Single, needs_los: true)").expect("legacy targeting");
        let none: TargetingSpec =
            ron::from_str("(range: 3, shape: Single, needs_los: false)").expect("legacy targeting");
        assert_eq!(direct.trajectory, Trajectory::Direct);
        assert_eq!(none.trajectory, Trajectory::None);

        let serialized = ron::to_string(&direct).expect("new targeting serializes");
        assert!(serialized.contains("trajectory:Direct"), "{serialized}");
        assert!(!serialized.contains("needs_los"), "{serialized}");
        assert!(
            ron::from_str::<TargetingSpec>(
                "(range: 3, shape: Single, trajectory: Direct, needs_los: true)"
            )
            .is_err(),
            "two trajectory authorities must be rejected"
        );
    }

    #[test]
    fn validate_rejects_a_do_nothing_spell() {
        let mut file = test_file();
        let mut inert = ember();
        inert.effects.clear();
        inert.casting = CastingAxis::Evocation;
        file.spells.insert("Fizzle".to_owned(), inert);
        assert!(
            file.validate().is_err(),
            "an evocation with no effects does nothing"
        );
    }

    #[test]
    fn a_defensive_enchantment_may_have_no_effects() {
        let mut file = test_file();
        let mut shield = ember();
        shield.effects.clear();
        shield.casting = CastingAxis::Enchantment { defense: 2 };
        file.spells.insert("Shield".to_owned(), shield);
        assert!(
            file.validate().is_ok(),
            "a defensive enchantment's point is its defense"
        );
    }

    #[test]
    fn validate_rejects_multiple_effects_that_need_defender_choices() {
        let mut file = test_file();
        file.spells
            .get_mut("Ember")
            .expect("the fixture contains Ember")
            .effects
            .push(Effect::DisableHexes {
                count: 2,
                targeted: false,
            });

        let error = file
            .validate()
            .expect_err("one cast cannot overwrite its own pending damage decision");
        assert!(
            error.contains("multiple exact-cell decision effects"),
            "{error}"
        );
    }

    #[test]
    fn validate_rejects_a_damage_choice_combined_with_a_restoration_choice() {
        let mut file = test_file();
        file.spells
            .get_mut("Ember")
            .expect("the fixture contains Ember")
            .effects
            .push(Effect::RestoreHexes { count: 1 });

        let error = file
            .validate()
            .expect_err("one cast cannot overwrite damage with restoration");
        assert!(
            error.contains("multiple exact-cell decision effects"),
            "{error}"
        );
    }

    #[test]
    fn terrain_creation_cannot_mix_effect_volumes_or_use_a_caster_relative_shape() {
        let mut file = test_file();
        let mut mixed = ember();
        mixed.effects = vec![
            Effect::SetTerrain {
                substance: "stone".to_owned(),
            },
            Effect::Burn { turns: 1 },
        ];
        file.spells.insert("Mixed".to_owned(), mixed);
        let error = file.validate().expect_err("mixed effect volumes must fail");
        assert!(error.contains("mixes terrain creation"), "{error}");

        let mut file = test_file();
        let mut directed = ember();
        directed.effects = vec![Effect::SetTerrain {
            substance: "stone".to_owned(),
        }];
        directed.targeting.shape = TargetShape::Line {
            length: 2,
            width: 0,
        };
        file.spells.insert("Directed".to_owned(), directed);
        let error = file
            .validate()
            .expect_err("creation needs a selected-surface anchor");
        assert!(
            error.contains("fully authorized vertical volume"),
            "{error}"
        );

        let mut file = test_file();
        let mut spillover = ember();
        spillover.effects = vec![Effect::SetTerrain {
            substance: "stone".to_owned(),
        }];
        spillover.targeting.shape = TargetShape::Sphere { radius: 1 };
        file.spells.insert("Spillover".to_owned(), spillover);
        let error = file
            .validate()
            .expect_err("positive-radius construction is not fully authorized");
        assert!(error.contains("use Single or Column"), "{error}");

        let mut file = test_file();
        let mut bound = ember();
        bound.casting = CastingAxis::Enchantment { defense: 1 };
        bound.effects = vec![Effect::SpawnWall {
            substance: "stone".to_owned(),
        }];
        file.spells.insert("Bound".to_owned(), bound);
        let error = file
            .validate()
            .expect_err("permanent construction requires Evocation");
        assert!(error.contains("not an Evocation"), "{error}");

        let mut file = test_file();
        let mut duplicate = ember();
        duplicate.effects = vec![
            Effect::SetTerrain {
                substance: "stone".to_owned(),
            },
            Effect::SpawnWall {
                substance: "stone".to_owned(),
            },
        ];
        file.spells.insert("Duplicate".to_owned(), duplicate);
        let error = file
            .validate()
            .expect_err("one cast cannot publish two construction materials");
        assert!(
            error.contains("multiple terrain-creation effects"),
            "{error}"
        );
    }

    /// The shipped roster is what proves the schema is expressible, so pin the two
    /// shapes carrying extents. A rename that silently reverted `Sphere` to a
    /// parameterless shape would still parse a file that never named a radius.
    #[test]
    fn shipped_spells_carry_their_shape_extents() {
        let file = shipped_file();
        let shape = |name: &str| {
            file.spells
                .get(name)
                .map(|spell| spell.targeting.shape.clone())
                .expect("the shipped roster defines this spell")
        };
        assert_eq!(shape("Fireball"), TargetShape::Sphere { radius: 2 });
        assert_eq!(
            shape("Flamethrower"),
            TargetShape::Line {
                length: 2,
                width: 0
            }
        );
    }

    /// A column of no voxels is a wall that does not exist. The resolver is total on
    /// it, but no content should be able to author one.
    #[test]
    fn validate_rejects_a_zero_height_column() {
        let mut file = test_file();
        let mut flat = ember();
        flat.targeting.shape = TargetShape::Column { height: 0 };
        file.spells.insert("Flat Wall".to_owned(), flat);
        assert!(file.validate().is_err(), "a wall needs at least one voxel");
    }

    /// The guard rail that matters: a resolved volume is a materialised vector, so a
    /// radius typed with an extra digit allocates tens of millions of voxels inside a
    /// frame. The resolvers do not clamp, so this is the only place it is caught.
    #[test]
    fn validate_rejects_an_implausible_extent() {
        let mut file = test_file();
        let mut huge = ember();
        huge.targeting.shape = TargetShape::Sphere { radius: 200 };
        file.spells.insert("Apocalypse".to_owned(), huge);
        assert!(
            file.validate().is_err(),
            "radius 200 is a typo, not a spell"
        );
    }

    /// A width-2 line's near end rounds back past the caster and takes in every
    /// neighbour, the one directly behind included. Whether to subtract that rear arc
    /// or rename the shape is a design call; until it is made, content may not express
    /// the case. Width 1 stops at the caster's own voxel, which is excluded anyway.
    #[test]
    fn validate_rejects_a_line_wide_enough_to_reach_behind_the_caster() {
        let mut file = test_file();
        let mut wide = ember();
        wide.targeting.shape = TargetShape::Line {
            length: 4,
            width: 2,
        };
        file.spells.insert("Backdraft".to_owned(), wide);
        assert!(
            file.validate().is_err(),
            "a line that burns the ally behind you is not a line"
        );

        let mut narrow = ember();
        narrow.targeting.shape = TargetShape::Line {
            length: 4,
            width: 1,
        };
        let mut ok = test_file();
        ok.spells.insert("Lance".to_owned(), narrow);
        assert!(
            ok.validate().is_ok(),
            "width 1 reaches no further than the caster's own voxel"
        );
    }

    /// Spread beyond a full disc says nothing the disc does not, and the resolver
    /// treats it as a disc. Accepting it in content would let a file mean something
    /// other than what it says.
    #[test]
    fn validate_rejects_cone_spread_past_a_full_disc() {
        let mut file = test_file();
        let mut wide = ember();
        wide.targeting.shape = TargetShape::Cone {
            length: 3,
            spread: 4,
        };
        file.spells.insert("Everywhere".to_owned(), wide);
        assert!(
            file.validate().is_err(),
            "four sectors a side is not a shape"
        );
    }

    /// A path with no voxels resolves to nothing, so a spell carrying one would parse,
    /// cost mana and do nothing at all.
    #[test]
    fn validate_rejects_an_empty_path() {
        let mut file = test_file();
        let mut nowhere = ember();
        nowhere.targeting.shape = TargetShape::Path { offsets: vec![] };
        file.spells.insert("Nowhere".to_owned(), nowhere);
        assert!(file.validate().is_err(), "an empty path affects nothing");
    }

    /// And a path voxel authored far outside the shape's plausible extent is the same
    /// typo the radius check catches, wearing a different hat.
    #[test]
    fn validate_rejects_a_distant_path_voxel() {
        let mut file = test_file();
        let mut distant = ember();
        distant.targeting.shape = TargetShape::Path {
            offsets: vec![VoxelOffset {
                coord: HexCoord::new_cubic(40, -40, 0),
                level: 0,
            }],
        };
        file.spells.insert("Far Wall".to_owned(), distant);
        assert!(file.validate().is_err(), "forty hexes out is not an offset");
    }

    /// A rotation never moves a voxel vertically, so a path's authored level survives
    /// the round trip through RON unchanged. The geometry lives in `hex_units`; this
    /// only pins that the schema can say it.
    #[test]
    fn a_path_shape_round_trips_through_ron() {
        let shape: TargetShape =
            ron::from_str("Path(offsets: [(coord: (q: 1, r: 0), level: 2)])").expect("parses");
        assert_eq!(
            shape,
            TargetShape::Path {
                offsets: vec![VoxelOffset {
                    coord: HexCoord::from_axial(1, 0),
                    level: 2,
                }],
            }
        );
    }

    #[test]
    fn validate_rejects_zero_mana_requirements() {
        let mut file = test_file();
        let mut free = ember();
        free.requirements = vec![gem("Fire", 0)];
        file.spells.insert("Freebie".to_owned(), free);
        assert!(file.validate().is_err(), "a gem must contribute mana");
    }
}
