//! Fixed-step exploration body. This never changes tactical occupancy or turn state.

use bevy::prelude::*;
use serde::de::Error;
use serde::{Deserialize, Deserializer};

use super::collision::{CollisionWorld, SKIN};

pub(super) const STEP: f32 = 1.0 / 120.0;

#[derive(Asset, Resource, TypePath, Clone, Debug)]
pub(super) struct Settings {
    pub walk_speed: f32,
    pub run_speed: f32,
    pub fly_speed: f32,
    pub body_levels: f32,
    pub body_radius: f32,
    pub step_levels: f32,
    pub jump_levels: f32,
    pub wade_levels: f32,
    pub gravity: f32,
    pub jump_buffer_seconds: f32,
    pub coyote_seconds: f32,
}

impl<'de> Deserialize<'de> for Settings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            walk_speed: f32,
            run_speed: f32,
            fly_speed: f32,
            body_levels: f32,
            body_radius: f32,
            step_levels: f32,
            jump_levels: f32,
            wade_levels: f32,
            gravity: f32,
            jump_buffer_seconds: f32,
            coyote_seconds: f32,
        }
        let raw = Raw::deserialize(deserializer)?;
        let settings = Self {
            walk_speed: raw.walk_speed,
            run_speed: raw.run_speed,
            fly_speed: raw.fly_speed,
            body_levels: raw.body_levels,
            body_radius: raw.body_radius,
            step_levels: raw.step_levels,
            jump_levels: raw.jump_levels,
            wade_levels: raw.wade_levels,
            gravity: raw.gravity,
            jump_buffer_seconds: raw.jump_buffer_seconds,
            coyote_seconds: raw.coyote_seconds,
        };
        settings.validate().map_err(D::Error::custom)?;
        Ok(settings)
    }
}

impl Settings {
    fn validate(&self) -> Result<(), String> {
        for (name, value, max) in [
            ("walk_speed", self.walk_speed, 50.0),
            ("run_speed", self.run_speed, 100.0),
            ("fly_speed", self.fly_speed, 200.0),
            ("body_levels", self.body_levels, 10.0),
            ("body_radius", self.body_radius, 2.0),
            ("step_levels", self.step_levels, 5.0),
            ("jump_levels", self.jump_levels, 20.0),
            ("wade_levels", self.wade_levels, 5.0),
            ("gravity", self.gravity, 100.0),
            ("jump_buffer_seconds", self.jump_buffer_seconds, 0.5),
            ("coyote_seconds", self.coyote_seconds, 0.5),
        ] {
            if !value.is_finite() || value <= 0.0 || value > max {
                return Err(format!("{name} must be finite and in (0, {max}]"));
            }
        }
        if self.run_speed < self.walk_speed
            || self.step_levels >= self.body_levels
            || self.wade_levels >= self.body_levels
        {
            return Err("Run must be at least walk speed; step and wade depths must be less than body height".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Walk,
    Fly,
}

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Intent {
    pub direction: Vec3,
    pub run: bool,
    pub jump: bool,
}

#[derive(Debug, Clone)]
pub(super) struct Body {
    pub position: Vec3,
    pub vertical_velocity: f32,
    pub grounded: bool,
    pub last_safe: Vec3,
    pub spawn: Vec3,
    coyote: f32,
    jump_buffer: f32,
}

impl Body {
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            vertical_velocity: 0.0,
            grounded: false,
            last_safe: position,
            spawn: position,
            coyote: 0.0,
            jump_buffer: 0.0,
        }
    }

    pub fn clear_motion(&mut self) {
        self.vertical_velocity = 0.0;
        self.grounded = false;
        self.coyote = 0.0;
        self.jump_buffer = 0.0;
    }

    /// Returns a notice for recoveries; the caller switches to fly if no safe
    /// previously grounded location survives a terrain edit.
    pub fn tick(
        &mut self,
        input: Intent,
        settings: &Settings,
        level: f32,
        world: &CollisionWorld,
    ) -> Option<&'static str> {
        let height = settings.body_levels * level;
        let radius = settings.body_radius;
        if !world.clear(self.position, height, radius) {
            return Some(self.recover(
                settings,
                level,
                world,
                "Terrain changed; returned to safe ground.",
            ));
        }
        self.jump_buffer = (self.jump_buffer - STEP).max(0.0);
        if input.jump {
            self.jump_buffer = settings.jump_buffer_seconds;
        }
        let was_grounded = self.grounded;
        let floor = (self.vertical_velocity <= 0.0)
            .then(|| world.ground(self.position, height, radius, SKIN * 8.0))
            .flatten();
        self.grounded = floor.is_some();
        if let Some(floor) = floor {
            self.position = floor;
        }
        self.coyote = if self.grounded {
            settings.coyote_seconds
        } else {
            (self.coyote - STEP).max(0.0)
        };
        if self.jump_buffer > 0.0 && self.coyote > 0.0 {
            self.vertical_velocity = (2.0 * settings.gravity * settings.jump_levels * level).sqrt();
            self.grounded = false;
            self.coyote = 0.0;
            self.jump_buffer = 0.0;
        }
        let direction = Vec3::new(input.direction.x, 0.0, input.direction.z).normalize_or_zero();
        let speed = if input.run {
            settings.run_speed
        } else {
            settings.walk_speed
        };
        let horizontal = direction * speed * STEP;
        let original = self.position;
        let slid = slide(world, original, horizontal, height, radius);
        self.position = slid;
        if self.grounded && horizontal.length_squared() > f32::EPSILON {
            if (slid - original).length_squared() + SKIN * SKIN < horizontal.length_squared() {
                let rise = Vec3::Y * (settings.step_levels * level + SKIN * 2.0);
                let raised = original + rise;
                if world.sweep(original, rise, height, radius).is_none()
                    && world.clear(raised, height, radius)
                {
                    let across = slide(world, raised, horizontal, height, radius);
                    if (across - raised).xz().length_squared()
                        > (slid - original).xz().length_squared() + SKIN * SKIN
                    {
                        if let Some(landing) =
                            world.ground(across, height, radius, rise.y + SKIN * 4.0)
                        {
                            if world.clear(landing, height, radius) {
                                self.position = landing;
                            }
                        }
                    }
                }
            }
            // Reject a grounded shore step before falling into a deep liquid bed.
            let drop = (self.position.y - world.floor).max(1.0);
            if let Some(landing) = world.ground(self.position, height, radius, drop) {
                if world.water_depth(landing, radius) > settings.wade_levels * level + SKIN * 4.0 {
                    self.position = original;
                }
            }
        }
        if !self.grounded || self.vertical_velocity > 0.0 {
            // Ballistic integration preserves authored jump height across fixed ticks.
            let delta =
                Vec3::Y * (self.vertical_velocity * STEP - 0.5 * settings.gravity * STEP * STEP);
            self.vertical_velocity -= settings.gravity * STEP;
            if let Some(hit) = world.sweep(self.position, delta, height, radius) {
                self.position += delta * hit.fraction + hit.normal * SKIN;
                self.vertical_velocity = 0.0;
                self.grounded = hit.normal.y > 0.5;
            } else {
                self.position += delta;
            }
        }
        // Ground contact after horizontal movement may have disappeared at an edge.
        if self.grounded
            && world
                .ground(self.position, height, radius, SKIN * 8.0)
                .is_none()
        {
            self.grounded = false;
        }
        if self.position.y < world.floor {
            return Some(self.recover(
                settings,
                level,
                world,
                "Fell below the map; returned to safe ground.",
            ));
        }
        if world.water_depth(self.position, radius) > settings.wade_levels * level + SKIN * 4.0 {
            return Some(self.recover(
                settings,
                level,
                world,
                "Deep water; returned to safe ground. Use F to fly across.",
            ));
        }
        if self.grounded {
            self.last_safe = self.position;
            // Buffered landing jumps fire on the very next fixed tick.
            if !was_grounded {
                self.coyote = settings.coyote_seconds;
            }
        }
        None
    }

    fn recover(
        &mut self,
        settings: &Settings,
        level: f32,
        world: &CollisionWorld,
        notice: &'static str,
    ) -> &'static str {
        for candidate in [self.last_safe, self.spawn] {
            if world.clear(
                candidate,
                settings.body_levels * level,
                settings.body_radius,
            ) {
                if let Some(ground) = world.ground(
                    candidate,
                    settings.body_levels * level,
                    settings.body_radius,
                    SKIN * 8.0,
                ) {
                    if world.water_depth(ground, settings.body_radius)
                        <= settings.wade_levels * level + SKIN * 4.0
                    {
                        self.position = ground;
                        self.clear_motion();
                        return notice;
                    }
                }
            }
        }
        self.clear_motion();
        "Safe ground changed; fly mode enabled."
    }
}

fn slide(
    world: &CollisionWorld,
    mut feet: Vec3,
    mut delta: Vec3,
    height: f32,
    radius: f32,
) -> Vec3 {
    // Three independent contact planes can stop all remaining motion. Extra
    // iterations handle several neighboring hex faces at the same corner.
    for _ in 0..6 {
        if delta.length_squared() < SKIN * SKIN {
            break;
        }
        let Some(hit) = world.sweep(feet, delta, height, radius) else {
            return feet + delta;
        };
        feet += delta * hit.fraction + hit.normal * SKIN;
        delta *= 1.0 - hit.fraction;
        delta -= hit.normal * delta.dot(hit.normal).min(0.0);
    }
    feet
}

#[cfg(test)]
mod tests {
    use super::super::collision::{Material, Span};
    use super::*;
    use hex_core::HexCoord;

    fn settings() -> Settings {
        ron::from_str(include_str!("../../../../assets/config/exploration.ron"))
            .expect("valid shipped settings")
    }
    fn floor(radius: u32) -> CollisionWorld {
        let mut world = CollisionWorld::default();
        world.initialized = true;
        world.floor = -20.0;
        world.replace(
            Entity::from_bits(1),
            HexCoord::default()
                .within_radius(radius)
                .into_iter()
                .map(|coord| Span {
                    coord,
                    bottom: -1.0,
                    top: 0.0,
                    material: Material::Solid,
                })
                .collect(),
        );
        world
    }
    fn advance(body: &mut Body, world: &CollisionWorld, input: Intent, ticks: usize) {
        for i in 0..ticks {
            body.tick(
                Intent {
                    jump: input.jump && i == 0,
                    ..input
                },
                &settings(),
                0.4,
                world,
            );
        }
    }

    #[test]
    fn walking_running_and_diagonals_have_authored_speed() {
        let world = floor(30);
        for (direction, run, expected) in [
            (Vec3::X, false, 3.0),
            (Vec3::new(1.0, 10.0, 1.0), false, 3.0),
            (Vec3::X, true, 6.0),
        ] {
            let mut body = Body::new(Vec3::ZERO);
            advance(
                &mut body,
                &world,
                Intent {
                    direction,
                    run,
                    ..default()
                },
                120,
            );
            assert!((body.position.xz().length() - expected).abs() < 0.002);
            assert!(body.position.y.abs() < 0.001);
        }
    }

    #[test]
    fn jump_falls_and_lands_without_airborne_repeat_jumps() {
        let world = floor(3);
        let mut body = Body::new(Vec3::ZERO);
        let mut peak: f32 = 0.0;
        for tick in 0..115 {
            body.tick(
                Intent {
                    jump: tick == 0 || tick == 15,
                    ..default()
                },
                &settings(),
                0.4,
                &world,
            );
            peak = peak.max(body.position.y);
        }
        assert!((peak - 1.2).abs() < 0.002);
        assert!(body.grounded && body.position.y.abs() < 0.001);
    }

    #[test]
    fn leaving_a_ledge_and_disabling_flight_in_air_both_fall() {
        let world = floor(0);
        let mut walker = Body::new(Vec3::ZERO);
        advance(
            &mut walker,
            &world,
            Intent {
                direction: Vec3::X,
                ..default()
            },
            65,
        );
        assert!(!walker.grounded && walker.position.y < -0.1);
        let mut airborne = Body::new(Vec3::Y * 5.0);
        advance(&mut airborne, &world, Intent::default(), 20);
        assert!(airborne.position.y < 4.9 && airborne.vertical_velocity < 0.0);
        advance(&mut airborne, &world, Intent::default(), 120);
        assert!(airborne.grounded);
    }

    #[test]
    fn ceiling_stops_jump_and_thin_walls_stop_running() {
        let mut world = floor(8);
        world.replace(
            Entity::from_bits(2),
            vec![Span {
                coord: HexCoord::default(),
                bottom: 1.2,
                top: 2.0,
                material: Material::Solid,
            }],
        );
        let mut body = Body::new(Vec3::ZERO);
        for i in 0..100 {
            body.tick(
                Intent {
                    jump: i == 0,
                    ..default()
                },
                &settings(),
                0.4,
                &world,
            );
            assert!(body.position.y + 0.8 <= 1.201);
        }
        world.replace(
            Entity::from_bits(3),
            vec![Span {
                coord: HexCoord::from_axial(2, 0),
                bottom: 0.0,
                top: 5.0,
                material: Material::Solid,
            }],
        );
        advance(
            &mut body,
            &world,
            Intent {
                direction: Vec3::X,
                run: true,
                ..default()
            },
            120,
        );
        assert!(body.position.x < HexCoord::from_axial(2, 0).to_world(0.0).x - 0.86);
    }

    #[test]
    fn invalid_tuning_cannot_replace_settings() {
        let source = include_str!("../../../../assets/config/exploration.ron");
        assert!(
            ron::from_str::<Settings>(&source.replace("gravity: 16.0", "gravity: -1.0")).is_err()
        );
        assert!(ron::from_str::<Settings>(
            &source.replace("body_radius: 0.25", "body_radius: 50.0")
        )
        .is_err());
    }
    #[test]
    fn one_level_steps_work_but_taller_walls_and_low_lintels_block() {
        for (top, ceiling, should_pass) in [
            (0.4, None, true),
            (0.8, None, false),
            (0.4, Some(1.1), false),
        ] {
            let mut world = floor(5);
            world.replace(
                Entity::from_bits(2),
                vec![Span {
                    coord: HexCoord::from_axial(1, 0),
                    bottom: 0.0,
                    top,
                    material: Material::Solid,
                }],
            );
            if let Some(bottom) = ceiling {
                world.replace(
                    Entity::from_bits(3),
                    HexCoord::default()
                        .within_radius(2)
                        .into_iter()
                        .map(|coord| Span {
                            coord,
                            bottom,
                            top: 2.0,
                            material: Material::Solid,
                        })
                        .collect(),
                );
            }
            let mut body = Body::new(Vec3::ZERO);
            advance(
                &mut body,
                &world,
                Intent {
                    direction: Vec3::X,
                    ..default()
                },
                55,
            );
            if should_pass {
                assert!(body.position.x > 1.1 && body.position.y > 0.39, "{body:?}");
            } else {
                assert!(body.position.x < 0.7, "{body:?}");
            }
        }
    }

    #[test]
    fn coyote_window_expires_and_does_not_allow_a_second_jump() {
        for (wait, can_jump) in [(5, true), (20, false)] {
            let world = floor(0);
            let mut body = Body::new(Vec3::ZERO);
            while body.grounded || body.position.x < 1.12 {
                body.tick(
                    Intent {
                        direction: Vec3::X,
                        ..default()
                    },
                    &settings(),
                    0.4,
                    &world,
                );
            }
            advance(&mut body, &world, Intent::default(), wait);
            body.tick(
                Intent {
                    jump: true,
                    ..default()
                },
                &settings(),
                0.4,
                &world,
            );
            assert_eq!(body.vertical_velocity > 0.0, can_jump, "{body:?}");
        }
    }

    #[test]
    fn landing_consumes_a_recent_buffered_jump() {
        let world = floor(3);
        let mut body = Body::new(Vec3::Y * 0.02);
        body.vertical_velocity = -1.0;
        body.tick(
            Intent {
                jump: true,
                ..default()
            },
            &settings(),
            0.4,
            &world,
        );
        advance(&mut body, &world, Intent::default(), 5);
        assert!(
            body.vertical_velocity > 0.0 && body.position.y > 0.05,
            "{body:?}"
        );
    }

    #[test]
    fn shallow_water_allows_wading_but_deep_shores_stop_walking() {
        for (depth, can_cross) in [(0.4, true), (0.8, false)] {
            let mut world = floor(5);
            world.replace(
                Entity::from_bits(2),
                vec![Span {
                    coord: HexCoord::from_axial(1, 0),
                    bottom: 0.0,
                    top: depth,
                    material: Material::Liquid,
                }],
            );
            let mut body = Body::new(Vec3::ZERO);
            advance(
                &mut body,
                &world,
                Intent {
                    direction: Vec3::X,
                    ..default()
                },
                70,
            );
            assert_eq!(body.position.x > 1.2, can_cross, "depth {depth}: {body:?}");
        }
    }

    #[test]
    fn falling_into_deep_water_or_below_map_recovers_to_valid_ground() {
        let mut world = floor(4);
        world.replace(
            Entity::from_bits(2),
            vec![Span {
                coord: HexCoord::from_axial(2, 0),
                bottom: 0.0,
                top: 2.0,
                material: Material::Liquid,
            }],
        );
        let mut body = Body::new(Vec3::ZERO);
        advance(&mut body, &world, Intent::default(), 1);
        let safe = body.position;
        body.position = HexCoord::from_axial(2, 0).to_world(1.0);
        let notice = body
            .tick(Intent::default(), &settings(), 0.4, &world)
            .expect("water recovery");
        assert!(notice.contains("Deep water"));
        assert!(body.position.distance(safe) < 0.001);
        body.position = Vec3::Y * -21.0;
        let notice = body
            .tick(Intent::default(), &settings(), 0.4, &world)
            .expect("map recovery");
        assert!(notice.contains("Fell below"));
        assert!(body.position.distance(safe) < 0.001);
    }

    #[test]
    fn removed_safe_ground_uses_flight_instead_of_repeated_teleports() {
        let mut world = floor(0);
        let mut body = Body::new(Vec3::ZERO);
        advance(&mut body, &world, Intent::default(), 1);
        world.remove(Entity::from_bits(1));
        body.position = Vec3::Y * -21.0;
        assert_eq!(
            body.tick(Intent::default(), &settings(), 0.4, &world),
            Some("Safe ground changed; fly mode enabled.")
        );
    }
}
