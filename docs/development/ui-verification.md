# Runtime UI verification

This document is the fail-closed inventory for runtime UI work. A screen being
spawned, a button having a nonzero Bevy `ComputedNode`, or a scripted click
completing is not evidence that a player can understand and complete the task.
Every player task below must have a populated presentation fixture, named primary
controls, a structural oracle, and an application transition test where the task
changes canonical state.

The inventory is deliberately task-oriented. Several tasks may share one Bevy
`Screen`, and a new enum variant or route is incomplete until this table and the
machine-readable task-case registry are updated together.

## Navigation model

The title exposes independent primary routes. `New Game` launches the configured
default game and is not repeated in a development catalog. `Map Scenarios` contains
map/world presentation scenarios. `Demos` contains focused gameplay demonstrations.
The two catalogs are separate destinations; they never share a mixed column or make
the player infer the distinction from card copy.

Creator tools are player-facing tools, not scenario cards. Character Creator and
Spell Creator are separately named, first-class title routes; neither is hidden
behind the other.
Combat Lab remains a primary title route because it is the main gameplay experiment
surface.

```text
Title
├── Continue ───────────────────────────────> Gameplay
├── New Game ───────────────────────────────> Loading -> Gameplay
├── Character Creator ──────────────────────> character library/workspace
├── Spell Creator ──────────────────────────> spell library/workspace
├── Combat Lab ─> Sandbox / Fixtures / Reports -> Deployment -> Gameplay
├── Map Scenarios ──────────────────────────> map-only catalog -> Loading
├── Demos ──────────────────────────────────> demo-only catalog -> Loading
└── Settings
```

## Complete task inventory

`Immediate` means the complete control must be visible in the initial viewport.
`Scrollable` means it may begin offscreen only when every clipping ancestor has a
real Bevy scroll owner that can bring the complete 44x44 target into view.

| Area | Populated task case | Primary controls (`Immediate`) | Secondary / scrollable content | Behavioral oracle |
|---|---|---|---|---|
| Startup | Splash | none | progress copy | state transition reaches Title |
| Startup | Loading | none | load/failure copy | terminal asset/setup state chooses Gameplay or Title failure |
| Title | cold start | Continue (disabled with reason), New Game, Character Creator, Spell Creator, Combat Lab, Map Scenarios, Demos, Settings, Quit | none | each typed title intent chooses its exact route and entry identity |
| Title | resumable start | all cold routes; Continue enabled | none | Continue restores the exact canonical save route |
| Title | setup failure | all title routes | failure reason | failure survives return without changing route identity |
| Map Scenarios | full production catalog | Back | every visible map card and generated-map reroll | projection contains only `Map`; Start preserves clicked snapshot; reroll is session-only |
| Demos | full production catalog | Back | every visible demo card | projection contains only `Demo` and excludes the configured New Game default |
| Settings | every preference row | Back | preference rows and persistence notice | every row emits its typed setting; v1/v2 persistence remains canonical |
| Character Creator | empty character library | New Blank Character, Open Spell Creator, Title | empty-state guidance | canonical Creator transition enters the requested workspace |
| Character Creator | populated character library | New Blank Character, Open Spell Creator, Title | shipped templates and saved characters | exact saved/template identity is retained |
| Character Creator | blank invalid workspace | Library, Save (refused), undo/redo/reset/delete as applicable | palette, lattice tools, validation details | canonical edit state reports the refusal and dirty state |
| Character Creator | ready/clean workspace | Library, Save, Test Locally, Test on Map | palette, spell inscription, metadata | save/test routes preserve exact character identity |
| Creator recovery | unreadable library/reset confirmation | Confirm Reset, Title | recovery notice | the inline second-step confirmation cannot reset on its first activation |
| Character Creator | delete confirmation | Library, Save | Confirm Delete and workspace detail | the inline second-step confirmation cannot delete on its first activation |
| Spell Creator | empty spell library | New Blank Spell, Title | empty-state guidance | canonical Creator transition enters the Spell library directly |
| Spell Creator | populated spell library | New Blank Spell, Title | shipped templates and saved spells | exact saved/template identity is retained |
| Spell Creator | blank invalid workspace | Library, Save (refused), undo/redo/reset/delete as applicable | shape/effect/cost controls and validation | canonical edit state reports the refusal and dirty state |
| Spell Creator | ready/clean workspace | Library, Save | shape/effect/cost controls | save preserves exact spell identity and map-readiness facts |
| Spell Creator | delete confirmation | Library, Save | Confirm Delete and workspace detail | the inline second-step confirmation cannot delete on its first activation |
| Lattice Demo | ordinary and maximum-content states | Cast controls, End Turn, Reset, Back | lattice/log detail | typed lattice-demo intents change renderer-free demo state |
| Combat Lab | Sandbox: Map | Back, Sandbox/Fixtures/Reports tabs, Continue to Rosters | full production map list | selected map ID and step transition are canonical |
| Combat Lab | Sandbox: Rosters 1v1 | Back to Map, Continue to Rules | roster/template/saved-character lists | ordered IDs and valid `UnitId(0)` survive edits |
| Combat Lab | Sandbox: Rosters 6v6/blocked saved content | Back to Map, Continue/refusal | all roster controls and blocked reasons | max-size and readiness rules come from `hex_gameplay_model` |
| Combat Lab | Sandbox: Rules shipped/tactical/custom | Back to Rosters, Load Map & Deploy | all rule fields/presets | exact profile enters deployment; invalid setup is refused |
| Combat Lab | Fixtures full/filter/empty | Back, tabs | complete fixture cards and search results | exact fixture/profile snapshot launches; no-result is explicit |
| Combat Lab | Reports empty | Back, tabs | empty-state guidance | no stale comparison identity exists |
| Combat Lab | Reports populated/compare/delete/error | Back, tabs; modal confirm/cancel when active | saved cards, annotations, compare selectors | comparison sides remain independent; delete and error lifecycle are canonical |
| Deployment | incomplete 1v1 | Undo, auto-place, clear, Back to Rules; Start disabled with reason | roster rows | exact positions and completeness come from gameplay snapshot |
| Deployment | complete and maximum 6v6 | Start Combat, Back to Rules | roster rows | launch keeps exact roster/profile/map positions |
| Gameplay | exploration party/formation | action rail; Group/Solo, Rest and formation actions | party/inspector detail | canonical mode, selection, formation and position snapshot |
| Gameplay | player combat turn/max actions | complete Now/Choose rail | inspector/log/lattices | budgets and Channel state from authority resources; `presented_actions` checks labels/refusal copy only, while command contracts prove legality |
| Gameplay | hostile turn | required rail status; no illegal player action | inspector/log | turn owner from authority state; presented refusal/absence from `presented_actions`; command rejection from owning contracts |
| Casting | spell list mixed enabled/blocked | Cancel and legal spell actions | spell detail | presented cost/blocked copy from the app snapshot; legality and refusal authority stay in owning command contracts |
| Casting | aiming legal/blocked | Confirm when legal, Cancel/cycle as applicable | target detail | exact target and refusal are canonical, never pixel-derived |
| Decision | disable partial/complete | required cells and Confirm/refusal | lattice detail | owed/chosen cells and one-action accounting are canonical |
| Decision | restore partial/complete | required cells and Confirm/refusal | lattice detail | owed/chosen cells and restoration are canonical |
| Gameplay chrome | HUD hidden ordinary/required | action rail remains visible; HUD toggle | lattice and statistics hide together; a blocking lattice is promoted into the rail | hiding chrome cannot leave statistics behind or hide/mutate a blocking decision |
| Gameplay chrome | target lattice opaque/known/absent; log empty/dense | action rail | lattice/log drawer | visibility does not reconstruct hidden gameplay truth |
| Combat Lab live | statistics collapsed/expanded/manual end | action rail | lattice first, then expand/collapse, statistics body, and End Experiment in the single Inspector scroll flow | typed toggle, actual wheel/Tab scrolling, drawer lifecycle, and report fingerprint from gameplay snapshot |
| Pause | ordinary/save success/save failure | Resume and applicable save/leave actions | notice | focus is trapped and typed pause/save actions retain state |
| Outcome | ordinary victory/defeat | return/retry actions | outcome summary | canonical encounter outcome chooses presentation |
| Lab outcome | Overview/Units/Spells & Effects/Timeline/Compare | report mode and Retry/Copy/Tune/Return actions | one-axis report body and comparisons | frozen identity, modes, comparison and launch routes remain canonical |
| Development overlays | UI debug, diagnostics, inspector | overlay toggles | diagnostic-only detail | never enabled in acceptance captures or shipping evidence |

## Coverage tiers and runtime budget

Comprehensive layout coverage stays headless and structural; it does not multiply
screenshots or renderer launches. The suite uses five complementary tiers:

1. Pure tests exhaust every scale mode, breakpoint boundary, priority rule, and
   renderer-free Creator/Combat Lab transition.
2. Every populated interactive task runs the complete physical-size, device-scale,
   and seven-mode semantic-scale matrix in the single headless hex_ui test binary.
   This includes intermediate 125%, 150%, and 175% modes; endpoint-only sampling is
   insufficient. Splash and Loading alone remain representative smoke states.
   Named primary controls must be immediate; named secondary controls must be
   scroll-reachable through an attainable range.
3. Real-input regressions complement that matrix for scroll ownership and focus:
   cursor plus wheel/trackpad events, Tab/Shift-Tab, nested-scroll boundary handoff,
   and visible focus rings. Static geometry cannot prove that Bevy will deliver an
   input event to the intended owner.
4. Responsive state is also tested as a transition. At minimum, enlarged Compact UI
   must return to Auto/Standard without retaining old flex direction, scroll
   ownership, insets, visibility, or control scale. Fresh-app matrix passes do not
   substitute for this re-entry check.
5. The bounded visual walk captures ten representative high-risk frames. Humans
   judge hierarchy, density, readability, and feel; pixels never prove gameplay.

The machine contract fails when a focusable control has no explicit visibility
classification. Shared controls default to `Immediate`; a secondary surface must
opt a control into `Scrollable` at the point where the real scroll owner is created.
This makes a missing annotation a false failure instead of allowing a primary action
to disappear as a false pass. These lightweight components are runtime semantic
metadata: they keep the authored hierarchy self-describing in shipping builds, while
the default-off observation harness is the only code that reads them as an oracle.
Unnamed interactive entities also fail closed, and the named controls in each task
contract declare their relative keyboard focus order.

One `UiTaskCase` may intentionally populate several closely coupled branches on the
same surface—for example the saved-report fixture contains an error, comparison, and
pending delete confirmation together. A branch may be grouped only when its named
control and presentation are simultaneously observable; mutually exclusive layouts
require separate task cases.

## Required review evidence

The structural run reports each task-case ID, viewport, scale mode, missing named
control, and exact clipping/scroll ancestor. The visual route reports the same case
ID before writing its PNG. Gameplay acceptance uses typed `ReviewCapture` steps;
generic `Capture` remains available to unchanged world-owner walks. Enabling UI-debug
overlays rejects gameplay acceptance captures, so diagnostic outlines cannot become
green evidence. A walk step that merely completes, a non-black image, or `coverage:
true` cannot satisfy the route.

The ten-image budget selects the riskiest presentation states; it is not the list of
everything tested. Map-owned Forest, Waterfall, V3, and map-review routes remain
unchanged and outside this inventory.
