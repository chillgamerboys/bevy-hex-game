//! The player's live lattice and the retained, knowledge-gated hostile target.

use bevy::picking::Pickable;
use bevy::prelude::*;
use hex_assets::{ElementCatalog, SpellBook};
use hex_combat::{CombatSystems, FactionLatticeKnowledge, TurnOrder};
use hex_core::{
    AppSystems, CommandQueue, ControlOwner, GameCommand, GameplaySystems, IssuedCommand, Mode,
    PendingDecision, Screen, UnitId,
};
use hex_lattice::{LatticeSpec, LatticeState, LatticeStats};
use hex_units::{Downed, Faction, Player, StandsOn, UnitRegistry};

use crate::casting::{AimExit, Aiming};
use crate::menus::lattice_view::{
    known_cell_view, live_cell_view, spawn_lattice_cells, CellInteraction, LatticeCellView,
    LatticeScale,
};
use crate::readouts::{
    region, DecisionHud, GameplayUiContext, HudElement, HudRegion, HudSetup, InspectorRole,
    TargetProvenance, READ_ONLY_HUD,
};
use hex_ui::{blurb, fine, heading, panel, row_button, UiAssets, EDGE, PANEL_BG};

const PULSE_COLOR: Color = Color::srgba(0.25, 0.10, 0.06, 0.9);
#[derive(Resource, Default, Debug, PartialEq)]
struct LatticeReadouts {
    own: Option<OwnLattice>,
    target: Option<TargetLattice>,
}

#[derive(Debug, PartialEq)]
struct OwnLattice {
    unit: UnitId,
    role: InspectorRole,
    identity: String,
    cells: Vec<LatticeCellView>,
    decision: Option<DecisionSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DecisionSummary {
    pub(crate) chosen: usize,
    pub(crate) owed: usize,
    pub(crate) restoring: bool,
}

#[derive(Debug, PartialEq)]
struct TargetLattice {
    unit: UnitId,
    provenance: TargetProvenance,
    identity: String,
    state: TargetState,
}

#[derive(Debug, PartialEq)]
enum TargetState {
    Opaque,
    Known {
        cells: Vec<LatticeCellView>,
        unknown: Option<usize>,
    },
}

/// The last hostile a real aim named and the turn that pinned it.
///
/// Empty anchors and a confirmed cast preserve this target for inspection. Explicit
/// cancellation, a turn change, combat exit, or target invalidation clears it.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetainedTarget {
    pub(crate) unit: Option<UnitId>,
    pinned_turn: Option<UnitId>,
}

/// Local interaction state for the one player disable decision.
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub(crate) struct DisableSelection {
    pub(super) decision: Option<DisableDecision>,
    cells: Vec<hex_core::LatticeCoord>,
}

impl DisableSelection {
    pub(super) fn is_active(&self) -> bool {
        self.decision.is_some()
    }

    pub(crate) fn summary(&self) -> Option<DecisionSummary> {
        self.decision.as_ref().map(|decision| DecisionSummary {
            chosen: self.cells.len(),
            owed: decision.owed,
            restoring: decision.restoring,
        })
    }

    #[cfg(test)]
    pub(crate) fn begin_test_decision(
        &mut self,
        decider: UnitId,
        target: UnitId,
        owed: usize,
        restoring: bool,
    ) {
        self.decision = Some(DisableDecision {
            decider,
            target,
            owed,
            restoring,
            live: vec![hex_core::LatticeCoord::ORIGIN],
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DisableDecision {
    pub(super) decider: UnitId,
    pub(super) target: UnitId,
    pub(super) owed: usize,
    pub(super) restoring: bool,
    pub(super) live: Vec<hex_core::LatticeCoord>,
}

#[derive(Component)]
struct OwnPanel;

#[derive(Component)]
struct OwnHeading;

#[derive(Component)]
pub(super) struct TargetPanel;

#[derive(Component)]
struct TargetHeading;

#[derive(Component)]
struct OwnBody;

#[derive(Component)]
struct TargetBody;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct OwnCell(hex_core::LatticeCoord);

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecisionControl {
    Clear,
    Confirm,
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<hex_core::InputBindings>()
        .init_resource::<LatticeReadouts>()
        .init_resource::<RetainedTarget>()
        .init_resource::<DisableSelection>()
        .add_systems(
            OnEnter(Screen::Gameplay),
            spawn_panels.in_set(HudSetup::Panels),
        )
        .add_systems(
            Update,
            handle_decision_input
                .in_set(AppSystems::RecordInput)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            (sync_disable_selection, retain_target)
                .chain()
                .in_set(AppSystems::Update)
                .after(CombatSystems::Advance)
                .after(GameplaySystems::Casting)
                .before(GameplaySystems::UiContext)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            (refresh_readouts, rebuild_panels)
                .chain()
                .in_set(AppSystems::Update)
                .after(GameplaySystems::UiContext)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(OnExit(Screen::Gameplay), clear_focus);
}

fn spawn_panels(
    mut commands: Commands,
    mut readouts: ResMut<LatticeReadouts>,
    mut focus: ResMut<RetainedTarget>,
    mut selection: ResMut<DisableSelection>,
    assets: Res<UiAssets>,
    regions: Query<(Entity, &HudRegion)>,
) {
    *readouts = LatticeReadouts::default();
    *focus = RetainedTarget::default();
    *selection = DisableSelection::default();

    let stack = commands
        .spawn((
            Name::new("Lattice Readout Stack"),
            Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|stack| {
            stack
                .spawn((
                    Name::new("Own Lattice Panel"),
                    OwnPanel,
                    DecisionHud,
                    HudElement,
                    panel(),
                    READ_ONLY_HUD,
                ))
                .insert(Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(12.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(10.0)),
                    row_gap: Val::Px(7.0),
                    ..default()
                })
                .with_children(|panel| {
                    panel.spawn((OwnHeading, heading(&assets, "selected ally")));
                    panel.spawn((
                        Name::new("Own Lattice Body"),
                        OwnBody,
                        Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(5.0),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                });
            stack
                .spawn((
                    Name::new("Target Lattice Panel"),
                    TargetPanel,
                    HudElement,
                    panel(),
                    READ_ONLY_HUD,
                ))
                .insert(Node {
                    display: Display::None,
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(12.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(10.0)),
                    row_gap: Val::Px(7.0),
                    ..default()
                })
                .with_children(|panel| {
                    panel.spawn((TargetHeading, heading(&assets, "aim target")));
                    panel.spawn((
                        Name::new("Target Lattice Body"),
                        TargetBody,
                        Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(5.0),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                });
        })
        .id();
    if let Some(inspector) = region(HudRegion::Inspector, &regions) {
        commands.entity(inspector).add_child(stack);
    }
}

/// Updates retention from an actual hostile occupying the aimed surface.
pub(super) fn retain_target(
    mode: Res<State<Mode>>,
    aiming: Res<Aiming>,
    mut exit: ResMut<AimExit>,
    order: Res<TurnOrder>,
    mut focus: ResMut<RetainedTarget>,
    units: Query<(&UnitId, &Faction, &StandsOn, Has<Downed>)>,
) {
    let ended = std::mem::take(&mut *exit);
    let target_valid = focus.unit.is_none_or(|retained| {
        units
            .iter()
            .any(|(unit, _, _, downed)| *unit == retained && !downed)
    });
    if reconcile_retained_lifecycle(
        &mut focus,
        *mode.get(),
        ended,
        order.current(),
        target_valid,
    ) {
        return;
    }

    let Some(aim) = aiming.0.as_ref() else {
        return;
    };
    if focus.pinned_turn.is_none() {
        focus.pinned_turn = order.current();
    }
    if let Some((unit, faction, _, downed)) = units
        .iter()
        .find(|(_, _, standing, _)| standing.0.pos == aim.anchor)
    {
        if Faction::Player.is_hostile_to(*faction) && !downed {
            focus.unit = Some(*unit);
            focus.pinned_turn = order.current();
        } else {
            *focus = RetainedTarget::default();
        }
    }
}

/// Clears a pin when its explicit lifetime ends.
///
/// Returns whether the caller should stop before considering the current aim.
fn reconcile_retained_lifecycle(
    focus: &mut RetainedTarget,
    mode: Mode,
    ended: AimExit,
    current_turn: Option<UnitId>,
    target_valid: bool,
) -> bool {
    if mode != Mode::Combat
        || ended == AimExit::Cancelled
        || !target_valid
        || (focus.pinned_turn.is_some() && focus.pinned_turn != current_turn)
    {
        *focus = RetainedTarget::default();
        return mode != Mode::Combat || ended == AimExit::Cancelled;
    }
    false
}

type OwnData<'w, 's> = Query<
    'w,
    's,
    (
        &'static UnitId,
        &'static Name,
        &'static LatticeSpec,
        &'static LatticeState,
        &'static LatticeStats,
    ),
>;

fn sync_disable_selection(
    pending: Res<PendingDecision>,
    registry: Res<UnitRegistry>,
    lattices: Query<(&LatticeSpec, &LatticeState)>,
    players: Query<(), With<Player>>,
    mut selection: ResMut<DisableSelection>,
) {
    let next = match *pending {
        PendingDecision::ChooseDisables { decider, count, .. } => registry
            .entity_of(decider)
            .filter(|&entity| players.contains(entity))
            .and_then(|entity| lattices.get(entity).ok())
            .map(|(spec, state)| {
                let live: Vec<_> = spec
                    .cells()
                    .filter(|&(coord, _)| !state.is_disabled(coord))
                    .map(|(coord, _)| coord)
                    .collect();
                DisableDecision {
                    decider,
                    target: decider,
                    owed: usize::from(count).min(live.len()),
                    restoring: false,
                    live,
                }
            }),
        PendingDecision::ChooseRestores {
            decider,
            target,
            count,
        } => registry
            .entity_of(decider)
            .filter(|&entity| players.contains(entity))
            .and_then(|_| registry.entity_of(target))
            .and_then(|entity| lattices.get(entity).ok())
            .map(|(spec, state)| {
                let disabled: Vec<_> = spec
                    .cells()
                    .filter(|&(coord, _)| state.is_disabled(coord))
                    .map(|(coord, _)| coord)
                    .collect();
                DisableDecision {
                    decider,
                    target,
                    owed: usize::from(count).min(disabled.len()),
                    restoring: true,
                    live: disabled,
                }
            }),
        PendingDecision::None => None,
    };
    reconcile_selection(&mut selection, next);
}

fn reconcile_selection(selection: &mut DisableSelection, next: Option<DisableDecision>) {
    if selection.decision == next {
        return;
    }
    selection.cells = next
        .as_ref()
        .filter(|decision| decision.owed == decision.live.len())
        .map(|decision| decision.live.clone())
        .unwrap_or_default();
    selection.decision = next;
}

fn refresh_readouts(
    mut readouts: ResMut<LatticeReadouts>,
    context: Res<GameplayUiContext>,
    selection: Res<DisableSelection>,
    registry: Res<UnitRegistry>,
    knowledge: Res<FactionLatticeKnowledge>,
    elements: Option<Res<ElementCatalog>>,
    spells: Option<Res<SpellBook>>,
    own: OwnData,
    identities: Query<(&Name, &Faction)>,
) {
    let (Some(elements), Some(spells)) = (elements, spells) else {
        return;
    };

    let own_view = context
        .inspector
        .as_ref()
        .map(|(role, identity)| (*role, identity.clone()))
        .map(|(role, identity)| {
            let unit = identity.unit;
            (unit, role, identity.label())
        })
        .and_then(|(unit, role, identity)| {
            let entity = registry.entity_of(unit)?;
            let (unit, _name, spec, state, stats) = own.get(entity).ok()?;
            Some((unit, role, identity, spec, state, stats))
        })
        .map(|(unit, role, identity, spec, state, stats)| OwnLattice {
            unit: *unit,
            role,
            identity,
            cells: spec
                .cells()
                .map(|(coord, kind)| {
                    let armed = selection.decision.as_ref().is_some_and(|decision| {
                        decision.target == *unit && decision.live.contains(&coord)
                    });
                    live_cell_view(
                        coord,
                        kind,
                        stats,
                        state,
                        &elements,
                        &spells,
                        if armed {
                            CellInteraction::Actionable
                        } else {
                            CellInteraction::ReadOnly
                        },
                        selection.cells.contains(&coord),
                    )
                })
                .collect(),
            decision: selection
                .decision
                .as_ref()
                .filter(|decision| decision.target == *unit)
                .map(|decision| DecisionSummary {
                    chosen: selection.cells.len(),
                    owed: decision.owed,
                    restoring: decision.restoring,
                }),
        });

    // No hostile `LatticeSpec` or `LatticeState` appears in this function. The
    // target projection can only be assembled from `FactionLatticeKnowledge::view`.
    let target_view = context.target.as_ref().and_then(|(provenance, identity)| {
        let unit = identity.unit;
        let entity = registry.entity_of(unit)?;
        let (_name, faction) = identities.get(entity).ok()?;
        if !Faction::Player.is_hostile_to(*faction) {
            return None;
        }
        let known = knowledge.view(Faction::Player, unit)?;
        let state = if known.is_opaque() {
            TargetState::Opaque
        } else {
            TargetState::Known {
                cells: known
                    .cells()
                    .map(|(coord, cell)| {
                        known_cell_view(
                            coord,
                            cell.kind,
                            cell.mana,
                            None,
                            cell.disabled,
                            &elements,
                            &spells,
                        )
                    })
                    .collect(),
                unknown: known.unknown_count(),
            }
        };
        Some(TargetLattice {
            unit,
            provenance: *provenance,
            identity: identity.label(),
            state,
        })
    });

    let next = LatticeReadouts {
        own: own_view,
        target: target_view,
    };
    if *readouts != next {
        *readouts = next;
    }
}

fn rebuild_panels(
    mut commands: Commands,
    readouts: Res<LatticeReadouts>,
    own_bodies: Query<Entity, With<OwnBody>>,
    target_bodies: Query<Entity, With<TargetBody>>,
    mut target_panels: Query<&mut Node, With<TargetPanel>>,
    mut own_headings: Query<&mut Text, (With<OwnHeading>, Without<TargetHeading>)>,
    mut target_headings: Query<&mut Text, (With<TargetHeading>, Without<OwnHeading>)>,
    assets: Res<UiAssets>,
) {
    if !readouts.is_changed() {
        return;
    }

    if let Ok(body) = own_bodies.single() {
        commands.entity(body).despawn_related::<Children>();
        commands.entity(body).with_children(|body| {
            let Some(own) = readouts.own.as_ref() else {
                body.spawn(blurb(&assets, "no player lattice"));
                return;
            };
            body.spawn(blurb(&assets, own.identity.clone()));
            spawn_lattice_cells(
                body,
                &own.cells,
                &assets,
                LatticeScale::PANEL,
                "Own",
                OwnCell,
            );
        });
    }
    if let (Ok(mut heading), Some(own)) = (own_headings.single_mut(), readouts.own.as_ref()) {
        heading.0 = own.role.label().to_lowercase();
    }

    let has_target = readouts.target.is_some();
    if let Ok(mut node) = target_panels.single_mut() {
        node.display = if has_target {
            Display::Flex
        } else {
            Display::None
        };
    }
    if let Ok(body) = target_bodies.single() {
        commands.entity(body).despawn_related::<Children>();
        commands.entity(body).with_children(|body| {
            let Some(target) = readouts.target.as_ref() else {
                return;
            };
            body.spawn(blurb(&assets, target.identity.clone()));
            match &target.state {
                TargetState::Opaque => {
                    body.spawn(blurb(&assets, "lattice unknown"));
                }
                TargetState::Known { cells, unknown } => {
                    spawn_lattice_cells(
                        body,
                        cells,
                        &assets,
                        LatticeScale::PANEL,
                        "Target",
                        |_| (),
                    );
                    if let Some(unknown) = unknown.filter(|unknown| *unknown > 0) {
                        body.spawn(fine(&assets, format!("{unknown} cells unknown")));
                    }
                }
            }
        });
    }
    if let (Ok(mut heading), Some(target)) =
        (target_headings.single_mut(), readouts.target.as_ref())
    {
        heading.0 = target.provenance.label().to_lowercase();
    }
}

pub(crate) fn spawn_decision_controls(
    body: &mut ChildSpawnerCommands,
    decision: DecisionSummary,
    assets: &UiAssets,
) {
    body.spawn(fine(
        assets,
        format!(
            "{}/{} {} cells chosen",
            decision.chosen,
            decision.owed,
            if decision.restoring {
                "disabled"
            } else {
                "live"
            }
        ),
    ));
    body.spawn((
        Name::new("Disable Decision Controls"),
        Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(8.0),
            ..default()
        },
        Pickable::IGNORE,
    ))
    .with_children(|controls| {
        controls
            .spawn((
                row_button("Clear Disable Selection", 119.0),
                DecisionControl::Clear,
            ))
            .with_children(|button| {
                button.spawn(blurb(assets, "clear"));
            });
        if decision.chosen == decision.owed {
            controls
                .spawn((
                    row_button("Confirm Disable Selection", 119.0),
                    DecisionControl::Confirm,
                ))
                .with_children(|button| {
                    button.spawn(blurb(assets, "confirm"));
                    button.spawn(fine(assets, "ENTER"));
                });
        } else {
            controls
                .spawn((
                    Name::new("Confirm Disable Selection Disabled"),
                    Node {
                        width: Val::Px(119.0),
                        height: Val::Px(46.0),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(1.0),
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        ..default()
                    },
                    BorderColor::all(EDGE),
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.03)),
                    Pickable::IGNORE,
                ))
                .with_children(|button| {
                    button.spawn(blurb(assets, "confirm"));
                    button.spawn(fine(assets, "choose more"));
                });
        }
    });
}

#[expect(
    clippy::too_many_arguments,
    reason = "the emitter mirrors every answer precondition before entering the funnel"
)]
fn handle_decision_input(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<hex_core::InputBindings>,
    pending: Res<PendingDecision>,
    registry: Res<UnitRegistry>,
    mut queue: ResMut<CommandQueue>,
    mut selection: ResMut<DisableSelection>,
    cells: Query<(&Interaction, &OwnCell), Changed<Interaction>>,
    controls: Query<(&Interaction, &DecisionControl), Changed<Interaction>>,
    lattices: Query<(&LatticeSpec, &LatticeState)>,
    owners: Query<&ControlOwner, With<Player>>,
) {
    let (decider, target, count, restoring) = match *pending {
        PendingDecision::ChooseDisables { decider, count, .. } => (decider, decider, count, false),
        PendingDecision::ChooseRestores {
            decider,
            target,
            count,
        } => (decider, target, count, true),
        PendingDecision::None => return,
    };
    if queue.holds_answer_for(decider) {
        return;
    }
    let Some(decider_entity) = registry.entity_of(decider) else {
        return;
    };
    let Some(target_entity) = registry.entity_of(target) else {
        return;
    };
    let (Ok((spec, state)), Ok(owner)) = (lattices.get(target_entity), owners.get(decider_entity))
    else {
        return;
    };
    let Some(decision) = selection
        .decision
        .as_ref()
        .filter(|decision| {
            decision.decider == decider
                && decision.target == target
                && decision.restoring == restoring
        })
        .cloned()
    else {
        return;
    };

    let mut clear = false;
    let mut confirm = bindings.just_pressed(&keys, hex_core::InputAction::Confirm);
    for (interaction, control) in &controls {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match control {
            DecisionControl::Clear => clear = true,
            DecisionControl::Confirm => confirm = true,
        }
    }
    if clear {
        selection.cells.clear();
    }
    for (interaction, cell) in &cells {
        if *interaction == Interaction::Pressed {
            toggle_cell(&mut selection, cell.0);
        }
    }

    if !confirm {
        return;
    }
    let selectable: Vec<_> = spec
        .cells()
        .filter(|&(coord, _)| state.is_disabled(coord) == restoring)
        .map(|(coord, _)| coord)
        .collect();
    let owed = usize::from(count).min(selectable.len());
    let valid = owed == decision.owed
        && selection.cells.len() == owed
        && selection.cells.iter().all(|cell| selectable.contains(cell));
    if !valid {
        return;
    }
    queue.push(IssuedCommand {
        seat: owner.0,
        command: if restoring {
            GameCommand::ChooseRestores {
                unit: decider,
                target,
                cells: selection.cells.clone(),
            }
        } else {
            GameCommand::ChooseDisables {
                unit: decider,
                cells: selection.cells.clone(),
            }
        },
    });
}

fn toggle_cell(selection: &mut DisableSelection, cell: hex_core::LatticeCoord) {
    let Some(decision) = selection.decision.as_ref() else {
        return;
    };
    if !decision.live.contains(&cell) {
        return;
    }
    if let Some(index) = selection
        .cells
        .iter()
        .position(|selected| *selected == cell)
    {
        selection.cells.remove(index);
    } else if selection.cells.len() < decision.owed {
        selection.cells.push(cell);
    }
}

fn clear_focus(mut focus: ResMut<RetainedTarget>) {
    *focus = RetainedTarget::default();
}

pub(super) fn set_pulse_color(
    active: bool,
    panels: &mut Query<&mut BackgroundColor, With<TargetPanel>>,
) {
    let Ok(mut background) = panels.single_mut() else {
        return;
    };
    background.0 = if active { PULSE_COLOR } else { PANEL_BG };
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bevy::MinimalPlugins;
    use hex_core::{ElementId, PlayerSeat};

    use hex_core::{HexCoord, LatticeCoord, TilePos};

    use super::*;

    fn pos(x: i32, y: i32) -> TilePos {
        TilePos::new(HexCoord::from_axial(x, y), 0)
    }

    #[test]
    fn an_empty_anchor_does_not_replace_the_retained_target() {
        let mut focus = RetainedTarget {
            unit: Some(UnitId(7)),
            pinned_turn: Some(UnitId(1)),
        };
        let aimed = Some(pos(9, 9));
        let units = [(UnitId(3), Faction::Hostile, pos(1, 0))];
        update_retained(&mut focus, aimed, &units);
        assert_eq!(focus.unit, Some(UnitId(7)));
    }

    #[test]
    fn a_hostile_on_the_anchor_replaces_the_retained_target() {
        let mut focus = RetainedTarget {
            unit: Some(UnitId(7)),
            pinned_turn: Some(UnitId(1)),
        };
        let units = [(UnitId(3), Faction::Hostile, pos(1, 0))];
        update_retained(&mut focus, Some(pos(1, 0)), &units);
        assert_eq!(focus.unit, Some(UnitId(3)));
    }

    #[test]
    fn aiming_at_an_ally_clears_a_hostile_pin_while_empty_terrain_retains_it() {
        let mut focus = RetainedTarget {
            unit: Some(UnitId(7)),
            pinned_turn: Some(UnitId(1)),
        };
        let units = [(UnitId(3), Faction::Player, pos(1, 0))];
        update_retained(&mut focus, Some(pos(1, 0)), &units);
        assert_eq!(focus, RetainedTarget::default());

        focus = RetainedTarget {
            unit: Some(UnitId(7)),
            pinned_turn: Some(UnitId(1)),
        };
        update_retained(&mut focus, Some(pos(9, 9)), &units);
        assert_eq!(focus.unit, Some(UnitId(7)));
    }

    #[test]
    fn a_hostile_ai_decision_never_creates_player_cell_selection() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<PendingDecision>()
            .init_resource::<UnitRegistry>()
            .init_resource::<DisableSelection>()
            .add_systems(Update, sync_disable_selection);
        let unit = UnitId(9);
        let spec = LatticeSpec::default().with(
            LatticeCoord::ORIGIN,
            hex_lattice::CellKind::Gem {
                element: ElementId(0),
            },
        );
        let state = LatticeState::new(&spec, &LatticeStats::default());
        let entity = app
            .world_mut()
            .spawn((unit, Faction::Hostile, spec, state))
            .id();
        app.world_mut()
            .resource_mut::<UnitRegistry>()
            .register(unit, entity);
        *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseDisables {
            decider: unit,
            count: 1,
            source: UnitId(1),
        };

        app.update();

        assert!(
            !app.world().resource::<DisableSelection>().is_active(),
            "AI-owned lattice truth must never enter the player decision UI"
        );
    }

    #[test]
    fn a_confirmed_aim_remains_pinned_until_its_turn_changes() {
        let actor = UnitId(1);
        let mut focus = RetainedTarget {
            unit: Some(UnitId(7)),
            pinned_turn: Some(actor),
        };

        assert!(!reconcile_retained_lifecycle(
            &mut focus,
            Mode::Combat,
            AimExit::Confirmed,
            Some(actor),
            true,
        ));
        assert_eq!(focus.unit, Some(UnitId(7)));

        assert!(!reconcile_retained_lifecycle(
            &mut focus,
            Mode::Combat,
            AimExit::None,
            Some(UnitId(2)),
            true,
        ));
        assert_eq!(focus, RetainedTarget::default());
    }

    #[test]
    fn cancelling_or_invalidating_an_aim_clears_its_pin() {
        let pinned = || RetainedTarget {
            unit: Some(UnitId(7)),
            pinned_turn: Some(UnitId(1)),
        };

        let mut cancelled = pinned();
        assert!(reconcile_retained_lifecycle(
            &mut cancelled,
            Mode::Combat,
            AimExit::Cancelled,
            Some(UnitId(1)),
            true,
        ));
        assert_eq!(cancelled, RetainedTarget::default());

        let mut invalid = pinned();
        assert!(!reconcile_retained_lifecycle(
            &mut invalid,
            Mode::Combat,
            AimExit::None,
            Some(UnitId(1)),
            false,
        ));
        assert_eq!(invalid, RetainedTarget::default());
    }

    #[test]
    fn owing_every_live_cell_preselects_all_but_keeps_the_decision_open() {
        let live = vec![LatticeCoord::ORIGIN, LatticeCoord::new(1, 0)];
        let mut selection = DisableSelection::default();
        reconcile_selection(
            &mut selection,
            Some(DisableDecision {
                decider: UnitId(4),
                target: UnitId(4),
                owed: 2,
                restoring: false,
                live: live.clone(),
            }),
        );
        assert_eq!(selection.cells, live);
        assert!(selection.is_active(), "confirmation is still owed");
    }

    #[test]
    fn selection_ignores_off_lattice_cells_and_any_pick_beyond_the_quota() {
        let first = LatticeCoord::ORIGIN;
        let second = LatticeCoord::new(1, 0);
        let mut selection = DisableSelection {
            decision: Some(DisableDecision {
                decider: UnitId(4),
                target: UnitId(4),
                owed: 1,
                restoring: false,
                // Disabled and off-lattice cells are absent from this live set.
                live: vec![first, second],
            }),
            cells: Vec::new(),
        };
        toggle_cell(&mut selection, LatticeCoord::new(9, 9));
        toggle_cell(&mut selection, first);
        toggle_cell(&mut selection, second);
        assert_eq!(selection.cells, vec![first]);

        // A repeated choice toggles the existing one; it can never duplicate it.
        toggle_cell(&mut selection, first);
        assert!(selection.cells.is_empty());
    }

    #[test]
    fn enter_emits_the_forced_player_answer_with_its_control_owner() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<hex_core::InputBindings>()
            .init_resource::<PendingDecision>()
            .init_resource::<UnitRegistry>()
            .init_resource::<CommandQueue>()
            .init_resource::<DisableSelection>()
            .add_systems(
                Update,
                (sync_disable_selection, handle_decision_input).chain(),
            );

        let unit = UnitId(5);
        let stats = LatticeStats::new(BTreeMap::from([(ElementId(0), 2)]), BTreeMap::new());
        let spec = LatticeSpec::default()
            .with(
                LatticeCoord::ORIGIN,
                hex_lattice::CellKind::Gem {
                    element: ElementId(0),
                },
            )
            .with(
                LatticeCoord::new(1, 0),
                hex_lattice::CellKind::Gem {
                    element: ElementId(0),
                },
            );
        let state = LatticeState::new(&spec, &stats);
        let entity = app
            .world_mut()
            .spawn((
                Player,
                unit,
                ControlOwner(PlayerSeat(8)),
                spec,
                state,
                stats,
            ))
            .id();
        app.world_mut()
            .resource_mut::<UnitRegistry>()
            .register(unit, entity);
        *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseDisables {
            decider: unit,
            count: 3,
            source: UnitId(2),
        };
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Enter);
        app.update();

        assert_eq!(
            app.world_mut().resource_mut::<CommandQueue>().pop(),
            Some(IssuedCommand {
                seat: PlayerSeat(8),
                command: GameCommand::ChooseDisables {
                    unit,
                    cells: vec![LatticeCoord::ORIGIN, LatticeCoord::new(1, 0)],
                },
            })
        );
    }

    #[test]
    fn restoration_uses_the_casters_owner_and_the_targets_disabled_cells() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<hex_core::InputBindings>()
            .init_resource::<PendingDecision>()
            .init_resource::<UnitRegistry>()
            .init_resource::<CommandQueue>()
            .init_resource::<DisableSelection>()
            .add_systems(
                Update,
                (sync_disable_selection, handle_decision_input).chain(),
            );
        let stats = LatticeStats::new(BTreeMap::from([(ElementId(0), 2)]), BTreeMap::new());
        let spec = LatticeSpec::default().with(
            LatticeCoord::ORIGIN,
            hex_lattice::CellKind::Gem {
                element: ElementId(0),
            },
        );
        let caster = UnitId(5);
        let caster_entity = app
            .world_mut()
            .spawn((
                Player,
                caster,
                ControlOwner(PlayerSeat(8)),
                spec.clone(),
                LatticeState::new(&spec, &stats),
                stats.clone(),
            ))
            .id();
        let target = UnitId(6);
        let mut target_state = LatticeState::new(&spec, &stats);
        hex_lattice::apply_disables(&mut target_state, &[LatticeCoord::ORIGIN]);
        let target_entity = app
            .world_mut()
            .spawn((Player, target, spec, target_state, stats))
            .id();
        app.world_mut()
            .resource_mut::<UnitRegistry>()
            .register(caster, caster_entity);
        app.world_mut()
            .resource_mut::<UnitRegistry>()
            .register(target, target_entity);
        *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseRestores {
            decider: caster,
            target,
            count: 2,
        };
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Enter);
        app.update();

        assert_eq!(
            app.world_mut().resource_mut::<CommandQueue>().pop(),
            Some(IssuedCommand {
                seat: PlayerSeat(8),
                command: GameCommand::ChooseRestores {
                    unit: caster,
                    target,
                    cells: vec![LatticeCoord::ORIGIN],
                },
            })
        );
    }

    fn update_retained(
        focus: &mut RetainedTarget,
        aimed: Option<TilePos>,
        units: &[(UnitId, Faction, TilePos)],
    ) {
        let Some(anchor) = aimed else { return };
        if let Some((unit, faction, _)) = units.iter().find(|(_, _, standing)| *standing == anchor)
        {
            if Faction::Player.is_hostile_to(*faction) {
                focus.unit = Some(*unit);
            } else {
                *focus = RetainedTarget::default();
            }
        }
    }
}
