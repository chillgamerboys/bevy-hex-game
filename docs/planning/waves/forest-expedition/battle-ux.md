# Battle navigation, readability and recording

Status: integrated local candidate with native acceptance in progress for the user-approved September 12 plan on the existing
`wave/forest-expedition` candidate, base `4b697e90be6ee52c00b1b0f7b984abb8b88a4024`.
The hourly automation remains paused. No remote publication or dev/main merge.

## Locked behavior

- M toggles an initially hidden north-up whole-map overview without pausing. Esc
  has a larger interactive map and one personal pin. Discover only observed
  Dragons, Shadow, Troll and fountains; remember last-seen positions and charge
  state across pause/death, clear on restart. Observation is geometric, 10 Hz,
  0.3 seconds, with frustum, projected size and occlusion checks.
- Only spells, compact player HP, reticle, optional map and recording state remain
  persistent in player combat. One visible target gets three/two/one health pips
  at >2/3, >1/3, otherwise. Recent-hit fallback lasts two seconds.
- Larger responsive menus: Overview, Map, Upgrades, Settings, Controls; scaling
  through 200%. Clear spell-ready/cooldown/held-charge states from gameplay.
- Expedition player only: height 1.2, eye 1.02, radius .25, movement 4.725.
  One physical profile drives movement, hits, prediction, camera and launch.
- Mac 15+ video-only window recorder: H.264 MP4, 30 fps, SDR, maximum 1080p,
  `~/Movies/Hex Game/Recordings`. Esc start/stop/open-folder, REC duration,
  F9 bookmarks, local metadata/events. Pause/death/restart stay in one clip;
  quit finalizes. No desktop/audio capture and no automatic recording.

## Ownership and composition

One coordinated wave, because the UI, observation and recording lifecycle share
one meaningful playtest and shared arena composition. Root owns integration and
all HUD/menu/map/input presentation. Workers use clean source worktrees from this
checkpoint; no worker writes the integration branch. Existing PRs 219/220 and
historical expedition branches are preserved, not merged by branch tip.

1. World worker owns `hex_core` overview DTO (separate foundation commit) and
   `hex_map` publication/cache. No gameplay or hex_game edits.
2. Gameplay worker owns `hex_arena` player profile, observations/discovery,
   targeted health and spell availability snapshots. No world or hex_game edits.
3. Recorder worker owns new `hex_game/src/arena/recording.rs` and its children,
   native Swift helper/build/package support. Root alone wires arena mod.rs/HUD.
4. Root owns `hex_game` HUD, navigation and shared wiring, preferences, combined
   tests and review. World foundation, world publication, gameplay, recording
   and presentation are integrated. Follow-up work preserves those ownership
   boundaries and is composed by root before validation.

## Contracts

- `hex_core::arena::ArenaOverview`: a default-empty Resource with `generation:u64`,
  `width:u32`, `height:u32`, `min:Vec2`, `max:Vec2`, `rgba:Vec<u8>`.
  Vec2 axes are world X/Z; first image row is minimum Z (north). World publishes
  it once per generation/selection, covering the full finite region. Empty on
  unsupported maps. Cache identity includes package and art fingerprints; ordinary
  terrain destruction intentionally leaves this authored overview unchanged. No
  enemy/fountain knowledge is encoded in these pixels.
- Gameplay exports `PlayerObservation` (camera origin/direction, vertical FOV,
  aspect, viewport height), `DiscoveredLandmark`, `LandmarkKind`,
  `CombatFeedbackSnapshot`, and spell availability facts, consumed by the integrated
  presentation adapter. Observations are input;
  gameplay owns filtering, dwell, memory and current health feedback.
- Recorder exports `install(&mut App)` and Resource `Recorder` with
  `request_toggle()`, `request_open_folder()`, `request_quit()`, `status_text()`,
  `is_recording()`, `elapsed_seconds()`, `can_record()` methods. It owns async
  lifecycle, exit interception, bookmarks and metadata, never combat state.

## Validation and evidence

Focused tests per lane; combined actor/world/UI/lifecycle tests on the root
candidate. The current selector promotes unclassified hex_game changes to the
complete gate. Do not call inherited failing lint debt passed. Actual 115-actor
routes must be rerun for new dimensions. Use true UI pointer/layout tests at
1280x720, 1600x900, 1920x1080 and scaling through 200%; inspect fresh windowless
captures. Native input, motion and recording require the named native playtest.
Measure UI p95 (<1ms target), recording on/off frame-time delta (<10% target), and
record limitations accurately. No static image substitutes for logical hooks.

### September 12 local validation checkpoint

At `a7013698fed80c18b87e7d346db864e7958a194d`, the combined candidate has:

- 401 pure arena tests, 138 Battle application tests and nine overview tests
  passing. Explicitly ignored tests are recorded separately, not counted as passed.
- Seven actual expedition checks passing, including admission of all 115 actors,
  taller-player route/bridge probes, fountain and reward access, full reset and
  the large encounter checks. The final defeat/Restart pointer regression also
  passes after the menu layout fixes.
- Pointer, keyboard, text bounds, full-map fit, pin clearing and overflow cues
  covered at 1280x720, 1600x900 and 1920x1080 with 100% and 200% scaling.
- Ten clean-source windowless frames individually inspected, followed by a full
  contact sheet and independent review. HP clipping, the 200% expanded map,
  scroll discoverability, V4 overview colors and partial/full charge captures
  were corrected and rechecked. This clears the named static surfaces only.
- Format, dependency policy, tracked Markdown links, 116 selector tests, 55
  launcher tests and strict game-library Clippy passing. The native Cargo build
  and Swift helper compile/protocol checks pass.

The exact canonical selector Clippy command executed for 205 seconds and failed
with 517 inherited `hex_map` library diagnostics. None of its diagnostic files
changed in the Battle UX diff. The candidate-validation workflow stops the full
gate at this first substantive failure; subsequent broad concerns and the shipping
release build are not claimed passed. This candidate is not merge-ready.

Native launch reached the ready screen without asset errors. The Mac was initially
locked; after user unlock, the computer-use tool could not address the unbundled
Cargo process. Authorization for the game-specific input driver is pending.
No focused active-play telemetry or completed video exists yet, so the <1 ms
additional UI and <10% recording frame-time targets remain unverified. Short
windowless timing samples are diagnostic wall spans, not exclusive UI CPU evidence.
Recorder review additionally identified saturated-command-queue Quit and a blocking
folder-open call. Follow-up `be62a96` retries a full Quit queue with one five-second
deadline and opens folders in a separate bounded job; all 11 recorder tests pass,
including four new lifecycle regressions; strict game-library Clippy passes again.
These changes do not alter map or UI
presentation, so the preceding static review retains its named-surface scope.

Logs, capture receipts, independent static review and the reusable native telemetry
analyzer are in the local `outputs/expedition-validation` artifact directory. The
hourly timer remains paused. No PR, `dev` merge or Linear mutation is part of this
local checkpoint.
