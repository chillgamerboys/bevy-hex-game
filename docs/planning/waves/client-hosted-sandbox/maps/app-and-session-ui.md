# App and session UI map

Banked against `origin/dev@92662d456746506093e8de61f54f1d619085e1fe`.
If refreshed source disagrees with an anchor or state transition, escalate and amend the
manifest before L4 edits it.

## Current navigation and launch seams

- `Screen` has no multiplayer state at `crates/hex_core/src/app.rs:20-44`.
- `MainMenuRoute` has Root/Campaign/Tools at
  `crates/hex_gameplay_model/src/main_menu.rs:7-17`.
- The pure route model/back behavior is at
  `crates/hex_gameplay_model/src/main_menu.rs:72-107`.
- The runtime main-menu adapter is at
  `crates/hex_game/src/screens/main_menu.rs:13-102`; its test locks exactly four product
  routes at lines 115-124.
- The renderer's four root actions are at
  `crates/hex_ui/src/main_menu.rs:76-115`.
- `MainMenuView` and `MainMenuIntent` live at
  `crates/hex_ui/src/model.rs:1232-1280`.
- Screen adapter composition is at `crates/hex_game/src/screens/mod.rs:1-30`.
- Sandbox freezes a private launch identity at
  `crates/hex_game/src/screens/sandbox.rs:103-165`; the multiplayer manifest must replace
  cross-process reliance on this private struct without making it public wholesale.
- Loading freezes `ScenarioToLoad` at `crates/hex_game/src/scenarios.rs:63-79` and clears
  stale world resources at lines 101-123.

## Dispositions

| Current symbol | Disposition |
|---|---|
| `Screen` | add `Multiplayer`; keep Sandbox/Loading/Gameplay state meanings |
| `MainMenuRoute` | add `Multiplayer` child route, making the root exactly five product actions |
| `MainMenuModel` | keep as renderer-free route owner |
| `MainMenuView` / `MainMenuIntent` | extend only for opening Multiplayer; detailed session views/intents live in a dedicated module |
| `SandboxLaunchSnapshot` | keep private local draft/retry adapter; freeze a public transport-neutral `SessionManifestV1` when hosting |
| `ScenarioToLoad` | keep private composition input; host and clients independently adapt the accepted manifest into it |
| `GameplayPhase` | keep; Multiplayer activation remains `Preparing/Deployment -> Active`, with lobby/map verification before Active |
| `Pause` | keep as host global state; remote Escape opens a separate local menu model and cannot transition `Pause` |
| `OutcomeView` | project host-only actions; client sees status/leave-only actions and cannot emit Retry/Close authority requests |

## New renderer-free session state

`hex_gameplay_model::multiplayer` owns pure screen behavior and immutable views for:

- Multiplayer home: Host Direct / Join Direct / Back;
- host setup handoff into existing Sandbox configuration/deployment;
- join-code input with redacted display/storage;
- six seats with connected/reserved/delegated state, assignments, readiness, and host;
- map/content/build/protocol verification progress and typed refusals;
- reconnecting, rejoined, host-ended, kicked, and closed states;
- host launch/retry/return/close actions and client leave/local-menu actions.

The model carries public credential handles or redacted display values only. Secret bytes
remain in `hex_multiplayer` secret wrappers and storage adapters.

Host/join/lobby/reconnect are session-scale lifecycle states, not merely a title submenu.
The root action enters `Screen::Multiplayer`; a pure multiplayer model owns its child
routes. `hex_ui` remains presentation-only and `hex_game` adapts session snapshots/intents,
preserving the UI crate's dependency ceiling.

Assignment changes clear readiness for every affected seat. Launch requires every
connected seat to own at least one member, the host to own at least one, and every non-host
connected seat ready. Admission closes at launch; reconnect identity is a distinct path.

## Pause, save, and outcome

Current gameplay directly toggles global `Pause` on any `GameplayAction::Pause` at
`crates/hex_game/src/screens/gameplay.rs:111-166`. L4 routes that action by session role:
host/offline toggles global pause; a remote replica toggles a local non-pausing overlay.
The existing pause renderer at `crates/hex_ui/src/screens.rs:82-140` remains the global host
overlay and gets a separate local-client view rather than lying about `Pause(true)`.

The existing save gate already requires paused, quiescent exploration at
`crates/hex_game/src/save.rs:1257-1271` and checks queue/decision/movement at
`crates/hex_game/src/save.rs:1273-1337`. Milestone A exposes no multiplayer Campaign save;
L4 merely ensures clients cannot trigger this path. Milestone B extends it host-only.
Persisted `selected` state in the current Campaign payload is local UI state and must not
enter the multiplayer durable snapshot.

Outcome actions are currently unconditional local intents at
`crates/hex_game/src/screens/gameplay.rs:475-529`. The session adapter makes the host the
only authority for Retry Exact, Return to Lobby, and Close Session; client buttons cannot
manufacture those transitions.

## UX and evidence

Direct host UI explains editable UDP port `7777`, router forwarding, and likely CGNAT
failure. It never displays or logs reconnect tokens and may display the connection code
only in the explicit host/share surface. Errors name compatibility category without
echoing invite material.

Static visual evidence is required for all named frames in the manifest. Native input,
local camera independence, movement interpolation, pause feel, and reconnect experience
require a named human on the exact combined head; screenshots are not logic evidence.

## Territory

#186/#190 overlap `hex_game` composition/gameplay readouts, #188 overlaps movement walk,
and #189 overlaps combat/save/`hex_ui::model`. L4 may be designed against stable views but
does not edit those regions until they land or the manifest is amended with exact symbols.
