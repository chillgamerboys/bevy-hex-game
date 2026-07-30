//! Dependency-limited synthetic fixtures for gameplay-owned headless tests.
//!
//! This crate publishes the same shared surface components that gameplay consumes,
//! without importing either the world systems that normally produce them or the
//! gameplay systems under test. Its Cargo dependencies are the enforcement
//! boundary: adding `hex_units`, `hex_combat`, `hex_game`, `hex_map`, `hex_world`,
//! or `hex_perception` here is an architecture violation.

use std::fmt;
use std::time::Duration;

use bevy::app::PluginsState;
use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use hex_assets::{
    ArtPalette, PaletteSwatch, SrgbColor, Substance, SubstanceFile, SubstanceTable, SwatchId,
};
use hex_core::{
    AppSystems, GameplaySetup, Headroom, HexCoord, HexSpan, HexTile, Mode, Pause, Screen,
    SubstanceId, TilePos, MAX_HEADROOM,
};

/// Stable synthetic stone id produced by [`fixture_assets`].
pub const STONE: SubstanceId = SubstanceId(1);

/// One exact surface published by a synthetic arena.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceSpec {
    /// Exact standable surface.
    pub position: TilePos,
    /// Vertical material span below the surface.
    pub span: HexSpan,
    /// Clear levels above the surface.
    pub headroom: i32,
    /// Authored material id.
    pub substance: SubstanceId,
}

impl SurfaceSpec {
    /// Builds one ordinary stone surface at `position`.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "test arenas use small exact voxel levels that round-trip through f32"
    )]
    pub fn stone(position: TilePos) -> Self {
        Self {
            position,
            span: HexSpan::new(position.level as f32, position.level as f32 + 1.0),
            headroom: MAX_HEADROOM,
            substance: STONE,
        }
    }
}

/// Synthetic exact-surface facts used in gameplay tests.
#[derive(Resource, Debug, Clone, PartialEq, Default)]
pub struct SyntheticArena {
    surfaces: Vec<SurfaceSpec>,
}

impl SyntheticArena {
    /// Creates an arena from explicit shared surface facts.
    #[must_use]
    pub fn new(surfaces: impl IntoIterator<Item = SurfaceSpec>) -> Self {
        let mut surfaces = surfaces.into_iter().collect::<Vec<_>>();
        surfaces.sort_by_key(|surface| surface.position);
        surfaces.dedup_by_key(|surface| surface.position);
        Self { surfaces }
    }

    /// Creates one flat hexagonal patch.
    #[must_use]
    pub fn flat_radius(radius: u32, level: i32) -> Self {
        Self::new(
            HexCoord::ORIGIN
                .within_radius(radius)
                .into_iter()
                .map(|coord| SurfaceSpec::stone(TilePos::new(coord, level))),
        )
    }

    /// Creates a one-surface-wide axial corridor including both endpoints.
    #[must_use]
    pub fn corridor(start_q: i32, end_q: i32, r: i32, level: i32) -> Self {
        let low = start_q.min(end_q);
        let high = start_q.max(end_q);
        Self::new(
            (low..=high)
                .map(|q| SurfaceSpec::stone(TilePos::new(HexCoord::from_axial(q, r), level))),
        )
    }

    /// Creates two rooms connected by one exact-surface-wide chokepoint.
    #[must_use]
    pub fn chokepoint(level: i32) -> Self {
        let left = HexCoord::from_axial(-3, 0);
        let right = HexCoord::from_axial(3, 0);
        Self::new(
            left.within_radius(2)
                .into_iter()
                .chain(right.within_radius(2))
                .chain((-2..=2).map(|q| HexCoord::from_axial(q, 0)))
                .map(|coord| SurfaceSpec::stone(TilePos::new(coord, level))),
        )
    }

    /// Creates a flat patch with a second surface at the origin.
    #[must_use]
    pub fn stacked(radius: u32, lower_level: i32, upper_level: i32) -> Self {
        let lower = Self::flat_radius(radius, lower_level);
        Self::new(
            lower
                .surfaces
                .into_iter()
                .chain([SurfaceSpec::stone(TilePos::new(
                    HexCoord::ORIGIN,
                    upper_level,
                ))]),
        )
    }

    /// Returns the exact surfaces in stable order.
    #[must_use]
    pub fn surfaces(&self) -> &[SurfaceSpec] {
        &self.surfaces
    }
}

/// Builds the small palette and substance table used by synthetic gameplay arenas.
pub fn fixture_assets() -> Result<(ArtPalette, SubstanceTable), String> {
    let stone_id = SwatchId::new("terrain/test-stone").map_err(|error| error.to_string())?;
    let stone = PaletteSwatch::new(
        "Test Stone",
        SrgbColor::new(0.5, 0.5, 0.5).map_err(|error| error.to_string())?,
        ["test".to_owned()].into_iter().collect(),
    )
    .map_err(|error| error.to_string())?;
    let palette = ArtPalette::new([(stone_id.clone(), stone)].into_iter().collect())
        .map_err(|error| error.to_string())?;
    let substances = SubstanceFile {
        substances: [
            ("air".to_owned(), Substance::invisible(false, false)),
            (
                "stone".to_owned(),
                Substance::from_swatch(stone_id, true, true),
            ),
        ]
        .into_iter()
        .collect(),
    };
    let table =
        SubstanceTable::from_file(&substances, &palette).map_err(|error| error.to_string())?;
    Ok((palette, table))
}

/// Builder for deterministic minimal Bevy apps used by gameplay tests.
pub struct TestAppBuilder {
    app: App,
}

impl Default for TestAppBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TestAppBuilder {
    /// Installs common states, schedules, input, assets, and a fixed 100 ms clock.
    #[must_use]
    pub fn new() -> Self {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            StatesPlugin,
            bevy::input::InputPlugin,
        ));
        app.init_asset::<Mesh>();
        app.init_asset::<StandardMaterial>();
        app.init_state::<Screen>();
        app.add_sub_state::<Mode>();
        app.add_sub_state::<Pause>();
        app.configure_sets(
            Update,
            (
                AppSystems::TickTimers,
                AppSystems::RecordInput,
                AppSystems::Update,
            )
                .chain(),
        );
        app.configure_sets(
            OnEnter(Screen::Gameplay),
            (
                GameplaySetup::Resources,
                GameplaySetup::Terrain,
                GameplaySetup::Actors,
                GameplaySetup::Restore,
                GameplaySetup::Perception,
                GameplaySetup::View,
                GameplaySetup::Finalize,
            )
                .chain(),
        );
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            Duration::from_millis(100),
        ));
        Self { app }
    }

    /// Gives the test access to the app before plugins are finalized.
    pub fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    /// Selects the deterministic duration advanced by every app update.
    #[must_use]
    pub fn with_fixed_step(mut self, duration: Duration) -> Self {
        self.app
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(duration));
        self
    }

    /// Publishes the standard synthetic palette, substances, and arena.
    pub fn with_arena(mut self, arena: SyntheticArena) -> Result<Self, String> {
        let (palette, substances) = fixture_assets()?;
        self.app.insert_resource(palette);
        self.app.insert_resource(substances);
        self.app.insert_resource(arena);
        self.app.add_systems(
            OnEnter(Screen::Gameplay),
            spawn_synthetic_arena.in_set(GameplaySetup::Terrain),
        );
        Ok(self)
    }

    /// Finalizes plugins and returns the runnable app.
    pub fn build(mut self) -> App {
        while self.app.plugins_state() != PluginsState::Cleaned {
            self.app.finish();
            self.app.cleanup();
        }
        self.app
    }
}

fn spawn_synthetic_arena(mut commands: Commands, arena: Res<SyntheticArena>) {
    for surface in arena.surfaces() {
        commands.spawn((
            HexTile,
            surface.position.coord,
            surface.position,
            surface.span,
            surface.substance,
            Headroom(surface.headroom),
        ));
    }
}

/// Enters gameplay through the same state transition used by production.
pub fn enter_gameplay(app: &mut App) {
    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Gameplay);
    app.update();
    app.update();
}

/// Bounded execution failure from [`run_until`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunLimitExceeded {
    /// Number of frames the caller permitted.
    pub frames: usize,
}

impl fmt::Display for RunLimitExceeded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "condition did not become true within {} deterministic frames",
            self.frames
        )
    }
}

impl std::error::Error for RunLimitExceeded {}

/// Advances a deterministic app until `done` observes success or `frames` expire.
pub fn run_until(
    app: &mut App,
    frames: usize,
    mut done: impl FnMut(&mut World) -> bool,
) -> Result<usize, RunLimitExceeded> {
    for frame in 0..frames {
        if done(app.world_mut()) {
            return Ok(frame);
        }
        app.update();
    }
    if done(app.world_mut()) {
        Ok(frames)
    } else {
        Err(RunLimitExceeded { frames })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arenas_publish_unique_exact_surfaces() {
        let arena = SyntheticArena::stacked(2, 1, 5);
        assert_eq!(arena.surfaces().len(), 20);
        assert!(arena
            .surfaces()
            .windows(2)
            .all(|pair| matches!(pair, [left, right] if left.position < right.position)));
        assert!(arena
            .surfaces()
            .iter()
            .any(|surface| surface.position == TilePos::new(HexCoord::ORIGIN, 5)));
    }

    #[test]
    fn bounded_runner_reports_non_completion_as_data() {
        let mut app = TestAppBuilder::new().build();
        assert_eq!(
            run_until(&mut app, 3, |_| false),
            Err(RunLimitExceeded { frames: 3 })
        );
    }
}
