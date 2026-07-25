//! Generic transform animation, driven by the engine clock.
//!
//! Deliberately knows nothing about hexes. Hex-specific movement lives in
//! [`crate::pathing`], which composes the primitives defined here.
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
