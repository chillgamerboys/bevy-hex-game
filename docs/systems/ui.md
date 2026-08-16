# Runtime UI

The runtime UI is a presentation adapter, not a gameplay authority. `hex_game`
observes canonical resources and components, projects immutable view models, and
applies typed `UiIntent` messages through the same command and navigation funnels as
other inputs. `hex_ui` renders those models. It must not infer legality, mutate
combat state, or import gameplay/world implementations.

## Dependency contract

`hex_ui` may depend only on Bevy, `hex_core`, `hex_assets`,
`hex_gameplay_model`, and serialization support. It must never depend on
`hex_game`, `hex_combat`, `hex_units`, `hex_lattice`, `hex_map`, `hex_world`, or
`hex_perception`.

The public seam is deliberately small:

- `UiPlugin` installs the design system, focus navigation, responsive scaling, and
  renderer-owned surfaces.
- Immutable screen view models contain display-ready facts. They do not expose
  mutable screen state or ECS entities.
- `UiIntent` describes player intent. The composition root decides whether and how
  it changes application or gameplay state.
- `ActionAffordance` carries the label, shortcut, priority, and canonical enabled or
  disabled reason for one action.
- The default-off `test-support` feature exposes immutable UI-tree observations.
  `HeadlessUiPlugin` runs the real Bevy UI, text, focus, and layout schedules on a
  synthetic primary window without Winit, a renderer, or gameplay plugins.

Main Menu, Campaign, Sandbox, and Creator transitions remain in
`hex_gameplay_model`. A widget may select or submit a typed transition, but it must
not duplicate the transition policy. Route-specific immutable views render the
Main Menu, three Campaign cards, Sandbox dashboard and child routes, while
`DeploymentView` renders one compact guided task card and its Review actions.
`hex_game` alone validates Campaign records and map readiness, resolves exact terrain
clicks, freezes launch identity, and admits Start. Exact 3D placement tokens remain
spatial world presentation, not runtime UI.

## Information hierarchy

Every interactive screen uses the same hierarchy:

1. A title or breadcrumb says where the player is.
2. Setup flows show the current step and the remaining steps.
3. Main content contains the current choice, with validation adjacent to the
   affected control.
4. A persistent footer contains Back and the next or confirm action.

Gameplay is intentionally map-first. Four ordinary components can participate in
screen-space layout independently:

- **Party** — a compact ordered roster with small, non-interactive lattice
  silhouettes;
- **Initiative** — the disclosed combat order only;
- **Activity** — a bounded history with All, Combat, and Activity tabs; and
- **Action Bar** — only the actions currently authorized by the application adapter.

The **Main View** is not a fifth visibility Boolean. It is a typed contextual host:
`Closed`, `Character(UnitId)`, `Formation`, or `RequiredDecision`. Character and
Formation are explicit destinations and may be replaced by another ordinary Main
View request. A required damage or restoration choice is different: it is forcibly
open, cannot be dismissed or replaced, and remains available until the canonical
decision resolves.

Default ordinary presentation is Party visible, Initiative eligible only during
combat, Activity closed, Action Bar visible when actions exist, and Main View closed.
Effective visibility is the saved component preference, contextual eligibility,
master suppression, and phase suppression resolved together. The master binding (`H`
by default) changes only transient suppression; restoring it returns to the exact
saved combination.
Deployment and terminal outcomes suppress ordinary gameplay components without
rewriting preferences. A hidden component leaves layout, picking, focus order,
scrolling, and the accessibility tree completely—there are no collapsed rails,
handles, or invisible hit targets.

The canonical default component bindings are `P`, `I`, `L`, and `B` for Party,
Initiative, Activity, and Action Bar. Character and Formation default to `V` and `F`,
and Cycle Camera View remains separate on `C`; all are configurable. On Standard/Wide,
component activation toggles its saved preference. While the HUD is master-hidden,
it summons only the requested temporary surface and leaves everything else hidden.
Main View activation changes no selection, turn, caster, or command authority.

Activating a Party or disclosed Initiative entry once inspects and centers that unit.
Activating the same entry again opens its Character Main View. Map camera centering is
one-shot; Third Person and First Person follow the authorized inspected subject.
Unobserved hostiles cannot be activated and publish neither a camera subject nor a
location.

Acting/selected identity, retained target identity, movement range, pathing, and aim
remain world-space feedback rather than another HUD panel. The continuous foot ring
marks the acting or selected unit and a shape-distinct reticle marks one disclosed
target. Both ignore picking, inherit unit visibility, and clear during Deployment and
outcomes. Existing terrain-health presentation remains unchanged and is not promoted
into this HUD model. Sandbox sessions add no statistics or report history;
`CombatSummary` remains a gameplay/test observation.

Builds with the default-off `dev` feature may add development-only time controls, but
they follow the same effective visibility, phase suppression, and one-scroll-owner
rules. Shipping builds contain neither that panel nor its adapter.

## Responsive model

The world camera always renders at native resolution. Bevy's global `UiScale`
remains `1.0`, and operating-system DPI scaling stays authoritative. Semantic
tokens scale UI content without scaling the entire Bevy layout tree.

Auto scale is:

```text
clamp(min(logical_width / 1920, logical_height / 1080), 1.0, 1.5)
```

Manual 75%, 100%, 125%, 150%, 175%, and 200% choices replace Auto; they do not
multiply it. Body and supporting type use that content scale subject to the 18px
essential-text floor. Display, title, and heading type use
`1 + 0.5 × (content_scale - 1)`, capped at `1.5`. Control
geometry uses the same moderated growth above 100% but never shrinks below its
authored baseline, preserving 44×44 logical-pixel targets. Spacing uses
`1 + 0.25 × (content_scale - 1)`, capped at `1.25`. Layout is selected from the
logical canvas divided by the greater of content and spacing scale. This matters at
200%: a 1920×1080 window must reflow to Compact instead of keeping Standard side
rails beside doubled essential copy. Below 100%, spacing remains the density limit so
smaller type does not unexpectedly promote an ordinary canvas to Wide.

| Class | Semantic-density-adjusted logical canvas | Behavior |
|---|---|---|
| Compact | below 1440×810 | blank map by default; one explicit shortcut may open one temporary full-screen task surface; a required decision forcibly owns that surface |
| Standard | at least 1440×810 and below 2400 px wide | map plus independently visible Party, Initiative, Activity, Action Bar, and at most one typed Main View |
| Wide | at least 2400 px wide | the same component contract with bounded horizontal compositions and more map breathing room |

Compact setup and Creator pages use one vertical page-scroll owner. The Character
Creator lattice is the sole exception that needs a bounded two-axis pan surface: its
custom Bevy-native handler consumes only motion the canvas can use, then bubbles the
remaining vertical delta to the page at its boundary. Keyboard focus first reveals a
cell inside that canvas and then reveals the canvas inside the outer page. Idle nested
ScrollArea components are forbidden because Bevy 0.19 consumes wheel events before
checking whether that child can move.

Representative gameplay structure:

```text
Compact                         Standard / Wide
┌──────────────────┐            ┌ Party ───── Initiative ─────┐
│       map        │            │                              │
│                  │            │ map / typed Main View        │
│ shortcut: one    │            │                              │
│ full-screen task │            ├ Activity ────── Action Bar ─┤
└──────────────────┘            └──────────────────────────────┘
```

Compact contains no drawer handle or collapsed residue. The same shortcut or Escape
closes an ordinary temporary surface and returns to the blank map. A required decision
captures that route until answered. Every active temporary or Main View surface owns
at most one vertical scroll route.

The required structural matrix covers 1280×720, 1920×1080, and 3840×2160 under Auto
and 200% semantic UI scale. Additional breakpoint and device-scale cases may extend
that minimum. Device pixels remain separate from logical layout.
Primary controls must be fully visible immediately; secondary catalog content
may instead prove complete scroll reachability. Every required control remains
unobscured, accessible, and at least 44×44. `UiTreeSnapshot` intersects each node
and named text node's actual glyph rectangles with the canvas and Bevy's inherited
`CalculatedClip`; a nonzero `ComputedNode` whose glyphs or box cross a clipped edge
is not treated as fully visible. The oracle also checks focus order and interactive
overlap without interpreting the text or pixels as gameplay truth.
The matrix uses Main Menu, all three Campaign record states, Sandbox Overview, map
browser and both map-detail modes, sparse and dense Party/Enemies rosters, character
picker, Tools, populated Settings, Creator, guided 6v6 Deployment placement and Review,
and the maximum ordinary gameplay component combination plus Main View, required,
aiming, master-hidden, Compact temporary-surface, and phase-suppressed states. A half
logical pixel is the only target-size tolerance, accounting for physical-pixel
rounding at fractional Auto scales.

## Typography, spacing, and contrast

Canonical element colors come from the same `ElementVisualCatalog` that owns the
Creator grid glyph treatments. Spell requirements, cast rows and previews, Creator
cells, and combat/demo lattice cells resolve that authored tint wherever the element's
stable identity is present. Unknown custom basics retain deterministic wheel hues,
and unknown custom fusions retain the generic fusion tint, so extensible content never
disappears.

At 100% scale the semantic type tokens are:

| Token | Size | Use |
|---|---:|---|
| Display | 48 | game/title display |
| Screen title | 32 | top-level screen heading |
| Heading | 24 | sections and surface titles |
| Body/control | 20 | required information and controls |
| Supporting | 18 | guidance and validation detail |
| Metadata | 16 | optional, nonessential annotations only |

Essential text must be at least 18 logical pixels. Pointer targets are at
least 44×44 logical pixels. Layout uses semantic gaps and panel padding rather than
screen-specific offsets where a shared token applies.

Normal text targets at least 4.5:1 contrast. Large text, focus indicators, and other
meaningful non-text boundaries target at least 3:1. State is never communicated by
color alone: labels such as “Required”, “Unavailable”, or the refusal reason remain
visible. These thresholds follow
[Microsoft's game text guidance](https://learn.microsoft.com/en-us/xbox/accessibility/xbox-accessibility-guidelines/101),
[Microsoft's UI-context guidance](https://learn.microsoft.com/en-us/xbox/accessibility/xbox-accessibility-guidelines/114),
and [WCAG 2.2](https://www.w3.org/TR/WCAG22/).

## Focus and keyboard behavior

Interactive controls participate in a logical Tab/Shift-Tab order and carry an
`AccessibleLabel`. Keyboard focus has a visible high-contrast outline. Enter and
Space activate the focused control through its ordinary interaction handler. Escape
first closes an ordinary gameplay Main View or Compact temporary task, then uses the
screen's typed Back/resume intent when no such surface owns it. A Required Decision
cannot consume Escape and remains open.

When a focusable control owns keyboard focus, Enter, Space, and Tab do not also
dispatch gameplay confirm, end-turn, or next-target shortcuts. The focused control's
typed intent is the single input for that keypress; explicit cancel/back keys remain
available.

The runtime derives sequential focusability from the visible hierarchy. Controls in
a hidden or `Display::None` subtree temporarily leave the tab order and return at
their original logical index when the surface reopens; focus is cleared if its
control becomes unreachable.

A true modal uses a Bevy `TabGroup` so focus cannot escape until its blocking choice
is resolved. Ordinary HUD components and Main View destinations are not modals and do
not trap focus. Controller navigation is intentionally deferred.

Settings exposes keyboard bindings by category tabs: Gameplay, Interface, Main View,
Camera, and System. One action owns one key chord in this slice. Rebinding captures
the next non-modifier key at highest input priority, Escape cancels, and no captured
key reaches gameplay. A conflict in an overlapping context offers Swap or Cancel and
never silently steals a binding; exclusive Menu and Gameplay uses may coexist. Each
row can restore its default, but a default currently owned after a swap reuses that
conflict modal and leaves preferences unchanged until the player chooses. Restore All
requires confirmation and affects only the active action inventory. Shipping excludes
the development-only Reveal Knowledge action from rows, conflicts, and Restore All,
while preserving its serialized override for a later development run. If a shipping
edit occupies that chord, development startup moves only Reveal Knowledge to the first
free deterministic modified chord instead of rejecting the player's other settings.
Enter and Space are accepted only for Confirm Decision, Next Target, and End Turn,
whose handlers already yield to a focused control. Tab and Escape navigation remain
fixed UI semantics.

This foundation uses Bevy's stable tab navigation, focus, accessibility, image
render targets, and screenshot components. The global `UiScale` remains 1.0;
semantic typography, control, and spacing tokens provide accessibility scaling
without doubling whole panels. Experimental widgets and BSN are outside this
stabilization change.

## Preferences

Preferences schema v3 persists `UiScaleMode`, per-component HUD visibility, and only
keyboard overrides from canonical defaults. Reading v1 or v2 preserves existing
display, audio, and UI-scale values while supplying the default HUD combination and
an empty override map. Changes preview immediately and use the existing atomic
preferences writer. Master suppression, Compact temporary surfaces, the inspected
unit, and the Main View destination are runtime-only and never survive restart.

## Testing oracle boundary

The exhaustive player-task inventory, coverage tiers, and fail-closed control
classification live in [Runtime UI verification](../development/ui-verification.md).
That inventory is the acceptance source for route and task-case completeness; this
section defines which oracle may prove each kind of fact.

Use the cheapest authoritative oracle:

- Pure view-model, scale, breakpoint, intent, and priority behavior stays inline in
  `hex_ui`.
- Game and UI wiring uses the existing `gameplay_app` integration target with
  `test-support`. `GameplayStateSnapshot` reads authority resources/components and
  labels its copied HUD affordances `presented_actions`; those affordances prove
  adapter parity, not command legality. `UiTreeSnapshot` reads presentation structure
  only.
- Deterministic combat and balance evidence belongs to the rules, contracts, and
  simulation partitions.
- The scoped presentation route reviews at most ten deterministic Bevy image-target
  frames from `walks/gameplay_ui.ron`. Every gameplay `ReviewCapture` declares an
  exact `UiTaskCase` and passes that task's live named-control contract before a PNG
  can be written; merely reaching the right screen is insufficient. A typed review
  viewport supplies logical size and device scale; Bevy renders into an
  `ImageRenderTarget` and captures it with `Screenshot::image`. Diagnostic UI
  overlays are rejected on this acceptance route. No operating-system capture API
  or primary-window screenshot participates. Generic world-owner `Capture` steps
  remain unchanged.

A screenshot must never prove legality, budgets, decisions, damage, Channel,
outcomes, persistence, deployment, or launch/retry identity. The scoped gameplay
visual script therefore uses no combat-solving steps; presentation fixtures open
authored states while typed tests prove their canonical facts. Generic world-owned
walks keep their existing driver verbs and acceptance criteria.

Forest, Waterfall, map-review, V3, and world-owned captures remain outside this
contract and are unchanged.

Local commands:

```sh
mkdir -p .context/ui-review
ui_review_data="$(mktemp -d .context/ui-review/data.XXXXXX)"
HEX_GAME_DATA_DIR="$ui_review_data" \
HEX_WALK_SCRIPT=walks/gameplay_ui.ron \
HEX_WALK_OUT=.context/ui-review/bevy \
cargo run -p hex_game --features visual-walk
```

## Screen audit

| Surface | Primary task | Persistent action | Secondary content |
|---|---|---|---|
| Splash/loading | understand progress | none | none |
| Main Menu | choose route | Campaign, Sandbox, Tools, Settings | none |
| Campaign | choose one of exactly three slots | Back / New Game or Continue | party, active time, or invalid reason |
| Sandbox Overview | review temporary encounter and launch | Back / Start Sandbox | committed map and two six-slot roster summaries |
| Sandbox Map Browser / Detail | choose and confirm one map | Back / Use Map | pending seed, Regenerate only for generated maps |
| Sandbox Party / Enemies | edit an ordered six-slot side | Back | shared roster component and Map-ready diagnostics |
| Character Picker | preview then commit one character | Back / Use Character | templates, saved characters, Creator entry |
| Multiplayer | discover/configure a host, assign seats, and launch | Back or the role-authorized lobby action | LAN browser compatibility, Direct code entry, six stable seats, readiness, and typed connection state |
| Tools | choose an authoring tool | Back | Character Creator, Spell Creator, disabled Map Creator |
| Settings | change general preferences or keyboard bindings | Back | category tabs, capture/conflict state, persistence notice |
| Character / Spell Creator | finish the current authoring step | Library / Save / Test where applicable | palettes, catalogs, validation, history, typed origin |
| Deployment | place occupied Party then Enemy slots one at a time and review exact choices | Undo / Return to Sandbox / Start Combat in Review | compact current-character card, exact legal-surface and occupancy refusal, no ordinary HUD |
| Gameplay | act or inspect without obscuring the map | eligible Action Bar or forced required decision | independently persisted Party/Initiative/Activity plus typed Main View |
| Pause | resume, save, or leave | Resume | save notice |
| Sandbox outcome | acknowledge result | Retry Exact / Return to Sandbox | Victory or Defeat only |

When a screen changes, review it against the hierarchy, focus, compact reflow, and
oracle boundary above before adding a screen-specific workaround.
