//! Continuous actor and spell authority for the opt-in local duel experiment.
//! World facts arrive through `hex_core::arena::ArenaTerrainView`; terrain mutations leave
//! through the existing world messages. No tactical unit or renderer owns combat.

use std::collections::BTreeMap;

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_math::{Vec2, Vec3};
use hex_core::arena::{
    ArenaMaterials, ArenaReset, ArenaSystems, ArenaTerrainView, ArenaTick, ArenaVoxelGeometry,
};
use hex_core::{
    TerrainBatchId, TerrainEdit, TerrainImpact, TerrainImpactOutcome, TerrainImpactResult, TilePos,
};
use serde::{Deserialize, Serialize};

mod battle;
mod bot;
#[cfg(any(test, feature = "test-support"))]
mod bot_baseline;
mod bot_config;
mod collision;
mod controller;
mod creatures;
mod encounter_config;
mod encounters;
mod motion;
mod shapes;
mod spells;
mod telemetry;

pub use battle::{
    ArenaBattleSetup, ArenaControl, BattlePreset, BattleResult, BattleSetupError, BattleSummary,
    BattleTeamSummary, TeamRoster, MAX_BATTLE_ACTORS,
};
pub use bot::BotDebugSnapshot;
pub use bot_config::BotTuning;
pub use creatures::{
    ActorId, AttackPhase, AttackSnapshot, AuraSnapshot, BarrierSnapshot, CreatureAbility,
    EncounterSummary, PartyId, PartyPhase, PartySnapshot, Species, TeamId,
};
pub use encounter_config::EncounterTuning;
pub use encounters::{EncounterActorStats, PartyKnowledgeSnapshot};
pub use telemetry::{ActorCombatStats, RoundSummary};
use telemetry::{CombatCue, CombatCueKind};

use bot::Bot;
use collision::{CollisionWorld, SKIN};
use controller::Body;
use spells::{PendingWall, ShotParameters};

/// Fixed simulation step in seconds.
pub const STEP: f32 = 1.0 / 120.0;
/// Accepted M01 standing height in world units (two arena voxels).
pub const BODY_HEIGHT: f32 = 0.8;
/// Accepted M01 standing radius in world units.
pub const BODY_RADIUS: f32 = 0.25;
/// First-person and projectile launch eye height above the feet.
pub const EYE_HEIGHT: f32 = 0.62;

/// Three independently cooled spells, in HUD/key order.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Spell {
    /// A ballistic seed that adds stone cover wherever its impact footprint fits.
    Shield,
    /// A ballistic projectile with an explosive impact.
    #[default]
    Fireball,
    /// A caster-centered burst that excludes its caster.
    AreaBlast,
}

impl Spell {
    /// Stable slot used by cooldown arrays and keys 1–3.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Shield => 0,
            Self::Fireball => 1,
            Self::AreaBlast => 2,
        }
    }

    /// Player-facing spell name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Shield => "Shield",
            Self::Fireball => "Fireball",
            Self::AreaBlast => "Area Blast",
        }
    }
}

/// A complete input sample. Bot and human commands share this exact path.
#[derive(Debug, Clone, Copy)]
pub struct ActorIntent {
    /// Horizontal axes: x is right and y is forward relative to aim yaw.
    pub movement: Vec2,
    /// World-space look direction; finite nonzero samples update authoritative aim.
    pub aim: Vec3,
    /// Request the accepted M01 run speed.
    pub run: bool,
    /// Single jump edge, consumed by the next fixed tick.
    pub jump: bool,
    /// Press edge, retained until the next fixed tick even for a quick tap.
    pub cast_pressed: bool,
    /// Release edge. Only an armed press can release a spell.
    pub cast_released: bool,
    /// Current held state, preserved across fixed ticks in one render frame.
    pub cast_held: bool,
    /// Optional selected spell, applied before same-tick casting input.
    pub selected: Option<Spell>,
}

impl Default for ActorIntent {
    fn default() -> Self {
        Self {
            movement: Vec2::ZERO,
            aim: Vec3::NEG_Z,
            run: false,
            jump: false,
            cast_pressed: false,
            cast_released: false,
            cast_held: false,
            selected: None,
        }
    }
}

/// Presentation writes current held input and queues human edges here.
#[derive(Resource, Debug, Default, Clone)]
pub struct ArenaInput {
    /// The local human's next input sample.
    pub human: ActorIntent,
}

/// Read-only projection of an actor's currently armed spell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChargeState {
    /// Spell captured on the accepted press.
    pub spell: Spell,
    /// Held simulation time, capped by the configured charge duration.
    pub elapsed: f32,
}

/// Runtime comparison controls. Geometry indices select compact, standard, or large.
#[derive(Resource, Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArenaTuning {
    /// Authored map creature hypotheses; unused by the accepted Duel.
    pub encounters: EncounterTuning,
    /// Fixed-strength opponent behavior; no adaptive difficulty or extra spell power.
    pub bot: BotTuning,
    /// Shield width/height preset (0, 1, or 2).
    pub shield_size: usize,
    /// Fireball sphere preset (0, 1, or 2).
    pub fireball_size: usize,
    /// Area Blast sphere preset (0, 1, or 2).
    pub blast_size: usize,
    /// Reference seed/fireball speed before the charge range multiplier.
    pub projectile_speed: f32,
    /// Seconds held to reach maximum projectile charge.
    pub charge_seconds: f32,
    /// Tap range relative to the reference same-height, same-angle shot.
    pub tap_range_multiplier: f32,
    /// Fully charged range relative to the same reference shot.
    pub max_range_multiplier: f32,
    /// Horizontal impulse speed from a shield's first actor impact; no HP damage.
    pub shield_push: f32,
    /// Downward projectile acceleration in world units per second squared.
    pub projectile_gravity: f32,
    /// Shield cooldown, charged immediately on release.
    pub shield_cooldown: f32,
    /// Fireball cooldown, charged immediately on release.
    pub fireball_cooldown: f32,
    /// Area Blast cooldown, charged immediately on release.
    pub blast_cooldown: f32,
    /// Maximum fireball HP damage before radial falloff.
    pub fireball_damage: f32,
    /// Maximum Area Blast HP damage before radial falloff.
    pub blast_damage: f32,
    /// Maximum fireball impulse speed before radial falloff.
    pub fireball_knockback: f32,
    /// Maximum Area Blast impulse speed before radial falloff.
    pub blast_knockback: f32,
    /// World-owned voxel HP damage per admitted blast.
    pub terrain_power: u8,
}

impl Default for ArenaTuning {
    fn default() -> Self {
        Self {
            encounters: EncounterTuning::default(),
            bot: BotTuning::default(),
            shield_size: 1,
            fireball_size: 1,
            blast_size: 1,
            projectile_speed: 32.0,
            charge_seconds: 0.75,
            tap_range_multiplier: 1.0 / 3.0,
            max_range_multiplier: 1.3,
            shield_push: 2.0,
            projectile_gravity: 12.0,
            shield_cooldown: 5.0,
            fireball_cooldown: 1.25,
            blast_cooldown: 7.0,
            fireball_damage: 35.0,
            blast_damage: 45.0,
            fireball_knockback: 8.0,
            blast_knockback: 11.0,
            terrain_power: 2,
        }
    }
}

impl ArenaTuning {
    /// Reject nonfinite, out-of-range, or unusable authoring values.
    pub fn validate(&self) -> Result<(), String> {
        self.bot.validate()?;
        self.encounters.validate()?;
        if [self.shield_size, self.fireball_size, self.blast_size]
            .into_iter()
            .any(|i| i > 2)
        {
            return Err("Spell size indices must be 0, 1, or 2.".into());
        }
        for (name, value, max) in [
            ("projectile_speed", self.projectile_speed, 100.0),
            ("charge_seconds", self.charge_seconds, 5.0),
            ("tap_range_multiplier", self.tap_range_multiplier, 4.0),
            ("max_range_multiplier", self.max_range_multiplier, 4.0),
            ("projectile_gravity", self.projectile_gravity, 100.0),
            ("shield_cooldown", self.shield_cooldown, 60.0),
            ("fireball_cooldown", self.fireball_cooldown, 60.0),
            ("blast_cooldown", self.blast_cooldown, 60.0),
            ("fireball_damage", self.fireball_damage, 100.0),
            ("blast_damage", self.blast_damage, 100.0),
        ] {
            if !value.is_finite() || value <= 0.0 || value > max {
                return Err(format!("{name} must be finite and in (0, {max}]."));
            }
        }
        if self.tap_range_multiplier > self.max_range_multiplier {
            return Err("Tap range must not exceed full-charge range.".into());
        }
        for value in [
            self.fireball_knockback,
            self.blast_knockback,
            self.shield_push,
        ] {
            if !value.is_finite() || !(0.0..=40.0).contains(&value) {
                return Err("Knockback must be finite and in 0..=40.".into());
            }
        }
        if self.terrain_power == 0 || self.terrain_power > 8 {
            return Err("terrain_power must be in 1..=8.".into());
        }
        Ok(())
    }

    /// One shared charge-to-speed rule for release, prediction, and bot aiming.
    #[must_use]
    pub fn launch_speed(&self, elapsed: f32) -> f32 {
        let progress = if elapsed.is_finite() {
            (elapsed / self.charge_seconds).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let range = self.tap_range_multiplier
            + (self.max_range_multiplier - self.tap_range_multiplier) * progress;
        self.projectile_speed * range.sqrt()
    }

    /// Charge duration closest to the original reference speed, useful for comparisons.
    #[must_use]
    pub fn reference_charge_seconds(&self) -> f32 {
        let span = self.max_range_multiplier - self.tap_range_multiplier;
        if span <= f32::EPSILON {
            0.0
        } else {
            ((1.0 - self.tap_range_multiplier) / span).clamp(0.0, 1.0) * self.charge_seconds
        }
    }

    /// Wall width in hex columns and height in voxel levels, thickness one column.
    #[must_use]
    pub fn shield_dimensions(&self) -> (i32, i32) {
        match self.shield_size {
            0 => (3, 4),
            2 => (7, 6),
            _ => (5, 5),
        }
    }

    /// Fireball radius in world units.
    #[must_use]
    pub fn fireball_radius(&self) -> f32 {
        match self.fireball_size {
            0 => 1.5,
            2 => 3.5,
            _ => 2.5,
        }
    }

    /// Area Blast radius in world units.
    #[must_use]
    pub fn blast_radius(&self) -> f32 {
        match self.blast_size {
            0 => 2.5,
            2 => 5.5,
            _ => 4.0,
        }
    }

    fn cooldown(&self, spell: Spell) -> f32 {
        match spell {
            Spell::Shield => self.shield_cooldown,
            Spell::Fireball => self.fireball_cooldown,
            Spell::AreaBlast => self.blast_cooldown,
        }
    }
}

/// One authoritative continuous actor. Presentation reads these fields only.
#[derive(Debug, Clone)]
pub struct Actor {
    /// Stable arena identity: human 0, disposable bot 1.
    pub id: u8,
    /// Physical/behavior profile; human and shadow preserve accepted Duel values.
    pub species: Species,
    /// Friendly bodies are skipped by projectiles except the caster itself.
    pub team: TeamId,
    /// Encounter group; the human and original Duel have no party.
    pub party: Option<PartyId>,
    /// Maximum recoverable life for this species.
    pub max_hp: f32,
    /// Authoritative feet position; this is not tactical TilePos occupancy.
    pub feet: Vec3,
    /// Feet position at the beginning of the latest simulation tick.
    pub previous_feet: Vec3,
    /// Unit world-space look direction.
    pub aim: Vec3,
    /// Remaining HP, clamped to 0–100.
    pub hp: f32,
    /// Selected spell for the next release.
    pub selected: Spell,
    /// Remaining seconds in Shield/Fireball/Area Blast order.
    pub cooldowns: [f32; 3],
    /// Latest accepted support contact.
    pub grounded: bool,
    /// Dragon flight mode; all other profiles retain ordinary grounded movement.
    pub flying: bool,
    charge: Option<ChargeState>,
    cast_needs_release: bool,
    body: Body,
    dimensions: Vec3,
    body_yaw: f32,
    previous_yaw: f32,
    attack: Option<AttackSnapshot>,
    damage_multiplier: f32,
    last_damage_tick: Option<u64>,
    last_activity_tick: u64,
}

impl Actor {
    fn spawn(id: u8, feet: Vec3, aim: Vec3) -> Self {
        Self {
            id,
            species: if id == 0 {
                Species::Human
            } else {
                Species::Shadow
            },
            team: u8::from(id != 0),
            party: None,
            max_hp: 100.0,
            feet,
            previous_feet: feet,
            aim,
            hp: 100.0,
            selected: Spell::Fireball,
            cooldowns: [0.0; 3],
            grounded: false,
            flying: false,
            charge: None,
            cast_needs_release: false,
            body: Body::default(),
            dimensions: Vec3::new(BODY_RADIUS * 2.0, BODY_HEIGHT, BODY_RADIUS * 2.0),
            body_yaw: (-aim.x).atan2(-aim.z),
            previous_yaw: (-aim.x).atan2(-aim.z),
            attack: None,
            damage_multiplier: 1.0,
            last_damage_tick: None,
            last_activity_tick: 0,
        }
    }

    /// Physical eye and launch position, without presentation interpolation.
    #[must_use]
    pub fn eye(&self) -> Vec3 {
        if self.species == Species::Dragon {
            self.center() + self.body_rotation() * Vec3::NEG_Z * (self.dimensions.z * 0.5 - 0.05)
        } else if self.species == Species::Goblin {
            self.feet + Vec3::Y * (self.dimensions.y * 0.775)
        } else {
            self.feet + Vec3::Y * EYE_HEIGHT
        }
    }

    /// Body center used to describe the player-centered blast.
    #[must_use]
    pub fn center(&self) -> Vec3 {
        self.feet + Vec3::Y * (self.dimensions.y * 0.5)
    }

    /// Current authoritative charge, without allowing input/presentation to set it.
    #[must_use]
    pub fn charge(&self) -> Option<ChargeState> {
        self.charge
    }

    fn cancel_charge(&mut self) {
        self.cast_needs_release |= self.charge.is_some();
        self.charge = None;
    }

    fn casting(&mut self, intent: ActorIntent, tuning: &ArenaTuning) -> Option<(Spell, f32)> {
        // A cancelled or refused hold cannot re-arm itself. A neutral sample or
        // release restores eligibility for the next genuine press.
        if !intent.cast_held && !intent.cast_pressed && !intent.cast_released {
            self.charge = None;
            self.cast_needs_release = false;
            return None;
        }
        if intent.cast_pressed && !self.cast_needs_release && self.charge.is_none() {
            self.cast_needs_release = true;
            if self
                .cooldowns
                .get(self.selected.index())
                .is_some_and(|v| *v <= STEP * 0.01)
            {
                self.charge = Some(ChargeState {
                    spell: self.selected,
                    elapsed: 0.0,
                });
            }
        }
        if intent.cast_released {
            self.cast_needs_release = false;
            let charge = self.charge.take()?;
            if self.hp > 0.0 && charge.spell == self.selected {
                let cooldown = self.cooldowns.get(charge.spell.index())?;
                if *cooldown <= STEP * 0.01 {
                    return Some((charge.spell, tuning.launch_speed(charge.elapsed)));
                }
            }
        } else if intent.cast_held {
            if let Some(charge) = &mut self.charge {
                charge.elapsed = (charge.elapsed + STEP).min(tuning.charge_seconds);
            }
        } else {
            self.cancel_charge();
        }
        None
    }

    /// Current independent knockback momentum, for exact-state review hooks.
    #[must_use]
    pub fn impulse_velocity(&self) -> Vec3 {
        self.body.impulse_velocity + Vec3::Y * self.body.vertical_velocity
    }

    /// Accepted automatic step rise on the latest tick, for camera-only easing.
    /// Jumps, falls, and impulses do not publish a step.
    #[must_use]
    pub fn step_rise_this_tick(&self) -> f32 {
        self.body.step_rise
    }
}

/// A released projectile. Its launch parameters are frozen through impact.
#[derive(Debug, Clone)]
pub struct Projectile {
    /// Stable identifier within the current reset generation.
    pub id: u64,
    /// Casting actor identity.
    pub owner: u8,
    /// Current physical center.
    pub position: Vec3,
    /// Physical center before the latest swept tick.
    pub previous_position: Vec3,
    /// Current world-space velocity.
    pub velocity: Vec3,
    /// Shield or Fireball.
    pub spell: Spell,
    /// Seconds since release.
    pub age: f32,
    parameters: ShotParameters,
    owner_cleared: bool,
}

/// Short-lived cosmetic event; neither rendering nor expiry mutates terrain or HP.
#[derive(Debug, Clone)]
pub struct VisualEffect {
    /// World-space effect center.
    pub center: Vec3,
    /// Physical radius, or the seed marker radius for Shield.
    pub radius: f32,
    /// Elapsed cosmetic time in seconds.
    pub age: f32,
    /// Total cosmetic lifetime in seconds.
    pub lifetime: f32,
    /// Spell that caused this event.
    pub kind: Spell,
}

/// Terminal result. The entire session waits for a full reset after KO.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArenaOutcome {
    /// Winning actor ID; zero also denotes all hostile encounter parties defeated.
    Winner(u8),
    /// No side survived the terminal simulation tick.
    Draw,
}

/// Readable immutable simulation projection plus privately owned control state.
#[derive(Resource, Debug)]
pub struct ArenaSession {
    /// Stable continuous actors for the selected duel or encounter roster.
    pub actors: Vec<Actor>,
    /// Live ballistic seeds/fireballs.
    pub projectiles: Vec<Projectile>,
    /// Live cosmetic explosions and shield markers.
    pub effects: Vec<VisualEffect>,
    /// Winner/draw once a combatant reaches zero HP or falls out of the arena.
    pub outcome: Option<ArenaOutcome>,
    /// Fixed simulation tick number, reset to zero with the authored arena.
    pub tick: u64,
    /// Allow the fixed-strength opponent controller to submit input.
    pub bot_enabled: bool,
    /// Last human-readable fizzle or validation message.
    pub notice: String,
    /// Number of correlated terrain outcomes accepted since reset.
    pub terrain_outcomes: u64,
    /// Number of shield impacts that added at least one safe cell since reset.
    pub shields_raised: u64,
    collision: CollisionWorld,
    generation: Option<u64>,
    bot: Bot,
    next_projectile: u64,
    next_impact: u64,
    pending_impacts: BTreeMap<TerrainBatchId, TerrainImpact>,
    pending_walls: Vec<PendingWall>,
    shield_notice_until: Option<u64>,
    combat_cues: Vec<CombatCue>,
    next_cue: u64,
    combat_stats: [ActorCombatStats; 2],
    encounter: encounters::EncounterState,
    #[cfg(any(test, feature = "test-support"))]
    baseline_bot: Option<bot_baseline::Bot>,
}

impl Default for ArenaSession {
    fn default() -> Self {
        Self {
            actors: Vec::new(),
            projectiles: Vec::new(),
            effects: Vec::new(),
            outcome: None,
            tick: 0,
            bot_enabled: true,
            notice: String::new(),
            terrain_outcomes: 0,
            shields_raised: 0,
            collision: CollisionWorld::default(),
            generation: None,
            bot: Bot::default(),
            next_projectile: 0,
            next_impact: 0,
            pending_impacts: BTreeMap::new(),
            pending_walls: Vec::new(),
            shield_notice_until: None,
            combat_cues: Vec::new(),
            next_cue: 0,
            combat_stats: Default::default(),
            encounter: encounters::EncounterState::default(),
            #[cfg(any(test, feature = "test-support"))]
            baseline_bot: None,
        }
    }
}

impl ArenaSession {
    /// Advance shared simulation housekeeping after mode initialization succeeds.
    fn begin_simulation_tick(&mut self) {
        self.tick += 1;
        self.combat_cues
            .retain(|cue| self.tick.saturating_sub(cue.tick) <= 120);
        if self
            .shield_notice_until
            .is_some_and(|until| self.tick >= until)
        {
            if self.notice == "No room for new shield blocks" {
                self.notice.clear();
            }
            self.shield_notice_until = None;
        }
    }

    fn shield_no_room_notice(&mut self) {
        self.notice = "No room for new shield blocks".into();
        self.shield_notice_until = Some(self.tick.saturating_add(240));
    }

    /// Cancel armed casts immediately, including while the fixed simulation is paused.
    /// Input adapters must also discard queued edges and wait for a fresh press.
    pub fn cancel_charges(&mut self) {
        for actor in &mut self.actors {
            actor.cancel_charge();
        }
        self.bot.cancel_charge();
        self.encounter.cancel_charges();
        #[cfg(any(test, feature = "test-support"))]
        if let Some(bot) = &mut self.baseline_bot {
            bot.cancel_charge();
        }
    }

    /// Currently available staged wall cells and cosmetic rise progress in 0–1.
    /// Formation rechecks the fixed impact footprint before adding physical cover.
    pub fn emerging_shields(&self) -> impl Iterator<Item = (&[TilePos], f32)> {
        self.pending_walls.iter().map(|wall| {
            (
                wall.voxels.as_slice(),
                (wall.age / spells::EMERGENCE_SECONDS).clamp(0.0, 1.0),
            )
        })
    }

    /// Retract a close third-person camera with the same solid collision cache.
    #[must_use]
    pub fn camera_position(&self, eye: Vec3, desired: Vec3) -> Vec3 {
        let delta = desired - eye;
        self.collision
            .sweep_sphere(eye, delta, 0.1)
            .map_or(desired, |hit| {
                eye + delta * hit.fraction + hit.normal * 0.04
            })
    }

    /// Convert a close-third-person crosshair ray into physical eye/muzzle aim.
    /// No ballistic compensation is applied; intervening muzzle cover still blocks.
    #[must_use]
    pub fn aim_from_camera(
        &self,
        actor_id: u8,
        camera_origin: Vec3,
        camera_direction: Vec3,
    ) -> Vec3 {
        spells::aim_from_camera(self, actor_id, camera_origin, camera_direction)
    }

    fn reset(&mut self, generation: u64, world: &ArenaTerrainView, geometry: ArenaVoxelGeometry) {
        let [human, bot] = world.spawns;
        let aim = (bot - human).normalize_or_zero();
        let bot_enabled = self.bot_enabled;
        #[cfg(any(test, feature = "test-support"))]
        let baseline = self.baseline_bot.is_some();
        *self = Self {
            actors: vec![Actor::spawn(0, human, aim), Actor::spawn(1, bot, -aim)],
            generation: Some(generation),
            bot_enabled,
            bot: Bot::default(),
            #[cfg(any(test, feature = "test-support"))]
            baseline_bot: baseline.then(bot_baseline::Bot::default),
            ..Default::default()
        };
        self.collision.refresh(world, geometry);
    }

    fn accept_outcome(&mut self, outcome: &TerrainImpactOutcome) {
        let Some(expected) = self.pending_impacts.remove(&outcome.batch) else {
            self.notice = "Unmatched terrain outcome; reset the arena.".into();
            return;
        };
        if !outcome.is_consistent_with(&expected) {
            self.notice = "Terrain outcome failed its exact-volume contract.".into();
            return;
        }
        self.terrain_outcomes += 1;
        if let TerrainImpactResult::Rejected(reason) = &outcome.result {
            self.notice = format!("Terrain impact refused: {reason:?}");
        }
    }

    fn advance(
        &mut self,
        human: ActorIntent,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        materials: ArenaMaterials,
        tuning: &ArenaTuning,
    ) -> CommandsOut {
        self.collision.refresh(world, geometry);
        let mut commands = CommandsOut::default();
        for effect in &mut self.effects {
            effect.age += STEP;
        }
        self.effects.retain(|effect| effect.age < effect.lifetime);
        if self.outcome.is_some() {
            return commands;
        }
        if world.selection.map != hex_core::arena::ArenaMap::Duel {
            return self.advance_encounter(human, world, geometry, materials, tuning);
        }
        self.begin_simulation_tick();
        let use_current_bot = self.bot_enabled;
        #[cfg(any(test, feature = "test-support"))]
        let use_current_bot = use_current_bot && self.baseline_bot.is_none();
        let bot = if use_current_bot {
            self.bot.intent(
                &self.actors,
                &self.projectiles,
                &self.collision,
                world,
                geometry,
                tuning,
                &self.combat_cues,
                self.tick,
            )
        } else {
            ActorIntent::default()
        };
        #[cfg(any(test, feature = "test-support"))]
        let bot = if let Some(baseline) = self.baseline_bot.as_mut().filter(|_| self.bot_enabled) {
            baseline.intent(
                &self.actors,
                &self.projectiles,
                &self.collision,
                world,
                geometry,
                tuning,
            )
        } else {
            bot
        };
        let mut casts = Vec::new();
        for actor in &mut self.actors {
            actor.previous_feet = actor.feet;
            if actor.hp <= 0.0 {
                continue;
            }
            let intent = if actor.id == 0 { human } else { bot };
            if intent.aim.is_finite() && intent.aim.length_squared() > 0.0001 {
                actor.aim = intent.aim.normalize();
            }
            if let Some(spell) = intent.selected {
                if spell != actor.selected && actor.charge.is_some() {
                    actor.cancel_charge();
                }
                actor.selected = spell;
            }
            for cooldown in &mut actor.cooldowns {
                *cooldown = (*cooldown - STEP).max(0.0);
            }
            let planar = Vec3::new(actor.aim.x, 0.0, actor.aim.z).normalize_or_zero();
            let forward = if planar.length_squared() > 0.5 {
                planar
            } else {
                Vec3::NEG_Z
            };
            let right = forward.cross(Vec3::Y);
            let movement = if intent.movement.is_finite() {
                intent.movement.clamp_length_max(1.0)
            } else {
                Vec2::ZERO
            };
            actor.body.tick(
                &mut actor.feet,
                forward * movement.y + right * movement.x,
                intent.run,
                intent.jump,
                &self.collision,
            );
            actor.grounded = actor.body.grounded;
            if actor.feet.y < -8.0 || !actor.feet.is_finite() {
                actor.hp = 0.0;
            }
            if actor.hp > 0.0 {
                if intent.cast_pressed
                    && actor
                        .cooldowns
                        .get(actor.selected.index())
                        .is_some_and(|v| *v > STEP * 0.01)
                    && actor.id == 0
                {
                    self.notice = format!("{} is cooling down.", actor.selected.name());
                }
                if let Some((spell, speed)) = actor.casting(intent, tuning) {
                    casts.push((actor.id, spell, speed));
                }
            }
        }
        separate_actors(&mut self.actors, &self.collision);
        // Existing shots share this tick's previous-to-current actor interval.
        // New casts originate at the current eye and start traveling next tick;
        // replaying completed actor motion against that origin creates false hits.
        self.advance_projectiles(world, geometry, materials, &mut commands);
        // A projectile already in flight can knock out an actor before release.
        // Admit this phase's living casters together, then preserve simultaneous
        // new-cast ordering without charging a dead actor's cancelled release.
        casts.retain(|(owner, _, _)| {
            self.actors
                .iter()
                .any(|actor| actor.id == *owner && actor.hp > 0.0)
        });
        for (owner, spell, speed) in casts {
            if let Some(cooldown) = self
                .actors
                .iter_mut()
                .find(|actor| actor.id == owner)
                .and_then(|actor| actor.cooldowns.get_mut(spell.index()))
            {
                *cooldown = tuning.cooldown(spell);
            }
            self.release(
                owner,
                spell,
                tuning,
                speed,
                world,
                geometry,
                materials,
                &mut commands,
            );
        }
        self.advance_walls(world, geometry, materials, &mut commands);
        let alive: Vec<_> = self
            .actors
            .iter()
            .filter(|a| a.hp > 0.0)
            .map(|a| a.id)
            .collect();
        self.outcome = match alive.as_slice() {
            [] => Some(ArenaOutcome::Draw),
            [id] => Some(ArenaOutcome::Winner(*id)),
            _ => None,
        };
        if self.outcome.is_some() {
            self.cancel_charges();
            self.projectiles.clear();
            self.pending_walls.clear();
        }
        commands
    }
}

#[derive(Default)]
struct CommandsOut {
    edits: Vec<TerrainEdit>,
    impacts: Vec<TerrainImpact>,
}

fn separate_actors(actors: &mut [Actor], world: &CollisionWorld) {
    let [a, b] = actors else {
        return;
    };
    if a.hp <= 0.0 || b.hp <= 0.0 || (a.feet.y - b.feet.y).abs() >= BODY_HEIGHT {
        return;
    }
    let delta = Vec3::new(a.feet.x - b.feet.x, 0.0, a.feet.z - b.feet.z);
    let distance = delta.length();
    if distance >= BODY_RADIUS * 2.0 {
        return;
    }
    let direction = if distance > SKIN {
        delta / distance
    } else {
        Vec3::X
    };
    let correction = direction * ((BODY_RADIUS * 2.0 - distance) * 0.5 + SKIN);
    a.feet = collision::slide(world, a.feet, correction, BODY_HEIGHT, BODY_RADIUS);
    b.feet = collision::slide(world, b.feet, -correction, BODY_HEIGHT, BODY_RADIUS);
}

/// One shared prediction: the same integration, nearest-hit and footprint routines
/// used by actual releases, with other actors held at their present positions.
#[derive(Debug, Default, Clone)]
pub struct Preview {
    /// Sampled launch-to-impact trajectory points in world units.
    pub points: Vec<Vec3>,
    /// Currently available shield cells after terrain, bounds, and body clipping.
    pub wall_voxels: Vec<TilePos>,
    /// Earliest physical contact or Area Blast center.
    pub impact: Option<Vec3>,
    /// Whether the predicted impact can produce the selected effect.
    pub valid: bool,
}

/// Trace the human's currently selected spell using authoritative aim and physics.
#[must_use]
pub fn preview(
    session: &ArenaSession,
    world: &ArenaTerrainView,
    geometry: &ArenaVoxelGeometry,
    tuning: &ArenaTuning,
) -> Preview {
    spells::preview(session, world, *geometry, tuning)
}

/// Install the headless simulation in the caller-owned fixed arena schedule.
pub fn plugin(app: &mut App) {
    app.init_resource::<ArenaSession>()
        .init_resource::<ArenaInput>()
        .init_resource::<ArenaTuning>()
        .add_message::<TerrainEdit>()
        .add_message::<TerrainImpact>()
        .add_message::<TerrainImpactOutcome>()
        .add_systems(ArenaTick, simulate.in_set(ArenaSystems::Simulate));
}

fn simulate(
    mut session: ResMut<ArenaSession>,
    mut input: ResMut<ArenaInput>,
    tuning: Res<ArenaTuning>,
    view: Res<ArenaTerrainView>,
    geometry: Res<ArenaVoxelGeometry>,
    materials: Res<ArenaMaterials>,
    reset: Res<ArenaReset>,
    mut outcomes: MessageReader<TerrainImpactOutcome>,
    mut edits: MessageWriter<TerrainEdit>,
    mut impacts: MessageWriter<TerrainImpact>,
) {
    if session.generation != Some(reset.generation) {
        session.reset(reset.generation, &view, *geometry);
        // Adopt spawn orientation rather than a stale look from the prior round.
        if let Some(actor) = session.actors.first() {
            input.human.aim = actor.aim;
        }
        input.human.cast_pressed = false;
        input.human.cast_released = false;
        input.human.cast_held = false;
        input.human.jump = false;
    }
    for outcome in outcomes.read() {
        session.accept_outcome(outcome);
    }
    let human = input.human;
    input.human.cast_pressed = false;
    input.human.cast_released = false;
    input.human.jump = false;
    input.human.selected = None;
    if let Err(reason) = tuning.validate() {
        session.notice = reason;
        return;
    }
    let emitted = session.advance(human, &view, *geometry, *materials, &tuning);
    for edit in emitted.edits {
        edits.write(edit);
    }
    for impact in emitted.impacts {
        impacts.write(impact);
    }
}

#[cfg(test)]
mod charge_tests;
#[cfg(test)]
mod knowledge_tests;
#[cfg(test)]
mod shield_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod shape_contract_tests;
