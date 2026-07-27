//! Opens every shipped content file and checks that all cross-file references
//! resolve. This is the hard CI gate for the content pipeline (HEX-7): at runtime a
//! dangling reference is logged and the last valid [`ContentIndex`] is kept, but here
//! it fails the build so shipped content can never drift out of sync.
//!
//! It rebuilds the tables the same way the game does — parse each file, assign ids
//! from sorted names, resolve across files — but headless, with no `App`.

use hex_assets::{
    ContentIndex, ElementCatalog, ElementFile, SpellBook, SpellFile, SubstanceFile, SubstanceTable,
};
use ron::error::SpannedError;

// Parsing returns a `Result` so the `expect` lives inside a `#[test]` function, where
// clippy's `allow-expect-in-tests` applies. A free helper that called `expect` itself
// would be flagged — the allowance only covers test functions, not their callees.
fn parse_elements() -> Result<ElementFile, SpannedError> {
    ron::from_str(include_str!("../../../assets/config/elements.ron"))
}

fn parse_spells() -> Result<SpellFile, SpannedError> {
    ron::from_str(include_str!("../../../assets/config/spells.ron"))
}

fn parse_substances() -> Result<SubstanceFile, SpannedError> {
    ron::from_str(include_str!("../../../assets/config/substances.ron"))
}

#[test]
fn shipped_content_cross_references_resolve() {
    let elements =
        ElementCatalog::from_file(&parse_elements().expect("elements.ron parses and validates"));
    let spells = SpellBook::from_file(&parse_spells().expect("spells.ron parses and validates"));
    let substances = SubstanceTable::from_file(&parse_substances().expect("substances.ron parses"));

    let index = match ContentIndex::build(&elements, &spells, &substances) {
        Ok(index) => index,
        Err(errors) => panic!("shipped content has dangling cross-file references: {errors:#?}"),
    };

    assert!(
        !spells.is_empty(),
        "the shipped spell book should not be empty"
    );
    assert_eq!(
        index.len(),
        spells.len(),
        "every shipped spell must be resolved in the content index"
    );

    // Every spell's requirements resolved to real element ids.
    for (id, name, _spell) in spells.iter() {
        assert!(
            index.requirements(id).is_some(),
            "spell '{name}' has unresolved requirements"
        );
    }
}

/// Guards the two orderings the pipeline deliberately keeps separate: ids come from
/// sorted names, opposition comes from the wheel array. A regression that collapsed
/// them would pass the resolve test but break the game.
#[test]
fn shipped_wheel_opposition_is_symmetric() {
    let elements =
        ElementCatalog::from_file(&parse_elements().expect("elements.ron parses and validates"));
    for id in elements.wheel() {
        let opposite = elements
            .opposite(*id)
            .expect("every wheel element has an opposite");
        assert_eq!(
            elements.opposite(opposite),
            Some(*id),
            "opposition must be an involution"
        );
        assert_ne!(opposite, *id, "no element opposes itself");
    }
}
