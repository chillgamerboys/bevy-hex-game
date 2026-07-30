//! App-wide vocabulary: screen states, pause state, and system ordering.
//!
//! These live in `hex_core` rather than in the binary because `hex_world` and
//! `hex_units` both need to gate systems on them, and those two crates are
//! forbidden from depending on each other.

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;
use bevy_state::prelude::*;
use serde::{Deserialize, Serialize};

/// The screen the player is currently looking at.
///
/// The app starts at `Splash`, advances to `Title`, and waits for the player to
/// continue to `Loading` and then `Gameplay`. `Loading` waits for settings-derived
/// resources and for asset handles to reach a terminal state. Spawning gameplay
/// entities on `OnEnter(Screen::Gameplay)` rather than at `Startup` is what removes
/// the old implicit ordering hazard, where `spawn_player` read a resource that only
/// existed because an unrelated plugin happened to run in `PreStartup`.
#[derive(States, Reflect, Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub enum Screen {
    /// Engine warm-up. Brief, and skipped past automatically.
    #[default]
    Splash,
    /// Main menu.
    Title,
    /// Persistent pre-alpha display and volume preferences.
    Settings,
    /// Interactive sandbox for the lattice ruleset, reached from the title menu.
    ///
    /// Exists as an isolated manual-verification surface for magic rules — casting,
    /// fusions, mana, disables, and enchantments. Slated for gating or removal
    /// before release.
    LatticeDemo,
    /// Waits for settings and terminal asset states before gameplay may spawn.
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

/// Whether the world is running in real time or taking turns.
///
/// The game plays like Baldur's Gate 3: real time while nothing is happening, turn
/// based the moment something is. There is **one map** and one set of units either
/// way — this is a change of tempo, not a change of place.
///
/// A [`SubStates`] of [`Screen::Gameplay`], for the same reason [`Pause`] is: "in
/// combat on the title screen" should be unrepresentable rather than merely unlikely.
///
/// Deliberately **not** sourced on `Pause(false)`. That would make the mode cease to
/// exist the instant someone hit escape, taking any `OnEnter(Mode::Combat)` UI with
/// it and resurrecting it on unpause.
#[derive(SubStates, Reflect, Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
#[source(Screen = Screen::Gameplay)]
pub enum Mode {
    /// Real time. Move freely; nothing is waiting on anyone.
    #[default]
    Exploring,
    /// Turn based. Units act in initiative order and a turn has to be ended.
    Combat,
}

/// Marks the one unit currently allowed to act, and what it has left.
///
/// Exactly one exists during [`Mode::Combat`] and none otherwise, so "is it my turn"
/// is a query filter rather than an index that can disagree with the world.
///
/// Lives here rather than with the rest of the combat machinery because **two crates
/// need it and neither can see the other**: `hex_combat` decides whose turn it is,
/// and `hex_units` has to refuse a move when it is not yours. That is the same
/// situation `Headroom` is in, and it gets the same answer — the shared fact goes in
/// the crate both sides already depend on.
#[derive(
    Component, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
)]
#[reflect(Component)]
pub struct Turn {
    /// Hexes of movement still available this turn.
    pub movement_left: u32,
    /// Whether this unit has taken its action.
    pub acted: bool,
}

/// A full cycle of the turn order has completed.
///
/// Written by the combat loop when the order wraps to the front. Anything with
/// per-round bookkeeping — effects that expire, knowledge that decays, fight
/// length — reads this instead of watching a round counter for changes, so
/// every consumer agrees on exactly when a round ended.
///
/// Lives here rather than in `hex_combat` for the same reason [`Turn`] does:
/// crates that cannot see each other need the same word for it.
#[derive(Message, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RoundElapsed;

/// Systems that must stop while the game is paused.
///
/// Attach with `.in_set(PausableSystems)`. Movement and animation belong here;
/// camera control generally does not, since players expect to look around while
/// paused.
#[derive(SystemSet, Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct PausableSystems;

/// Ordering for world construction on entering [`Screen::Gameplay`].
///
/// Building a world has a dependency chain — resources, terrain, the things standing
/// on the terrain, presentation derived from that complete geometry, then final
/// contract checks — and each step lives in a different crate. `hex_map` validates and
/// builds the map, `hex_units` spawns the player onto it, `hex_perception` derives what
/// those actors can observe, and `hex_world` frames the result. Systems added to
/// the same `OnEnter` schedule otherwise run in **unspecified order**, and `.chain()`
/// cannot express ordering across a crate boundary because no leaf crate can see all
/// the others' systems.
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
    /// Apply a validated exploration resume after default actors spawn.
    ///
    /// This phase is intentionally before perception: restored positions must be the
    /// positions observed on the first gameplay frame.
    Restore,
    /// Derive illumination and initial faction knowledge from terrain and actors.
    ///
    /// This phase lets later presentation depend on published knowledge without
    /// creating direct crate dependencies.
    Perception,
    /// Apply presentation that depends on the completed terrain and its actors.
    ///
    /// Generated camera framing belongs here so a view hint cannot race terrain
    /// generation, and future actor-aware framing sees commands flushed by
    /// [`Self::Perception`].
    View,
    /// Verify that terrain and required actors were published successfully.
    ///
    /// This terminal phase sees commands flushed by [`Self::View`], so setup
    /// failures can return to a visible screen instead of leaving an empty world.
    Finalize,
}

/// Shared cross-crate phases for systems in `Update`.
///
/// Bevy runs systems in parallel and in unspecified order unless told otherwise.
/// Before this existed, `orbit_camera` and `pan_camera` both took `&mut Transform`
/// on the same entity with no declared ordering — benign only because they happen
/// to touch different fields. Systems opt into these phases when their ordering
/// participates in the frame's shared input/update flow; self-contained state and
/// UI systems do not need to.
#[derive(SystemSet, Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum AppSystems {
    /// Advance timers.
    TickTimers,
    /// Read input into components and resources.
    RecordInput,
    /// Everything else. Split this further as the game grows.
    Update,
}

/// Shared gameplay reconciliation boundaries used across crate ownership.
///
/// These are narrower than [`AppSystems`]: they name completed projections that a
/// presentation consumer may safely read after deferred component commands flush.
#[derive(SystemSet, Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum GameplaySystems {
    /// Selected unit, camera focus, rings, and movement overlays agree.
    Selection,
    /// Casting controls are projected from the reconciled actor and selection.
    Casting,
    /// The explicit gameplay UI role context is ready for panels to consume.
    UiContext,
}
