//! Capability-based deterministic Bevy app construction for owning tests.
//!
//! This crate shares mechanics rather than fixtures. Callers opt into assets,
//! states, input, and shared schedule sets explicitly so a test for a missing
//! plugin or resource cannot receive it merely by adopting the harness.

use std::fmt;
use std::time::Duration;

use bevy::app::PluginsState;
use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use hex_core::{AppSystems, GameplaySetup, Mode, Pause, Screen};

/// Default deterministic duration advanced by each update in the complete shell.
pub const DEFAULT_FIXED_STEP: Duration = Duration::from_millis(100);

/// Builds a deterministic headless app from explicit capabilities.
pub struct HeadlessAppBuilder {
    app: App,
}

impl Default for HeadlessAppBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl HeadlessAppBuilder {
    /// Starts with a bare app and no plugins, resources, states, or schedules.
    #[must_use]
    pub fn new() -> Self {
        Self { app: App::new() }
    }

    /// Installs Bevy's minimal headless plugin group and a deterministic clock.
    #[must_use]
    pub fn with_minimal_plugins(mut self) -> Self {
        self.app.add_plugins(MinimalPlugins);
        self.app
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                DEFAULT_FIXED_STEP,
            ));
        self
    }

    /// Installs Bevy's asset server and task-backed asset infrastructure.
    #[must_use]
    pub fn with_asset_plugin(mut self) -> Self {
        self.app.add_plugins(AssetPlugin::default());
        self
    }

    /// Installs an explicitly configured asset plugin.
    ///
    /// This keeps file-backed tests authoritative when they need a temporary
    /// asset root or another non-default asset-server setting.
    #[must_use]
    pub fn with_asset_plugin_config(mut self, plugin: AssetPlugin) -> Self {
        self.app.add_plugins(plugin);
        self
    }

    /// Initializes the mesh/material stores used by headless presentation tests.
    #[must_use]
    pub fn with_render_assets(mut self) -> Self {
        self.app.init_asset::<Mesh>();
        self.app.init_asset::<StandardMaterial>();
        self
    }

    /// Installs both the asset plugin and common headless render-asset stores.
    #[must_use]
    pub fn with_assets(self) -> Self {
        self.with_asset_plugin().with_render_assets()
    }

    /// Installs Bevy's state-transition plugin without initializing a state.
    #[must_use]
    pub fn with_state_plugin(mut self) -> Self {
        self.app.add_plugins(StatesPlugin);
        self
    }

    /// Initializes the shared screen, mode, and pause state vocabulary.
    #[must_use]
    pub fn with_gameplay_states(mut self) -> Self {
        self.app.init_state::<Screen>();
        self.app.add_sub_state::<Mode>();
        self.app.add_sub_state::<Pause>();
        self
    }

    /// Installs the state plugin and initializes the shared gameplay state vocabulary.
    #[must_use]
    pub fn with_states(self) -> Self {
        self.with_state_plugin().with_gameplay_states()
    }

    /// Installs Bevy's input resources without a window or renderer.
    #[must_use]
    pub fn with_input(mut self) -> Self {
        self.app.add_plugins(bevy::input::InputPlugin);
        self
    }

    /// Configures the shared deterministic update phases.
    #[must_use]
    pub fn with_update_sets(mut self) -> Self {
        self.app.configure_sets(
            Update,
            (
                AppSystems::TickTimers,
                AppSystems::RecordInput,
                AppSystems::Update,
            )
                .chain(),
        );
        self
    }

    /// Configures the complete cross-owner gameplay-entry ordering contract.
    #[must_use]
    pub fn with_gameplay_sets(mut self) -> Self {
        self.app.configure_sets(
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
        self
    }

    /// Installs the complete compatibility shell used by existing gameplay/map tests.
    #[must_use]
    pub fn with_gameplay_shell(self) -> Self {
        self.with_minimal_plugins()
            .with_assets()
            .with_states()
            .with_input()
            .with_update_sets()
            .with_gameplay_sets()
    }

    /// Gives the owning test access before plugins are finalized.
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

    /// Finalizes plugins and returns the runnable app.
    pub fn build(mut self) -> App {
        while self.app.plugins_state() != PluginsState::Cleaned {
            self.app.finish();
            self.app.cleanup();
        }
        self.app
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
    fn bare_builder_inserts_no_hidden_capabilities() {
        let app = HeadlessAppBuilder::new().build();
        assert!(!app
            .world()
            .contains_resource::<bevy::time::TimeUpdateStrategy>());
        assert!(!app.world().contains_resource::<State<Screen>>());
        assert!(!app.world().contains_resource::<Assets<Mesh>>());
    }

    #[test]
    fn complete_shell_has_deterministic_time_states_and_assets() {
        let app = HeadlessAppBuilder::new().with_gameplay_shell().build();
        assert!(matches!(
            app.world().resource::<bevy::time::TimeUpdateStrategy>(),
            bevy::time::TimeUpdateStrategy::ManualDuration(duration)
                if *duration == DEFAULT_FIXED_STEP
        ));
        assert_eq!(
            app.world().resource::<State<Screen>>().get(),
            &Screen::Splash
        );
        assert!(app.world().contains_resource::<Assets<Mesh>>());
    }

    #[test]
    fn infrastructure_plugins_do_not_imply_gameplay_or_render_stores() {
        let app = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_asset_plugin()
            .with_state_plugin()
            .build();

        assert!(!app.world().contains_resource::<State<Screen>>());
        assert!(!app.world().contains_resource::<Assets<Mesh>>());
    }

    #[test]
    fn configured_asset_plugin_installs_the_asset_server() {
        let app = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_asset_plugin_config(AssetPlugin {
                file_path: "test-assets".to_owned(),
                ..default()
            })
            .build();

        assert!(app.world().contains_resource::<AssetServer>());
    }

    #[test]
    fn bounded_runner_reports_non_completion_as_data() {
        let mut app = HeadlessAppBuilder::new().with_minimal_plugins().build();
        assert_eq!(
            run_until(&mut app, 3, |_| false),
            Err(RunLimitExceeded { frames: 3 })
        );
    }
}
