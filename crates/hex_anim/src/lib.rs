//! Generic transform animation, driven by the engine clock.
//!
//! **This crate deliberately knows nothing about hexes.** It moves a [`Transform`]
//! from one place to another over time and has no opinion about what those places
//! mean. Anything that knows a tile is a hex belongs a layer up, in `hex_units`,
//! which composes these primitives into hex-by-hex paths.
//!
//! That separation is enforced by Cargo rather than remembered: this crate cannot see
//! `hex_assets`, the map, or a unit. It was extracted from a crate called
//! `hex_gameplay`, where it had accumulated alongside three unrelated things — the
//! name described none of them, which is how they came to share a home.
//!
//! # The contract
//!
//! A [`Transformer`] is a **pure function of elapsed time**. The driver may call it
//! more than once for the same instant, so one that accumulates state instead of
//! computing from `time` will drift.

use bevy::prelude::*;

use hex_core::{AppSystems, PausableSystems};

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ System ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ //

/// Registers the transform-animation driver.
pub fn plugin(app: &mut App) {
    // Pausable: movement should stop when the game is paused. Because timings come
    // from `Res<Time>` rather than a wall clock, a paused animation resumes where
    // it left off instead of jumping forward by the length of the pause.
    app.add_systems(
        Update,
        transformation_driver
            .in_set(AppSystems::Update)
            .in_set(PausableSystems),
    );
}

/// Advances every in-flight [`Transformation`] and drops the ones that have finished.
///
/// Timings come from [`Res<Time>`], so animations follow the engine clock: they respect
/// pausing, `Time::set_relative_speed`, and step correctly when the app is throttled.
/// Each transformer is fed *seconds since its own component was attached*, not an
/// absolute timestamp, which keeps transformers independent of when they were built.
fn transformation_driver(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &mut Transform, &mut Transformation)>,
) {
    let now = time.elapsed_secs_f64();
    for (entity, mut transform, mut transformation) in query.iter_mut() {
        // Anchor on the first frame this component is seen rather than at construction
        // time, so an animation queued during a pause doesn't burn through its duration
        // before it ever renders. Assigning through `Mut` here also means change
        // detection only fires on that first frame.
        let start = match transformation.started_at {
            Some(start) => start,
            None => {
                transformation.started_at = Some(now);
                now
            }
        };

        let elapsed = now - start;
        transformation.update(&mut transform, elapsed);
        if transformation.is_finished(elapsed) {
            commands.entity(entity).remove::<Transformation>();
        }
    }
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Wrapper Struct ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ //

#[derive(Component)]
/// Wrapper Struct for Transformer which allows Transformers to be queried as a component
/// An in-flight animation attached to an entity.
///
/// Removed automatically once finished, so the presence of this component means
/// "currently moving".
pub struct Transformation {
    transformer: Box<dyn Transformer>,
    /// Engine-clock reading of the first frame this component was driven, or `None`
    /// until then. All times handed to the inner transformer are relative to it.
    started_at: Option<f64>,
}

impl Transformation {
    /// Wraps a transformer so it can be attached to an entity.
    pub fn new(transformer: impl Transformer) -> Self {
        Self {
            transformer: Box::new(transformer),
            started_at: None,
        }
    }

    /// `elapsed` is seconds since this transformation started.
    /// Advances the transform to where it should be at `elapsed`.
    pub fn update(&self, transform: &mut Transform, elapsed: f64) {
        self.transformer.update(transform, elapsed);
    }

    /// `elapsed` is seconds since this transformation started.
    /// Whether the animation has run to completion.
    pub fn is_finished(&self, elapsed: f64) -> bool {
        self.transformer.is_finished(elapsed)
    }
}

impl<T: Transformer> From<T> for Transformation {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Inner Trait ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ //

/// Something that drives a [`Transform`] over time.
///
/// Implementations must be **pure functions of elapsed time** — calling `update`
/// with the same time twice must give the same result. The driver may call it more
/// than once per logical instant, and a transformer that accumulates state instead
/// of computing from `time` will drift.
pub trait Transformer: Send + Sync + 'static {
    /// Edits a transform based on a time.
    ///
    /// `time`: seconds elapsed since this transformer's [`Transformation`] started.
    /// Always relative — never a wall-clock or unix timestamp.
    ///
    /// If a time that is passed in is after the ending time of this transformer then
    /// the transformer should update the transformer to min(time, transformer.end_time)
    /// rather than going past its desired ending position
    fn update(&self, transform: &mut Transform, time: f64);

    /// Whether this transformer has reached its end state by `time`.
    ///
    /// Once true, the driver removes the animation. It must stay true for all
    /// larger times.
    fn is_finished(&self, time: f64) -> bool;
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~ Trait Implementors ~~~~~~~~~~~~~~~~~~~~~~~~~~~~ //

#[derive(Debug)]
/// Moves an entity in a straight line at constant speed.
pub struct LinearMovement {
    start_time: f64,
    end_time: f64,
    start_pos: Vec3,
    velocity: Vec3,
}

impl LinearMovement {
    /// `speed` is world units per second; `start_time` is seconds after the owning
    /// [`Transformation`] starts that this movement should begin.
    ///
    /// # Degenerate input
    ///
    /// A zero-length leg makes the direction `NaN`, and a `speed` of zero divides by
    /// zero. Neither is checked here, because the caller is better placed to say what
    /// should happen: a path of one step is not a movement at all, and a speed of zero
    /// is a settings error rather than an animation to play.
    ///
    /// **The invariant lives in the caller.** `hex_units::route` never emits two
    /// consecutive steps at the same coordinate, and `HexPathingLine` produces no legs
    /// at all from a path shorter than two steps. That guarantee now sits in a
    /// different crate from this constructor, so it is written down here rather than
    /// left as something a reader has to go and discover.
    pub fn new(start_pos: Vec3, end_pos: Vec3, speed: f32, start_time: f64) -> Self {
        let path = end_pos - start_pos;
        let dir = path.normalize();
        let velocity = dir * speed;
        let duration = (path.length() / speed) as f64;
        let end_time = duration + start_time;
        LinearMovement {
            start_time,
            end_time,
            start_pos,
            velocity,
        }
    }
}

impl Transformer for LinearMovement {
    fn update(&self, transform: &mut Transform, time: f64) {
        let time = f64::min(time, self.end_time);
        let dur = time - self.start_time;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "seconds since an animation started; f32 is ample and the \
                      result feeds a f32 transform anyway"
        )]
        let curr_pos = self.start_pos + self.velocity * dur as f32;
        transform.translation = curr_pos;
    }

    fn is_finished(&self, time: f64) -> bool {
        time >= self.end_time
    }
}

#[derive(Default)]
/// Runs transformers one after another, each starting where its own schedule says.
///
/// Used to chain the hex-by-hex legs of a path into one animation.
pub struct TransformerSeries {
    transformers: Vec<Box<dyn Transformer>>,
}

impl TransformerSeries {
    /// An empty series, which is finished immediately.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a transformer to the end of the series.
    pub fn push(&mut self, transformer: impl Transformer) {
        self.transformers.push(Box::new(transformer))
    }
}

impl Transformer for TransformerSeries {
    fn update(&self, transform: &mut Transform, time: f64) {
        for transformer in &self.transformers {
            // if finished go to next
            if transformer.is_finished(time) {
                continue;
            } else {
                // if not finished, update and return
                transformer.update(transform, time);
                return;
            }
        }
        // here all finished. so update last to get to final state
        if let Some(transformer) = self.transformers.last() {
            transformer.update(transform, time)
        }
    }

    fn is_finished(&self, time: f64) -> bool {
        match self.transformers.last() {
            Some(transformer) => transformer.is_finished(time),
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A leg from the origin to `x` at one unit per second, so elapsed seconds and
    /// distance travelled are the same number and the assertions read directly.
    fn leg(x: f32, start_time: f64) -> LinearMovement {
        LinearMovement::new(Vec3::ZERO, Vec3::new(x, 0.0, 0.0), 1.0, start_time)
    }

    fn at(transformer: &impl Transformer, time: f64) -> Vec3 {
        let mut transform = Transform::default();
        transformer.update(&mut transform, time);
        transform.translation
    }

    /// Positions are floats, so compare within a tolerance rather than exactly —
    /// the house style everywhere in this workspace.
    fn assert_close(actual: Vec3, expected: Vec3) {
        assert!(
            actual.distance(expected) < 1e-5,
            "expected {expected:?}, got {actual:?}"
        );
    }

    /// An empty series is finished immediately — which is the correct answer for
    /// "move to where you already are", and what stops a zero-step path hanging.
    #[test]
    fn an_empty_series_is_finished_immediately() {
        let series = TransformerSeries::new();
        assert!(series.is_finished(0.0));
    }

    /// A series is finished exactly when its last leg is, not when the longest one is
    /// or when the count runs out. The second leg here starts at t=1 and is two units
    /// long at one unit per second, so the series ends at t=3.
    #[test]
    fn a_series_finishes_when_its_last_leg_does() {
        let mut series = TransformerSeries::new();
        series.push(leg(1.0, 0.0));
        series.push(leg(2.0, 1.0));

        assert!(!series.is_finished(1.5), "the second leg is still running");
        assert!(
            !series.is_finished(2.9),
            "still running just before the end"
        );
        assert!(series.is_finished(3.0), "both legs are done at t=3");
    }

    /// Overshooting must pin to the end state rather than sailing past it. A piece
    /// that keeps travelling after its animation finishes ends up inside the terrain.
    #[test]
    fn a_finished_leg_holds_its_end_position() {
        let movement = leg(3.0, 0.0);
        assert_close(at(&movement, 300.0), Vec3::new(3.0, 0.0, 0.0));
    }

    /// The driver may call `update` repeatedly for the same instant, so transformers
    /// must be pure functions of time rather than accumulating state.
    ///
    /// Compared exactly on purpose: purity means *bitwise identical* output for
    /// identical input, and an epsilon here would pass for a transformer that drifts
    /// slightly on every call — which is exactly the bug this guards against.
    #[test]
    fn updating_twice_at_the_same_time_gives_the_same_answer() {
        let movement = leg(4.0, 0.0);
        assert_eq!(at(&movement, 1.5), at(&movement, 1.5));
    }

    /// Once a series has run out of legs it reports the last leg's end state, not the
    /// starting transform.
    #[test]
    fn a_spent_series_reports_its_final_position() {
        let mut series = TransformerSeries::new();
        series.push(leg(1.0, 0.0));
        series.push(leg(2.0, 1.0));

        assert_close(at(&series, 99.0), Vec3::new(2.0, 0.0, 0.0));
    }
}
