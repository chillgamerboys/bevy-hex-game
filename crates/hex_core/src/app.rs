//! App-wide vocabulary: screen states, pause state, and system ordering.
//!
//! These live in `hex_core` rather than in the binary because `hex_world` and
//! `hex_gameplay` both need to gate systems on them, and those two crates are
//! forbidden from depending on each other.

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;
use bevy_state::prelude::*;

/// The screen the player is currently looking at.
///
/// Transitions run in order at startup — `Splash` → `Title` → `Loading` →
/// `Gameplay` — with `Loading` the point where assets are guaranteed present.
/// Spawning gameplay entities on `OnEnter(Screen::Gameplay)` rather than at
/// `Startup` is what removes the old implicit ordering hazard, where
/// `spawn_player` read a resource that only existed because an unrelated plugin
/// happened to run in `PreStartup`.
#[derive(States, Reflect, Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub enum Screen {
    /// Engine warm-up. Brief, and skipped past automatically.
    #[default]
    Splash,
    /// Main menu.
    Title,
    /// Waits for assets to finish loading before gameplay is allowed to spawn.
    Loading,
    /// The game itself.
    Gameplay,
}

/// Whether gameplay is paused.
///
/// A [`SubStates`] of [`Screen::Gameplay`], so it cannot exist while the player
/// is in a menu — the type system rules out "paused on the title screen".
#[derive(SubStates, Reflect, Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
#[source(Screen = Screen::Gameplay)]
pub struct Pause(pub bool);

/// Systems that must stop while the game is paused.
///
/// Attach with `.in_set(PausableSystems)`. Movement and animation belong here;
/// camera control generally does not, since players expect to look around while
/// paused.
#[derive(SystemSet, Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct PausableSystems;

/// Ordering for world construction on entering [`Screen::Gameplay`].
///
/// Building a world has a dependency chain — resources, then the terrain, then the
/// things standing on the terrain — and each step lives in a different crate.
/// `hex_map` builds the map; `hex_gameplay` spawns the player onto it. Systems added
/// to the same `OnEnter` schedule otherwise run in **unspecified order**, and
/// `.chain()` cannot express ordering across a crate boundary because neither crate
/// can see the other's systems.
///
/// Bevy inserts a sync point between ordered sets, which matters here beyond mere
/// ordering: entities spawned through `Commands` in one set are not queryable until
/// those commands are applied. Placing [`Self::Actors`] in a later set than
/// [`Self::Terrain`] is what makes the terrain *visible* to the systems that need
/// it, not just earlier.
#[derive(SystemSet, Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum GameplaySetup {
    /// Insert resources the world is built from, such as generator configuration.
    Resources,
    /// Spawn the map itself — the terrain everything else stands on.
    Terrain,
    /// Spawn entities that need the terrain to already exist, such as the player.
    ///
    /// Systems here can query tiles and read their
    /// [`HexSpan`](crate::HexSpan)s. Systems in [`Self::Terrain`] cannot.
    Actors,
}

/// Coarse ordering for everything in `Update`.
///
/// Bevy runs systems in parallel and in unspecified order unless told otherwise.
/// Before this existed, `orbit_camera` and `pan_camera` both took `&mut Transform`
/// on the same entity with no declared ordering — benign only because they happen
/// to touch different fields. Putting every system in one of these sets makes the
/// frame's shape explicit rather than emergent.
#[derive(SystemSet, Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum AppSystems {
    /// Advance timers.
    TickTimers,
    /// Read input into components and resources.
    RecordInput,
    /// Everything else. Split this further as the game grows.
    Update,
}
