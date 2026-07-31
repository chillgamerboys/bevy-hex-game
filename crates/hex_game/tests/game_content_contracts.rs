//! Shared game/content contracts: opens every shipped file and checks cross-file
//! references.
//! resolve. This is the hard CI gate for the content pipeline (HEX-7): at runtime a
//! dangling reference is logged and the last valid [`ContentIndex`] is kept, but here
//! it fails the build so shipped content can never drift out of sync.
//!
//! It rebuilds the tables the same way the game does — parse each file, assign ids
//! from sorted names, resolve across files — but headless, with no `App`.

use std::collections::BTreeSet;

use hex_ai::AiProfileId;
use hex_assets::{
    AiProfileCatalog, ArtPalette, ContentIndex, Effect, ElementCatalog, ElementFile, Encounter,
    FormationCatalog, LatticeFile, LatticeLibrary, SpellBook, SpellFile, SubstanceFile,
    SubstanceTable,
};
use hex_core::{LatticeCoord, Sextant};
use hex_lattice::{castable, CellKind, LatticeState};
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

fn parse_palette() -> Result<ArtPalette, SpannedError> {
    ron::from_str(include_str!("../../../assets/art/palette.ron"))
}

fn parse_lattices() -> Result<LatticeFile, SpannedError> {
    ron::from_str(include_str!("../../../assets/config/lattices.ron"))
}

fn parse_ai_profiles() -> Result<AiProfileCatalog, SpannedError> {
    ron::from_str(include_str!("../../../assets/config/ai_profiles.ron"))
}

fn parse_formations() -> Result<FormationCatalog, SpannedError> {
    ron::from_str(include_str!("../../../assets/config/formations.ron"))
}

#[test]
fn shipped_ai_and_formation_content_is_valid_and_cross_referenced() {
    let profiles = parse_ai_profiles().expect("ai_profiles.ron parses and validates");
    let formations = parse_formations().expect("formations.ron parses and validates");
    assert_eq!(formations.presets.len(), 3);
    for name in ["Compact", "Column", "Wedge"] {
        assert!(formations.get(name).is_some(), "missing {name} formation");
    }

    let elements =
        ElementCatalog::from_file(&parse_elements().expect("elements.ron parses and validates"));
    let spells = SpellBook::from_file(&parse_spells().expect("spells.ron parses and validates"));
    let library = LatticeLibrary::build(
        &parse_lattices().expect("lattices.ron parses"),
        &elements,
        &spells,
    )
    .expect("shipped lattices resolve");
    for (name, archetype) in library.iter() {
        let profile = archetype
            .ai_profile
            .as_ref()
            .unwrap_or_else(|| panic!("archetype {name:?} has no default AI profile"));
        assert!(
            profiles.get(&AiProfileId(profile.clone())).is_some(),
            "archetype {name:?} references unknown AI profile {profile:?}"
        );
    }
}

#[test]
fn every_shipped_formation_is_congruent_through_six_rotations() {
    let formations = parse_formations().expect("formations.ron parses and validates");
    for preset in &formations.presets {
        let original: BTreeSet<_> = preset.slots.iter().map(|slot| slot.offset).collect();
        let mut original_distances: Vec<_> = original
            .iter()
            .map(|offset| offset.distance(hex_core::HexCoord::ORIGIN))
            .collect();
        original_distances.sort_unstable();
        for facing in Sextant::ALL {
            let rotated: BTreeSet<_> = original
                .iter()
                .copied()
                .map(|offset| hex_units::rotated(offset, facing))
                .collect();
            let mut distances: Vec<_> = rotated
                .iter()
                .map(|offset| offset.distance(hex_core::HexCoord::ORIGIN))
                .collect();
            distances.sort_unstable();
            assert_eq!(
                distances, original_distances,
                "{:?} changed shape at {facing:?}",
                preset.name
            );
        }
        for offset in original {
            let turned = (0..6).fold(offset, |offset, _| hex_units::rotated(offset, Sextant::B));
            assert_eq!(
                turned, offset,
                "{:?} did not close a full turn",
                preset.name
            );
        }
    }
}

/// Every spell a shipped archetype inscribes must actually be castable on a fresh
/// lattice.
///
/// **This is the test that checks the drawings, and it is the only thing that can.**
/// Adjacency is the entire power mechanism, so a spell cell one hex away from the gems
/// meant to fund it parses, loads, spawns, and is simply never castable — a unit that
/// stands there doing nothing, with no error anywhere. Nothing else in the pipeline
/// asks whether a lattice *works*: the resolver checks that names exist, and the engine
/// reports an unsatisfiable cast at cast time, which in a shipped build means a player
/// finding it.
///
/// It also covers the fusion path end to end, which no other shipped content reaches:
/// the hedge-mage's bolt draws on a Lightning cell that is itself fed by two gems, so a
/// broken link anywhere in that chain fails here.
#[test]
fn every_shipped_archetype_can_cast_what_it_inscribes() {
    let elements =
        ElementCatalog::from_file(&parse_elements().expect("elements.ron parses and validates"));
    let spells = SpellBook::from_file(&parse_spells().expect("spells.ron parses and validates"));
    let substances = SubstanceTable::from_file(
        &parse_substances().expect("substances.ron parses"),
        &parse_palette().expect("palette.ron parses and validates"),
    )
    .expect("shipped substances resolve through the art palette");
    let index = ContentIndex::build(&elements, &spells, &substances).expect("content resolves");
    let file = parse_lattices().expect("lattices.ron parses");

    let library = match LatticeLibrary::build(&file, &elements, &spells) {
        Ok(library) => library,
        Err(errors) => panic!("shipped lattices have unresolvable names: {errors:#?}"),
    };
    assert!(
        !library.is_empty(),
        "the shipped library should not be empty"
    );

    let tables = index.tables(&elements);
    let mut checked = 0;
    for (name, archetype) in library.iter() {
        let state = LatticeState::new(&archetype.spec, &archetype.stats);
        for (coord, kind) in archetype.spec.cells() {
            let CellKind::Spell { spell } = kind else {
                continue;
            };
            let label = spells.name(spell).unwrap_or("<unknown>");
            castable(&archetype.spec, &state, coord, &tables).unwrap_or_else(|blocked| {
                panic!(
                    "archetype {name:?} inscribes {label:?} at {coord:?}, which a fresh \
                     lattice cannot cast: {blocked:?} — its neighbours cannot pay"
                )
            });
            checked += 1;
        }
    }

    // Otherwise an archetype file that lost its spell cells — or a rename that made the
    // `CellKind::Spell` arm stop matching — would leave this passing while checking
    // nothing, which is the failure it exists to prevent.
    assert!(
        checked >= 3,
        "expected the shipped archetypes to inscribe at least three spells, found {checked}"
    );
}

/// Every archetype a shipped encounter names must have a lattice.
///
/// The runtime fallback for a missing one is deliberately soft — the unit spawns inert
/// with a warning, so a designer writing content can still look at the rest of the
/// fight — and that softness is exactly why this has to be hard in CI. An inert unit
/// stands, walks and strikes; it cannot cast and **nothing can damage it**. A typo in an
/// archetype name would ship as an enemy that cannot be killed.
#[test]
fn every_encounter_archetype_has_a_lattice() {
    let elements =
        ElementCatalog::from_file(&parse_elements().expect("elements.ron parses and validates"));
    let spells = SpellBook::from_file(&parse_spells().expect("spells.ron parses and validates"));
    let library = LatticeLibrary::build(
        &parse_lattices().expect("lattices.ron parses"),
        &elements,
        &spells,
    )
    .expect("shipped lattices resolve");
    let profiles = parse_ai_profiles().expect("ai_profiles.ron parses and validates");

    // Read the directory rather than listing files: `include_str!` cannot glob, so a
    // hardcoded list silently stops covering the fifth encounter somebody adds — and
    // the whole point of this test is that an unlisted file is the dangerous case.
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/config/encounters");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "ron"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no encounter files found under {}",
        dir.display()
    );

    let mut checked = 0;
    for path in &files {
        let raw = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let encounter: Encounter = ron::from_str(&raw)
            .unwrap_or_else(|error| panic!("{} does not parse: {error}", path.display()));
        for unit in encounter.entries() {
            assert!(
                library.get(unit.archetype).is_some(),
                "encounter {:?} rosters a {:?}, which lattices.ron does not define — it \
                 would spawn unable to cast and impossible to damage",
                encounter.name,
                unit.archetype,
            );
            if let Some(profile) = unit.ai_profile {
                assert!(
                    profiles.get(&AiProfileId(profile.to_owned())).is_some(),
                    "encounter {:?} references unknown AI profile {profile:?}",
                    encounter.name
                );
            }
            checked += 1;
        }
    }
    assert!(
        checked >= files.len(),
        "every encounter should roster at least one unit: {checked} units across {} files",
        files.len()
    );
}

/// "Four hexes and a bite" is a claim about content, so content is where it is checked.
#[test]
fn the_shipped_archetypes_match_the_design() {
    let elements =
        ElementCatalog::from_file(&parse_elements().expect("elements.ron parses and validates"));
    let spells = SpellBook::from_file(&parse_spells().expect("spells.ron parses and validates"));
    let library = LatticeLibrary::build(
        &parse_lattices().expect("lattices.ron parses"),
        &elements,
        &spells,
    )
    .expect("shipped lattices resolve");

    let wolf = library.get("wolf").expect("a wolf is shipped");
    assert_eq!(wolf.spec.capacity(), 4, "four hexes");
    assert!(
        !wolf
            .spec
            .cells()
            .any(|(_, kind)| matches!(kind, CellKind::Spell { .. })),
        "a wolf casts nothing — its threat is the strike verb every unit has"
    );

    let raider = library.get("raider").expect("a raider is shipped");
    assert_eq!(raider.spec.capacity(), 8, "eight hexes");

    let mage = library.get("hedge-mage").expect("a hedge-mage is shipped");
    assert_eq!(mage.spec.capacity(), 13, "thirteen hexes");
    assert!(
        mage.spec
            .cells()
            .any(|(_, kind)| matches!(kind, CellKind::Fusion { .. })),
        "the hedge-mage is the roster's only fusion chain"
    );
    let scrying_eye = spells.id("Scrying Eye").expect("Scrying Eye is shipped");
    assert_eq!(
        mage.spec.get(LatticeCoord::new(-2, -1)),
        Some(CellKind::Spell { spell: scrying_eye }),
        "the hedge-mage can expose the divination system in ordinary play"
    );

    let ember = spells
        .id("Ember")
        .and_then(|id| spells.spell(id))
        .expect("Ember is shipped");
    assert!(
        ember.effects.contains(&Effect::Burn { turns: 2 }),
        "Ember exposes persistent damage for two real turns"
    );
}

#[test]
fn shipped_content_cross_references_resolve() {
    let elements =
        ElementCatalog::from_file(&parse_elements().expect("elements.ron parses and validates"));
    let spells = SpellBook::from_file(&parse_spells().expect("spells.ron parses and validates"));
    let substances = SubstanceTable::from_file(
        &parse_substances().expect("substances.ron parses"),
        &parse_palette().expect("palette.ron parses and validates"),
    )
    .expect("shipped substances resolve through the art palette");

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

/// An area spell on a lattice must not load while the applier only reaches the anchor.
///
/// This is the wave's one place where the interface could tell a lie the player has no
/// way to catch. `hex_units::volumes` resolves a shape to an exact voxel set and the
/// casting preview paints every surface in it; `hex_combat`'s applier still routes each
/// unit-affecting effect through the single unit standing on the anchor. Inscribe
/// Fireball on a lattice and the player would light up thirty-odd surfaces, spend the
/// mana and the turn, and hurt one of them.
///
/// So it is refused at load. The test builds the failing content on purpose rather than
/// asserting the shipped file happens to be clean, because the shipped file being clean
/// is what would make this silently stop covering anything: every current lattice
/// inscribes `Single` spells, so nothing here exercises the check by accident.
#[test]
fn an_area_spell_on_a_lattice_is_refused_while_the_applier_only_reaches_the_anchor() {
    use hex_assets::{AxialPair, UnvalidatedArchetype, UnvalidatedCell, UnvalidatedEntry};
    use std::collections::BTreeMap;

    let elements =
        ElementCatalog::from_file(&parse_elements().expect("elements.ron parses and validates"));
    let spells = SpellBook::from_file(&parse_spells().expect("spells.ron parses and validates"));

    // Fireball is shipped, and its shape is a sphere — see spells.ron.
    let cell = |kind| UnvalidatedEntry {
        at: AxialPair { q: 0, r: 0 },
        kind,
    };
    let with_spell = |name: &str, q: i32| UnvalidatedEntry {
        at: AxialPair { q, r: 0 },
        kind: UnvalidatedCell::Spell(name.to_owned()),
    };
    let archetype = |entries: Vec<UnvalidatedEntry>| UnvalidatedArchetype {
        cells: entries,
        attunement: BTreeMap::from([("Fire".to_owned(), 3)]),
        channelling: BTreeMap::from([("Fire".to_owned(), 3)]),
        ai_profile: None,
    };

    let mut file = LatticeFile {
        archetypes: BTreeMap::from([(
            "area-caster".to_owned(),
            archetype(vec![
                cell(UnvalidatedCell::Gem("Fire".to_owned())),
                with_spell("Fireball", 1),
            ]),
        )]),
    };
    let errors = LatticeLibrary::build(&file, &elements, &spells)
        .expect_err("an area damage spell on a lattice must not resolve");
    let reported = format!("{:?}", errors);
    assert!(
        reported.contains("AreaEffectUnapplied"),
        "the refusal must name the gap it waits on, got: {reported}"
    );

    // The positive control, and the one that keeps the check from being a blanket ban on
    // shapes: the same lattice with a Single spell resolves. Without this, deleting the
    // shape test and refusing every spell cell would still pass.
    file.archetypes.insert(
        "area-caster".to_owned(),
        archetype(vec![
            cell(UnvalidatedCell::Gem("Fire".to_owned())),
            with_spell("Ember", 1),
        ]),
    );
    LatticeLibrary::build(&file, &elements, &spells)
        .expect("a Single-shaped spell on the same lattice resolves");
}
