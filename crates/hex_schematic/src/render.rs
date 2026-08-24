//! Pure, deterministic SVG and HTML projections of validated schematic facts.
//!
//! These renderers deliberately consume a narrow presentation model. The generator
//! and validator remain the only logical authorities; a color, glyph, or pixel in
//! these documents never establishes plan validity.

use std::fmt::{self, Write as _};

use hex_schematic::{
    canonical_coordinates, CellFacts, CellPlan, FeatureKind, LandformKind, LayerProvenance,
    NetworkKind, SchematicPlanV1, SchematicTemplateV1, StableId, SurfaceKind, VegetationDensity,
    SCHEMATIC_RADIUS,
};

const HEX_SIZE: f64 = 31.0;
const MAP_MARGIN: f64 = 78.0;
const SQRT_THREE: f64 = 1.732_050_807_568_877_2;
const COMPOSITE_WIDTH: f64 = 1_260.0;
const COMPOSITE_HEIGHT: f64 = 1_180.0;
const DIAGNOSTIC_WIDTH: f64 = 1_600.0;
const DIAGNOSTIC_HEIGHT: f64 = 1_180.0;
const CONTACT_COLUMNS: usize = 4;
const CONTACT_ROWS: usize = 3;
const GALLERY_ENTRY_COUNT: usize = CONTACT_COLUMNS * CONTACT_ROWS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CellStyle {
    Unassigned,
    Sea,
    Lake,
    River,
    Island,
    Beach,
    Shore,
    Valley,
    Plateau,
    Hill,
    Mountain,
    Massif,
    SharpPeak,
}

impl CellStyle {
    #[cfg(test)]
    const ALL: [Self; 13] = [
        Self::Unassigned,
        Self::Sea,
        Self::Lake,
        Self::River,
        Self::Island,
        Self::Beach,
        Self::Shore,
        Self::Valley,
        Self::Plateau,
        Self::Hill,
        Self::Mountain,
        Self::Massif,
        Self::SharpPeak,
    ];

    const fn class(self) -> &'static str {
        match self {
            Self::Unassigned => "unassigned",
            Self::Sea => "sea",
            Self::Lake => "lake",
            Self::River => "river",
            Self::Island => "island",
            Self::Beach => "beach",
            Self::Shore => "shore",
            Self::Valley => "valley",
            Self::Plateau => "plateau",
            Self::Hill => "hill",
            Self::Mountain => "mountain",
            Self::Massif => "massif",
            Self::SharpPeak => "sharp-peak",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Unassigned => "Unassigned",
            Self::Sea => "Sea",
            Self::Lake => "Lake",
            Self::River => "River",
            Self::Island => "Island",
            Self::Beach => "Beach",
            Self::Shore => "Shore",
            Self::Valley => "Valley",
            Self::Plateau => "Plateau",
            Self::Hill => "Hill",
            Self::Mountain => "Mountain",
            Self::Massif => "Massif",
            Self::SharpPeak => "Sharp peak",
        }
    }

    const fn abbreviation(self) -> &'static str {
        match self {
            Self::Unassigned => "·",
            Self::Sea => "SEA",
            Self::Lake => "LAKE",
            Self::River => "RIV",
            Self::Island => "ISLE",
            Self::Beach => "BCH",
            Self::Shore => "SHR",
            Self::Valley => "VLY",
            Self::Plateau => "PLT",
            Self::Hill => "HIL",
            Self::Mountain => "MTN",
            Self::Massif => "MSF",
            Self::SharpPeak => "PEAK",
        }
    }

    const fn pattern(self) -> &'static str {
        match self {
            Self::Unassigned => "grid-dots",
            Self::Sea | Self::Lake | Self::River => "water-waves",
            Self::Island => "island-lines",
            Self::Beach => "beach-lines",
            Self::Shore => "shore-lines",
            Self::Valley => "valley-lines",
            Self::Plateau => "plateau-lines",
            Self::Hill => "hill-lines",
            Self::Mountain => "mountain-lines",
            Self::Massif => "massif-lines",
            Self::SharpPeak => "peak-lines",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CellAccent {
    Woodland,
    FrozenWoodland,
    CrystalAscent,
}

impl CellAccent {
    const fn label(self) -> &'static str {
        match self {
            Self::Woodland => "Woodland",
            Self::FrozenWoodland => "Frozen woodland",
            Self::CrystalAscent => "Crystal Ascent",
        }
    }

    const fn abbreviation(self) -> &'static str {
        match self {
            Self::Woodland => "WOOD",
            Self::FrozenWoodland => "FRZ",
            Self::CrystalAscent => "ASC",
        }
    }

    const fn pattern(self) -> &'static str {
        match self {
            Self::Woodland => "woodland-dots",
            Self::FrozenWoodland => "frozen-crosses",
            Self::CrystalAscent => "crystal-lines",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AuthorshipStyle {
    TracingGrid,
    Locked,
    Bounded,
    Seeded,
    ReferenceFallback,
}

impl AuthorshipStyle {
    const ALL: [Self; 5] = [
        Self::TracingGrid,
        Self::Locked,
        Self::Bounded,
        Self::Seeded,
        Self::ReferenceFallback,
    ];

    const fn class(self) -> &'static str {
        match self {
            Self::TracingGrid => "authorship-grid",
            Self::Locked => "authorship-locked",
            Self::Bounded => "authorship-bounded",
            Self::Seeded => "authorship-seeded",
            Self::ReferenceFallback => "authorship-fallback",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::TracingGrid => "Neutral tracing cell",
            Self::Locked => "Locked traced fact",
            Self::Bounded => "Bounded generated fact",
            Self::Seeded => "Seeded generated fact",
            Self::ReferenceFallback => "Reference fallback fact",
        }
    }

    const fn abbreviation(self) -> &'static str {
        match self {
            Self::TracingGrid => "N",
            Self::Locked => "L",
            Self::Bounded => "B",
            Self::Seeded => "S",
            Self::ReferenceFallback => "F",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum TraceStyle {
    River,
    Tunnel,
}

impl TraceStyle {
    const fn class(self) -> &'static str {
        match self {
            Self::River => "trace-river",
            Self::Tunnel => "trace-tunnel",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::River => "River / waterfall route",
            Self::Tunnel => "Tunnel route",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderCell {
    pub(crate) q: i32,
    pub(crate) r: i32,
    pub(crate) ordinal: usize,
    pub(crate) style: CellStyle,
    pub(crate) accents: Vec<CellAccent>,
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) provenance_signature: String,
    pub(crate) authorship: Vec<AuthorshipStyle>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderTrace {
    pub(crate) label: String,
    pub(crate) style: TraceStyle,
    pub(crate) cells: Vec<(i32, i32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiagnosticSeverity {
    Information,
    Warning,
}

impl DiagnosticSeverity {
    const fn label(self) -> &'static str {
        match self {
            Self::Information => "information",
            Self::Warning => "warning",
        }
    }

    const fn class(self) -> &'static str {
        match self {
            Self::Information => "diagnostic-information",
            Self::Warning => "diagnostic-warning",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderDiagnostic {
    pub(crate) severity: DiagnosticSeverity,
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) cell: Option<(i32, i32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderPlan {
    pub(crate) title: String,
    pub(crate) subtitle: String,
    pub(crate) seed: Option<u64>,
    pub(crate) fingerprint: Option<String>,
    pub(crate) cells: Vec<RenderCell>,
    pub(crate) traces: Vec<RenderTrace>,
    pub(crate) metrics: Vec<(String, String)>,
    pub(crate) diagnostics: Vec<RenderDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GalleryEntry {
    pub(crate) heading: String,
    pub(crate) seed: Option<u64>,
    pub(crate) fingerprint: String,
    pub(crate) summary: String,
    pub(crate) composite_href: String,
    pub(crate) diagnostic_href: String,
    pub(crate) plan_href: String,
    pub(crate) metrics_href: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderError {
    detail: String,
}

impl RenderError {
    fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for RenderError {}

pub(crate) fn grid_projection() -> RenderPlan {
    let cells = canonical_coordinates()
        .into_iter()
        .enumerate()
        .map(|(ordinal, coord)| RenderCell {
            q: coord.q(),
            r: coord.r(),
            ordinal,
            style: CellStyle::Unassigned,
            accents: Vec::new(),
            label: format!("canonical cell {ordinal}"),
            detail: format!(
                "canonical coordinate q={}, r={}, s={}",
                coord.q(),
                coord.r(),
                coord.s()
            ),
            provenance_signature: "grid:N".to_owned(),
            authorship: vec![AuthorshipStyle::TracingGrid],
        })
        .collect::<Vec<_>>();
    RenderPlan {
        title: "Canonical radius-eight schematic grid".to_owned(),
        subtitle: "Coordinate and ordinal projection; no biome facts are assigned".to_owned(),
        seed: None,
        fingerprint: None,
        cells,
        traces: Vec::new(),
        metrics: vec![
            ("radius".to_owned(), SCHEMATIC_RADIUS.to_string()),
            (
                "cell-count".to_owned(),
                canonical_coordinates().len().to_string(),
            ),
        ],
        diagnostics: vec![RenderDiagnostic {
            severity: DiagnosticSeverity::Information,
            code: "GRID-PROJECTION".to_owned(),
            message: "canonical coordinates only; no logical biome plan".to_owned(),
            cell: None,
        }],
    }
}

pub(crate) fn plan_projection(
    template: &SchematicTemplateV1,
    plan: &SchematicPlanV1,
    metrics: Vec<(String, String)>,
) -> Result<RenderPlan, RenderError> {
    if plan.template_id != template.id || plan.template_revision != template.revision {
        return Err(RenderError::new(
            "plan template identity or revision does not match the rendering template",
        ));
    }
    let cells = plan.cells.iter().map(render_cell).collect::<Vec<_>>();
    let traces = plan
        .networks
        .iter()
        .flat_map(|network| {
            network.edges.iter().map(move |edge| RenderTrace {
                label: semantic_trace_label(network.kind, edge.id.as_str()).to_owned(),
                style: match network.kind {
                    NetworkKind::Hydrology => TraceStyle::River,
                    NetworkKind::Tunnel => TraceStyle::Tunnel,
                },
                cells: edge
                    .path
                    .iter()
                    .map(|coord| (coord.q(), coord.r()))
                    .collect(),
            })
        })
        .collect();
    let mut diagnostics = vec![RenderDiagnostic {
        severity: DiagnosticSeverity::Information,
        code: "TYPED-PROJECTION".to_owned(),
        message: "render-only; validator is authoritative".to_owned(),
        cell: None,
    }];
    let (title, selection, provenance_diagnostic) = match (
        plan.provenance.is_reference_artifact,
        plan.provenance.used_reference_fallback,
        plan.provenance.selected_candidate,
    ) {
        (true, false, None) => (
            "Grand V3 schematic — canonical reference artifact".to_owned(),
            "direct reference artifact; no candidates evaluated".to_owned(),
            RenderDiagnostic {
                severity: DiagnosticSeverity::Information,
                code: "REFERENCE-ARTIFACT".to_owned(),
                message: "direct artifact; not a generator fallback".to_owned(),
                cell: None,
            },
        ),
        (false, true, None) => (
            format!(
                "Grand V3 schematic — seed {} exhausted-candidate fallback",
                plan.provenance.world_seed
            ),
            "reference fallback after every candidate failed".to_owned(),
            RenderDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "REFERENCE-FALLBACK".to_owned(),
                message: "all candidates failed; validated reference selected".to_owned(),
                cell: None,
            },
        ),
        (false, false, Some(candidate)) => (
            format!("Grand V3 schematic — seed {}", plan.provenance.world_seed),
            format!("selected candidate {candidate}"),
            RenderDiagnostic {
                severity: DiagnosticSeverity::Information,
                code: "SELECTED-CANDIDATE".to_owned(),
                message: format!("normal generation selected candidate {candidate}"),
                cell: None,
            },
        ),
        _ => {
            return Err(RenderError::new(
                "plan has contradictory candidate, fallback, or reference-artifact provenance",
            ));
        }
    };
    diagnostics.push(provenance_diagnostic);
    Ok(RenderPlan {
        title,
        subtitle: format!(
            "template {} revision {} · {} · {}/{} hard-valid candidates",
            template.id,
            template.revision,
            selection,
            plan.provenance.hard_valid_candidates,
            plan.provenance.candidates_evaluated,
        ),
        seed: Some(plan.provenance.world_seed),
        fingerprint: Some(format!("{:016x}", plan.semantic_fingerprint)),
        cells,
        traces,
        metrics,
        diagnostics,
    })
}

fn semantic_trace_label(kind: NetworkKind, edge_id: &str) -> &'static str {
    match (kind, edge_id) {
        (NetworkKind::Hydrology, "edge/hydrology-lake-to-falls") => "lake → falls",
        (NetworkKind::Hydrology, "edge/hydrology-falls-to-valley") => "falls → valley",
        (NetworkKind::Hydrology, "edge/hydrology-valley-to-sea") => "valley → sea",
        (NetworkKind::Hydrology, _) => "river route",
        (NetworkKind::Tunnel, "edge/tunnel-complete") => "ascent → hill",
        (NetworkKind::Tunnel, _) => "tunnel route",
    }
}

fn render_cell(cell: &CellPlan) -> RenderCell {
    let mut accents = cell_accents(&cell.facts);
    accents.sort_unstable();
    accents.dedup();
    let mut authorship = cell_authorship(cell);
    authorship.sort_unstable();
    authorship.dedup();
    RenderCell {
        q: cell.coord.q(),
        r: cell.coord.r(),
        ordinal: usize::from(cell.id.get()),
        style: cell_style(&cell.facts),
        accents,
        label: if cell.facts.overlays.is_empty() {
            format!("{:?}; {:?}", cell.facts.climate, cell.facts.access)
        } else {
            format!("{:?}", cell.facts.overlays)
        },
        detail: cell_detail(cell),
        provenance_signature: provenance_signature(cell),
        authorship,
    }
}

fn cell_style(facts: &CellFacts) -> CellStyle {
    if facts.overlays.contains(&FeatureKind::LakeIsland)
        || facts.overlays.contains(&FeatureKind::SeaIsland)
    {
        return CellStyle::Island;
    }
    if facts.surface == SurfaceKind::OpenWater {
        if facts.overlays.contains(&FeatureKind::River)
            || facts.overlays.contains(&FeatureKind::Waterfall)
        {
            return CellStyle::River;
        }
        if facts.overlays.contains(&FeatureKind::ValleyLake)
            || facts.overlays.contains(&FeatureKind::MountainLake)
        {
            return CellStyle::Lake;
        }
        return CellStyle::Sea;
    }
    match facts.landform {
        LandformKind::None => CellStyle::Unassigned,
        LandformKind::Island => CellStyle::Island,
        LandformKind::Beach => CellStyle::Beach,
        LandformKind::Shore => CellStyle::Shore,
        LandformKind::Valley => CellStyle::Valley,
        LandformKind::Plateau => CellStyle::Plateau,
        LandformKind::Hill => CellStyle::Hill,
        LandformKind::Mountain => CellStyle::Mountain,
        LandformKind::Massif => CellStyle::Massif,
        LandformKind::SharpPeak => CellStyle::SharpPeak,
    }
}

fn cell_accents(facts: &CellFacts) -> Vec<CellAccent> {
    let mut accents = Vec::new();
    if matches!(
        facts.vegetation,
        VegetationDensity::Light | VegetationDensity::Moderate | VegetationDensity::Dense
    ) {
        accents.push(CellAccent::Woodland);
    }
    if facts.overlays.contains(&FeatureKind::FrozenWoods) {
        accents.push(CellAccent::FrozenWoodland);
    }
    if facts.overlays.contains(&FeatureKind::CrystalAscent) {
        accents.push(CellAccent::CrystalAscent);
    }
    accents
}

fn cell_authorship(cell: &CellPlan) -> Vec<AuthorshipStyle> {
    let mut sources = vec![
        &cell.provenance.surface,
        &cell.provenance.landform,
        &cell.provenance.climate,
        &cell.provenance.vegetation,
        &cell.provenance.access,
    ];
    sources.extend(
        cell.provenance
            .overlays
            .iter()
            .map(|overlay| &overlay.source),
    );
    sources.into_iter().map(authorship_style).collect()
}

fn authorship_style(source: &LayerProvenance) -> AuthorshipStyle {
    match source {
        LayerProvenance::Locked { .. } => AuthorshipStyle::Locked,
        LayerProvenance::Bounded { .. } => AuthorshipStyle::Bounded,
        LayerProvenance::Seeded { .. } => AuthorshipStyle::Seeded,
        LayerProvenance::ReferenceFallback { source } => fallback_source_authorship(source),
    }
}

fn fallback_source_authorship(source: &StableId) -> AuthorshipStyle {
    if source.as_str().starts_with("claim/") {
        AuthorshipStyle::Locked
    } else if source.as_str().starts_with("rule/") {
        AuthorshipStyle::Bounded
    } else if source.as_str().starts_with("stream/") {
        AuthorshipStyle::Seeded
    } else {
        AuthorshipStyle::ReferenceFallback
    }
}

fn cell_detail(cell: &CellPlan) -> String {
    format!(
        "surface={:?}; landform={:?}; climate={:?}; vegetation={:?}; access={:?}; overlays={:?}; provenance: surface={}, landform={}, climate={}, vegetation={}, access={}, overlays={:?}",
        cell.facts.surface,
        cell.facts.landform,
        cell.facts.climate,
        cell.facts.vegetation,
        cell.facts.access,
        cell.facts.overlays,
        provenance_label(&cell.provenance.surface),
        provenance_label(&cell.provenance.landform),
        provenance_label(&cell.provenance.climate),
        provenance_label(&cell.provenance.vegetation),
        provenance_label(&cell.provenance.access),
        cell.provenance.overlays,
    )
}

fn provenance_label(source: &LayerProvenance) -> String {
    match source {
        LayerProvenance::Locked { claim } => format!("Locked({claim})"),
        LayerProvenance::Bounded { rule } => format!("Bounded({rule})"),
        LayerProvenance::Seeded { stream } => format!("Seeded({stream})"),
        LayerProvenance::ReferenceFallback { source } => {
            format!(
                "ReferenceFallback({source}; underlying={})",
                fallback_source_authorship(source).label()
            )
        }
    }
}

fn provenance_signature(cell: &CellPlan) -> String {
    let mut signature = format!(
        "s{} l{} c{} v{} a{}",
        authorship_style(&cell.provenance.surface).abbreviation(),
        authorship_style(&cell.provenance.landform).abbreviation(),
        authorship_style(&cell.provenance.climate).abbreviation(),
        authorship_style(&cell.provenance.vegetation).abbreviation(),
        authorship_style(&cell.provenance.access).abbreviation(),
    );
    for overlay in &cell.provenance.overlays {
        signature.push_str(" o");
        signature.push_str(feature_abbreviation(overlay.feature));
        signature.push_str(authorship_style(&overlay.source).abbreviation());
    }
    signature
}

const fn feature_abbreviation(feature: FeatureKind) -> &'static str {
    match feature {
        FeatureKind::Coastline => "CO",
        FeatureKind::River => "RV",
        FeatureKind::Waterfall => "WF",
        FeatureKind::ValleyLake => "VL",
        FeatureKind::MountainLake => "ML",
        FeatureKind::LakeIsland => "LI",
        FeatureKind::FrozenWoods => "FW",
        FeatureKind::PeakRing => "PR",
        FeatureKind::CrystalAscent => "CA",
        FeatureKind::Tunnel => "TU",
        FeatureKind::SeaIsland => "SI",
    }
}

pub(crate) fn composite_svg(plan: &RenderPlan) -> Result<String, RenderError> {
    validate_plan_projection(plan)?;
    let canvas = HexCanvas::from_cells(&plan.cells, COMPOSITE_WIDTH, COMPOSITE_HEIGHT, 0.0)?;
    let mut svg = svg_header(
        "schematic-composite-title",
        "schematic-composite-description",
        &plan.title,
        &format!(
            "{} Diagnostic projection of {} canonical cells; typed RON and validation remain authoritative.",
            plan.subtitle,
            plan.cells.len()
        ),
        COMPOSITE_WIDTH,
        COMPOSITE_HEIGHT,
    )?;
    write_common_definitions(&mut svg);
    write_header(&mut svg, plan, COMPOSITE_WIDTH)?;
    let tracing_grid = plan.seed.is_none();
    write_map_group(&mut svg, plan, &canvas, !tracing_grid, tracing_grid)?;
    write_metric_band(
        &mut svg,
        plan,
        34.0,
        COMPOSITE_HEIGHT - 168.0,
        COMPOSITE_WIDTH - 68.0,
    )?;
    write_legend(
        &mut svg,
        plan,
        34.0,
        COMPOSITE_HEIGHT - 116.0,
        COMPOSITE_WIDTH - 68.0,
    )?;
    svg.push_str("</svg>\n");
    Ok(svg)
}

pub(crate) fn diagnostic_svg(plan: &RenderPlan) -> Result<String, RenderError> {
    validate_plan_projection(plan)?;
    let map_width = 1_160.0;
    let canvas = HexCanvas::from_cells(&plan.cells, map_width, DIAGNOSTIC_HEIGHT, 0.0)?;
    let mut svg = svg_header(
        "schematic-diagnostic-title",
        "schematic-diagnostic-description",
        &format!("{} — diagnostics", plan.title),
        &format!(
            "{} Coordinate-labelled diagnostic projection with {} typed diagnostic entries.",
            plan.subtitle,
            plan.diagnostics.len()
        ),
        DIAGNOSTIC_WIDTH,
        DIAGNOSTIC_HEIGHT,
    )?;
    write_common_definitions(&mut svg);
    write_header(&mut svg, plan, DIAGNOSTIC_WIDTH)?;
    write_map_group(&mut svg, plan, &canvas, true, true)?;
    write_diagnostic_panel(&mut svg, plan, 1_170.0, 90.0, 400.0, 1_020.0)?;
    svg.push_str("</svg>\n");
    Ok(svg)
}

pub(crate) fn contact_sheet_svg(
    entries: &[GalleryEntry],
    plans: &[RenderPlan],
) -> Result<String, RenderError> {
    if entries.len() != GALLERY_ENTRY_COUNT {
        return Err(RenderError::new(format!(
            "contact sheet requires exactly {GALLERY_ENTRY_COUNT} entries, received {}",
            entries.len()
        )));
    }
    if entries.iter().any(|entry| entry.seed.is_none()) {
        return Err(RenderError::new(
            "contact sheet entries must all be seeded candidate plans",
        ));
    }
    if plans.len() != entries.len() {
        return Err(RenderError::new(format!(
            "contact sheet received {} entries but {} map projections",
            entries.len(),
            plans.len(),
        )));
    }
    for (entry, plan) in entries.iter().zip(plans) {
        validate_plan_projection(plan)?;
        if entry.seed != plan.seed {
            return Err(RenderError::new(
                "contact sheet entry seed does not match its map projection",
            ));
        }
    }
    let panel_width = 360.0;
    let panel_height = 300.0;
    let gap = 20.0;
    let width = 40.0 + (panel_width + gap) * 4.0;
    let height = 118.0 + (panel_height + gap) * 3.0;
    let mut svg = svg_header(
        "gallery-contact-title",
        "gallery-contact-description",
        "Grand V3 schematic — twelve-seed contact sheet",
        "Complete self-contained four-by-three projection of twelve validated seeds; typed plans remain authoritative.",
        width,
        height,
    )?;
    write_common_definitions(&mut svg);
    writeln!(
        svg,
        "<style>\n.sheet-bg{{fill:#111827}} .panel{{fill:#f8fafc;stroke:#334155;stroke-width:2}} .seed{{font:700 17px system-ui,sans-serif;fill:#0f172a}} .fingerprint{{font:12px ui-monospace,monospace;fill:#334155}} .summary{{font:9px ui-monospace,monospace;fill:#334155}} .mini-cell{{stroke:#0f172a;stroke-width:.65}} .mini-pattern{{opacity:.68;pointer-events:none}} .mini-accent{{fill-opacity:.5;pointer-events:none}} .mini-trace{{fill:none;stroke-linecap:round;stroke-linejoin:round}} .mini-river{{stroke:#0369a1;stroke-width:1.8}} .mini-tunnel{{stroke:#dc2626;stroke-width:1.8;stroke-dasharray:5 3}}\n</style>"
    )
    .map_err(render_write_error)?;
    writeln!(
        svg,
        "<rect class=\"sheet-bg\" x=\"0\" y=\"0\" width=\"{width}\" height=\"{height}\"/>"
    )
    .map_err(render_write_error)?;
    for (index, (entry, plan)) in entries.iter().zip(plans).enumerate() {
        let column = index % CONTACT_COLUMNS;
        let row = index / CONTACT_COLUMNS;
        let column = f64::from(u32::try_from(column).unwrap_or(u32::MAX));
        let row = f64::from(u32::try_from(row).unwrap_or(u32::MAX));
        let x = 30.0 + column * (panel_width + gap);
        let y = 88.0 + row * (panel_height + gap);
        let fingerprint = xml_escape(&entry.fingerprint);
        let seed = entry.seed.ok_or_else(|| {
            RenderError::new("contact sheet entries must all be seeded candidate plans")
        })?;
        writeln!(
            svg,
            "<g role=\"group\" aria-label=\"Seed {}; fingerprint {}; {}\">\n<rect class=\"panel\" x=\"{x:.1}\" y=\"{y:.1}\" width=\"{panel_width:.1}\" height=\"{panel_height:.1}\" rx=\"8\"/>\n<text class=\"seed\" x=\"{:.1}\" y=\"{:.1}\">Seed {}</text>\n<text class=\"fingerprint\" x=\"{:.1}\" y=\"{:.1}\">{fingerprint}</text>\n<text class=\"summary\" x=\"{:.1}\" y=\"{:.1}\">{}</text>",
            seed,
            fingerprint,
            xml_escape(&entry.summary),
            x + 12.0,
            y + 24.0,
            seed,
            x + 12.0,
            y + 43.0,
            x + 12.0,
            y + 61.0,
            xml_escape(&entry.summary),
        )
        .map_err(render_write_error)?;
        write_contact_map(
            &mut svg,
            plan,
            x + 8.0,
            y + 69.0,
            panel_width - 16.0,
            panel_height - 77.0,
        )?;
        svg.push_str("</g>\n");
    }
    svg.push_str("</svg>\n");
    Ok(svg)
}

fn write_contact_map(
    svg: &mut String,
    plan: &RenderPlan,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), RenderError> {
    let canvas = HexCanvas::from_region(&plan.cells, x, y, width, height)?;
    svg.push_str("<g class=\"contact-map\" aria-hidden=\"true\">\n");
    let mut cells = plan.cells.iter().collect::<Vec<_>>();
    cells.sort_unstable_by_key(|cell| (cell.q, cell.r, cell.ordinal));
    for cell in cells {
        let points = canvas.polygon(cell.q, cell.r);
        writeln!(
            svg,
            "<polygon class=\"mini-cell base-{}\" points=\"{points}\"/><polygon class=\"mini-pattern\" fill=\"url(#{})\" points=\"{points}\"/>",
            cell.style.class(),
            cell.style.pattern(),
        )
        .map_err(render_write_error)?;
        for accent in &cell.accents {
            writeln!(
                svg,
                "<polygon class=\"mini-accent\" fill=\"url(#{})\" points=\"{points}\"/>",
                accent.pattern(),
            )
            .map_err(render_write_error)?;
        }
    }
    for trace in &plan.traces {
        let points = trace
            .cells
            .iter()
            .map(|(q, r)| {
                let (center_x, center_y) = canvas.center(*q, *r);
                format!("{center_x:.2},{center_y:.2}")
            })
            .collect::<Vec<_>>()
            .join(" ");
        let class = match trace.style {
            TraceStyle::River => "mini-river",
            TraceStyle::Tunnel => "mini-tunnel",
        };
        writeln!(
            svg,
            "<polyline class=\"mini-trace {class}\" points=\"{points}\"/>"
        )
        .map_err(render_write_error)?;
    }
    svg.push_str("</g>\n");
    Ok(())
}

pub(crate) fn complete_gallery_html(
    entries: &[GalleryEntry],
    reference: &GalleryEntry,
    contact_sheet_href: &str,
) -> Result<String, RenderError> {
    if entries.len() != GALLERY_ENTRY_COUNT {
        return Err(RenderError::new(format!(
            "gallery requires exactly {GALLERY_ENTRY_COUNT} entries, received {}",
            entries.len()
        )));
    }
    if reference.seed.is_some() || entries.iter().any(|entry| entry.seed.is_none()) {
        return Err(RenderError::new(
            "gallery requires one unseeded reference entry and twelve seeded candidate entries",
        ));
    }
    let mut html = String::from(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<title>Grand V3 schematic gallery</title>\n<style>body{margin:0;background:#111827;color:#f8fafc;font:16px/1.45 system-ui,sans-serif}main{max-width:1500px;margin:auto;padding:32px}h1{margin-bottom:4px}.boundary{max-width:78ch;color:#cbd5e1}.contact{display:block;width:100%;background:#0f172a;border:2px solid #64748b;margin:24px 0}.reference{background:#312e81;border:3px solid #c4b5fd;border-radius:10px;padding:18px;margin:24px 0}.reference img{display:block;width:100%;max-width:1000px;background:#f8fafc}.cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(330px,1fr));gap:20px}.card{background:#1e293b;border:2px solid #64748b;border-radius:10px;padding:14px}.card img{display:block;width:100%;background:#f8fafc}.meta{font-family:ui-monospace,monospace;overflow-wrap:anywhere}.links{display:flex;flex-wrap:wrap;gap:12px}.links a{color:#7dd3fc}</style>\n</head>\n<body><main>\n<h1>Grand V3 schematic gallery</h1>\n<p><strong>Capture status: COMPLETE. Human review status: UNREVIEWED.</strong></p>\n<p class=\"boundary\">This pack contains a separately marked, coordinate-labelled canonical reference artifact and twelve validated seeded plans. The artifact was requested directly and is not evidence of candidate exhaustion. Colors, patterns, SVG geometry, and visual inspection do not establish any logical claim; inspect the canonical RON, metrics, and validator result for authority.</p>\n",
    );
    writeln!(
        html,
        "<section class=\"reference\" aria-label=\"Canonical reference artifact\"><h2>{}</h2><p class=\"meta\">Fingerprint: {}</p><a href=\"{}\"><img src=\"{}\" alt=\"Coordinate-labelled Grand V3 canonical reference artifact\"></a><p>{}</p><p class=\"links\"><a href=\"{}\">reference diagnostics SVG</a><a href=\"{}\">canonical reference plan RON</a><a href=\"{}\">reference metrics RON</a></p></section>",
        html_escape(&reference.heading),
        html_escape(&reference.fingerprint),
        html_escape(&reference.composite_href),
        html_escape(&reference.composite_href),
        html_escape(&reference.summary),
        html_escape(&reference.diagnostic_href),
        html_escape(&reference.plan_href),
        html_escape(&reference.metrics_href),
    )
    .map_err(render_write_error)?;
    writeln!(
        html,
        "<a href=\"{}\"><img class=\"contact\" src=\"{}\" alt=\"Complete four-by-three contact sheet for all twelve generated seeds\"></a>",
        html_escape(contact_sheet_href),
        html_escape(contact_sheet_href),
    )
    .map_err(render_write_error)?;
    html.push_str("<section class=\"cards\" aria-label=\"Generated seed plans\">\n");
    for entry in entries {
        writeln!(
            html,
            "<article class=\"card\"><h2>{}</h2><p class=\"meta\">Fingerprint: {}</p><a href=\"{}\"><img src=\"{}\" alt=\"Labelled composite schematic for {}\"></a><p>{}</p><p class=\"links\"><a href=\"{}\">diagnostics SVG</a><a href=\"{}\">canonical plan RON</a><a href=\"{}\">metrics RON</a></p></article>",
            html_escape(&entry.heading),
            html_escape(&entry.fingerprint),
            html_escape(&entry.composite_href),
            html_escape(&entry.composite_href),
            html_escape(&entry.heading),
            html_escape(&entry.summary),
            html_escape(&entry.diagnostic_href),
            html_escape(&entry.plan_href),
            html_escape(&entry.metrics_href),
        )
        .map_err(render_write_error)?;
    }
    html.push_str("</section>\n</main></body></html>\n");
    Ok(html)
}

fn validate_plan_projection(plan: &RenderPlan) -> Result<(), RenderError> {
    if plan.title.trim().is_empty() || plan.subtitle.trim().is_empty() {
        return Err(RenderError::new(
            "render title and subtitle must both be non-empty",
        ));
    }
    if plan.cells.is_empty() {
        return Err(RenderError::new("cannot render an empty schematic"));
    }
    let mut coordinates = plan
        .cells
        .iter()
        .map(|cell| (cell.q, cell.r))
        .collect::<Vec<_>>();
    coordinates.sort_unstable();
    if coordinates
        .windows(2)
        .any(|pair| pair.first() == pair.get(1))
    {
        return Err(RenderError::new("cannot render duplicate cell coordinates"));
    }
    for cell in &plan.cells {
        if cell.provenance_signature.trim().is_empty() {
            return Err(RenderError::new(format!(
                "cell {} has no visible per-layer provenance signature",
                cell.ordinal
            )));
        }
        if cell.authorship.is_empty() {
            return Err(RenderError::new(format!(
                "cell {} has no visible provenance class",
                cell.ordinal
            )));
        }
        if !strictly_ordered(&cell.authorship) {
            return Err(RenderError::new(format!(
                "cell {} provenance classes are duplicated or unordered",
                cell.ordinal
            )));
        }
        if !strictly_ordered(&cell.accents) {
            return Err(RenderError::new(format!(
                "cell {} accents are duplicated or unordered",
                cell.ordinal
            )));
        }
    }
    for trace in &plan.traces {
        if trace.label.trim().is_empty() || trace.cells.is_empty() {
            return Err(RenderError::new(
                "every rendered trace needs a label and at least one cell",
            ));
        }
    }
    Ok(())
}

fn strictly_ordered<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| {
        pair.first()
            .zip(pair.get(1))
            .is_some_and(|(left, right)| left < right)
    })
}

#[derive(Debug, Clone, Copy)]
struct HexCanvas {
    min_x: f64,
    min_y: f64,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
}

impl HexCanvas {
    fn from_cells(
        cells: &[RenderCell],
        width: f64,
        height: f64,
        right_reserved: f64,
    ) -> Result<Self, RenderError> {
        let available_width = width - right_reserved - 2.0 * MAP_MARGIN;
        let available_height = height - 280.0;
        Self::from_region(cells, MAP_MARGIN, 92.0, available_width, available_height)
    }

    fn from_region(
        cells: &[RenderCell],
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Result<Self, RenderError> {
        let mut positions = cells.iter().map(|cell| raw_center(cell.q, cell.r));
        let Some((first_x, first_y)) = positions.next() else {
            return Err(RenderError::new("cannot fit an empty schematic"));
        };
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (first_x, first_x, first_y, first_y);
        for (x, y) in positions {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        let raw_width = (max_x - min_x) + 2.0 * HEX_SIZE;
        let raw_height = (max_y - min_y) + 2.0 * HEX_SIZE;
        let scale = (width / raw_width).min(height / raw_height);
        if !scale.is_finite() || scale <= 0.0 {
            return Err(RenderError::new("schematic does not fit the SVG canvas"));
        }
        let fitted_width = raw_width * scale;
        let fitted_height = raw_height * scale;
        Ok(Self {
            min_x: min_x - HEX_SIZE,
            min_y: min_y - HEX_SIZE,
            scale,
            offset_x: x + (width - fitted_width) / 2.0,
            offset_y: y + (height - fitted_height) / 2.0,
        })
    }

    fn center(self, q: i32, r: i32) -> (f64, f64) {
        let (raw_x, raw_y) = raw_center(q, r);
        (
            self.offset_x + (raw_x - self.min_x) * self.scale,
            self.offset_y + (raw_y - self.min_y) * self.scale,
        )
    }

    fn polygon(self, q: i32, r: i32) -> String {
        let (center_x, center_y) = self.center(q, r);
        let radius = HEX_SIZE * self.scale;
        [
            (SQRT_THREE / 2.0, -0.5),
            (SQRT_THREE / 2.0, 0.5),
            (0.0, 1.0),
            (-SQRT_THREE / 2.0, 0.5),
            (-SQRT_THREE / 2.0, -0.5),
            (0.0, -1.0),
        ]
        .into_iter()
        .map(|(dx, dy)| {
            format!(
                "{:.2},{:.2}",
                center_x + dx * radius,
                center_y + dy * radius
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
    }
}

fn raw_center(q: i32, r: i32) -> (f64, f64) {
    (
        SQRT_THREE * (f64::from(q) + f64::from(r) / 2.0) * HEX_SIZE,
        1.5 * f64::from(r) * HEX_SIZE,
    )
}

fn svg_header(
    title_id: &str,
    description_id: &str,
    title: &str,
    description: &str,
    width: f64,
    height: f64,
) -> Result<String, RenderError> {
    let mut svg = String::new();
    writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" viewBox=\"0 0 {width:.0} {height:.0}\" role=\"img\" aria-labelledby=\"{title_id} {description_id}\">\n<title id=\"{title_id}\">{}</title>\n<desc id=\"{description_id}\">{}</desc>\n<metadata>Review-only diagnostic projection. Typed RON and the validator are authoritative; pixels are not a logical oracle.</metadata>",
        xml_escape(title),
        xml_escape(description),
    )
    .map_err(render_write_error)?;
    Ok(svg)
}

fn write_common_definitions(svg: &mut String) {
    svg.push_str(
        "<defs>\n<marker id=\"arrow-river\" markerUnits=\"userSpaceOnUse\" viewBox=\"0 0 10 10\" refX=\"9\" refY=\"5\" markerWidth=\"10\" markerHeight=\"10\" orient=\"auto\"><path d=\"M0 0L10 5 0 10Z\" fill=\"#0369a1\"/></marker>\n<pattern id=\"grid-dots\" width=\"10\" height=\"10\" patternUnits=\"userSpaceOnUse\"><circle cx=\"5\" cy=\"5\" r=\"1.2\" fill=\"#64748b\"/></pattern>\n<pattern id=\"water-waves\" width=\"18\" height=\"12\" patternUnits=\"userSpaceOnUse\"><path d=\"M-3 5 Q1 1 5 5 T13 5 T21 5\" fill=\"none\" stroke=\"#075985\" stroke-width=\"2\"/></pattern>\n<pattern id=\"beach-lines\" width=\"12\" height=\"12\" patternUnits=\"userSpaceOnUse\" patternTransform=\"rotate(30)\"><path d=\"M0 0V12\" stroke=\"#92400e\" stroke-width=\"2\"/></pattern>\n<pattern id=\"shore-lines\" width=\"14\" height=\"14\" patternUnits=\"userSpaceOnUse\"><path d=\"M0 4H14M0 10H14\" stroke=\"#7c2d12\" stroke-width=\"2.5\"/></pattern>\n<pattern id=\"valley-lines\" width=\"20\" height=\"14\" patternUnits=\"userSpaceOnUse\"><path d=\"M0 2Q10 13 20 2\" fill=\"none\" stroke=\"#3f6212\" stroke-width=\"2\"/></pattern>\n<pattern id=\"plateau-lines\" width=\"20\" height=\"14\" patternUnits=\"userSpaceOnUse\"><path d=\"M1 12L5 4H15L19 12\" fill=\"none\" stroke=\"#854d0e\" stroke-width=\"2\"/></pattern>\n<pattern id=\"hill-lines\" width=\"18\" height=\"14\" patternUnits=\"userSpaceOnUse\"><path d=\"M-3 13Q4 1 11 13T25 13\" fill=\"none\" stroke=\"#4d7c0f\" stroke-width=\"2\"/></pattern>\n<pattern id=\"woodland-dots\" width=\"16\" height=\"16\" patternUnits=\"userSpaceOnUse\"><circle cx=\"5\" cy=\"5\" r=\"3\" fill=\"#14532d\"/><circle cx=\"13\" cy=\"12\" r=\"2\" fill=\"#14532d\"/></pattern>\n<pattern id=\"frozen-crosses\" width=\"16\" height=\"16\" patternUnits=\"userSpaceOnUse\"><path d=\"M8 2V14M2 8H14\" stroke=\"#155e75\" stroke-width=\"2\"/></pattern>\n<pattern id=\"mountain-lines\" width=\"20\" height=\"16\" patternUnits=\"userSpaceOnUse\"><path d=\"M0 14L7 4l5 7 3-5 7 8\" fill=\"none\" stroke=\"#713f12\" stroke-width=\"2\"/></pattern>\n<pattern id=\"massif-lines\" width=\"20\" height=\"16\" patternUnits=\"userSpaceOnUse\"><path d=\"M-2 14L4 4 8 8 12 1 22 14\" fill=\"none\" stroke=\"#3f3f46\" stroke-width=\"3\"/></pattern>\n<pattern id=\"peak-lines\" width=\"18\" height=\"15\" patternUnits=\"userSpaceOnUse\"><path d=\"M-3 15L5 1l8 14M7 15l5-9 7 9\" fill=\"none\" stroke=\"#111827\" stroke-width=\"2.5\"/></pattern>\n<pattern id=\"island-lines\" width=\"14\" height=\"14\" patternUnits=\"userSpaceOnUse\" patternTransform=\"rotate(-25)\"><path d=\"M0 3H14\" stroke=\"#9a3412\" stroke-width=\"3\"/></pattern>\n<pattern id=\"crystal-lines\" width=\"18\" height=\"18\" patternUnits=\"userSpaceOnUse\"><path d=\"M9 1L16 9 9 17 2 9Z\" fill=\"none\" stroke=\"#9f1239\" stroke-width=\"2\"/></pattern>\n<style>.background{fill:#f8fafc}.cell{stroke:#0f172a;stroke-width:1.5;vector-effect:non-scaling-stroke}.authorship-outline{fill:none;vector-effect:non-scaling-stroke}.authorship-grid{stroke:#94a3b8;stroke-width:1;stroke-dasharray:2 5}.authorship-locked{stroke:#be123c;stroke-width:5}.authorship-bounded{stroke:#b45309;stroke-width:4;stroke-dasharray:9 4}.authorship-seeded{stroke:#1d4ed8;stroke-width:3;stroke-dasharray:2 4}.authorship-fallback{stroke:#7e22ce;stroke-width:2;stroke-dasharray:13 4 2 4}.base-unassigned{fill:#e2e8f0}.base-sea{fill:#7dd3fc}.base-lake{fill:#38bdf8}.base-river{fill:#7dd3fc}.base-island{fill:#fdba74}.base-beach{fill:#fed7aa}.base-shore{fill:#fdba74}.base-valley{fill:#d9f99d}.base-plateau{fill:#fde68a}.base-hill{fill:#bef264}.base-mountain{fill:#d6b890}.base-massif{fill:#a8a29e}.base-sharp-peak{fill:#94a3b8}.pattern{opacity:.82;pointer-events:none}.accent{fill-opacity:.58;pointer-events:none}.cell-label{font:700 10px system-ui,sans-serif;text-anchor:middle;fill:#0f172a;paint-order:stroke;stroke:#f8fafc;stroke-width:2.5px}.provenance-label{font:6px ui-monospace,monospace;text-anchor:middle;fill:#0f172a;paint-order:stroke;stroke:#f8fafc;stroke-width:1.5px}.coord-label{font:8px ui-monospace,monospace;text-anchor:middle;fill:#0f172a;paint-order:stroke;stroke:#f8fafc;stroke-width:2px}.trace{fill:none;stroke-linecap:round;stroke-linejoin:round;vector-effect:non-scaling-stroke}.trace-river{stroke:#0369a1;stroke-width:4;marker-end:url(#arrow-river)}.trace-tunnel{stroke:#dc2626;stroke-width:4;stroke-dasharray:8 6}.trace-label{font:700 10px system-ui,sans-serif;fill:#0f172a;paint-order:stroke;stroke:#f8fafc;stroke-width:3px}.diagnostic-information{fill:#2563eb}.diagnostic-warning{fill:#f59e0b}.title{font:700 27px system-ui,sans-serif;fill:#0f172a}.subtitle{font:15px system-ui,sans-serif;fill:#334155}.legend{font:13px system-ui,sans-serif;fill:#0f172a}.metrics{font:11px ui-monospace,monospace;fill:#0f172a}.panel-title{font:700 20px system-ui,sans-serif;fill:#0f172a}.panel-text{font:13px system-ui,sans-serif;fill:#334155}.panel-code{font:12px ui-monospace,monospace;fill:#0f172a}</style>\n</defs>\n",
    );
    svg.push_str(
        "<style>.cell-label{font:700 9px system-ui,sans-serif;stroke:#fff;stroke-opacity:.72;stroke-width:1px}.coord-label{font:7px ui-monospace,monospace;stroke:#fff;stroke-opacity:.72;stroke-width:.8px}.grid-label{font:600 5.5px ui-monospace,monospace;text-anchor:middle;fill:#334155}.trace-label{stroke:#fff;stroke-opacity:.82;stroke-width:1.5px}</style>\n",
    );
}

fn write_header(svg: &mut String, plan: &RenderPlan, width: f64) -> Result<(), RenderError> {
    writeln!(
        svg,
        "<rect class=\"background\" x=\"0\" y=\"0\" width=\"{width:.0}\" height=\"{:.0}\"/>\n<text class=\"title\" x=\"34\" y=\"42\">{}</text>\n<text class=\"subtitle\" x=\"34\" y=\"68\">{}</text>",
        if width > COMPOSITE_WIDTH {
            DIAGNOSTIC_HEIGHT
        } else {
            COMPOSITE_HEIGHT
        },
        xml_escape(&plan.title),
        xml_escape(&plan.subtitle),
    )
    .map_err(render_write_error)?;
    let mut metadata = Vec::new();
    if let Some(seed) = plan.seed {
        metadata.push(format!("seed {seed}"));
    }
    if let Some(fingerprint) = &plan.fingerprint {
        metadata.push(format!("fingerprint {fingerprint}"));
    }
    if !metadata.is_empty() {
        writeln!(
            svg,
            "<text class=\"subtitle\" text-anchor=\"end\" x=\"{:.0}\" y=\"42\">{}</text>",
            width - 34.0,
            xml_escape(&metadata.join(" · ")),
        )
        .map_err(render_write_error)?;
    }
    Ok(())
}

fn write_map_group(
    svg: &mut String,
    plan: &RenderPlan,
    canvas: &HexCanvas,
    feature_labels: bool,
    coordinate_labels: bool,
) -> Result<(), RenderError> {
    svg.push_str("<g id=\"schematic-cells\" aria-label=\"Canonical schematic cells\">\n");
    let mut cells = plan.cells.iter().collect::<Vec<_>>();
    cells.sort_unstable_by_key(|cell| (cell.q, cell.r, cell.ordinal));
    for cell in cells {
        let points = canvas.polygon(cell.q, cell.r);
        let (center_x, center_y) = canvas.center(cell.q, cell.r);
        let class = cell.style.class();
        let accent_labels = cell
            .accents
            .iter()
            .map(|accent| accent.label())
            .collect::<Vec<_>>()
            .join(", ");
        let authorship_labels = cell
            .authorship
            .iter()
            .map(|authorship| authorship.label())
            .collect::<Vec<_>>()
            .join(", ");
        let aria_label = format!(
            "Cell {}, coordinate q {}, r {}; {}; {}; accents {}; authorship {}; provenance signature {}; {}",
            cell.ordinal,
            cell.q,
            cell.r,
            cell.style.label(),
            cell.label,
            accent_labels,
            authorship_labels,
            cell.provenance_signature,
            cell.detail,
        );
        let description = format!(
            "{}; provenance signature {}",
            cell.detail, cell.provenance_signature
        );
        writeln!(
            svg,
            "<g id=\"cell-q{}-r{}\" role=\"group\" aria-label=\"{}\"><desc>{}</desc><polygon class=\"cell base-{class}\" points=\"{points}\"/><polygon class=\"pattern\" fill=\"url(#{})\" points=\"{points}\"/>",
            signed_id(cell.q),
            signed_id(cell.r),
            xml_escape(&aria_label),
            xml_escape(&description),
            cell.style.pattern(),
        )
        .map_err(render_write_error)?;
        for accent in &cell.accents {
            writeln!(
                svg,
                "<polygon class=\"accent\" fill=\"url(#{})\" points=\"{points}\" aria-hidden=\"true\"/>",
                accent.pattern(),
            )
            .map_err(render_write_error)?;
        }
        for authorship in &cell.authorship {
            writeln!(
                svg,
                "<polygon class=\"authorship-outline {}\" points=\"{points}\" aria-hidden=\"true\"/>",
                authorship.class(),
            )
            .map_err(render_write_error)?;
        }
        if feature_labels {
            writeln!(
                svg,
                "<text class=\"cell-label\" x=\"{center_x:.2}\" y=\"{:.2}\">{}</text>",
                if coordinate_labels {
                    center_y - 4.0
                } else {
                    center_y + 3.0
                },
                cell.style.abbreviation(),
            )
            .map_err(render_write_error)?;
        }
        if coordinate_labels {
            if feature_labels {
                writeln!(
                    svg,
                    "<text class=\"coord-label\" x=\"{center_x:.2}\" y=\"{:.2}\">{},{} · {}</text>",
                    center_y + 8.0,
                    cell.q,
                    cell.r,
                    cell.ordinal,
                )
                .map_err(render_write_error)?;
            } else {
                writeln!(
                    svg,
                    "<text class=\"grid-label\" x=\"{center_x:.2}\" y=\"{:.2}\">{},{}#{}</text>",
                    center_y + 2.0,
                    cell.q,
                    cell.r,
                    cell.ordinal,
                )
                .map_err(render_write_error)?;
            }
        }
        svg.push_str("</g>\n");
    }
    svg.push_str("</g>\n");
    write_traces(svg, plan, canvas)?;
    write_diagnostic_markers(svg, plan, canvas)?;
    Ok(())
}

fn write_traces(
    svg: &mut String,
    plan: &RenderPlan,
    canvas: &HexCanvas,
) -> Result<(), RenderError> {
    let mut traces = plan.traces.iter().collect::<Vec<_>>();
    traces.sort_unstable_by(|left, right| {
        left.label
            .cmp(&right.label)
            .then_with(|| left.style.cmp(&right.style))
    });
    svg.push_str("<g id=\"schematic-traces\" aria-label=\"Authored and generated routes\">\n");
    for trace in traces {
        let points = trace
            .cells
            .iter()
            .map(|(q, r)| {
                let (x, y) = canvas.center(*q, *r);
                format!("{x:.2},{y:.2}")
            })
            .collect::<Vec<_>>()
            .join(" ");
        writeln!(
            svg,
            "<polyline class=\"trace {}\" points=\"{points}\" role=\"img\" aria-label=\"{}: {} cells\"/>",
            trace.style.class(),
            xml_escape(&trace.label),
            trace.cells.len(),
        )
        .map_err(render_write_error)?;
    }
    svg.push_str("</g>\n");
    Ok(())
}

fn write_diagnostic_markers(
    svg: &mut String,
    plan: &RenderPlan,
    canvas: &HexCanvas,
) -> Result<(), RenderError> {
    svg.push_str("<g id=\"diagnostic-markers\" aria-label=\"Diagnostic cell markers\">\n");
    for diagnostic in &plan.diagnostics {
        let Some((q, r)) = diagnostic.cell else {
            continue;
        };
        let (x, y) = canvas.center(q, r);
        let aria_label = format!(
            "{} {} at q {}, r {}: {}",
            diagnostic.severity.label(),
            diagnostic.code,
            q,
            r,
            diagnostic.message
        );
        writeln!(
            svg,
            "<circle class=\"{}\" cx=\"{x:.2}\" cy=\"{y:.2}\" r=\"8\" stroke=\"#fff\" stroke-width=\"3\" role=\"img\" aria-label=\"{}\"/>",
            diagnostic.severity.class(),
            xml_escape(&aria_label),
        )
        .map_err(render_write_error)?;
    }
    svg.push_str("</g>\n");
    Ok(())
}

fn write_legend(
    svg: &mut String,
    plan: &RenderPlan,
    x: f64,
    y: f64,
    width: f64,
) -> Result<(), RenderError> {
    let mut styles = plan.cells.iter().map(|cell| cell.style).collect::<Vec<_>>();
    styles.sort_unstable();
    styles.dedup();
    let mut traces = plan
        .traces
        .iter()
        .map(|trace| trace.style)
        .collect::<Vec<_>>();
    traces.sort_unstable();
    traces.dedup();
    let mut accents = plan
        .cells
        .iter()
        .flat_map(|cell| cell.accents.iter().copied())
        .collect::<Vec<_>>();
    accents.sort_unstable();
    accents.dedup();
    writeln!(
        svg,
        "<g id=\"legend\" aria-label=\"Map legend\"><rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{width:.1}\" height=\"104\" rx=\"8\" fill=\"#fff\" stroke=\"#94a3b8\"/><text class=\"legend\" x=\"{:.1}\" y=\"{:.1}\">Patterns, abbreviations, and line styles duplicate the review-only palette:</text>",
        x + 14.0,
        y + 21.0,
    )
    .map_err(render_write_error)?;
    let mut cursor_x = x + 14.0;
    let mut cursor_y = y + 48.0;
    let max_x = x + width - 14.0;
    for style in styles {
        let label = format!("{} {}", style.abbreviation(), style.label());
        write_legend_item(svg, &label, None, x, max_x, &mut cursor_x, &mut cursor_y)?;
    }
    for style in traces {
        let label = style.label();
        write_legend_item(
            svg,
            label,
            Some(style.class()),
            x,
            max_x,
            &mut cursor_x,
            &mut cursor_y,
        )?;
    }
    for accent in accents {
        let label = format!("{} {}", accent.abbreviation(), accent.label());
        write_legend_item(svg, &label, None, x, max_x, &mut cursor_x, &mut cursor_y)?;
    }
    for authorship in AuthorshipStyle::ALL {
        let label = format!("{} {}", authorship.abbreviation(), authorship.label());
        write_legend_item(
            svg,
            &label,
            Some(authorship.class()),
            x,
            max_x,
            &mut cursor_x,
            &mut cursor_y,
        )?;
    }
    svg.push_str("</g>\n");
    Ok(())
}

fn write_legend_item(
    svg: &mut String,
    label: &str,
    line_class: Option<&str>,
    origin_x: f64,
    max_x: f64,
    cursor_x: &mut f64,
    cursor_y: &mut f64,
) -> Result<(), RenderError> {
    let label_width = 44.0 + f64::from(u32::try_from(label.len()).unwrap_or(u32::MAX)) * 7.5;
    if *cursor_x + label_width > max_x {
        *cursor_x = origin_x + 14.0;
        *cursor_y += 23.0;
    }
    if let Some(class) = line_class {
        writeln!(
            svg,
            "<line class=\"trace {class}\" x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" aria-hidden=\"true\"/>",
            *cursor_x,
            *cursor_y - 5.0,
            *cursor_x + 18.0,
            *cursor_y - 5.0,
        )
        .map_err(render_write_error)?;
    }
    writeln!(
        svg,
        "<text class=\"legend\" x=\"{:.1}\" y=\"{:.1}\">{}</text>",
        *cursor_x + 21.0,
        *cursor_y,
        xml_escape(label),
    )
    .map_err(render_write_error)?;
    *cursor_x += label_width;
    Ok(())
}

fn write_metric_band(
    svg: &mut String,
    plan: &RenderPlan,
    x: f64,
    y: f64,
    width: f64,
) -> Result<(), RenderError> {
    writeln!(
        svg,
        "<g id=\"typed-metric-summary\" aria-label=\"Typed metric summary\"><rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{width:.1}\" height=\"50\" rx=\"8\" fill=\"#fff\" stroke=\"#94a3b8\"/>",
    )
    .map_err(render_write_error)?;
    if plan.metrics.is_empty() {
        writeln!(
            svg,
            "<text class=\"metrics\" x=\"{:.1}\" y=\"{:.1}\">No public metrics were supplied for this projection.</text>",
            x + 14.0,
            y + 28.0,
        )
        .map_err(render_write_error)?;
    } else {
        let column_width = (width - 28.0) / 4.0;
        for (index, (name, value)) in plan.metrics.iter().enumerate() {
            let column = f64::from(u32::try_from(index % 4).unwrap_or(u32::MAX));
            let row = f64::from(u32::try_from(index / 4).unwrap_or(u32::MAX));
            writeln!(
                svg,
                "<text class=\"metrics\" x=\"{:.1}\" y=\"{:.1}\">{}={}</text>",
                x + 14.0 + column * column_width,
                y + 18.0 + row * 19.0,
                xml_escape(name),
                xml_escape(value),
            )
            .map_err(render_write_error)?;
        }
    }
    svg.push_str("</g>\n");
    Ok(())
}

fn write_diagnostic_panel(
    svg: &mut String,
    plan: &RenderPlan,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), RenderError> {
    writeln!(
        svg,
        "<g id=\"diagnostic-panel\" aria-label=\"Metrics and diagnostics\"><rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{width:.1}\" height=\"{height:.1}\" rx=\"10\" fill=\"#fff\" stroke=\"#64748b\" stroke-width=\"2\"/><text class=\"panel-title\" x=\"{:.1}\" y=\"{:.1}\">Typed metrics</text>",
        x + 18.0,
        y + 34.0,
    )
    .map_err(render_write_error)?;
    let mut cursor_y = y + 62.0;
    for (name, value) in &plan.metrics {
        writeln!(
            svg,
            "<text class=\"panel-code\" x=\"{:.1}\" y=\"{cursor_y:.1}\">{}: {}</text>",
            x + 18.0,
            xml_escape(name),
            xml_escape(value),
        )
        .map_err(render_write_error)?;
        cursor_y += 21.0;
    }
    cursor_y += 18.0;
    writeln!(
        svg,
        "<text class=\"panel-title\" x=\"{:.1}\" y=\"{cursor_y:.1}\">Diagnostics ({})</text>",
        x + 18.0,
        plan.diagnostics.len(),
    )
    .map_err(render_write_error)?;
    cursor_y += 29.0;
    if plan.diagnostics.is_empty() {
        writeln!(
            svg,
            "<text class=\"panel-text\" x=\"{:.1}\" y=\"{cursor_y:.1}\">No renderer diagnostics. This does not replace validation.</text>",
            x + 18.0,
        )
        .map_err(render_write_error)?;
    } else {
        for diagnostic in &plan.diagnostics {
            let location = diagnostic
                .cell
                .map_or_else(|| "global".to_owned(), |(q, r)| format!("q {q}, r {r}"));
            writeln!(
                svg,
                "<circle class=\"{}\" cx=\"{:.1}\" cy=\"{:.1}\" r=\"6\"/><text class=\"panel-code\" x=\"{:.1}\" y=\"{cursor_y:.1}\">{} · {}</text>",
                diagnostic.severity.class(),
                x + 23.0,
                cursor_y - 4.0,
                x + 37.0,
                xml_escape(&diagnostic.code),
                xml_escape(&location),
            )
            .map_err(render_write_error)?;
            cursor_y += 18.0;
            writeln!(
                svg,
                "<text class=\"panel-text\" x=\"{:.1}\" y=\"{cursor_y:.1}\">{}</text>",
                x + 37.0,
                xml_escape(&diagnostic.message),
            )
            .map_err(render_write_error)?;
            cursor_y += 25.0;
        }
    }
    svg.push_str("</g>\n");
    Ok(())
}

fn signed_id(value: i32) -> String {
    if value < 0 {
        format!("n{}", value.unsigned_abs())
    } else {
        format!("p{value}")
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn html_escape(value: &str) -> String {
    xml_escape(value)
}

fn render_write_error(error: fmt::Error) -> RenderError {
    RenderError::new(format!("could not compose diagnostic document: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_plan() -> RenderPlan {
        RenderPlan {
            title: "Grand <V3> & trace".to_owned(),
            subtitle: "review-only projection".to_owned(),
            seed: Some(42),
            fingerprint: Some("0123456789abcdef".to_owned()),
            cells: vec![
                RenderCell {
                    q: 0,
                    r: 0,
                    ordinal: 0,
                    style: CellStyle::Lake,
                    accents: vec![CellAccent::FrozenWoodland],
                    label: "lake \"heart\"".to_owned(),
                    detail: "surface=Water; landform=Basin; climate=Frozen".to_owned(),
                    provenance_signature: "sL lB cL vS aL oMLB".to_owned(),
                    authorship: vec![AuthorshipStyle::Locked],
                },
                RenderCell {
                    q: 1,
                    r: 0,
                    ordinal: 1,
                    style: CellStyle::Mountain,
                    accents: Vec::new(),
                    label: "mountain".to_owned(),
                    detail: "surface=Land; landform=Mountain; climate=Alpine".to_owned(),
                    provenance_signature: "sB lB cS vS aB".to_owned(),
                    authorship: vec![AuthorshipStyle::Bounded, AuthorshipStyle::Seeded],
                },
            ],
            traces: vec![
                RenderTrace {
                    label: "waterfall".to_owned(),
                    style: TraceStyle::River,
                    cells: vec![(0, 0), (1, 0)],
                },
                RenderTrace {
                    label: "complete tunnel".to_owned(),
                    style: TraceStyle::Tunnel,
                    cells: vec![(1, 0), (0, 0)],
                },
            ],
            metrics: vec![("cells".to_owned(), "2".to_owned())],
            diagnostics: vec![RenderDiagnostic {
                severity: DiagnosticSeverity::Information,
                code: "RENDER-ONLY".to_owned(),
                message: "typed validator passed".to_owned(),
                cell: None,
            }],
        }
    }

    fn gallery_entries() -> Vec<GalleryEntry> {
        (0..GALLERY_ENTRY_COUNT)
            .map(|index| GalleryEntry {
                heading: format!("Seed {index}"),
                seed: Some(u64::try_from(index).expect("small fixture index fits u64")),
                fingerprint: format!("fp-{index}"),
                summary: "validated plan".to_owned(),
                composite_href: format!("seed-{index}/composite.svg"),
                diagnostic_href: format!("seed-{index}/diagnostics.svg"),
                plan_href: format!("seed-{index}/plan.ron"),
                metrics_href: format!("seed-{index}/metrics.ron"),
            })
            .collect()
    }

    #[test]
    fn composite_is_labelled_patterned_and_escaped() {
        let svg = composite_svg(&sample_plan()).expect("sample projection should render");
        assert!(svg.contains("role=\"img\""));
        assert!(svg.contains("aria-labelledby"));
        assert!(svg.contains("water-waves"));
        assert!(svg.contains(">LAKE</text>"));
        assert!(!svg.contains("class=\"coord-label\" x="));
        assert!(!svg.contains("class=\"provenance-label\" x="));
        assert!(svg.contains("authorship-locked"));
        assert!(svg.contains("authorship-bounded"));
        assert!(svg.contains("authorship-seeded"));
        assert!(svg.contains("sL lB cL vS aL oMLB"));
        assert!(svg.contains("cells=2"));
        assert!(svg.contains("marker-end:url(#arrow-river)"));
        assert!(svg.contains("markerUnits=\"userSpaceOnUse\""));
        assert!(svg.contains("trace-tunnel"));
        assert!(svg.contains("Grand &lt;V3&gt; &amp; trace"));
        assert!(svg.contains("Typed RON and the validator are authoritative"));
    }

    #[test]
    fn tracing_grid_shows_only_one_compact_coordinate_and_ordinal_label() {
        let svg = composite_svg(&grid_projection()).expect("canonical grid should render");
        assert!(svg.contains(">0,0#0</text>"));
        assert!(!svg.contains("class=\"cell-label\" x="));
        assert!(!svg.contains("class=\"provenance-label\" x="));
        assert!(!svg.contains(">grid-N</text>"));
    }

    #[test]
    fn diagnostic_document_contains_metrics_and_non_oracle_boundary() {
        let svg = diagnostic_svg(&sample_plan()).expect("sample diagnostic should render");
        assert!(svg.contains("Typed metrics"));
        assert!(svg.contains("cells: 2"));
        assert!(svg.contains("pixels are not a logical oracle"));
        assert!(svg.contains("RENDER-ONLY"));
        assert!(svg.contains(">LAKE</text>"));
        assert!(svg.contains(">0,0 · 0</text>"));
        assert!(!svg.contains("class=\"provenance-label\" x="));
        assert!(svg.contains("sL lB cL vS aL oMLB"));
        assert!(svg.contains("marker-end:url(#arrow-river)"));
        assert!(svg.contains("trace-tunnel"));
    }

    #[test]
    fn route_labels_are_short_semantic_phrases() {
        assert_eq!(
            semantic_trace_label(NetworkKind::Hydrology, "edge/hydrology-lake-to-falls"),
            "lake → falls"
        );
        assert_eq!(
            semantic_trace_label(NetworkKind::Tunnel, "edge/tunnel-complete"),
            "ascent → hill"
        );
        assert_eq!(
            semantic_trace_label(NetworkKind::Hydrology, "edge/unknown"),
            "river route"
        );
    }

    #[test]
    fn reference_artifact_and_exhausted_fallback_are_visibly_distinct() {
        let template = hex_schematic::grand_v3_reference_template()
            .expect("packaged reference template should parse");
        let generated = hex_schematic::reference_plan(&template, 0)
            .expect("packaged reference plan should validate");
        let projection = plan_projection(&template, &generated.plan, Vec::new())
            .expect("reference plan should project");
        let authorship = projection
            .cells
            .iter()
            .flat_map(|cell| cell.authorship.iter().copied())
            .collect::<Vec<_>>();
        assert!(authorship.contains(&AuthorshipStyle::Locked));
        assert!(authorship.contains(&AuthorshipStyle::Bounded));
        assert!(authorship.contains(&AuthorshipStyle::Seeded));
        assert!(!authorship.contains(&AuthorshipStyle::ReferenceFallback));
        assert!(projection
            .cells
            .iter()
            .any(|cell| cell.detail.contains("Locked(")));
        assert!(projection
            .cells
            .iter()
            .any(|cell| cell.detail.contains("Bounded(")));
        assert!(projection
            .cells
            .iter()
            .any(|cell| cell.detail.contains("Seeded(")));
        assert!(projection
            .cells
            .iter()
            .all(|cell| !cell.detail.contains("ReferenceFallback(")));

        let svg = diagnostic_svg(&projection).expect("reference diagnostic should render");
        assert!(svg.contains("canonical reference artifact"));
        assert!(svg.contains("direct reference artifact; no candidates evaluated"));
        assert!(svg.contains("REFERENCE-ARTIFACT"));
        assert!(!svg.contains("REFERENCE-FALLBACK"));
        assert!(svg.contains("class=\"authorship-outline authorship-locked\""));
        assert!(svg.contains("class=\"authorship-outline authorship-bounded\""));
        assert!(svg.contains("class=\"authorship-outline authorship-seeded\""));

        let fallback_source = |source: &LayerProvenance| {
            let source = match source {
                LayerProvenance::Locked { claim } => claim.clone(),
                LayerProvenance::Bounded { rule } => rule.clone(),
                LayerProvenance::Seeded { stream } => stream.clone(),
                LayerProvenance::ReferenceFallback { source } => source.clone(),
            };
            LayerProvenance::ReferenceFallback { source }
        };
        let mut fallback_plan = generated.plan.clone();
        fallback_plan.provenance = hex_schematic::PlanProvenance::reference_fallback(0);
        for cell in &mut fallback_plan.cells {
            cell.provenance.surface = fallback_source(&cell.provenance.surface);
            cell.provenance.landform = fallback_source(&cell.provenance.landform);
            cell.provenance.climate = fallback_source(&cell.provenance.climate);
            cell.provenance.vegetation = fallback_source(&cell.provenance.vegetation);
            cell.provenance.access = fallback_source(&cell.provenance.access);
            for overlay in &mut cell.provenance.overlays {
                overlay.source = fallback_source(&overlay.source);
            }
        }
        for feature in &mut fallback_plan.features {
            feature.provenance = LayerProvenance::ReferenceFallback {
                source: feature.id.clone(),
            };
        }
        let fallback = plan_projection(&template, &fallback_plan, Vec::new())
            .expect("exhausted fallback should project");
        assert!(fallback
            .cells
            .iter()
            .any(|cell| cell.detail.contains("ReferenceFallback(")));
        assert!(fallback
            .cells
            .iter()
            .any(|cell| cell.detail.contains("underlying=Locked")));
        assert!(fallback
            .cells
            .iter()
            .any(|cell| cell.detail.contains("underlying=Bounded")));
        assert!(fallback
            .cells
            .iter()
            .any(|cell| cell.detail.contains("underlying=Seeded")));
        let fallback_svg =
            diagnostic_svg(&fallback).expect("exhausted fallback diagnostic should render");
        assert!(fallback_svg.contains("exhausted-candidate fallback"));
        assert!(fallback_svg.contains("reference fallback after every candidate failed"));
        assert!(fallback_svg.contains("REFERENCE-FALLBACK"));
        assert!(!fallback_svg.contains("REFERENCE-ARTIFACT"));

        let candidate =
            hex_schematic::generate(&template, 0).expect("normal candidate plan should generate");
        let candidate_projection = plan_projection(&template, &candidate.plan, Vec::new())
            .expect("normal candidate should project");
        let candidate_svg =
            diagnostic_svg(&candidate_projection).expect("candidate diagnostic should render");
        assert!(candidate_svg.contains("Grand V3 schematic — seed 0"));
        assert!(candidate_svg.contains("selected candidate"));
        assert!(candidate_svg.contains("SELECTED-CANDIDATE"));
        assert!(!candidate_svg.contains("REFERENCE-ARTIFACT"));
        assert!(!candidate_svg.contains("REFERENCE-FALLBACK"));
    }

    #[test]
    fn contact_sheet_and_gallery_require_exactly_twelve_entries() {
        let entries = gallery_entries();
        let reference = GalleryEntry {
            heading: "Canonical reference artifact".to_owned(),
            seed: None,
            fingerprint: "reference-fingerprint".to_owned(),
            summary: "validated reference plan".to_owned(),
            composite_href: "reference/composite.svg".to_owned(),
            diagnostic_href: "reference/diagnostics.svg".to_owned(),
            plan_href: "reference/plan.ron".to_owned(),
            metrics_href: "reference/metrics.ron".to_owned(),
        };
        let plans = (0..GALLERY_ENTRY_COUNT)
            .map(|index| {
                let mut plan = sample_plan();
                plan.seed = Some(u64::try_from(index).expect("small fixture index fits u64"));
                plan
            })
            .collect::<Vec<_>>();
        let contact = contact_sheet_svg(&entries, &plans).expect("twelve panels should render");
        let html = complete_gallery_html(&entries, &reference, "contact-sheet.svg")
            .expect("twelve cards should render");
        assert_eq!(
            contact.matches("role=\"group\"").count(),
            GALLERY_ENTRY_COUNT
        );
        assert_eq!(
            contact.matches(">validated plan</text>").count(),
            GALLERY_ENTRY_COUNT,
        );
        assert!(!contact.contains("<image"));
        assert_eq!(
            contact.matches("class=\"contact-map\"").count(),
            GALLERY_ENTRY_COUNT,
        );
        assert_eq!(
            contact.matches("class=\"mini-cell ").count(),
            GALLERY_ENTRY_COUNT * 2,
        );
        assert_eq!(html.matches("class=\"card\"").count(), GALLERY_ENTRY_COUNT);
        assert!(html.contains("Canonical reference artifact"));
        assert!(html.contains("reference/plan.ron"));
        assert!(entries
            .get(..GALLERY_ENTRY_COUNT - 1)
            .is_some_and(|short_entries| contact_sheet_svg(short_entries, &plans).is_err()));
        assert!(plans
            .get(..GALLERY_ENTRY_COUNT - 1)
            .is_some_and(|short_plans| contact_sheet_svg(&entries, short_plans).is_err()));
        assert!(complete_gallery_html(&[], &reference, "contact-sheet.svg").is_err());
    }

    #[test]
    fn duplicate_cell_coordinates_fail_closed() {
        let mut plan = sample_plan();
        plan.cells.push(RenderCell {
            q: 0,
            r: 0,
            ordinal: 2,
            style: CellStyle::Sea,
            accents: Vec::new(),
            label: "duplicate".to_owned(),
            detail: String::new(),
            provenance_signature: "sB lB cB vB aB".to_owned(),
            authorship: vec![AuthorshipStyle::Bounded],
        });
        assert!(composite_svg(&plan).is_err());
    }

    #[test]
    fn every_style_has_text_and_pattern_redundancy() {
        for style in CellStyle::ALL {
            assert!(!style.abbreviation().is_empty());
            assert!(!style.label().is_empty());
            assert!(!style.pattern().is_empty());
        }
        for authorship in AuthorshipStyle::ALL {
            assert!(!authorship.abbreviation().is_empty());
            assert!(!authorship.label().is_empty());
            assert!(!authorship.class().is_empty());
        }
    }
}
