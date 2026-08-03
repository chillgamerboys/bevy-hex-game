//! Lattice presentation adapter.
//!
//! Gameplay and perception truth is projected here into renderer-owned cell views.

use bevy::prelude::*;
use hex_assets::{ElementCatalog, SpellBook};
use hex_core::LatticeCoord;
use hex_lattice::{CellKind, LatticeState, LatticeStats};

use hex_ui::element_color;
pub(crate) use hex_ui::{
    paint_lattice_interactions as paint_interactions, short_name, CellInteraction, LatticeCellView,
};

const SPELL_COLOR: Color = Color::srgba(0.30, 0.33, 0.40, 0.95);
const LOCKED_COLOR: Color = Color::srgba(0.72, 0.54, 0.18, 0.95);
const DISABLED_COLOR: Color = Color::srgba(0.46, 0.13, 0.11, 0.95);

/// Projects a revealed cell without inventing a maximum mana value or a lock
/// state the viewer did not learn.
pub(crate) fn known_cell_view(
    coord: LatticeCoord,
    kind: CellKind,
    known_mana: Option<u16>,
    known_locked: Option<bool>,
    disabled: bool,
    elements: &ElementCatalog,
    spells: &SpellBook,
) -> LatticeCellView {
    let locked = known_locked == Some(true);
    let color = if disabled {
        DISABLED_COLOR
    } else if locked {
        LOCKED_COLOR
    } else {
        match kind {
            CellKind::Gem { element } => element_color(Some(element), elements),
            CellKind::Fusion { output } => element_color(Some(output), elements),
            _ => SPELL_COLOR,
        }
    };
    let (label, mut detail) = match kind {
        CellKind::Gem { element } => (
            short_name(elements.name(element).unwrap_or("gem")),
            known_mana.map_or_else(|| "mana unknown".to_owned(), |mana| format!("{mana} mana")),
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
        known_locked,
        disabled,
        selected: false,
        interaction: CellInteraction::ReadOnly,
    }
}

/// Projects a fully known live cell for the renderer.
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
            CellKind::Gem { element } => element_color(Some(element), elements),
            CellKind::Fusion { output } => element_color(Some(output), elements),
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

#[cfg(test)]
mod tests {
    use hex_assets::ElementFile;

    use super::*;

    fn shipped_elements() -> ElementCatalog {
        let file: ElementFile = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/elements.ron"
        )))
        .expect("elements.ron parses");
        ElementCatalog::from_file(&file)
    }

    #[test]
    fn known_lattice_fusions_use_their_authored_school_tint() {
        let elements = shipped_elements();
        let lightning = elements.id("Lightning").expect("Lightning ships");
        let view = known_cell_view(
            LatticeCoord::ORIGIN,
            CellKind::Fusion { output: lightning },
            None,
            None,
            false,
            &elements,
            &SpellBook::default(),
        );

        assert_eq!(view.color, element_color(Some(lightning), &elements));
        assert_ne!(view.color, hex_ui::FUSION_COLOR);
    }
}
