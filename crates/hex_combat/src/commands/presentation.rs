//! The visible half of an action, shared by every verb that has one.
//!
//! Built from the same primitives as walking, which is a useful check that the
//! `hex_anim` split holds: the animation engine needed nothing added to express
//! a swing it was never designed for.

use bevy::prelude::*;

use hex_anim::Transformation;
use hex_units::{HexPathingLine, Standing};

/// How far the actor leans toward its target, as a fraction of the distance.
///
/// Small on purpose: it must read as a swing rather than as a move, or the
/// player cannot tell an attack from a step.
const LUNGE_FRACTION: f32 = 0.35;

/// Seconds for the lunge out and the return. Slow enough to notice, short
/// enough not to make a turn feel like waiting.
const LUNGE_SECONDS: f32 = 0.18;

/// A short lean toward the target and back.
pub(super) fn lunge(
    commands: &mut Commands,
    entity: Entity,
    from: Standing,
    toward: Standing,
    speed: f32,
) {
    let start = from.world_position();
    let tip = start + (toward.world_position() - start) * LUNGE_FRACTION;
    commands
        .entity(entity)
        .insert(there_and_back(start, tip, speed));
}

/// The target's half: a smaller flinch directly away from the actor.
pub(super) fn recoil(
    commands: &mut Commands,
    entity: Entity,
    target: Standing,
    attacker: Standing,
    speed: f32,
) {
    let start = target.world_position();
    let away = start + (start - attacker.world_position()) * (LUNGE_FRACTION * 0.5);
    commands
        .entity(entity)
        .insert(there_and_back(start, away, speed));
}

/// An out-and-back movement that finishes exactly where it started.
///
/// Returning to `start` matters: the animation ends and the component is
/// removed, and anything that ended somewhere else would leave the piece off
/// its tile with nothing to correct it.
fn there_and_back(start: Vec3, tip: Vec3, speed: f32) -> Transformation {
    // Speed is derived from the distance so both legs take `LUNGE_SECONDS`,
    // whatever the lunge length works out to. Guard the degenerate case: a
    // zero-length leg makes `LinearMovement` produce NaN.
    let distance = start.distance(tip);
    if distance <= f32::EPSILON {
        // Nothing to animate. A stationary "swing" is better than a NaN
        // transform, which would put the piece somewhere unrenderable and
        // never come back.
        return HexPathingLine::new(&[], speed).into();
    }
    let leg_speed = distance / LUNGE_SECONDS;

    let mut series = hex_anim::TransformerSeries::new();
    series.push(hex_anim::LinearMovement::new(start, tip, leg_speed, 0.0));
    series.push(hex_anim::LinearMovement::new(
        tip,
        start,
        leg_speed,
        f64::from(LUNGE_SECONDS),
    ));
    Transformation::new(series)
}
