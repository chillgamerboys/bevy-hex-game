//! Dev-only proof that the element/spell content pipeline resolves end to end.
//!
//! Once [`ContentIndex`] and the [`SpellBook`] are built, this logs the parsed spell
//! list — each spell's tier, effect count, whether it is a ritual, and how many of
//! its requirements the cross-file index resolved. It is the smallest consumer that
//! exercises files → tables → cross-file resolution, standing in until the lattice
//! engine reads this content for real (HEX-12). Behind the `dev` feature only.

use bevy::prelude::*;
use hex_assets::{ContentIndex, SpellBook};

/// Registers the one-shot content dump.
pub fn plugin(app: &mut App) {
    app.add_systems(Update, log_parsed_spells);
}

/// Logs the resolved spell list exactly once, after the content index is built.
fn log_parsed_spells(
    spells: Option<Res<SpellBook>>,
    index: Option<Res<ContentIndex>>,
    mut logged: Local<bool>,
) {
    if *logged {
        return;
    }
    let (Some(spells), Some(index)) = (spells, index) else {
        return;
    };
    if spells.is_empty() {
        return;
    }

    info!(
        "content pipeline: ContentIndex resolved {} of {} spell(s):",
        index.len(),
        spells.len()
    );
    for (id, name, spell) in spells.iter() {
        let resolved = index.requirements(id).map_or(0, <[_]>::len);
        info!(
            "  {name}: tier {}, {} effect(s){}, {resolved} requirement(s) resolved",
            spell.tier(),
            spell.effects.len(),
            if spell.is_ritual() { ", ritual" } else { "" },
        );
    }
    *logged = true;
}
