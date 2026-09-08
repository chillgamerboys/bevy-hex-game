//! Fixed authored creature hypotheses, isolated from the accepted Duel tuning.

use serde::{Deserialize, Serialize};

/// New map encounter values; the original duel ignores these settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EncounterTuning {
    /// Seven-hex Golem starting HP.
    pub golem_hp: f32,
    /// Golem grounded speed; it has no running, jumping or flight mode.
    pub golem_speed: f32,
    /// Total damage of the Golem's single spherical slam.
    pub golem_slam_damage: f32,
    /// Physical sphere radius from the Golem's center.
    pub golem_slam_range: f32,
    /// Visible slam preparation duration.
    pub golem_slam_windup: f32,
    /// Time between Golem slams.
    pub golem_slam_cooldown: f32,
    /// Slam impulse magnitude.
    pub golem_slam_knockback: f32,
    /// Physical terrain power per contacted voxel and complete slam.
    pub golem_slam_terrain_power: u8,
    /// Maximum actor damage over a complete laser cast.
    pub golem_laser_damage: f32,
    /// Minimum laser admission distance, preserving its medium-range gap.
    pub golem_laser_min_range: f32,
    /// Full visible laser charge duration.
    pub golem_laser_charge: f32,
    /// Final charge interval whose direction is already locked.
    pub golem_laser_lock_seconds: f32,
    /// Active beam duration.
    pub golem_laser_seconds: f32,
    /// Time between laser casts.
    pub golem_laser_cooldown: f32,
    /// Actual swept beam radius.
    pub golem_laser_radius: f32,
    /// Fire terrain power paid once per voxel per complete laser cast.
    pub golem_laser_terrain_power: u8,
    /// Dragon health before damage.
    pub dragon_hp: f32,
    /// Literal physical dragon height.
    pub dragon_height: f32,
    /// Literal full dragon length.
    pub dragon_length: f32,
    /// Full width of the physical dragon body.
    pub dragon_width: f32,
    /// Dragon ground speed.
    pub dragon_ground_speed: f32,
    /// Dragon flight speed.
    pub dragon_flight_speed: f32,
    /// Desired cruise height above reachable ground.
    pub dragon_cruise_height: f32,
    /// Maximum body yaw change in radians per second.
    pub dragon_turn_speed: f32,
    /// Terrain durability power once per swipe and contacted voxel.
    pub swipe_terrain_power: u8,
    /// Terrain durability power once per bite and contacted voxel.
    pub bite_terrain_power: u8,
    /// Terrain durability power per complete breath and contacted voxel.
    pub breath_terrain_power: u8,
    /// Visible preparation before the first breath pulse.
    pub breath_windup: f32,
    /// Full forward bite angle in degrees.
    pub bite_angle: f32,
    /// Shaman maximum unbuffed fireball actor damage.
    pub shaman_fireball_damage: f32,
    /// Total breath damage over its three pulses.
    pub breath_damage: f32,
    /// Duration of three finite breath pulses.
    pub breath_seconds: f32,
    /// Breath reach from the mouth.
    pub breath_range: f32,
    /// Full breath angle in degrees.
    pub breath_angle: f32,
    /// Time between breath casts.
    pub breath_cooldown: f32,
    /// Single bite HP damage.
    pub bite_damage: f32,
    /// Bite reach from the mouth.
    pub bite_range: f32,
    /// Bite preparation duration.
    pub bite_windup: f32,
    /// Time between bites.
    pub bite_cooldown: f32,
    /// Retreat after recent damage.
    pub dragon_retreat_seconds: f32,
    /// Retreat while below half maximum HP.
    pub dragon_hurt_retreat_seconds: f32,
    /// Delay after damage before dragon healing begins.
    pub dragon_regen_delay: f32,
    /// Dragon HP recovered per eligible second.
    pub dragon_regen_rate: f32,
    /// Barrier placement distance ahead of the mouth.
    pub barrier_distance: f32,
    /// Full barrier width.
    pub barrier_width: f32,
    /// Full barrier height.
    pub barrier_height: f32,
    /// Barrier HP.
    pub barrier_hp: f32,
    /// Barrier lifetime in simulation seconds.
    pub barrier_seconds: f32,
    /// Time between barrier casts.
    pub barrier_cooldown: f32,
    /// Goblin starting HP.
    pub goblin_hp: f32,
    /// Goblin walking speed.
    pub goblin_walk: f32,
    /// Goblin pursuing speed.
    pub goblin_run: f32,
    /// Goblin physical standing height; defaults to the player's height.
    pub goblin_height: f32,
    /// Goblin body radius; defaults to the player's radius.
    pub goblin_radius: f32,
    /// Swipe HP damage.
    pub swipe_damage: f32,
    /// Swipe reach from the body center.
    pub swipe_range: f32,
    /// Full swipe angle in degrees.
    pub swipe_angle: f32,
    /// Swipe preparation interval.
    pub swipe_windup: f32,
    /// Time between swipes.
    pub swipe_cooldown: f32,
    /// Shaman starting HP.
    pub shaman_hp: f32,
    /// Shaman walking speed.
    pub shaman_walk: f32,
    /// Shaman returning speed.
    pub shaman_run: f32,
    /// Shaman Fireball cooldown.
    pub shaman_fireball_cooldown: f32,
    /// Shaman ordinary hold duration before admission.
    pub shaman_charge: f32,
    /// Shaman horizontal error relative to the accepted shadow.
    pub shaman_spread_multiplier: f32,
    /// Shared reaction/cast gap for the shaman.
    pub shaman_reaction: f32,
    /// Shaman permanent Shield cooldown.
    pub shaman_shield_cooldown: f32,
    /// Aura preparation interval.
    pub aura_windup: f32,
    /// Aura eligible ally radius.
    pub aura_radius: f32,
    /// Active aura duration.
    pub aura_seconds: f32,
    /// Time between aura casts.
    pub aura_cooldown: f32,
    /// Aura HP recovery per second.
    pub aura_heal: f32,
    /// Nonstacking actor damage multiplier.
    pub aura_damage_multiplier: f32,
    /// Proximity required for sight activation.
    pub activation_radius: f32,
    /// Goblin/shaman home leash.
    pub ground_leash: f32,
    /// Goblin/shaman memory/search duration.
    pub ground_search: f32,
    /// Shadow encounter home leash; Duel remains unchanged.
    pub shadow_leash: f32,
    /// Shadow encounter search duration.
    pub shadow_search: f32,
    /// Dragon home leash.
    pub dragon_leash: f32,
    /// Dragon search duration.
    pub dragon_search: f32,
    /// Map-only player healing rate.
    pub human_regen_rate: f32,
    /// Time without casting/dealing/receiving damage before player healing.
    pub human_regen_delay: f32,
    /// Time unseen by active enemies before player healing.
    pub human_unseen_delay: f32,
}

impl Default for EncounterTuning {
    fn default() -> Self {
        Self {
            golem_hp: 160.0,
            golem_speed: 2.0,
            golem_slam_damage: 35.0,
            golem_slam_range: hex_core::config::HEX_SMALL_DIAMETER * 4.0,
            golem_slam_windup: 0.8,
            golem_slam_cooldown: 5.0,
            golem_slam_knockback: 5.0,
            golem_slam_terrain_power: 2,
            golem_laser_damage: 45.0,
            golem_laser_min_range: 12.0,
            golem_laser_charge: 2.0,
            golem_laser_lock_seconds: 0.35,
            golem_laser_seconds: 1.0,
            golem_laser_cooldown: 8.0,
            golem_laser_radius: 0.08,
            golem_laser_terrain_power: 2,
            swipe_terrain_power: 1,
            bite_terrain_power: 2,
            breath_terrain_power: 2,
            breath_windup: 0.15,
            bite_angle: 70.0,
            shaman_fireball_damage: 35.0,
            dragon_hp: 220.0,
            dragon_height: 0.4,
            dragon_length: 3.5,
            dragon_width: 1.732_050_8,
            dragon_ground_speed: 7.5,
            dragon_flight_speed: 3.0,
            dragon_cruise_height: 2.0,
            dragon_turn_speed: 2.0,
            breath_damage: 45.0,
            breath_seconds: 0.75,
            breath_range: 6.0,
            breath_angle: 50.0,
            breath_cooldown: 3.0,
            bite_damage: 50.0,
            bite_range: 1.732_050_8,
            bite_windup: 0.25,
            bite_cooldown: 1.8,
            dragon_retreat_seconds: 4.0,
            dragon_hurt_retreat_seconds: 8.0,
            dragon_regen_delay: 4.0,
            dragon_regen_rate: 3.0,
            barrier_distance: 2.5,
            barrier_width: 3.464_101_6,
            barrier_height: 1.6,
            barrier_hp: 60.0,
            barrier_seconds: 4.0,
            barrier_cooldown: 20.0,
            goblin_hp: 50.0,
            goblin_walk: 3.5,
            goblin_run: 6.0,
            goblin_height: crate::BODY_HEIGHT,
            goblin_radius: crate::BODY_RADIUS,
            swipe_damage: 12.0,
            swipe_range: 1.732_050_8,
            swipe_angle: 90.0,
            swipe_windup: 0.25,
            swipe_cooldown: 1.2,
            shaman_hp: 60.0,
            shaman_walk: 3.5,
            shaman_run: 5.0,
            shaman_fireball_cooldown: 2.5,
            shaman_charge: 0.52,
            shaman_spread_multiplier: 2.0,
            shaman_reaction: 0.35,
            shaman_shield_cooldown: 8.0,
            aura_windup: 0.5,
            aura_radius: 6.0,
            aura_seconds: 5.0,
            aura_cooldown: 12.0,
            aura_heal: 3.0,
            aura_damage_multiplier: 1.25,
            activation_radius: 12.0,
            ground_leash: 18.0,
            ground_search: 4.0,
            shadow_leash: 24.0,
            shadow_search: 6.0,
            dragon_leash: 32.0,
            dragon_search: 8.0,
            human_regen_rate: 2.0,
            human_regen_delay: 8.0,
            human_unseen_delay: 3.0,
        }
    }
}

impl EncounterTuning {
    /// Reject unusable or unbounded authored creature values.
    pub fn validate(&self) -> Result<(), String> {
        if [
            self.dragon_hp,
            self.goblin_hp,
            self.shaman_hp,
            self.golem_hp,
        ]
        .into_iter()
        .any(|hp| !hp.is_finite() || hp <= 0.0 || hp > 1000.0)
        {
            return Err("Encounter actor HP must be finite and in (0, 1000].".into());
        }
        let values = [
            self.golem_speed,
            self.golem_slam_damage,
            self.golem_slam_range,
            self.golem_slam_windup,
            self.golem_slam_cooldown,
            self.golem_slam_knockback,
            self.golem_laser_damage,
            self.golem_laser_min_range,
            self.golem_laser_charge,
            self.golem_laser_lock_seconds,
            self.golem_laser_seconds,
            self.golem_laser_cooldown,
            self.golem_laser_radius,
            self.dragon_height,
            self.dragon_length,
            self.dragon_width,
            self.dragon_ground_speed,
            self.dragon_flight_speed,
            self.dragon_cruise_height,
            self.dragon_turn_speed,
            self.breath_damage,
            self.breath_seconds,
            self.breath_range,
            self.breath_angle,
            self.breath_cooldown,
            self.bite_damage,
            self.bite_range,
            self.bite_windup,
            self.bite_cooldown,
            self.dragon_retreat_seconds,
            self.dragon_hurt_retreat_seconds,
            self.dragon_regen_delay,
            self.dragon_regen_rate,
            self.barrier_distance,
            self.barrier_width,
            self.barrier_height,
            self.barrier_hp,
            self.barrier_seconds,
            self.barrier_cooldown,
            self.goblin_walk,
            self.goblin_run,
            self.goblin_height,
            self.goblin_radius,
            self.swipe_damage,
            self.swipe_range,
            self.swipe_angle,
            self.swipe_windup,
            self.swipe_cooldown,
            self.shaman_walk,
            self.shaman_run,
            self.shaman_fireball_cooldown,
            self.shaman_charge,
            self.shaman_spread_multiplier,
            self.shaman_reaction,
            self.shaman_shield_cooldown,
            self.aura_windup,
            self.aura_radius,
            self.aura_seconds,
            self.aura_cooldown,
            self.aura_heal,
            self.aura_damage_multiplier,
            self.activation_radius,
            self.ground_leash,
            self.ground_search,
            self.shadow_leash,
            self.shadow_search,
            self.dragon_leash,
            self.dragon_search,
            self.human_regen_rate,
            self.human_regen_delay,
            self.human_unseen_delay,
        ];
        if values
            .into_iter()
            .any(|v| !v.is_finite() || v <= 0.0 || v > 180.0)
        {
            return Err("Encounter values must be finite and in (0, 180].".into());
        }
        if self.goblin_height < self.goblin_radius * 2.0
            || self.golem_laser_min_range <= self.golem_slam_range
            || self.golem_laser_lock_seconds >= self.golem_laser_charge
            || self.golem_laser_radius > 0.5
            || self.dragon_length < self.dragon_width
            || self.dragon_height > self.dragon_width
            || self.breath_angle > 120.0
            || self.swipe_angle > 180.0
            || self.aura_damage_multiplier < 1.0
            || self.aura_damage_multiplier > 2.0
            || self.dragon_length > 8.0
            || self.dragon_width > 4.0
        {
            return Err(
                "Encounter body dimensions, attack angles or aura multiplier are invalid.".into(),
            );
        }
        if [
            self.swipe_terrain_power,
            self.bite_terrain_power,
            self.breath_terrain_power,
            self.golem_slam_terrain_power,
            self.golem_laser_terrain_power,
        ]
        .into_iter()
        .any(|p| p == 0 || p > 10)
        {
            return Err("creature terrain power must be 1..=10".into());
        }
        for value in [
            self.breath_windup,
            self.bite_angle,
            self.shaman_fireball_damage,
        ] {
            if !value.is_finite() || value <= 0.0 || value > 180.0 {
                return Err("invalid creature attack setting".into());
            }
        }
        Ok(())
    }
}
