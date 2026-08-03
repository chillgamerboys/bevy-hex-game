//! Shared visual projection and geometry for lattice readouts.
//!
//! Combat knowledge, demo truth, and player decisions all arrive here as an
//! already-resolved [`LatticeCellView`]. Keeping the renderer ignorant of the
//! source is the boundary that prevents a hostile readout from reaching around
//! `FactionLatticeKnowledge` for more information.

use crate::{compact_glyph_role, owner_resolved_control_role, OwnColors, UiAssets, LABEL};
use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use hex_core::LatticeCoord;

/// Pixel width of one hex cell sprite (pointy-top, so height runs longer).
const CELL_SIZE: f32 = 62.0;

/// Pixel height of the hex sprite: width times the 256/222 sprite ratio.
const CELL_HEIGHT: f32 = 71.5;

/// Horizontal distance between neighbouring cell centres.
const CELL_STEP: f32 = 66.0;

/// Vertical distance between rows: three quarters of the hex height.
const ROW_STEP: f32 = 56.0;

/// One logical pixel absorbs Yoga's edge rounding so the final absolute cell
/// remains inside its declared lattice box at fractional semantic scales.
const LAYOUT_ROUNDING_PAD: f32 = 1.0;

/// Geometry scale for a lattice surface.
#[derive(Clone, Copy)]
pub struct LatticeScale(f32);

impl LatticeScale {
    /// Full-size cells on the dedicated lattice demo.
    pub const DEMO: Self = Self(1.0);

    /// Compact cells in gameplay side panels.
    pub const PANEL: Self = Self(0.72);

    /// Smallest actionable cells for an ultra-constrained required-choice surface.
    pub const TIGHT: Self = Self(0.72);
}

/// Whether a projected cell participates in UI picking.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CellInteraction {
    /// Informational cell that passes pointer input through.
    ReadOnly,
    /// Cell that can be focused and activated.
    Actionable,
}

/// Marks intentionally tessellated hex controls. Their rectangular Bevy node
/// bounds meet at transparent corners without visually obscuring a sibling.
#[derive(Component)]
pub(crate) struct TessellatedControl;

/// Everything the renderer is allowed to know about one cell.
///
/// `known_mana` and `known_locked` remain optional so knowledge-backed callers
/// cannot accidentally turn an unknown value into a plausible-looking zero.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct LatticeCellView {
    /// Canonical lattice coordinate represented by this cell.
    pub coord: LatticeCoord,
    /// Short primary cell label.
    pub label: String,
    /// Secondary status or mana label.
    pub detail: String,
    /// Presentation color selected by the application adapter.
    pub color: Color,
    /// Disclosed mana, when known to the viewer.
    pub known_mana: Option<u16>,
    /// Disclosed lock state, when known to the viewer.
    pub known_locked: Option<bool>,
    /// Whether the canonical cell is disabled.
    pub disabled: bool,
    /// Whether this cell is part of the current UI selection.
    pub selected: bool,
    /// Whether the cell participates in UI picking.
    pub interaction: CellInteraction,
}

/// Spawns a projected lattice. The marker callback lets each caller retain its
/// own stable click/query component without duplicating any visual code.
pub fn spawn_lattice_cells<M: Bundle>(
    parent: &mut ChildSpawnerCommands,
    views: &[LatticeCellView],
    assets: &UiAssets,
    scale: LatticeScale,
    semantic_control_scale: f32,
    name_prefix: &str,
    mut marker: impl FnMut(LatticeCoord) -> M,
) {
    let semantic_control_scale = semantic_control_scale.max(1.0);
    let resolved_scale = LatticeScale(scale.0 * semantic_control_scale);
    let Some((min, max)) = bounds(views, resolved_scale) else {
        return;
    };

    parent
        .spawn((
            Name::new(format!("{name_prefix} Lattice")),
            Node {
                width: Val::Px(max.0 - min.0 + CELL_SIZE * resolved_scale.0 + LAYOUT_ROUNDING_PAD),
                height: Val::Px(
                    max.1 - min.1 + CELL_HEIGHT * resolved_scale.0 + LAYOUT_ROUNDING_PAD,
                ),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|lattice| {
            for view in views {
                let (x, y) = cell_position(view.coord, resolved_scale);
                let mut cell = lattice.spawn((
                    Name::new(format!(
                        "{name_prefix} Cell ({}, {})",
                        view.coord.q(),
                        view.coord.r()
                    )),
                    OwnColors,
                    TessellatedControl,
                    ImageNode {
                        image: assets.hex_cell.clone(),
                        color: styled_color(view, Interaction::None),
                        ..default()
                    },
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(x - min.0),
                        top: Val::Px(y - min.1),
                        width: Val::Px(CELL_SIZE * resolved_scale.0),
                        height: Val::Px(CELL_HEIGHT * resolved_scale.0),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        flex_direction: FlexDirection::Column,
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    view.clone(),
                    marker(view.coord),
                ));
                if view.interaction == CellInteraction::Actionable {
                    cell.insert((
                        Button,
                        crate::DefaultImmediateControl,
                        owner_resolved_control_role(),
                        AccessibleLabel::new(format!(
                            "{} · {} · lattice cell {}, {}",
                            view.label,
                            view.detail,
                            view.coord.q(),
                            view.coord.r()
                        )),
                        TabIndex(0),
                    ));
                } else {
                    cell.insert(Pickable::IGNORE);
                }
                cell.with_children(|cell| {
                    let label_size = (11.0 * scale.0).max(9.0);
                    cell.spawn((
                        Text::new(view.label.clone()),
                        compact_glyph_role(label_size),
                        TextFont {
                            font: assets.body.clone().into(),
                            ..TextFont::from_font_size(label_size)
                        },
                        TextColor(LABEL),
                        Pickable::IGNORE,
                    ));
                    let detail_size = (10.0 * scale.0).max(8.0);
                    cell.spawn((
                        Text::new(view.detail.clone()),
                        compact_glyph_role(detail_size),
                        TextFont {
                            font: assets.body.clone().into(),
                            ..TextFont::from_font_size(detail_size)
                        },
                        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.75)),
                        Pickable::IGNORE,
                    ));
                });
            }
        });
}

/// Explicitly paints actionable lattice cells without handing their semantic
/// colors to the ordinary button palette.
pub fn paint_interactions(
    mut cells: Query<
        (&Interaction, &LatticeCellView, &mut ImageNode),
        (Changed<Interaction>, With<Button>),
    >,
) {
    for (interaction, view, mut image) in &mut cells {
        image.color = styled_color(view, *interaction);
    }
}

fn styled_color(view: &LatticeCellView, interaction: Interaction) -> Color {
    let interaction_lift: f32 = match interaction {
        Interaction::Pressed => 0.28,
        Interaction::Hovered => 0.16,
        Interaction::None => 0.0,
    };
    let lift = interaction_lift.max(if view.selected { 0.24 } else { 0.0 });
    if lift == 0.0 {
        return view.color;
    }
    let color = view.color.to_srgba();
    Color::srgba(
        color.red + (1.0 - color.red) * lift,
        color.green + (1.0 - color.green) * lift,
        color.blue + (1.0 - color.blue) * lift,
        color.alpha,
    )
}

fn bounds(views: &[LatticeCellView], scale: LatticeScale) -> Option<((f32, f32), (f32, f32))> {
    let mut positions = views.iter().map(|view| cell_position(view.coord, scale));
    let first = positions.next()?;
    let mut min = first;
    let mut max = first;
    for (x, y) in positions {
        min = (min.0.min(x), min.1.min(y));
        max = (max.0.max(x), max.1.max(y));
    }
    Some((min, max))
}

fn cell_position(coord: LatticeCoord, scale: LatticeScale) -> (f32, f32) {
    #[expect(
        clippy::cast_precision_loss,
        reason = "lattice coordinates are single digits; f32 is exact far beyond them"
    )]
    let (q, r) = (coord.q() as f32, coord.r() as f32);
    (CELL_STEP * scale.0 * (q + r * 0.5), ROW_STEP * scale.0 * r)
}

/// Truncates a name to what fits inside one hex cell.
pub fn short_name(name: &str) -> String {
    const FITS: usize = 8;
    if name.chars().count() <= FITS {
        name.to_owned()
    } else {
        let head: String = name.chars().take(FITS - 1).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use bevy::MinimalPlugins;

    use super::*;

    fn view(interaction: CellInteraction, selected: bool) -> LatticeCellView {
        LatticeCellView {
            coord: LatticeCoord::ORIGIN,
            label: "Fire".to_owned(),
            detail: "2 mana".to_owned(),
            color: Color::srgb(0.2, 0.3, 0.4),
            known_mana: Some(2),
            known_locked: Some(false),
            disabled: false,
            selected,
            interaction,
        }
    }

    #[test]
    fn read_only_cells_have_no_button_and_defer_picking_to_the_panel() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            logo: Handle::default(),
            hex_cell: Handle::default(),
        });
        app.add_systems(Startup, |mut commands: Commands, assets: Res<UiAssets>| {
            commands.spawn(Node::default()).with_children(|parent| {
                spawn_lattice_cells(
                    parent,
                    &[view(CellInteraction::ReadOnly, false)],
                    &assets,
                    LatticeScale::PANEL,
                    1.0,
                    "Target",
                    |_| (),
                );
            });
        });
        app.update();

        let mut cells = app
            .world_mut()
            .query_filtered::<(Has<Button>, Option<&Pickable>), With<LatticeCellView>>();
        let rows: Vec<_> = cells.iter(app.world()).collect();
        assert_eq!(rows, vec![(false, Some(&Pickable::IGNORE))]);

        let mut containers = app
            .world_mut()
            .query_filtered::<(&Name, &Pickable), Without<LatticeCellView>>();
        assert!(containers
            .iter(app.world())
            .any(|(name, pickable)| name.as_str() == "Target Lattice"
                && *pickable == Pickable::IGNORE));
    }

    #[test]
    fn hover_and_selection_both_lift_an_actionable_cells_color() {
        let plain = view(CellInteraction::Actionable, false);
        let selected = view(CellInteraction::Actionable, true);
        assert_ne!(styled_color(&selected, Interaction::None), selected.color);
        assert_ne!(styled_color(&plain, Interaction::Hovered), plain.color);
        assert_eq!(styled_color(&plain, Interaction::None), plain.color);
    }
}
