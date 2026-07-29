//! The player's live lattice and the retained, knowledge-gated hostile target.

use bevy::picking::Pickable;
use bevy::prelude::*;
use hex_assets::{ElementCatalog, SpellBook};
use hex_combat::{CombatSystems, FactionKnowledge};
use hex_core::{
    AppSystems, CommandQueue, ControlOwner, GameCommand, IssuedCommand, Mode, PendingDecision,
    Screen, UnitId,
};
use hex_lattice::{LatticeSpec, LatticeState, LatticeStats};
use hex_units::{Downed, Faction, Player, Selected, StandsOn, UnitRegistry};

use crate::casting::{Aiming, CastReadout};
use crate::menus::lattice_view::{
    known_cell_view, live_cell_view, spawn_lattice_cells, CellInteraction, LatticeCellView,
    LatticeScale,
};
use crate::menus::widgets::{blurb, fine, heading, panel, row_button, UiAssets, EDGE, PANEL_BG};
use crate::readouts::HudElement;
use crate::screens::DespawnOnExit;

const PANEL_WIDTH: f32 = 286.0;
const TARGET_TOP: f32 = 326.0;
const PULSE_COLOR: Color = Color::srgba(0.25, 0.10, 0.06, 0.9);
const FRAME: Pickable = Pickable {
    should_block_lower: true,
    is_hoverable: false,
};

#[derive(Resource, Default, Debug, PartialEq)]
struct LatticeReadouts {
    own: Option<OwnLattice>,
    target: Option<TargetLattice>,
}

#[derive(Debug, PartialEq)]
struct OwnLattice {
    unit: UnitId,
    name: String,
    cells: Vec<LatticeCellView>,
    decision: Option<DecisionSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DecisionSummary {
    chosen: usize,
    owed: usize,
}

#[derive(Debug, PartialEq)]
struct TargetLattice {
    unit: UnitId,
    name: String,
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

/// The last hostile a real aim named. Empty anchors never erase it.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetainedTarget(pub(crate) Option<UnitId>);

/// Local interaction state for the one player disable decision.
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub(super) struct DisableSelection {
    pub(super) decision: Option<DisableDecision>,
    cells: Vec<hex_core::LatticeCoord>,
}

impl DisableSelection {
    pub(super) fn is_active(&self) -> bool {
        self.decision.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DisableDecision {
    pub(super) decider: UnitId,
    pub(super) owed: usize,
    pub(super) live: Vec<hex_core::LatticeCoord>,
}

#[derive(Component)]
struct OwnPanel;

/// The one ordinary HUD root that remains visible while it is command-modal.
#[derive(Component)]
pub(super) struct DecisionLattice;

#[derive(Component)]
pub(super) struct TargetPanel;

#[derive(Component)]
struct OwnBody;

#[derive(Component)]
struct TargetBody;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct OwnCell(hex_core::LatticeCoord);

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
enum DecisionControl {
    Clear,
    Confirm,
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<LatticeReadouts>()
        .init_resource::<RetainedTarget>()
        .init_resource::<DisableSelection>()
        .add_systems(OnEnter(Screen::Gameplay), spawn_panels)
        .add_systems(
            Update,
            handle_decision_input
                .in_set(AppSystems::RecordInput)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            (sync_disable_selection, retain_target, refresh_readouts)
                .chain()
                .in_set(AppSystems::Update)
                .after(CombatSystems::Advance)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            rebuild_panels
                .after(refresh_readouts)
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
) {
    *readouts = LatticeReadouts::default();
    *focus = RetainedTarget::default();
    *selection = DisableSelection::default();

    commands
        .spawn((
            Name::new("Own Lattice Panel"),
            OwnPanel,
            DecisionLattice,
            HudElement,
            panel(),
            FRAME,
            DespawnOnExit(Screen::Gameplay),
        ))
        .insert(Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.0),
            left: Val::Px(12.0),
            width: Val::Px(PANEL_WIDTH),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(12.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            row_gap: Val::Px(7.0),
            ..default()
        })
        .with_children(|panel| {
            panel.spawn(heading(&assets, "your lattice"));
            panel.spawn((Name::new("Own Lattice Body"), OwnBody, Pickable::IGNORE));
        });

    commands
        .spawn((
            Name::new("Target Lattice Panel"),
            TargetPanel,
            HudElement,
            panel(),
            FRAME,
            DespawnOnExit(Screen::Gameplay),
        ))
        .insert(Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            top: Val::Px(TARGET_TOP),
            left: Val::Px(12.0),
            width: Val::Px(PANEL_WIDTH),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(12.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            row_gap: Val::Px(7.0),
            ..default()
        })
        .with_children(|panel| {
            panel.spawn(heading(&assets, "target lattice"));
            panel.spawn((
                Name::new("Target Lattice Body"),
                TargetBody,
                Pickable::IGNORE,
            ));
        });
}

/// Updates retention from an actual hostile occupying the aimed surface.
pub(super) fn retain_target(
    mode: Res<State<Mode>>,
    aiming: Res<Aiming>,
    mut focus: ResMut<RetainedTarget>,
    units: Query<(&UnitId, &Faction, &StandsOn), Without<Downed>>,
) {
    if *mode.get() != Mode::Combat {
        focus.0 = None;
        return;
    }
    let Some(aim) = aiming.0.as_ref() else {
        return;
    };
    if let Some((unit, _, _)) = units.iter().find(|(_, faction, standing)| {
        Faction::Player.is_hostile_to(**faction) && standing.0.pos == aim.anchor
    }) {
        focus.0 = Some(*unit);
    }
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
    With<Player>,
>;

fn sync_disable_selection(
    pending: Res<PendingDecision>,
    registry: Res<UnitRegistry>,
    players: Query<(&LatticeSpec, &LatticeState), With<Player>>,
    mut selection: ResMut<DisableSelection>,
) {
    let next = match *pending {
        PendingDecision::ChooseDisables { decider, count, .. } => registry
            .entity_of(decider)
            .and_then(|entity| players.get(entity).ok())
            .map(|(spec, state)| {
                let live: Vec<_> = spec
                    .cells()
                    .filter(|&(coord, _)| !state.is_disabled(coord))
                    .map(|(coord, _)| coord)
                    .collect();
                DisableDecision {
                    decider,
                    owed: usize::from(count).min(live.len()),
                    live,
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
    focus: Res<RetainedTarget>,
    pending: Res<PendingDecision>,
    casting: Res<CastReadout>,
    selection: Res<DisableSelection>,
    registry: Res<UnitRegistry>,
    knowledge: Res<FactionKnowledge>,
    elements: Option<Res<ElementCatalog>>,
    spells: Option<Res<SpellBook>>,
    own: OwnData,
    selected: Query<&UnitId, (With<Player>, With<Selected>)>,
    identities: Query<(&Name, &Faction)>,
) {
    let (Some(elements), Some(spells)) = (elements, spells) else {
        return;
    };

    let own_unit = own_focus(
        player_decider(&pending, &registry, &own),
        casting.caster.map(|caster| caster.unit),
        selected.iter().copied().next(),
    );
    let own_view = own_unit
        .and_then(|unit| registry.entity_of(unit))
        .and_then(|entity| own.get(entity).ok())
        .map(|(unit, name, spec, state, stats)| OwnLattice {
            unit: *unit,
            name: name.as_str().to_owned(),
            cells: spec
                .cells()
                .map(|(coord, kind)| {
                    let armed = selection.decision.as_ref().is_some_and(|decision| {
                        decision.decider == *unit && decision.live.contains(&coord)
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
                .filter(|decision| decision.decider == *unit)
                .map(|decision| DecisionSummary {
                    chosen: selection.cells.len(),
                    owed: decision.owed,
                }),
        });

    // No hostile `LatticeSpec` or `LatticeState` appears in this function. The
    // target projection can only be assembled from `FactionKnowledge::view`.
    let target_view = focus.0.and_then(|unit| {
        let entity = registry.entity_of(unit)?;
        let (name, faction) = identities.get(entity).ok()?;
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
            name: name.as_str().to_owned(),
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

fn player_decider(
    pending: &PendingDecision,
    registry: &UnitRegistry,
    own: &OwnData,
) -> Option<UnitId> {
    let unit = pending.decider()?;
    let entity = registry.entity_of(unit)?;
    own.contains(entity).then_some(unit)
}

fn own_focus(
    player_decider: Option<UnitId>,
    casting_unit: Option<UnitId>,
    selected_player: Option<UnitId>,
) -> Option<UnitId> {
    player_decider.or(casting_unit).or(selected_player)
}

fn rebuild_panels(
    mut commands: Commands,
    readouts: Res<LatticeReadouts>,
    own_bodies: Query<Entity, With<OwnBody>>,
    target_bodies: Query<Entity, With<TargetBody>>,
    mut target_panels: Query<&mut Node, With<TargetPanel>>,
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
            body.spawn(fine(&assets, own.name.clone()));
            spawn_lattice_cells(
                body,
                &own.cells,
                &assets,
                LatticeScale::PANEL,
                "Own",
                OwnCell,
            );
            if let Some(decision) = own.decision {
                spawn_decision_controls(body, decision, &assets);
            }
        });
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
            body.spawn(fine(&assets, target.name.clone()));
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
}

fn spawn_decision_controls(
    body: &mut ChildSpawnerCommands,
    decision: DecisionSummary,
    assets: &UiAssets,
) {
    body.spawn(fine(
        assets,
        format!("{}/{} cells chosen", decision.chosen, decision.owed),
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
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        ..default()
                    },
                    BorderColor::all(EDGE),
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.03)),
                    Pickable::IGNORE,
                ))
                .with_children(|button| {
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
    pending: Res<PendingDecision>,
    registry: Res<UnitRegistry>,
    mut queue: ResMut<CommandQueue>,
    mut selection: ResMut<DisableSelection>,
    cells: Query<(&Interaction, &OwnCell), Changed<Interaction>>,
    controls: Query<(&Interaction, &DecisionControl), Changed<Interaction>>,
    players: Query<(&LatticeSpec, &LatticeState, &ControlOwner), With<Player>>,
) {
    let PendingDecision::ChooseDisables { decider, count, .. } = *pending else {
        return;
    };
    if queue.holds_answer_for(decider) {
        return;
    }
    let Some(entity) = registry.entity_of(decider) else {
        return;
    };
    let Ok((spec, state, owner)) = players.get(entity) else {
        return;
    };
    let Some(decision) = selection
        .decision
        .as_ref()
        .filter(|decision| decision.decider == decider)
        .cloned()
    else {
        return;
    };

    let mut clear = false;
    let mut confirm = keys.just_pressed(KeyCode::Enter);
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
    let live: Vec<_> = spec
        .cells()
        .filter(|&(coord, _)| !state.is_disabled(coord))
        .map(|(coord, _)| coord)
        .collect();
    let owed = usize::from(count).min(live.len());
    let valid = owed == decision.owed
        && selection.cells.len() == owed
        && selection.cells.iter().all(|cell| live.contains(cell));
    if !valid {
        return;
    }
    queue.push(IssuedCommand {
        seat: owner.0,
        command: GameCommand::ChooseDisables {
            unit: decider,
            cells: selection.cells.clone(),
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
    focus.0 = None;
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
        let mut focus = RetainedTarget(Some(UnitId(7)));
        let aimed = Some(pos(9, 9));
        let units = [(UnitId(3), Faction::Hostile, pos(1, 0))];
        update_retained(&mut focus, aimed, &units);
        assert_eq!(focus.0, Some(UnitId(7)));
    }

    #[test]
    fn a_hostile_on_the_anchor_replaces_the_retained_target() {
        let mut focus = RetainedTarget(Some(UnitId(7)));
        let units = [(UnitId(3), Faction::Hostile, pos(1, 0))];
        update_retained(&mut focus, Some(pos(1, 0)), &units);
        assert_eq!(focus.0, Some(UnitId(3)));
    }

    #[test]
    fn own_focus_prefers_the_decider_then_caster_then_selection() {
        let decider = UnitId(1);
        let caster = UnitId(2);
        let selected = UnitId(3);
        assert_eq!(
            own_focus(Some(decider), Some(caster), Some(selected)),
            Some(decider)
        );
        assert_eq!(own_focus(None, Some(caster), Some(selected)), Some(caster));
        assert_eq!(own_focus(None, None, Some(selected)), Some(selected));
        assert_eq!(own_focus(None, None, None), None);
    }

    #[test]
    fn owing_every_live_cell_preselects_all_but_keeps_the_decision_open() {
        let live = vec![LatticeCoord::ORIGIN, LatticeCoord::new(1, 0)];
        let mut selection = DisableSelection::default();
        reconcile_selection(
            &mut selection,
            Some(DisableDecision {
                decider: UnitId(4),
                owed: 2,
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
                owed: 1,
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

    fn update_retained(
        focus: &mut RetainedTarget,
        aimed: Option<TilePos>,
        units: &[(UnitId, Faction, TilePos)],
    ) {
        let Some(anchor) = aimed else { return };
        if let Some((unit, _, _)) = units.iter().find(|(_, faction, standing)| {
            Faction::Player.is_hostile_to(*faction) && *standing == anchor
        }) {
            focus.0 = Some(*unit);
        }
    }
}
