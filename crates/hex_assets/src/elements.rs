//! The element wheel and fusion recipes, loaded from `assets/config/elements.ron`.
//!
//! Registering an element or a fusion is a content change rather than a code change,
//! exactly like [`substances`](crate::substances). The six-element wheel, opposition
//! and the fusion graph are **data**: no code matches on a specific element.
//!
//! # Two orderings, deliberately separate
//!
//! [`ElementId`] is assigned from **sorted names** (the same reason
//! [`SubstanceTable`](crate::SubstanceTable) sorts: reordering the file must never
//! silently rewrite what an id means). The **wheel** is a *different* ordering — the
//! six basic elements in the order that makes opposition `wheel[(i + len/2) % len]`,
//! index arithmetic over the array. An element's id says nothing about where it sits
//! on the wheel, and it must not: the two orderings answer different questions.
//!
//! # Higher-order elements
//!
//! The full element set is the wheel (basic elements) plus every fusion output
//! (higher-order elements). A fusion output is *not* on the wheel and has no
//! opposite; a basic element is never a fusion output. `validate()` enforces that
//! partition, that the fusion graph is acyclic, and that every fusion input is
//! *feedable* — a basic element or something another fusion produces.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use hex_core::{ElementId, Screen};
use serde::Deserialize;
use std::collections::HashSet;

use crate::fingerprint::FingerprintEncoder;
use crate::{LoadSettings, CONFIG_EXTENSIONS};

/// One input a fusion draws from an adjacent gem or live fusion output.
///
/// Mirrors `hex_lattice::Requirement`'s shape (an element and the mana it
/// contributes) so the future `FusionTable` implementation is a direct map.
#[derive(Reflect, Debug, Clone, Deserialize)]
pub struct FusionInput {
    /// The element the adjacent source must provide, by name.
    pub element: String,
    /// How much mana that source contributes to the fusion.
    pub mana: u16,
}

/// The raw file, before names are turned into ids.
///
/// `Deserialize` is hand-written (via `UnvalidatedElementFile`) so every
/// intra-file invariant is checked at parse time: an invalid `elements.ron` fails to
/// load and the previous valid [`ElementCatalog`] stays active.
#[derive(Asset, Resource, Reflect, Debug, Clone)]
#[reflect(Resource)]
pub struct ElementFile {
    /// The basic elements, in wheel order. Opposition pairs them at `len/2` apart.
    pub wheel: Vec<String>,
    /// Fusion recipes: an output element name to the inputs it consumes.
    pub fusions: HashMap<String, Vec<FusionInput>>,
}

/// The same shape as [`ElementFile`], but with a derived `Deserialize` and no
/// validation. [`ElementFile`]'s manual `Deserialize` parses this, then validates.
#[derive(Deserialize)]
struct UnvalidatedElementFile {
    wheel: Vec<String>,
    fusions: HashMap<String, Vec<FusionInput>>,
}

/// A fusion cell has six neighbours, so a recipe can never draw from more —
/// the same ring geometry that caps a spell's tier at six.
const MAX_FUSION_INPUTS: usize = 6;

impl<'de> Deserialize<'de> for ElementFile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = UnvalidatedElementFile::deserialize(deserializer)?;
        let file = Self {
            wheel: raw.wheel,
            fusions: raw.fusions,
        };
        file.validate().map_err(serde::de::Error::custom)?;
        Ok(file)
    }
}

impl ElementFile {
    /// Checks every intra-file invariant before the file can become a catalog.
    ///
    /// Cross-file references (a spell requiring an element, an effect naming a
    /// substance) are **not** checked here — that is
    /// [`ContentIndex`](crate::ContentIndex)'s job, because a single file cannot see
    /// the others.
    pub fn validate(&self) -> Result<(), String> {
        // The wheel must pair under opposition: non-empty, even, no repeats.
        if self.wheel.is_empty() {
            return Err("wheel must list the basic elements".to_owned());
        }
        if !self.wheel.len().is_multiple_of(2) {
            return Err(format!(
                "wheel must have an even length so opposition pairs every element (got {})",
                self.wheel.len()
            ));
        }
        let mut seen = HashSet::new();
        for element in &self.wheel {
            if !seen.insert(element.as_str()) {
                return Err(format!("wheel lists '{element}' more than once"));
            }
        }

        // An element is basic (on the wheel) or higher-order (a fusion output), never
        // both, never neither.
        for output in self.fusions.keys() {
            if self.wheel.contains(output) {
                return Err(format!(
                    "'{output}' is a basic wheel element and cannot also be a fusion output"
                ));
            }
        }

        // Every fusion input must be feedable, and every fusion must draw real mana.
        let producible: HashSet<&str> = self
            .wheel
            .iter()
            .map(String::as_str)
            .chain(self.fusions.keys().map(String::as_str))
            .collect();
        for (output, inputs) in &self.fusions {
            if inputs.is_empty() {
                return Err(format!("fusion '{output}' has no inputs"));
            }
            // A fusion is a cell drawing from adjacent gems, so its recipe is
            // ring-bounded exactly like a spell's tier: six neighbours, no more.
            if inputs.len() > MAX_FUSION_INPUTS {
                return Err(format!(
                    "fusion '{output}' has {} inputs; the maximum is {MAX_FUSION_INPUTS} (a full ring)",
                    inputs.len()
                ));
            }
            for input in inputs {
                if !producible.contains(input.element.as_str()) {
                    return Err(format!(
                        "fusion '{output}' needs '{}', which is neither a basic element nor a fusion output",
                        input.element
                    ));
                }
                if input.mana == 0 {
                    return Err(format!(
                        "fusion '{output}' input '{}' must draw at least 1 mana",
                        input.element
                    ));
                }
            }
        }

        // The fusion graph must be acyclic, or a chain never bottoms out at basics.
        detect_fusion_cycle(&self.fusions)
    }
}

/// Depth-first colouring state for cycle detection.
enum Visit {
    /// On the current DFS stack — seeing it again is a cycle.
    InProgress,
    /// Fully explored, cannot be part of a new cycle.
    Done,
}

/// Fails if the fusion recipes form a cycle (an output reachable from its own
/// inputs), which would mean a fusion chain that never reduces to basic elements.
fn detect_fusion_cycle(fusions: &HashMap<String, Vec<FusionInput>>) -> Result<(), String> {
    let mut state: HashMap<String, Visit> = HashMap::default();
    for output in fusions.keys() {
        walk_fusion(output, fusions, &mut state)?;
    }
    Ok(())
}

/// One DFS step of [`detect_fusion_cycle`], following only inputs that are themselves
/// fusion outputs (basic-element inputs are leaves).
fn walk_fusion(
    node: &str,
    fusions: &HashMap<String, Vec<FusionInput>>,
    state: &mut HashMap<String, Visit>,
) -> Result<(), String> {
    match state.get(node) {
        Some(Visit::Done) => return Ok(()),
        Some(Visit::InProgress) => {
            return Err(format!("fusion recipes form a cycle through '{node}'"));
        }
        None => {}
    }
    state.insert(node.to_owned(), Visit::InProgress);
    if let Some(inputs) = fusions.get(node) {
        for input in inputs {
            if fusions.contains_key(&input.element) {
                walk_fusion(&input.element, fusions, state)?;
            }
        }
    }
    state.insert(node.to_owned(), Visit::Done);
    Ok(())
}

/// Elements indexed by the [`ElementId`] assigned from sorted names.
///
/// Holds the wheel (for opposition) and the resolved fusion recipes. See the
/// [module documentation](self) for why id order and wheel order are separate.
#[derive(Resource, Reflect, Debug, Clone, Default)]
#[reflect(Resource)]
pub struct ElementCatalog {
    /// Names indexed by id; `by_id[i]` is the name of `ElementId(i)`.
    by_id: Vec<String>,
    #[reflect(ignore)]
    by_name: HashMap<String, ElementId>,
    /// The basic elements in wheel order, for opposition arithmetic.
    wheel: Vec<ElementId>,
    /// Fusion recipes: output id to its resolved `(input id, mana)` inputs.
    #[reflect(ignore)]
    fusions: HashMap<ElementId, Vec<(ElementId, u16)>>,
    /// Canonical semantics of the `ElementFile` this catalog was built from.
    #[reflect(ignore)]
    source_fingerprint: u64,
}

impl ElementCatalog {
    /// The id a name maps to, or [`None`] if there is no such element.
    #[must_use]
    pub fn id(&self, name: &str) -> Option<ElementId> {
        self.by_name.get(name).copied()
    }

    /// The name of an element, for logs and content resolution.
    #[must_use]
    pub fn name(&self, id: ElementId) -> Option<&str> {
        self.by_id.get(id.0 as usize).map(String::as_str)
    }

    /// The element opposite `id` on the wheel, or [`None`] if `id` is a higher-order
    /// element (only basic elements sit on the wheel).
    ///
    /// Opposition is `wheel[(i + len/2) % len]` — index arithmetic, never a match on a
    /// specific element.
    #[must_use]
    pub fn opposite(&self, id: ElementId) -> Option<ElementId> {
        let len = self.wheel.len();
        if len == 0 {
            return None;
        }
        let position = self.wheel.iter().position(|&candidate| candidate == id)?;
        self.wheel.get((position + len / 2) % len).copied()
    }

    /// The inputs a fusion producing `output` consumes, or [`None`] if `output` is a
    /// basic element rather than a fusion output.
    ///
    /// Shaped to feed `hex_lattice::FusionTable::recipe`: each entry is an
    /// `(element, mana)` pair.
    #[must_use]
    pub fn recipe(&self, output: ElementId) -> Option<&[(ElementId, u16)]> {
        self.fusions.get(&output).map(Vec::as_slice)
    }

    /// Whether `id` is a basic element (on the wheel).
    #[must_use]
    pub fn is_basic(&self, id: ElementId) -> bool {
        self.wheel.contains(&id)
    }

    /// Whether `id` is a higher-order element (a fusion output).
    #[must_use]
    pub fn is_higher_order(&self, id: ElementId) -> bool {
        self.fusions.contains_key(&id)
    }

    /// The basic elements in wheel order.
    #[must_use]
    pub fn wheel(&self) -> &[ElementId] {
        &self.wheel
    }

    /// How many elements the catalog holds, basic and higher-order.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Whether the catalog is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Whether this catalog was built from the current authored element semantics.
    #[must_use]
    pub fn matches_source(&self, file: &ElementFile) -> bool {
        self.source_fingerprint == element_file_fingerprint(file)
    }

    pub(crate) const fn source_fingerprint(&self) -> u64 {
        self.source_fingerprint
    }

    /// Builds a catalog from a loaded file, assigning ids from sorted names.
    ///
    /// Relies on [`ElementFile::validate`] having already run (it does, in
    /// `Deserialize`): unresolvable names are skipped rather than panicking, which
    /// cannot happen for a file that parsed.
    #[must_use]
    pub fn from_file(file: &ElementFile) -> Self {
        let mut names: Vec<String> = file
            .wheel
            .iter()
            .cloned()
            .chain(file.fusions.keys().cloned())
            .collect();
        names.sort();
        names.dedup();

        let mut by_name: HashMap<String, ElementId> = HashMap::default();
        for (index, name) in names.iter().enumerate() {
            let id = u16::try_from(index).unwrap_or(u16::MAX);
            by_name.insert(name.clone(), ElementId(id));
        }

        let wheel: Vec<ElementId> = file
            .wheel
            .iter()
            .filter_map(|name| by_name.get(name).copied())
            .collect();

        let mut fusions: HashMap<ElementId, Vec<(ElementId, u16)>> = HashMap::default();
        for (output, inputs) in &file.fusions {
            let Some(&output_id) = by_name.get(output) else {
                continue;
            };
            let resolved = inputs
                .iter()
                .filter_map(|input| by_name.get(&input.element).map(|&id| (id, input.mana)))
                .collect();
            fusions.insert(output_id, resolved);
        }

        Self {
            by_id: names,
            by_name,
            wheel,
            fusions,
            source_fingerprint: element_file_fingerprint(file),
        }
    }
}

fn element_file_fingerprint(file: &ElementFile) -> u64 {
    let mut encoder = FingerprintEncoder::new(b"hex-element-file-v1");
    encoder.usize(file.wheel.len());
    for element in &file.wheel {
        encoder.string(element);
    }

    let mut outputs: Vec<_> = file.fusions.iter().collect();
    outputs.sort_by_key(|(name, _)| *name);
    encoder.usize(outputs.len());
    for (output, inputs) in outputs {
        encoder.string(output);
        encoder.usize(inputs.len());
        for input in inputs {
            encoder.string(&input.element);
            encoder.u16(input.mana);
        }
    }
    encoder.finish()
}

/// Registers the element catalog for loading.
pub fn plugin(app: &mut App) {
    app.register_type::<ElementCatalog>();
    app.load_settings::<ElementFile>("config/elements.ron", CONFIG_EXTENSIONS);
    register_catalog_builder(app);
}

/// Rebuilds the catalog when the file loads or hot-reloads, but never during
/// gameplay — reassigning ids under a live lattice would reinterpret it.
fn register_catalog_builder(app: &mut App) {
    app.add_systems(
        Update,
        build_element_catalog.run_if(not(in_state(Screen::Gameplay))),
    );
}

/// Turns the loaded file into the indexed catalog, and rebuilds it on hot-reload.
fn build_element_catalog(
    mut commands: Commands,
    file: Option<Res<ElementFile>>,
    catalog: Option<Res<ElementCatalog>>,
) {
    let Some(file) = file else { return };
    if !file.is_changed() && catalog.is_some() {
        return;
    }
    commands.insert_resource(ElementCatalog::from_file(&file));
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;

    use super::*;

    fn wheel() -> Vec<String> {
        ["Light", "Air", "Fire", "Metal", "Earth", "Water"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    fn fusion(element: &str, mana: u16) -> FusionInput {
        FusionInput {
            element: element.to_owned(),
            mana,
        }
    }

    fn test_file() -> ElementFile {
        let mut fusions = HashMap::default();
        fusions.insert(
            "Lightning".to_owned(),
            vec![fusion("Light", 1), fusion("Fire", 1)],
        );
        ElementFile {
            wheel: wheel(),
            fusions,
        }
    }

    fn shipped_file() -> ElementFile {
        ron::from_str(include_str!("../../../assets/config/elements.ron"))
            .expect("the shipped element file should parse and validate")
    }

    #[test]
    fn shipped_elements_parse() {
        let catalog = ElementCatalog::from_file(&shipped_file());
        assert!(catalog.id("Fire").is_some());
        assert!(catalog.id("Lightning").is_some());
    }

    /// The failure this guards is silent: if ids came from file order, moving an
    /// entry would rewrite every lattice that stored the old id.
    #[test]
    fn ids_do_not_depend_on_file_order() {
        let first = ElementCatalog::from_file(&test_file());
        let second = ElementCatalog::from_file(&test_file());
        for name in ["Fire", "Light", "Water", "Lightning"] {
            assert_eq!(
                first.id(name),
                second.id(name),
                "{name} moved between builds"
            );
        }
    }

    /// Id order is alphabetical and independent of wheel order.
    #[test]
    fn ids_are_alphabetical_not_wheel_order() {
        let catalog = ElementCatalog::from_file(&test_file());
        // Sorted names: Air, Earth, Fire, Light, Lightning, Metal, Water.
        assert_eq!(catalog.name(ElementId(0)), Some("Air"));
        assert_eq!(catalog.name(ElementId(1)), Some("Earth"));
        assert_eq!(catalog.name(ElementId(2)), Some("Fire"));
    }

    #[test]
    fn opposition_is_index_arithmetic_over_the_wheel() {
        let catalog = ElementCatalog::from_file(&test_file());
        for (a, b) in [("Light", "Metal"), ("Air", "Earth"), ("Fire", "Water")] {
            let (Some(ida), Some(idb)) = (catalog.id(a), catalog.id(b)) else {
                unreachable!("the test wheel defines {a} and {b}")
            };
            assert_eq!(catalog.opposite(ida), Some(idb), "{a} opposes {b}");
            assert_eq!(catalog.opposite(idb), Some(ida), "opposition is symmetric");
        }
    }

    #[test]
    fn higher_order_elements_have_no_opposite() {
        let catalog = ElementCatalog::from_file(&test_file());
        let lightning = catalog
            .id("Lightning")
            .expect("test file defines Lightning");
        assert!(catalog.opposite(lightning).is_none());
        assert!(catalog.is_higher_order(lightning));
        assert!(!catalog.is_basic(lightning));
    }

    #[test]
    fn fusion_recipe_resolves_to_ids_with_mana() {
        let catalog = ElementCatalog::from_file(&test_file());
        let lightning = catalog
            .id("Lightning")
            .expect("test file defines Lightning");
        let recipe = catalog.recipe(lightning).expect("Lightning is a fusion");
        assert_eq!(recipe.len(), 2, "Lightning fuses two elements");
        for (id, mana) in recipe {
            assert!(*mana >= 1, "every fusion input draws mana");
            assert!(
                catalog.is_basic(*id),
                "Lightning's inputs are basic elements"
            );
        }
    }

    #[test]
    fn validate_rejects_an_odd_wheel() {
        let mut file = test_file();
        file.wheel.pop();
        assert!(
            file.validate().is_err(),
            "an odd wheel cannot pair opposition"
        );
    }

    #[test]
    fn validate_rejects_a_dangling_fusion_input() {
        let mut file = test_file();
        file.fusions.insert(
            "Steam".to_owned(),
            vec![fusion("Fire", 1), fusion("Nonexistent", 1)],
        );
        assert!(
            file.validate().is_err(),
            "a fusion cannot feed on a non-element"
        );
    }

    #[test]
    fn validate_rejects_a_fusion_with_more_inputs_than_a_ring() {
        let mut file = test_file();
        file.fusions.insert(
            "Overload".to_owned(),
            std::iter::repeat_with(|| fusion("Fire", 1))
                .take(7)
                .collect(),
        );
        assert!(
            file.validate().is_err(),
            "seven inputs cannot fit a six-neighbour ring"
        );
    }

    #[test]
    fn validate_rejects_a_fusion_cycle() {
        let mut file = test_file();
        // Plasma <- Lightning, and make Lightning depend back on Plasma.
        file.fusions
            .insert("Plasma".to_owned(), vec![fusion("Lightning", 1)]);
        file.fusions
            .insert("Lightning".to_owned(), vec![fusion("Plasma", 1)]);
        assert!(
            file.validate().is_err(),
            "a fusion cycle never reduces to basics"
        );
    }

    #[test]
    fn validate_rejects_a_basic_element_as_fusion_output() {
        let mut file = test_file();
        file.fusions
            .insert("Fire".to_owned(), vec![fusion("Light", 1)]);
        assert!(
            file.validate().is_err(),
            "an element is basic xor higher-order, never both"
        );
    }

    /// Reassigning sorted ids under a live world would reinterpret existing lattices,
    /// so the catalog waits for gameplay to end — the same rule as the substance table.
    #[test]
    fn catalog_rebuild_waits_until_gameplay_ends() {
        let original = test_file();
        let mut replacement = test_file();
        replacement.fusions.insert(
            "Steam".to_owned(),
            vec![fusion("Fire", 1), fusion("Water", 1)],
        );

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.insert_state(Screen::Gameplay);
        app.insert_resource(ElementCatalog::from_file(&original));
        app.insert_resource(replacement);
        register_catalog_builder(&mut app);

        app.update();
        assert!(
            app.world()
                .resource::<ElementCatalog>()
                .id("Steam")
                .is_none(),
            "the live world must keep the catalog its lattices were built from"
        );

        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Title);
        app.update();
        assert!(
            app.world()
                .resource::<ElementCatalog>()
                .id("Steam")
                .is_some(),
            "the catalog should rebuild once gameplay has torn down"
        );
    }
}
