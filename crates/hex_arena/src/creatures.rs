//! Species identity and bounded read-only creature presentation.

use bevy_math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Stable actor identities are allocated once per run and never reused after death.
pub type ActorId = u8;
/// Combat allegiance; party membership is independent of friendly-fire policy.
pub type TeamId = u8;
/// Stable encounter group identity within the selected map.
pub type PartyId = u16;

/// One world-oriented native hex prism in a compound creature body.
/// Its pointy horizontal hex has circumradius one world unit, matching terrain.
#[derive(Debug, Clone, Copy)]
pub struct BodyHexPrism {
    /// Prism base offset from the actor's feet, expressed in world axes.
    pub offset: Vec3,
    /// Full vertical height in world units.
    pub height: f32,
}

/// Authoritative finite projection of a charged or active direct beam.
/// Presentation does not extend this segment or query hidden targets.
#[derive(Debug, Clone, Copy)]
pub struct BeamSnapshot {
    /// Physical mouth at the current actor pose.
    pub origin: Vec3,
    /// Unit direction, fixed after the attack locks its aim.
    pub direction: Vec3,
    /// Current nearest impact or actual world-boundary endpoint.
    pub end: Vec3,
    /// Physical beam radius in world units.
    pub radius: f32,
    /// Whether the action has committed its direction.
    pub locked: bool,
}

/// Authored continuous actor profile.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Species {
    /// The local player, with unchanged M01 controls.
    #[default]
    Human,
    /// Accepted fixed-strength spell opponent.
    Shadow,
    /// Low, long creature that walks and flies.
    Dragon,
    /// Light melee swarmer.
    Goblin,
    /// Ranged support caster.
    Shaman,
}

/// Creature attack or support action; player hotbar slots remain separate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CreatureAbility {
    /// Ordinary charged explosive projectile.
    Fireball,
    /// Ordinary permanent stone shield.
    Shield,
    /// Three finite forward fire pulses.
    FireCone,
    /// Delayed single physical bite.
    Bite,
    /// Delayed single physical swipe.
    Swipe,
    /// Temporary transparent direct-attack blocker.
    Barrier,
    /// Timed party healing and damage support.
    Aura,
}

/// Current authoritative phase of a creature action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AttackPhase {
    /// Preparing an attack, before any impact.
    Windup,
    /// The action's finite damaging/support interval.
    Active,
    /// Brief visible follow-through after admission.
    Recovery,
}

/// Read-only attack geometry and phase, never a hidden target or AI plan.
#[derive(Debug, Clone, Copy)]
pub struct AttackSnapshot {
    /// Action currently presented.
    pub kind: CreatureAbility,
    /// Actual action phase.
    pub phase: AttackPhase,
    /// Physical mouth/hand/cast origin.
    pub origin: Vec3,
    /// Unit attack direction.
    pub direction: Vec3,
    /// Maximum direct attack reach.
    pub range: f32,
    /// Cone half-angle in radians; zero for non-cone actions.
    pub half_angle: f32,
    /// Progress through this phase, clamped to 0–1.
    pub progress: f32,
}

/// A gameplay-owned transparent barrier, independent of terrain material/HP.
#[derive(Debug, Clone)]
pub struct BarrierSnapshot {
    /// Stable object identity within this run.
    pub id: u64,
    /// Actor that created the barrier.
    pub owner: ActorId,
    /// Physical center of the upright rectangle.
    pub center: Vec3,
    /// Horizontal face normal; both directions block direct attacks.
    pub normal: Vec3,
    /// Full horizontal width.
    pub width: f32,
    /// Full vertical height.
    pub height: f32,
    /// Remaining barrier HP.
    pub hp: f32,
    /// Initial barrier HP.
    pub max_hp: f32,
    /// Remaining simulation seconds.
    pub remaining: f32,
    /// Initial duration in simulation seconds.
    pub lifetime: f32,
}

/// Active timed shaman support field.
#[derive(Debug, Clone, Copy)]
pub struct AuraSnapshot {
    /// Living owner of the aura.
    pub owner: ActorId,
    /// Current owner-centered field origin.
    pub center: Vec3,
    /// Maximum eligible ally distance.
    pub radius: f32,
    /// Remaining simulation duration.
    pub remaining: f32,
    /// Original simulation duration.
    pub lifetime: f32,
}

/// Local encounter engagement state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PartyPhase {
    /// Patrol without knowledge of the player.
    #[default]
    Dormant,
    /// Pursuing directly observed or recently remembered combat information.
    Active,
    /// Searching ended or the home leash was exceeded.
    Returning,
    /// Every party member has been defeated; never respawns automatically.
    Cleared,
}

/// Stable public party state for typed encounter evidence.
#[derive(Debug, Clone, Copy)]
pub struct PartySnapshot {
    /// Stable group identity.
    pub id: PartyId,
    /// Current engagement state.
    pub phase: PartyPhase,
    /// Authoritative home position.
    pub home: Vec3,
    /// Number of living members.
    pub living: usize,
}

/// Counts for encounter completion and the local HUD.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct EncounterSummary {
    /// False for the accepted two-actor duel fixture.
    pub enabled: bool,
    /// Living hostile actors, including dormant parties.
    pub living_enemies: usize,
    /// Currently pursuing parties.
    pub active_parties: usize,
    /// Parties that have not activated.
    pub dormant_parties: usize,
    /// Parties searching/returning home.
    pub returning_parties: usize,
    /// Permanently defeated parties.
    pub defeated_parties: usize,
}

impl crate::Actor {
    /// Compound hex geometry when this profile uses it; existing capsules and
    /// oriented boxes publish no prisms. The projection never allocates.
    pub fn body_hex_prisms(&self) -> impl Iterator<Item = BodyHexPrism> {
        std::iter::empty()
    }

    /// Current authoritative beam, if this actor has admitted one.
    #[must_use]
    pub fn beam(&self) -> Option<BeamSnapshot> {
        self.beam
    }

    /// Full physical width, height and length; wings are decorative.
    #[must_use]
    pub fn body_dimensions(&self) -> Vec3 {
        self.dimensions
    }

    /// Collision orientation, independent of the current attack direction.
    #[must_use]
    pub fn body_rotation(&self) -> Quat {
        Quat::from_rotation_y(self.body_yaw)
    }

    /// Current creature action phase and geometry, if any.
    #[must_use]
    pub fn attack_state(&self) -> Option<AttackSnapshot> {
        self.attack
    }
}
