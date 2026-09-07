//! Accepted M01 walk/jump/step behavior, extracted from lab revision 9ffc6bf.
//! The arena combines input and persistent impulse velocity before sweeping and
//! removes the inspection controller's fly/recovery behavior.

use bevy_math::{Vec3, Vec3Swizzles};

use crate::collision::{slide_with_contacts, CollisionWorld, SKIN};
use crate::{BODY_HEIGHT, BODY_RADIUS, STEP};

const GRAVITY: f32 = 17.333_334;
const WALK: f32 = 3.5;
const RUN: f32 = 7.0;
const STEP_HEIGHT: f32 = 0.4;
const JUMP_HEIGHT: f32 = 3.25 * 0.4;

#[derive(Debug, Default, Clone)]
pub(crate) struct Body {
    pub vertical_velocity: f32,
    pub impulse_velocity: Vec3,
    pub grounded: bool,
    pub step_rise: f32,
    coyote: f32,
    jump_buffer: f32,
}

impl Body {
    pub fn tick(
        &mut self,
        feet: &mut Vec3,
        direction: Vec3,
        run: bool,
        jump: bool,
        world: &CollisionWorld,
    ) {
        self.step_rise = 0.0;
        // Creation checks prevent embedding either body. Corrupt external state
        // stops safely without the exploration controller's teleport/noclip.
        if !world.clear(*feet, BODY_HEIGHT, BODY_RADIUS) {
            self.vertical_velocity = 0.0;
            self.impulse_velocity = Vec3::ZERO;
            return;
        }
        self.jump_buffer = (self.jump_buffer - STEP).max(0.0);
        if jump {
            self.jump_buffer = 0.1;
        }
        let floor = (self.vertical_velocity <= 0.0 && self.impulse_velocity.y <= 0.0)
            .then(|| world.ground(*feet, BODY_HEIGHT, BODY_RADIUS, SKIN * 8.0))
            .flatten();
        self.grounded = floor.is_some();
        if let Some(floor) = floor {
            *feet = floor;
        }
        self.coyote = if self.grounded {
            0.1
        } else {
            (self.coyote - STEP).max(0.0)
        };
        if self.jump_buffer > 0.0 && self.coyote > 0.0 {
            self.vertical_velocity = (2.0 * GRAVITY * JUMP_HEIGHT).sqrt();
            self.grounded = false;
            self.coyote = 0.0;
            self.jump_buffer = 0.0;
        }
        // Transfer vertical impulse into the ordinary ballistic channel exactly
        // once, then sweep the sum of requested motion and external momentum.
        self.vertical_velocity += self.impulse_velocity.y;
        if self.impulse_velocity.y > 0.0 {
            self.grounded = false;
        }
        self.impulse_velocity.y = 0.0;
        let direction = Vec3::new(direction.x, 0.0, direction.z).normalize_or_zero();
        let horizontal =
            (direction * (if run { RUN } else { WALK }) + self.impulse_velocity) * STEP;
        let vertical = if !self.grounded || self.vertical_velocity > 0.0 {
            let delta = self.vertical_velocity * STEP - 0.5 * GRAVITY * STEP * STEP;
            self.vertical_velocity -= GRAVITY * STEP;
            Vec3::Y * delta
        } else {
            self.vertical_velocity = 0.0;
            Vec3::ZERO
        };
        let original = *feet;
        let (slid, mut contacts) = slide_with_contacts(
            world,
            original,
            horizontal + vertical,
            BODY_HEIGHT,
            BODY_RADIUS,
        );
        *feet = slid;
        if self.grounded && horizontal.length_squared() > f32::EPSILON {
            if (slid - original).xz().length_squared() + SKIN * SKIN < horizontal.length_squared() {
                let rise = Vec3::Y * (STEP_HEIGHT + SKIN * 2.0);
                let rise = world
                    .sweep(original, rise, BODY_HEIGHT, BODY_RADIUS)
                    .map_or(rise, |hit| rise * hit.fraction);
                let raised = original + rise;
                if world.clear(raised, BODY_HEIGHT, BODY_RADIUS) {
                    let (across, across_contacts) =
                        slide_with_contacts(world, raised, horizontal, BODY_HEIGHT, BODY_RADIUS);
                    if (across - raised).xz().length_squared()
                        > (slid - original).xz().length_squared() + SKIN * SKIN
                    {
                        if let Some(landing) =
                            world.ground(across, BODY_HEIGHT, BODY_RADIUS, rise.y + SKIN * 4.0)
                        {
                            if world.clear(landing, BODY_HEIGHT, BODY_RADIUS) {
                                *feet = landing;
                                self.step_rise = (landing.y - original.y).max(0.0);
                                // The lower-wall trial was rejected. Only the
                                // accepted step route clips retained momentum.
                                contacts = across_contacts;
                            }
                        }
                    }
                }
            }
        }

        // Input remains a request; only independent momentum is retained and
        // clipped by actual contact normals. Opposing input cannot be discarded
        // at a wall before an outward impulse gets applied.
        for normal in contacts {
            self.impulse_velocity -= normal * self.impulse_velocity.dot(normal).min(0.0);
            if normal.y.abs() > 0.5 && self.vertical_velocity * normal.y < 0.0 {
                self.vertical_velocity = 0.0;
                self.grounded = normal.y > 0.5;
            }
        }
        self.impulse_velocity *= (-3.0 * STEP).exp();
        if self.grounded
            && world
                .ground(*feet, BODY_HEIGHT, BODY_RADIUS, SKIN * 8.0)
                .is_none()
        {
            self.grounded = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::arena::{ArenaTerrainView, ArenaVoxelGeometry};
    use hex_core::{HexCoord, SubstanceId, TilePos};

    fn floor(radius: u32) -> CollisionWorld {
        let view = ArenaTerrainView {
            voxels: HexCoord::ORIGIN
                .within_radius(radius)
                .into_iter()
                .map(|coord| (TilePos::new(coord, 0), SubstanceId(1)))
                .collect(),
            ..Default::default()
        };
        let mut world = CollisionWorld::default();
        world.refresh(&view, ArenaVoxelGeometry::default());
        world
    }

    #[test]
    fn walking_running_and_diagonals_preserve_m01_speed() {
        let world = floor(30);
        for (direction, run, expected) in [
            (Vec3::X, false, WALK),
            (Vec3::X + Vec3::Z, false, WALK),
            (Vec3::X, true, RUN),
        ] {
            let mut body = Body::default();
            let mut feet = Vec3::ZERO;
            for _ in 0..120 {
                body.tick(&mut feet, direction, run, false, &world);
            }
            assert!((feet.xz().length() - expected).abs() < 0.001);
        }
    }

    #[test]
    fn jump_reaches_accepted_height_then_lands_without_repeat() {
        let world = floor(10);
        let mut feet = Vec3::ZERO;
        let mut body = Body::default();
        let mut peak: f32 = 0.0;
        for tick in 0..240 {
            body.tick(&mut feet, Vec3::ZERO, false, tick == 0, &world);
            peak = peak.max(feet.y);
        }
        assert!((peak - JUMP_HEIGHT).abs() < 0.002);
        assert!(feet.y.abs() < 0.001 && body.grounded);
    }

    #[test]
    fn fixed_tick_results_do_not_depend_on_render_batching() {
        let world = floor(30);
        let simulate = |ticks_per_frame, frames| {
            let mut body = Body::default();
            let mut feet = Vec3::ZERO;
            for frame in 0..frames {
                for tick in 0..ticks_per_frame {
                    body.tick(&mut feet, Vec3::X, true, frame == 0 && tick == 0, &world);
                }
            }
            feet
        };
        assert!(simulate(4, 30).distance(simulate(2, 60)) < 0.00001);
    }

    #[test]
    fn external_momentum_survives_idle_input_and_falls_normally() {
        let world = floor(30);
        let mut body = Body {
            impulse_velocity: Vec3::new(8.0, 4.0, 0.0),
            ..Default::default()
        };
        let mut feet = Vec3::ZERO;
        for _ in 0..30 {
            body.tick(&mut feet, Vec3::ZERO, false, false, &world);
        }
        assert!(feet.x > 1.0 && feet.y > 0.0);
        for _ in 0..240 {
            body.tick(&mut feet, Vec3::ZERO, false, false, &world);
        }
        assert!(feet.y.abs() < 0.001);
    }

    #[test]
    fn removed_support_causes_falling_without_teleport_or_flight() {
        let world = floor(1);
        let mut body = Body::default();
        let mut feet = Vec3::ZERO;
        body.tick(&mut feet, Vec3::ZERO, false, false, &world);
        let empty = CollisionWorld::default();
        for _ in 0..120 {
            body.tick(&mut feet, Vec3::ZERO, false, false, &empty);
        }
        assert!(feet.y < -5.0);
    }

    #[test]
    fn one_level_step_is_accepted_but_two_levels_require_a_jump() {
        let make_world = |levels| {
            let mut view = ArenaTerrainView {
                voxels: HexCoord::ORIGIN
                    .within_radius(10)
                    .into_iter()
                    .map(|coord| (TilePos::new(coord, 0), SubstanceId(1)))
                    .collect(),
                ..Default::default()
            };
            for level in 1..=levels {
                view.voxels.insert(
                    TilePos::new(HexCoord::from_axial(1, 0), level),
                    SubstanceId(1),
                );
            }
            let mut world = CollisionWorld::default();
            world.refresh(&view, ArenaVoxelGeometry::default());
            world
        };
        let mut body = Body::default();
        let mut feet = Vec3::ZERO;
        let mut accepted_step: f32 = 0.0;
        let low = make_world(1);
        for _ in 0..90 {
            body.tick(&mut feet, Vec3::X, false, false, &low);
            accepted_step = accepted_step.max(body.step_rise);
        }
        assert!(feet.x > 2.0 && (accepted_step - 0.4).abs() < 0.001);
        let high = make_world(2);
        body = Body::default();
        feet = Vec3::ZERO;
        for _ in 0..120 {
            body.tick(&mut feet, Vec3::X, false, false, &high);
        }
        assert!(feet.x < 0.62);
        body = Body::default();
        feet = Vec3::ZERO;
        for tick in 0..120 {
            body.tick(&mut feet, Vec3::X, false, tick == 0, &high);
        }
        assert!(feet.x > 2.0);
    }

    #[test]
    fn coyote_jump_expires_and_airborne_edges_cannot_jump_again() {
        let supported = floor(5);
        let empty = CollisionWorld::default();
        let mut body = Body::default();
        let mut feet = Vec3::ZERO;
        body.tick(&mut feet, Vec3::ZERO, false, false, &supported);
        for _ in 0..5 {
            body.tick(&mut feet, Vec3::ZERO, false, false, &empty);
        }
        body.tick(&mut feet, Vec3::ZERO, false, true, &empty);
        assert!(body.vertical_velocity > 5.0);
        let before = body.vertical_velocity;
        body.tick(&mut feet, Vec3::ZERO, false, true, &empty);
        assert!(body.vertical_velocity < before);
        body = Body::default();
        feet = Vec3::ZERO;
        body.tick(&mut feet, Vec3::ZERO, false, false, &supported);
        for _ in 0..20 {
            body.tick(&mut feet, Vec3::ZERO, false, false, &empty);
        }
        body.tick(&mut feet, Vec3::ZERO, false, true, &empty);
        assert!(body.vertical_velocity < 0.0);
    }

    #[test]
    fn buffered_jump_is_consumed_after_landing_and_does_not_report_a_step() {
        let world = floor(5);
        let mut body = Body::default();
        let mut feet = Vec3::Y * 0.05;
        let mut jumped = false;
        for tick in 0..25 {
            body.tick(&mut feet, Vec3::ZERO, false, tick == 0, &world);
            assert!(body.step_rise.abs() < 0.00001);
            jumped |= body.vertical_velocity > 1.0;
        }
        assert!(jumped);
    }

    #[test]
    fn opposing_input_and_outward_wall_impulse_sweep_their_net_displacement() {
        let mut view = ArenaTerrainView {
            voxels: HexCoord::ORIGIN
                .within_radius(5)
                .into_iter()
                .map(|coord| (TilePos::new(coord, 0), SubstanceId(1)))
                .collect(),
            ..Default::default()
        };
        for level in 1..=6 {
            view.voxels
                .insert(TilePos::new(HexCoord::ORIGIN, level), SubstanceId(1));
        }
        let mut world = CollisionWorld::default();
        world.refresh(&view, ArenaVoxelGeometry::default());
        let mut feet = Vec3::new(
            -hex_core::config::HEX_SMALL_DIAMETER * 0.5 - BODY_RADIUS - SKIN * 2.0,
            SKIN,
            0.0,
        );
        let start = feet;
        let mut body = Body {
            impulse_velocity: Vec3::NEG_X * 8.0,
            ..Default::default()
        };
        body.tick(&mut feet, Vec3::X, false, false, &world);
        let expected = (WALK - 8.0) * STEP;
        assert!((feet.x - start.x - expected).abs() < 0.0001);
        assert!(body.impulse_velocity.x < -7.0);
    }
}
