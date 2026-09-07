//! Small, validated policy surface for the disposable Strong arena opponent.

use serde::{Deserialize, Serialize};

/// Observation and reaction policy; physical spell and movement rules remain shared.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BotTuning {
    /// Lifetime of a target position established by sight.
    pub memory_seconds: f32,
    /// Lifetime of a nearby, coarse combat cue.
    pub cue_memory_seconds: f32,
    /// Maximum distance at which a combat cue can be heard.
    pub cue_radius: f32,
    /// Permit one speculative shot per loss-of-sight episode.
    pub blind_fire_enabled: bool,
    /// Maximum direct-sighting age when admitting a speculative shot.
    pub blind_fire_max_age: f32,
    /// Seeded horizontal aiming error half-width in world units.
    pub aim_error: f32,
    /// Maximum constant-velocity prediction horizon.
    pub prediction_seconds: f32,
    /// Probability of waiting briefly before a prepared peek.
    pub ambush_chance: f32,
    /// Minimum prepared waiting time.
    pub ambush_min_seconds: f32,
    /// Maximum prepared waiting time.
    pub ambush_max_seconds: f32,
    /// Enable bounded local wall-following rollouts.
    pub flank_enabled: bool,
    /// Minimum time between route planning passes.
    pub flank_replan_seconds: f32,
    /// Preferred open-fight separation.
    pub preferred_distance: f32,
    /// Minimum desired separation in an ordinary visible fight.
    pub minimum_distance: f32,
    /// Side commitment time for a local flank.
    pub flank_commit_seconds: f32,
    /// Time without sufficient movement before trying the other side.
    pub stuck_seconds: f32,
    /// Minimum horizontal progress during the stuck interval.
    pub stuck_distance: f32,
    /// Horizontal tolerance for reaching a local waypoint.
    pub waypoint_distance: f32,
    /// Separation beyond which ordinary pursuit uses sprint.
    pub sprint_distance: f32,
    /// Shared brief reaction gap after a spell release.
    pub reaction_seconds: f32,
}

impl Default for BotTuning {
    fn default() -> Self {
        Self {
            memory_seconds: 6.0,
            cue_memory_seconds: 2.0,
            cue_radius: 12.0,
            blind_fire_enabled: true,
            blind_fire_max_age: 1.5,
            aim_error: 0.16,
            prediction_seconds: 0.5,
            ambush_chance: 0.25,
            ambush_min_seconds: 1.2,
            ambush_max_seconds: 1.2,
            flank_enabled: true,
            flank_replan_seconds: 0.5,
            preferred_distance: 10.0,
            minimum_distance: 6.0,
            flank_commit_seconds: 1.2,
            stuck_seconds: 0.4,
            stuck_distance: 0.12,
            waypoint_distance: 0.25,
            sprint_distance: 14.0,
            reaction_seconds: 0.35,
        }
    }
}

impl BotTuning {
    /// Reject nonfinite and unbounded policy values before simulation admission.
    pub fn validate(&self) -> Result<(), String> {
        for (name, value, minimum, maximum) in [
            ("memory_seconds", self.memory_seconds, 0.2, 10.0),
            ("cue_memory_seconds", self.cue_memory_seconds, 0.2, 5.0),
            ("cue_radius", self.cue_radius, 1.0, 40.0),
            ("blind_fire_max_age", self.blind_fire_max_age, 0.1, 5.0),
            ("aim_error", self.aim_error, 0.0, 2.0),
            ("prediction_seconds", self.prediction_seconds, 0.0, 1.0),
            ("ambush_chance", self.ambush_chance, 0.0, 1.0),
            ("ambush_min_seconds", self.ambush_min_seconds, 0.0, 3.0),
            ("ambush_max_seconds", self.ambush_max_seconds, 0.0, 4.0),
            ("flank_replan_seconds", self.flank_replan_seconds, 0.5, 2.0),
            ("preferred_distance", self.preferred_distance, 4.0, 20.0),
            ("minimum_distance", self.minimum_distance, 3.0, 15.0),
            ("flank_commit_seconds", self.flank_commit_seconds, 0.5, 3.0),
            ("stuck_seconds", self.stuck_seconds, 0.2, 1.0),
            ("stuck_distance", self.stuck_distance, 0.05, 0.5),
            ("waypoint_distance", self.waypoint_distance, 0.1, 0.5),
            ("sprint_distance", self.sprint_distance, 5.0, 30.0),
            ("reaction_seconds", self.reaction_seconds, 0.0, 1.0),
        ] {
            if !value.is_finite() || !(minimum..=maximum).contains(&value) {
                return Err(format!(
                    "bot.{name} must be finite and in {minimum}..={maximum}."
                ));
            }
        }
        if self.minimum_distance > self.preferred_distance {
            return Err("bot.minimum_distance must not exceed preferred_distance.".into());
        }
        if self.ambush_min_seconds > self.ambush_max_seconds {
            return Err("bot.ambush_min_seconds must not exceed ambush_max_seconds.".into());
        }
        Ok(())
    }
}
