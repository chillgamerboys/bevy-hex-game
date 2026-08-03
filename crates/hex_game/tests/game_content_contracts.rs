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
    character_lattice_file, combined_spell_file, creator_character_issues, AiProfileCatalog,
    ArtPalette, AxialPair, CastingAxis, ContentIndex, CreationCellKind, CreationLibraryFile,
    CreationPresetCatalog, CustomCharacterId, Effect, ElementCatalog, ElementFile, Encounter,
    FormationCatalog, GemRequirement, LatticeFile, LatticeLibrary, ManaAxis, PresetAudience,
    SavedCharacter, Spell, SpellBook, SpellFile, SubstanceFile, SubstanceTable, TargetShape,
    TargetingSpec, TerrainDamageFile, TerrainDamageTable, Trajectory, UnvalidatedArchetype,
    UnvalidatedCell, UnvalidatedEntry,
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

fn parse_terrain_damage() -> Result<TerrainDamageFile, SpannedError> {
    ron::from_str(include_str!("../../../assets/config/terrain_damage.ron"))
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

fn parse_creation_presets() -> Result<CreationPresetCatalog, SpannedError> {
    ron::from_str(include_str!("../../../assets/config/creation_presets.ron"))
}

const CANONICAL_FUSIONS: [(&str, &[&str]); 12] = [
    ("Lightning", &["Air", "Fire"]),
    ("Volcano", &["Fire", "Metal"]),
    ("Crystal", &["Metal", "Earth"]),
    ("Transmutation", &["Earth", "Life"]),
    ("Divination", &["Life", "Water"]),
    ("Illusion", &["Water", "Air"]),
    ("Destruction", &["Air", "Fire", "Metal"]),
    ("Artifice", &["Fire", "Metal", "Earth"]),
    ("Necromancy", &["Metal", "Earth", "Life"]),
    ("Wild", &["Earth", "Life", "Water"]),
    ("Storm", &["Life", "Water", "Air"]),
    ("Space", &["Water", "Air", "Fire"]),
];

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

#[test]
fn every_packaged_creator_character_is_valid_and_can_cast_every_inscription() {
    let elements =
        ElementCatalog::from_file(&parse_elements().expect("elements.ron parses and validates"));
    let shipped_file = parse_spells().expect("spells.ron parses and validates");
    let shipped_spells = SpellBook::from_file(&shipped_file);
    let substances = SubstanceTable::from_file(
        &parse_substances().expect("substances.ron parses"),
        &parse_palette().expect("palette.ron parses and validates"),
    )
    .expect("shipped substances resolve through the art palette");
    let presets = parse_creation_presets().expect("creation_presets.ron parses");
    let library = presets.library_for(PresetAudience::HumanTemplate);
    let combined_file = combined_spell_file(&shipped_file, library.spells.clone())
        .expect("packaged custom spell names do not collide");
    let combined_spells = SpellBook::from_file(&combined_file);
    let index = ContentIndex::build(&elements, &combined_spells, &substances)
        .expect("packaged Creator spell content resolves");

    for character in &library.characters {
        assert!(
            creator_character_issues(character, &library, &elements, &shipped_spells).is_empty(),
            "packaged Creator character {:?} is invalid",
            character.name
        );
        let lattice_file = character_lattice_file(character, &library)
            .unwrap_or_else(|error| panic!("{:?} did not convert: {error}", character.name));
        let lattices = LatticeLibrary::build(&lattice_file, &elements, &combined_spells)
            .unwrap_or_else(|errors| panic!("{:?} did not resolve: {errors:#?}", character.name));
        let runtime_key = hex_assets::character_runtime_key(character.id);
        let archetype = lattices
            .get(&runtime_key)
            .unwrap_or_else(|| panic!("missing converted {:?}", character.name));
        let state = LatticeState::new(&archetype.spec, &archetype.stats);
        for (coord, kind) in archetype.spec.cells() {
            if !matches!(kind, CellKind::Spell { .. }) {
                continue;
            }
            castable(&archetype.spec, &state, coord, &index.tables(&elements))
                .unwrap_or_else(|blocked| {
                    panic!(
                        "packaged Creator character {:?} has uncastable spell at {coord:?}: {blocked:?}",
                        character.name
                    )
                });
        }
    }

    let hedge = library
        .characters
        .iter()
        .find(|character| character.name == "Hedge Mage Template")
        .expect("the packaged hedge mage remains available");
    assert_eq!(hedge.cells.len(), 13);
}

#[test]
fn a_creator_draft_with_removed_light_is_preserved_but_visibly_invalid() {
    let elements =
        ElementCatalog::from_file(&parse_elements().expect("elements.ron parses and validates"));
    let spells = SpellBook::from_file(&parse_spells().expect("spells.ron parses and validates"));
    let mut draft = SavedCharacter::blank(CustomCharacterId(91), "Old Light Draft");
    draft
        .cells
        .first_mut()
        .expect("a blank Creator character retains its origin cell")
        .kind = CreationCellKind::Gem("Light".to_owned());
    draft.attunement.insert("Light".to_owned(), 3);
    draft.channelling.insert("Light".to_owned(), 1);
    let library = CreationLibraryFile {
        next_character_id: 92,
        characters: vec![draft.clone()],
        ..CreationLibraryFile::default()
    };

    let encoded = ron::to_string(&library).expect("legacy name-based draft serializes");
    let decoded: CreationLibraryFile =
        ron::from_str(&encoded).expect("legacy name-based draft remains structurally readable");
    assert_eq!(decoded.characters, vec![draft]);
    let decoded_draft = decoded
        .characters
        .first()
        .expect("the round trip preserves the one draft");
    let issues = creator_character_issues(decoded_draft, &decoded, &elements, &spells);
    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("unknown gem element \"Light\"")),
        "removed Light must be diagnosed without rewriting the draft: {issues:#?}"
    );
    assert_eq!(
        decoded_draft
            .cells
            .first()
            .expect("the round trip preserves the origin cell")
            .kind,
        CreationCellKind::Gem("Light".to_owned()),
        "no Light to Life/Air/Divination/Illusion substitution is permitted"
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
        mage.spec.get(LatticeCoord::new(-2, 1)),
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
fn shipped_element_catalog_is_the_exact_canonical_grid() {
    let elements =
        ElementCatalog::from_file(&parse_elements().expect("elements.ron parses and validates"));
    assert_eq!(elements.len(), 18, "six basics plus twelve fusions");

    let wheel = elements
        .wheel()
        .iter()
        .map(|id| elements.name(*id).expect("wheel ids resolve"))
        .collect::<Vec<_>>();
    assert_eq!(wheel, ["Air", "Fire", "Metal", "Earth", "Life", "Water"]);

    for (left, right) in [("Air", "Earth"), ("Fire", "Life"), ("Metal", "Water")] {
        let left = elements.id(left).expect("canonical basic exists");
        let right = elements.id(right).expect("canonical basic exists");
        assert_eq!(elements.opposite(left), Some(right));
        assert_eq!(elements.opposite(right), Some(left));
    }

    for (output, expected_inputs) in CANONICAL_FUSIONS {
        let output_id = elements.id(output).expect("canonical fusion exists");
        let recipe = elements.recipe(output_id).expect("fusion has a recipe");
        let actual = recipe
            .iter()
            .map(|(id, mana)| {
                assert_eq!(*mana, 1, "{output} uses one mana per direct feeder");
                elements.name(*id).expect("recipe ids resolve")
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected_inputs, "wrong direct recipe for {output}");
        assert!(
            recipe.iter().all(|(id, _)| elements.is_basic(*id)),
            "{output} must draw directly from basics, not another fusion"
        );
    }
    assert!(elements.id("Light").is_none());
    assert!(elements.id("Rainstorm").is_none());
}

/// Proves the authored direct recipes against the real recursive lattice solver, not
/// just by inspecting their data. Each output gets one adjacent fusion cell, one
/// distinct basic gem per input, and a probe spell. Breaking any feeder must make the
/// same spell uncastable. Space is therefore an explicit Water + Air + Fire proof.
#[test]
fn every_canonical_fusion_is_castable_only_with_all_distinct_direct_feeders() {
    use std::collections::BTreeMap;

    let elements =
        ElementCatalog::from_file(&parse_elements().expect("elements.ron parses and validates"));
    let substances = SubstanceTable::from_file(
        &parse_substances().expect("substances.ron parses"),
        &parse_palette().expect("palette.ron parses and validates"),
    )
    .expect("shipped substances resolve through the art palette");
    let feeder_coords = [
        AxialPair { q: 0, r: 1 },
        AxialPair { q: -1, r: 1 },
        AxialPair { q: -1, r: 0 },
    ];

    for (output, inputs) in CANONICAL_FUSIONS {
        let spell_name = format!("{output} Probe");
        let mut spell_file = SpellFile {
            spells: Default::default(),
        };
        spell_file.spells.insert(
            spell_name.clone(),
            Spell {
                requirements: vec![GemRequirement {
                    element: output.to_owned(),
                    mana: 1,
                }],
                casting: CastingAxis::Evocation,
                mana: ManaAxis::Fixed,
                co_castable: false,
                targeting: TargetingSpec {
                    range: 0,
                    shape: TargetShape::SelfCast,
                    trajectory: Trajectory::None,
                },
                effects: vec![Effect::DisableHexes {
                    count: 1,
                    targeted: false,
                }],
            },
        );
        let spells = SpellBook::from_file(&spell_file);
        let index = ContentIndex::build(&elements, &spells, &substances)
            .unwrap_or_else(|errors| panic!("{output} probe content failed: {errors:#?}"));

        let mut cells = vec![
            UnvalidatedEntry {
                at: AxialPair { q: 0, r: 0 },
                kind: UnvalidatedCell::Fusion(output.to_owned()),
            },
            UnvalidatedEntry {
                at: AxialPair { q: 1, r: 0 },
                kind: UnvalidatedCell::Spell(spell_name.clone()),
            },
        ];
        let mut attunement = BTreeMap::new();
        let mut channelling = BTreeMap::new();
        for (input, coord) in inputs.iter().zip(feeder_coords) {
            cells.push(UnvalidatedEntry {
                at: coord,
                kind: UnvalidatedCell::Gem((*input).to_owned()),
            });
            attunement.insert((*input).to_owned(), 1);
            channelling.insert((*input).to_owned(), 1);
        }
        let lattice_file = LatticeFile {
            archetypes: BTreeMap::from([(
                output.to_owned(),
                UnvalidatedArchetype {
                    cells,
                    attunement,
                    channelling,
                    ai_profile: None,
                },
            )]),
        };
        let library = LatticeLibrary::build(&lattice_file, &elements, &spells)
            .unwrap_or_else(|errors| panic!("{output} probe lattice failed: {errors:#?}"));
        let archetype = library.get(output).expect("probe archetype resolves");
        let tables = index.tables(&elements);
        let spell_coord = LatticeCoord::new(1, 0);
        let fresh = LatticeState::new(&archetype.spec, &archetype.stats);
        castable(&archetype.spec, &fresh, spell_coord, &tables)
            .unwrap_or_else(|blocked| panic!("{output} should be castable: {blocked:?}"));

        for coord in feeder_coords.into_iter().take(inputs.len()) {
            let coord = LatticeCoord::new(coord.q, coord.r);
            let mut broken = LatticeState::new(&archetype.spec, &archetype.stats);
            hex_lattice::apply_disables(&mut broken, &[coord]);
            assert!(
                castable(&archetype.spec, &broken, spell_coord, &tables).is_err(),
                "{output} stayed castable after disabling feeder {coord:?}"
            );
        }
    }
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
    let terrain_damage = TerrainDamageTable::from_file(
        &parse_terrain_damage().expect("terrain_damage.ron parses and validates"),
        &elements,
        &substances,
    )
    .expect("shipped terrain damage has no dangling or indestructible references");

    assert!(
        !spells.is_empty(),
        "the shipped spell book should not be empty"
    );
    assert_eq!(
        index.len(),
        spells.len(),
        "every shipped spell must be resolved in the content index"
    );
    assert_eq!(
        terrain_damage.len(),
        162,
        "all 18 canonical elements must be admitted against all 9 tough substances"
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

/// The Creator route promised by this wave can inscribe and fund shipped Fireball.
///
/// This deliberately builds the same full Fire ring a player must author rather than
/// relying on the shipped archetypes, which still inscribe only single-target spells.
/// It protects both sides of the runtime route: area Impact/Disable is admitted now
/// that the applier reaches the resolved volume, and the resulting lattice can actually
/// pay Fireball's six requirements.
#[test]
fn shipped_fireball_is_admitted_and_castable_from_a_full_fire_ring() {
    use hex_assets::{AxialPair, UnvalidatedArchetype, UnvalidatedCell, UnvalidatedEntry};
    use std::collections::BTreeMap;

    let elements =
        ElementCatalog::from_file(&parse_elements().expect("elements.ron parses and validates"));
    let spells = SpellBook::from_file(&parse_spells().expect("spells.ron parses and validates"));
    let entry = |q, r, kind| UnvalidatedEntry {
        at: AxialPair { q, r },
        kind,
    };
    let fire_gem = |q, r| entry(q, r, UnvalidatedCell::Gem("Fire".to_owned()));
    let file = LatticeFile {
        archetypes: BTreeMap::from([(
            "fireball-adept".to_owned(),
            UnvalidatedArchetype {
                cells: vec![
                    entry(0, 0, UnvalidatedCell::Spell("Fireball".to_owned())),
                    fire_gem(1, 0),
                    fire_gem(0, 1),
                    fire_gem(-1, 1),
                    fire_gem(-1, 0),
                    fire_gem(0, -1),
                    fire_gem(1, -1),
                ],
                attunement: BTreeMap::from([("Fire".to_owned(), 3)]),
                channelling: BTreeMap::from([("Fire".to_owned(), 2)]),
                ai_profile: None,
            },
        )]),
    };
    let library = LatticeLibrary::build(&file, &elements, &spells)
        .expect("supported shipped area effects should resolve from Creator content");
    let archetype = library
        .get("fireball-adept")
        .expect("the resolved library retains the authored character");
    let fireball = spells.id("Fireball").expect("Fireball is shipped");
    assert_eq!(
        archetype.spec.get(LatticeCoord::ORIGIN),
        Some(CellKind::Spell { spell: fireball })
    );

    let substances = SubstanceTable::from_file(
        &parse_substances().expect("substances.ron parses"),
        &parse_palette().expect("palette.ron parses and validates"),
    )
    .expect("shipped substances resolve through the art palette");
    let index = ContentIndex::build(&elements, &spells, &substances).expect("content resolves");
    let state = LatticeState::new(&archetype.spec, &archetype.stats);
    castable(
        &archetype.spec,
        &state,
        LatticeCoord::ORIGIN,
        &index.tables(&elements),
    )
    .expect("the full Fire ring pays all six Fireball requirements");
}
