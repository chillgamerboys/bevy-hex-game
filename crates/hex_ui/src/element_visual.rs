//! Presentation-only metadata for the canonical elemental grid.
//!
//! Coordinates, icon paths, and colors are visual facts. Whether an element is a
//! basic gem or a fusion—and the formula shown for that fusion—comes from the live
//! [`ElementCatalog`], so presentation cannot drift into a second recipe authority.

use bevy::prelude::*;
use hex_assets::ElementCatalog;
use hex_core::ElementId;

/// One authored visual treatment in the elemental grid.
#[derive(Debug, Clone)]
pub struct ElementVisual {
    /// Stable element name matched against the live element catalog.
    pub name: &'static str,
    /// Runtime asset path retained for diagnostics and asset review.
    pub icon_path: &'static str,
    /// Loaded transparent glyph.
    pub icon: Handle<Image>,
    /// Authored sRGB tint for the element's pointy-top hex.
    pub tint: Color,
    /// Approved axial coordinate in the radius-two chart.
    pub coord: IVec2,
}

/// Recipe-derived classification used by visible and spoken Creator copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementClassification {
    /// One of the six basic wheel elements.
    Basic,
    /// A direct two-input fusion.
    Pair,
    /// A direct three-input fusion.
    Triple,
    /// A live higher-order recipe with a non-canonical input count.
    HigherOrder(usize),
}

impl ElementClassification {
    /// Short player-facing class label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Basic => "basic element",
            Self::Pair => "pair fusion",
            Self::Triple => "triple fusion",
            Self::HigherOrder(_) => "higher-order fusion",
        }
    }
}

/// Live catalog facts resolved for one visual element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedElementVisual {
    /// Stable live element id.
    pub id: ElementId,
    /// Classification derived from the live wheel or recipe arity.
    pub classification: ElementClassification,
    /// Text fallback derived from the live fusion recipe.
    pub formula: String,
}

/// Presentation resource for the complete canonical radius-two elemental grid.
#[derive(Resource, Debug, Clone)]
pub struct ElementVisualCatalog {
    entries: Vec<ElementVisual>,
}

impl FromWorld for ElementVisualCatalog {
    fn from_world(world: &mut World) -> Self {
        let asset_server = world.resource::<AssetServer>();
        let entries = ELEMENT_VISUAL_SPECS
            .iter()
            .map(|spec| ElementVisual {
                name: spec.name,
                icon_path: spec.icon_path,
                icon: asset_server.load(spec.icon_path),
                tint: spec.tint,
                coord: spec.coord,
            })
            .collect();
        Self { entries }
    }
}

impl ElementVisualCatalog {
    /// All 18 visuals in deterministic spatial keyboard order: the basic inner ring,
    /// then the alternating pair/triple outer ring, clockwise within each ring.
    #[must_use]
    pub fn entries(&self) -> &[ElementVisual] {
        &self.entries
    }

    /// Finds visual metadata by the stable element name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ElementVisual> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    /// Resolves classification and formula from the accepted gameplay catalog.
    ///
    /// `None` means this presentation entry is absent from the live catalog; callers
    /// must not offer a tool that the Creator cannot author.
    #[must_use]
    pub fn resolve(&self, name: &str, elements: &ElementCatalog) -> Option<ResolvedElementVisual> {
        self.get(name)?;
        resolve_catalog_element(name, elements)
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<ElementVisualCatalog>();
}

#[derive(Debug, Clone, Copy)]
struct ElementVisualSpec {
    name: &'static str,
    icon_path: &'static str,
    tint: Color,
    coord: IVec2,
}

// Visual metadata only. This order is also the stable keyboard order. Recipes and
// Basic/Pair/Triple classification deliberately do not appear here.
const ELEMENT_VISUAL_SPECS: [ElementVisualSpec; 18] = [
    ElementVisualSpec {
        name: "Air",
        icon_path: "ui/elements/air.png",
        tint: Color::srgb(0.18, 0.43, 0.55),
        coord: IVec2::new(0, -1),
    },
    ElementVisualSpec {
        name: "Fire",
        icon_path: "ui/elements/fire.png",
        tint: Color::srgb(0.62, 0.18, 0.10),
        coord: IVec2::new(1, -1),
    },
    ElementVisualSpec {
        name: "Metal",
        icon_path: "ui/elements/metal.png",
        tint: Color::srgb(0.33, 0.39, 0.46),
        coord: IVec2::new(1, 0),
    },
    ElementVisualSpec {
        name: "Earth",
        icon_path: "ui/elements/earth.png",
        tint: Color::srgb(0.39, 0.27, 0.13),
        coord: IVec2::new(0, 1),
    },
    ElementVisualSpec {
        name: "Life",
        icon_path: "ui/elements/life.png",
        tint: Color::srgb(0.17, 0.46, 0.24),
        coord: IVec2::new(-1, 1),
    },
    ElementVisualSpec {
        name: "Water",
        icon_path: "ui/elements/water.png",
        tint: Color::srgb(0.10, 0.31, 0.58),
        coord: IVec2::new(-1, 0),
    },
    ElementVisualSpec {
        name: "Space",
        icon_path: "ui/elements/space.png",
        tint: Color::srgb(0.23, 0.18, 0.42),
        coord: IVec2::new(0, -2),
    },
    ElementVisualSpec {
        name: "Lightning",
        icon_path: "ui/elements/lightning.png",
        tint: Color::srgb(0.55, 0.43, 0.08),
        coord: IVec2::new(1, -2),
    },
    ElementVisualSpec {
        name: "Destruction",
        icon_path: "ui/elements/destruction.png",
        tint: Color::srgb(0.49, 0.10, 0.16),
        coord: IVec2::new(2, -2),
    },
    ElementVisualSpec {
        name: "Volcano",
        icon_path: "ui/elements/volcano.png",
        tint: Color::srgb(0.50, 0.13, 0.07),
        coord: IVec2::new(2, -1),
    },
    ElementVisualSpec {
        name: "Artifice",
        icon_path: "ui/elements/artifice.png",
        tint: Color::srgb(0.49, 0.29, 0.12),
        coord: IVec2::new(2, 0),
    },
    ElementVisualSpec {
        name: "Crystal",
        icon_path: "ui/elements/crystal.png",
        tint: Color::srgb(0.16, 0.45, 0.47),
        coord: IVec2::new(1, 1),
    },
    ElementVisualSpec {
        name: "Necromancy",
        icon_path: "ui/elements/necromancy.png",
        tint: Color::srgb(0.28, 0.36, 0.17),
        coord: IVec2::new(0, 2),
    },
    ElementVisualSpec {
        name: "Transmutation",
        icon_path: "ui/elements/transmutation.png",
        tint: Color::srgb(0.32, 0.47, 0.13),
        coord: IVec2::new(-1, 2),
    },
    ElementVisualSpec {
        name: "Wild",
        icon_path: "ui/elements/wild.png",
        tint: Color::srgb(0.11, 0.41, 0.27),
        coord: IVec2::new(-2, 2),
    },
    ElementVisualSpec {
        name: "Divination",
        icon_path: "ui/elements/divination.png",
        tint: Color::srgb(0.12, 0.43, 0.39),
        coord: IVec2::new(-2, 1),
    },
    ElementVisualSpec {
        name: "Storm",
        icon_path: "ui/elements/storm.png",
        tint: Color::srgb(0.16, 0.35, 0.48),
        coord: IVec2::new(-2, 0),
    },
    ElementVisualSpec {
        name: "Illusion",
        icon_path: "ui/elements/illusion.png",
        tint: Color::srgb(0.34, 0.27, 0.56),
        coord: IVec2::new(-1, -1),
    },
];

/// Returns the authored tint for one canonical element name.
///
/// This lookup deliberately shares [`ELEMENT_VISUAL_SPECS`] with the loaded icon
/// catalog. Pure presentation adapters can therefore resolve the same color without
/// acquiring an `AssetServer` merely to construct image handles.
pub(crate) fn authored_element_tint(name: &str) -> Option<Color> {
    ELEMENT_VISUAL_SPECS
        .iter()
        .find(|spec| spec.name == name)
        .map(|spec| spec.tint)
}

pub(crate) fn resolve_catalog_element(
    name: &str,
    elements: &ElementCatalog,
) -> Option<ResolvedElementVisual> {
    let id = elements.id(name)?;
    if elements.is_basic(id) {
        return Some(ResolvedElementVisual {
            id,
            classification: ElementClassification::Basic,
            formula: "basic element".to_owned(),
        });
    }
    let recipe = elements.recipe(id)?;
    let classification = match recipe.len() {
        2 => ElementClassification::Pair,
        3 => ElementClassification::Triple,
        inputs => ElementClassification::HigherOrder(inputs),
    };
    let formula = recipe
        .iter()
        .filter_map(|(input, mana)| {
            elements.name(*input).map(|input_name| {
                if *mana == 1 {
                    input_name.to_owned()
                } else {
                    format!("{mana} {input_name}")
                }
            })
        })
        .collect::<Vec<_>>()
        .join(" + ");
    Some(ResolvedElementVisual {
        id,
        classification,
        formula,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    use super::*;

    fn production_elements() -> hex_assets::ElementFile {
        ron::from_str(include_str!("../../../assets/config/elements.ron"))
            .expect("production elements must parse")
    }

    #[test]
    fn visual_specs_exactly_cover_the_approved_radius_two_grid() {
        let names = ELEMENT_VISUAL_SPECS
            .iter()
            .map(|spec| spec.name)
            .collect::<BTreeSet<_>>();
        let expected = [
            "Air",
            "Artifice",
            "Crystal",
            "Destruction",
            "Divination",
            "Earth",
            "Fire",
            "Illusion",
            "Life",
            "Lightning",
            "Metal",
            "Necromancy",
            "Space",
            "Storm",
            "Transmutation",
            "Volcano",
            "Water",
            "Wild",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        assert_eq!(names, expected);

        let coords = ELEMENT_VISUAL_SPECS
            .iter()
            .map(|spec| (spec.name, (spec.coord.x, spec.coord.y)))
            .collect::<BTreeMap<_, _>>();
        let expected_coords = BTreeMap::from([
            ("Air", (0, -1)),
            ("Fire", (1, -1)),
            ("Metal", (1, 0)),
            ("Earth", (0, 1)),
            ("Life", (-1, 1)),
            ("Water", (-1, 0)),
            ("Space", (0, -2)),
            ("Lightning", (1, -2)),
            ("Destruction", (2, -2)),
            ("Volcano", (2, -1)),
            ("Artifice", (2, 0)),
            ("Crystal", (1, 1)),
            ("Necromancy", (0, 2)),
            ("Transmutation", (-1, 2)),
            ("Wild", (-2, 2)),
            ("Divination", (-2, 1)),
            ("Storm", (-2, 0)),
            ("Illusion", (-1, -1)),
        ]);
        assert_eq!(coords, expected_coords);
        let unique_coords = coords.values().copied().collect::<BTreeSet<_>>();
        assert_eq!(unique_coords.len(), 18);
        assert!(!unique_coords.contains(&(0, 0)));
        assert!(unique_coords.iter().all(|(q, r)| {
            let cube_y = -q - r;
            q.abs().max(r.abs()).max(cube_y.abs()) <= 2
        }));
        assert!(ELEMENT_VISUAL_SPECS.iter().all(|spec| {
            spec.icon_path == format!("ui/elements/{}.png", spec.name.to_ascii_lowercase())
        }));

        let tints = ELEMENT_VISUAL_SPECS
            .iter()
            .map(|spec| {
                let tint = spec.tint.to_srgba();
                (
                    tint.red.to_bits(),
                    tint.green.to_bits(),
                    tint.blue.to_bits(),
                    tint.alpha.to_bits(),
                )
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            tints.len(),
            18,
            "canonical schools need distinct authored tints"
        );
    }

    #[test]
    fn every_visual_path_has_one_vector_master_and_runtime_export() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let expected = ELEMENT_VISUAL_SPECS
            .iter()
            .map(|spec| spec.name.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        for (directory, extension) in [
            (repository.join("brand/elements"), "svg"),
            (repository.join("assets/ui/elements"), "png"),
        ] {
            let found = std::fs::read_dir(&directory)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
                .map(|entry| entry.expect("element asset directory entry").path())
                .filter(|path| path.extension().is_some_and(|found| found == extension))
                .map(|path| {
                    path.file_stem()
                        .and_then(|stem| stem.to_str())
                        .expect("element asset names are UTF-8")
                        .to_owned()
                })
                .collect::<BTreeSet<_>>();
            assert_eq!(
                found,
                expected,
                "{} must exactly cover the visual catalog",
                directory.display()
            );
        }
    }

    #[test]
    fn formula_and_classification_follow_the_live_catalog() {
        let file = production_elements();
        let catalog = ElementCatalog::from_file(&file);
        let classes = ELEMENT_VISUAL_SPECS
            .iter()
            .filter_map(|spec| resolve_catalog_element(spec.name, &catalog))
            .map(|resolved| resolved.classification)
            .collect::<Vec<_>>();
        assert_eq!(
            classes
                .iter()
                .filter(|class| **class == ElementClassification::Basic)
                .count(),
            6
        );
        assert_eq!(
            classes
                .iter()
                .filter(|class| **class == ElementClassification::Pair)
                .count(),
            6
        );
        assert_eq!(
            classes
                .iter()
                .filter(|class| **class == ElementClassification::Triple)
                .count(),
            6
        );

        let mut reordered = file;
        reordered
            .fusions
            .get_mut("Lightning")
            .expect("Lightning recipe must exist")
            .reverse();
        let reordered = ElementCatalog::from_file(&reordered);
        let lightning = resolve_catalog_element("Lightning", &reordered)
            .expect("Lightning visual must resolve against live content");
        assert_eq!(lightning.formula, "Fire + Air");
    }

    #[test]
    fn shared_element_color_resolves_every_authored_visual_tint() {
        let catalog = ElementCatalog::from_file(&production_elements());
        for spec in ELEMENT_VISUAL_SPECS {
            assert_eq!(
                crate::theme::element_color(catalog.id(spec.name), &catalog),
                spec.tint,
                "{} must use its authored visual tint on every presentation surface",
                spec.name
            );
        }
    }
}
