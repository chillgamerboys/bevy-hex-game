//! Shared visual projection and geometry for lattice readouts.
//!
//! Combat knowledge, demo truth, and player decisions all arrive here as an
//! already-resolved [`LatticeCellView`]. Keeping the renderer ignorant of the
//! source is the boundary that prevents a hostile readout from reaching around
//! `FactionKnowledge` for more information.

use bevy::prelude::*;
use hex_assets::{ElementCatalog, SpellBook};
use hex_core::LatticeCoord;
use hex_lattice::{CellKind, LatticeState, LatticeStats};

use super::widgets::{OwnColors, UiAssets, FUSION_COLOR, GEM_COLOR, LABEL};

/// Pixel width of one hex cell sprite (pointy-top, so height runs longer).
const CELL_SIZE: f32 = 62.0;

/// Pixel height of the hex sprite: width times the 256/222 sprite ratio.
const CELL_HEIGHT: f32 = 71.5;

/// Horizontal distance between neighbouring cell centres.
const CELL_STEP: f32 = 66.0;

/// Vertical distance between rows: three quarters of the hex height.
const ROW_STEP: f32 = 56.0;

const SPELL_COLOR: Color = Color::srgba(0.30, 0.33, 0.40, 0.95);
const LOCKED_COLOR: Color = Color::srgba(0.72, 0.54, 0.18, 0.95);
const DISABLED_COLOR: Color = Color::srgba(0.46, 0.13, 0.11, 0.95);

/// Geometry scale for a lattice surface.
#[derive(Clone, Copy)]
pub(crate) struct LatticeScale(f32);

impl LatticeScale {
    /// Full-size cells on the dedicated lattice demo.
    pub(crate) const DEMO: Self = Self(1.0);

    /// Compact cells in gameplay side panels.
    #[expect(
        dead_code,
        reason = "the gameplay panel adopts this scale in the following subsystem commit"
    )]
    pub(crate) const PANEL: Self = Self(0.65);
}

/// Whether a projected cell participates in UI picking.
#[expect(
    dead_code,
    reason = "read-only combat projections arrive in the following subsystem commit"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CellInteraction {
    ReadOnly,
    Actionable,
}

/// Everything the renderer is allowed to know about one cell.
///
/// `known_mana` and `known_locked` remain optional so knowledge-backed callers
/// cannot accidentally turn an unknown value into a plausible-looking zero.
#[derive(Component, Clone)]
pub(crate) struct LatticeCellView {
    pub(crate) coord: LatticeCoord,
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) color: Color,
    #[expect(
        dead_code,
        reason = "combat projections consume knowledge and selection in the following subsystem"
    )]
    pub(crate) known_mana: Option<u16>,
    #[expect(
        dead_code,
        reason = "combat projections consume knowledge and selection in the following subsystem"
    )]
    pub(crate) known_locked: Option<bool>,
    #[expect(
        dead_code,
        reason = "combat projections consume knowledge and selection in the following subsystem"
    )]
    pub(crate) disabled: bool,
    #[expect(
        dead_code,
        reason = "combat projections consume knowledge and selection in the following subsystem"
    )]
    pub(crate) selected: bool,
    pub(crate) interaction: CellInteraction,
}

/// Projects a fully known, live cell without changing the demo's established
/// labels or palette.
pub(crate) fn live_cell_view(
    coord: LatticeCoord,
    kind: CellKind,
    stats: &LatticeStats,
    state: &LatticeState,
    elements: &ElementCatalog,
    spells: &SpellBook,
    interaction: CellInteraction,
    selected: bool,
) -> LatticeCellView {
    let disabled = state.is_disabled(coord);
    let locked = state.is_locked(coord);
    let known_mana = matches!(kind, CellKind::Gem { .. }).then(|| state.mana(coord));
    let color = if disabled {
        DISABLED_COLOR
    } else if locked {
        LOCKED_COLOR
    } else {
        match kind {
            CellKind::Gem { .. } => GEM_COLOR,
            CellKind::Fusion { .. } => FUSION_COLOR,
            _ => SPELL_COLOR,
        }
    };
    let (label, mut detail) = match kind {
        CellKind::Gem { element } => (
            short_name(elements.name(element).unwrap_or("gem")),
            format!("{}/{}", state.mana(coord), stats.capacity(element)),
        ),
        CellKind::Fusion { output } => (
            "fusion".to_owned(),
            short_name(elements.name(output).unwrap_or("?")),
        ),
        CellKind::Spell { spell } => (
            short_name(spells.name(spell).unwrap_or("spell")),
            spells
                .spell(spell)
                .map(|entry| format!("tier {}", entry.tier()))
                .unwrap_or_default(),
        ),
        CellKind::Blank => ("-".to_owned(), String::new()),
    };
    if disabled {
        detail = "disabled".to_owned();
    } else if locked {
        detail = format!("{detail} locked");
    }

    LatticeCellView {
        coord,
        label,
        detail,
        color,
        known_mana,
        known_locked: Some(locked),
        disabled,
        selected,
        interaction,
    }
}

/// Spawns a projected lattice. The marker callback lets each caller retain its
/// own stable click/query component without duplicating any visual code.
pub(crate) fn spawn_lattice_cells<M: Bundle>(
    parent: &mut ChildSpawnerCommands,
    views: &[LatticeCellView],
    assets: &UiAssets,
    scale: LatticeScale,
    name_prefix: &str,
    mut marker: impl FnMut(LatticeCoord) -> M,
) {
    let Some((min, max)) = bounds(views, scale) else {
        return;
    };

    parent
        .spawn((
            Name::new(format!("{name_prefix} Lattice")),
            Node {
                width: Val::Px(max.0 - min.0 + CELL_SIZE * scale.0),
                height: Val::Px(max.1 - min.1 + CELL_HEIGHT * scale.0),
                ..default()
            },
        ))
        .with_children(|lattice| {
            for view in views {
                let (x, y) = cell_position(view.coord, scale);
                let mut cell = lattice.spawn((
                    Name::new(format!(
                        "{name_prefix} Cell ({}, {})",
                        view.coord.q(),
                        view.coord.r()
                    )),
                    OwnColors,
                    ImageNode {
                        image: assets.hex_cell.clone(),
                        color: view.color,
                        ..default()
                    },
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(x - min.0),
                        top: Val::Px(y - min.1),
                        width: Val::Px(CELL_SIZE * scale.0),
                        height: Val::Px(CELL_HEIGHT * scale.0),
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
                    cell.insert(Button);
                }
                cell.with_children(|cell| {
                    cell.spawn((
                        Text::new(view.label.clone()),
                        TextFont {
                            font: assets.body.clone().into(),
                            ..TextFont::from_font_size((11.0 * scale.0).max(9.0))
                        },
                        TextColor(LABEL),
                        Pickable::IGNORE,
                    ));
                    cell.spawn((
                        Text::new(view.detail.clone()),
                        TextFont {
                            font: assets.body.clone().into(),
                            ..TextFont::from_font_size((10.0 * scale.0).max(8.0))
                        },
                        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.75)),
                        Pickable::IGNORE,
                    ));
                });
            }
        });
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
pub(crate) fn short_name(name: &str) -> String {
    const FITS: usize = 8;
    if name.chars().count() <= FITS {
        name.to_owned()
    } else {
        let head: String = name.chars().take(FITS - 1).collect();
        format!("{head}…")
    }
}
