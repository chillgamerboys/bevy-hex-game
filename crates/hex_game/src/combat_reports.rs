//! Versioned deterministic Combat Lab reports and bounded local history.

use std::collections::BTreeSet;
use std::io;

use bevy::prelude::*;
use hex_assets::{CombatRulesProfile, CombatSettings};
use hex_combat::{CombatSummary, EncounterOutcome, MAX_COMBAT_SUMMARY_DETAILS};
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::xxh3_64;

use crate::storage::{read, write_atomic, StoragePaths};

pub use hex_gameplay_model::{CombatLabReportDeployment, CombatLabReportId};

/// Current serialized report schema.
pub const COMBAT_LAB_REPORT_VERSION: u16 = 1;
/// Current local report-history schema.
pub const COMBAT_LAB_REPORT_HISTORY_VERSION: u16 = 1;
/// Maximum explicitly saved reports retained locally.
pub const MAX_COMBAT_LAB_REPORTS: usize = 64;
const MAX_REPORT_TEXT: usize = 128;
const MAX_REPORT_ROSTER: usize = 6;

/// Whether a frozen report came from a human Sandbox or immutable fixture.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum CombatLabReportOrigin {
    /// Human-composed transient Sandbox.
    Sandbox,
    /// Immutable fixture addressed by stable machine id.
    FixedFixture {
        /// Stable fixture id.
        stable_id: String,
    },
}

/// Frozen map identity used by one run.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CombatLabReportMap {
    /// Stable Combat Lab map-catalog id.
    pub catalog_id: String,
    /// Stable scenario name resolved by that catalog entry or fixture.
    pub scenario: String,
    /// Exact resolved procedural seed, absent only for authored maps with no seed.
    pub resolved_seed: Option<u64>,
}

/// Controller identity recorded for one ordered roster member.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatLabReportController {
    /// Human player seat.
    Human,
    /// Shipped deterministic baseline AI.
    BaselineAi,
}

/// One frozen ordered roster entry.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CombatLabReportRosterEntry {
    /// Stable session unit id used by per-unit summary projections.
    ///
    /// An all-zero roster is accepted only as the compatibility shape for
    /// reports written before per-unit headers were added. In current reports,
    /// zero is also the first valid canonical session id.
    #[serde(default)]
    pub unit_id: u64,
    /// Stable runtime archetype key.
    pub archetype: String,
    /// Player-facing name frozen at launch.
    pub display_name: String,
    /// Controller used by this run.
    pub controller: CombatLabReportController,
}

/// Both ordered rosters frozen into a run.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CombatLabReportRosters {
    /// Player side in exact spawn and UI order.
    pub players: Vec<CombatLabReportRosterEntry>,
    /// Hostile side in exact spawn and UI order.
    pub hostiles: Vec<CombatLabReportRosterEntry>,
}

/// Complete deterministic result of one Combat Lab run.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CombatLabReport {
    /// Serialized report schema.
    pub version: u16,
    /// Frozen validated numeric rules.
    pub profile: CombatRulesProfile,
    /// Sandbox or stable fixture origin.
    pub origin: CombatLabReportOrigin,
    /// Exact map and seed identity.
    pub map: CombatLabReportMap,
    /// Accepted semantic content fingerprint frozen at launch.
    pub content_revision: u64,
    /// Exact ordered roster headers.
    pub rosters: CombatLabReportRosters,
    /// Exact deployment, including stacked-surface level.
    pub deployment: CombatLabReportDeployment,
    /// Final retained-world result.
    pub outcome: EncounterOutcome,
    /// Deterministic identity of `summary`.
    pub summary_fingerprint: u64,
    /// Gameplay-owned bounded statistics and structured event source.
    pub summary: CombatSummary,
}

/// Complete report for the retained Lab battlefield currently on screen.
#[derive(Resource, Debug, Clone)]
pub(crate) struct CurrentCombatLabReport(pub(crate) CombatLabReport);

impl CombatLabReport {
    /// Builds a report and captures the canonical summary identity.
    pub fn new(
        profile: CombatRulesProfile,
        origin: CombatLabReportOrigin,
        map: CombatLabReportMap,
        content_revision: u64,
        rosters: CombatLabReportRosters,
        deployment: CombatLabReportDeployment,
        outcome: EncounterOutcome,
        summary: CombatSummary,
    ) -> Self {
        let summary_fingerprint = summary.fingerprint();
        Self {
            version: COMBAT_LAB_REPORT_VERSION,
            profile,
            origin,
            map,
            content_revision,
            rosters,
            deployment,
            outcome,
            summary_fingerprint,
            summary,
        }
    }

    /// Validates the complete frozen report without consulting mutable local state.
    pub fn validate(&self, shipped: &CombatSettings) -> Result<(), String> {
        if self.version != COMBAT_LAB_REPORT_VERSION {
            return Err(format!(
                "combat report version {} is unsupported; expected {}",
                self.version, COMBAT_LAB_REPORT_VERSION
            ));
        }
        self.profile.validate(shipped)?;
        validate_text("map catalog id", &self.map.catalog_id)?;
        validate_text("scenario", &self.map.scenario)?;
        validate_roster("player", &self.rosters.players)?;
        validate_roster("hostile", &self.rosters.hostiles)?;
        let unit_ids = self
            .rosters
            .players
            .iter()
            .chain(&self.rosters.hostiles)
            .map(|entry| entry.unit_id)
            .collect::<Vec<_>>();
        if unit_ids.iter().any(|id| *id != 0) {
            let mut unique = BTreeSet::new();
            if unit_ids.iter().any(|id| !unique.insert(*id)) {
                return Err("report roster unit ids must be unique when present".to_owned());
            }
        }
        if let CombatLabReportOrigin::FixedFixture { stable_id } = &self.origin {
            validate_text("fixture stable id", stable_id)?;
        }
        if self.deployment.players.len() != self.rosters.players.len()
            || self.deployment.hostiles.len() != self.rosters.hostiles.len()
        {
            return Err(
                "report deployment must correspond one-to-one with both rosters".to_owned(),
            );
        }
        let mut occupied = BTreeSet::new();
        for position in self
            .deployment
            .players
            .iter()
            .chain(&self.deployment.hostiles)
        {
            if !occupied.insert(*position) {
                return Err(format!(
                    "report deployment places two bodies on {position:?}"
                ));
            }
        }
        if self.summary.outcome != Some(self.outcome) {
            return Err("report outcome disagrees with the gameplay summary".to_owned());
        }
        if self.summary.events.len() > MAX_COMBAT_SUMMARY_DETAILS
            || self.summary.ai_selections.len() > MAX_COMBAT_SUMMARY_DETAILS
        {
            return Err("report summary detail window exceeds its gameplay bound".to_owned());
        }
        if self.summary_fingerprint != self.summary.fingerprint() {
            return Err("report summary fingerprint does not match its statistics".to_owned());
        }
        Ok(())
    }

    /// Deterministic identity of all frozen inputs and gameplay-owned results.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"hex-combat-lab-report-v1");
        if serde_json::to_writer(&mut bytes, self).is_err() {
            bytes.extend_from_slice(b"<serialization-error>");
        }
        xxh3_64(&bytes)
    }
}

fn validate_text(field: &str, value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} cannot be empty"));
    }
    if trimmed != value {
        return Err(format!(
            "{field} cannot have leading or trailing whitespace"
        ));
    }
    if value.chars().count() > MAX_REPORT_TEXT {
        return Err(format!(
            "{field} exceeds the {MAX_REPORT_TEXT}-character report bound"
        ));
    }
    Ok(())
}

fn validate_roster(side: &str, roster: &[CombatLabReportRosterEntry]) -> Result<(), String> {
    if !(1..=MAX_REPORT_ROSTER).contains(&roster.len()) {
        return Err(format!(
            "{side} report roster must contain 1..={MAX_REPORT_ROSTER} units"
        ));
    }
    for unit in roster {
        validate_text("roster archetype", &unit.archetype)?;
        validate_text("roster display name", &unit.display_name)?;
    }
    Ok(())
}

/// One explicitly saved local report.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SavedCombatLabReport {
    /// Stable local id.
    pub id: CombatLabReportId,
    /// Frozen deterministic report.
    pub report: CombatLabReport,
}

/// Versioned bounded report history, separate from Creator and Continue data.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CombatLabReportHistory {
    /// Serialized history schema.
    pub version: u16,
    /// Next monotonic local id.
    pub next_id: u64,
    /// Explicitly saved reports in save order.
    pub reports: Vec<SavedCombatLabReport>,
}

impl Default for CombatLabReportHistory {
    fn default() -> Self {
        Self {
            version: COMBAT_LAB_REPORT_HISTORY_VERSION,
            next_id: 1,
            reports: Vec::new(),
        }
    }
}

impl CombatLabReportHistory {
    /// Validates bounds, ids, and every contained report.
    pub fn validate(&self, shipped: &CombatSettings) -> Result<(), String> {
        if self.version != COMBAT_LAB_REPORT_HISTORY_VERSION {
            return Err(format!(
                "combat report history version {} is unsupported; expected {}",
                self.version, COMBAT_LAB_REPORT_HISTORY_VERSION
            ));
        }
        if self.reports.len() > MAX_COMBAT_LAB_REPORTS {
            return Err(format!(
                "combat report history exceeds its {MAX_COMBAT_LAB_REPORTS}-report bound"
            ));
        }
        let mut ids = BTreeSet::new();
        for saved in &self.reports {
            if saved.id.0 == 0 || !ids.insert(saved.id) {
                return Err("combat report history contains an invalid or duplicate id".to_owned());
            }
            if saved.id.0 >= self.next_id {
                return Err("combat report history next id is not monotonic".to_owned());
            }
            saved.report.validate(shipped)?;
        }
        Ok(())
    }
}

/// Loaded local report history and the last non-destructive storage problem.
#[derive(Resource, Debug, Clone, Default)]
pub(crate) struct CombatLabReportStore {
    /// Valid explicitly saved history.
    pub(crate) history: CombatLabReportHistory,
    /// Parse, validation, read, or write failure shown by Combat Lab.
    pub(crate) error: Option<String>,
}

impl CombatLabReportStore {
    /// Explicitly saves one validated report and returns its monotonic local id.
    pub(crate) fn save(
        &mut self,
        report: CombatLabReport,
        shipped: &CombatSettings,
        paths: &StoragePaths,
    ) -> Result<CombatLabReportId, String> {
        report.validate(shipped)?;
        if self.history.reports.len() >= MAX_COMBAT_LAB_REPORTS {
            return Err(format!(
                "report history is full; delete a report before saving another ({MAX_COMBAT_LAB_REPORTS} maximum)"
            ));
        }
        let before = self.history.clone();
        let id = CombatLabReportId(self.history.next_id);
        self.history.next_id = self.history.next_id.saturating_add(1);
        self.history
            .reports
            .push(SavedCombatLabReport { id, report });
        self.persist_or_restore(before, shipped, paths)?;
        Ok(id)
    }

    /// Deletes one confirmed report while preserving all data on write failure.
    pub(crate) fn delete(
        &mut self,
        id: CombatLabReportId,
        shipped: &CombatSettings,
        paths: &StoragePaths,
    ) -> Result<(), String> {
        let before = self.history.clone();
        let original_len = self.history.reports.len();
        self.history.reports.retain(|saved| saved.id != id);
        if self.history.reports.len() == original_len {
            return Err(format!("saved report {} does not exist", id.0));
        }
        self.persist_or_restore(before, shipped, paths)
    }

    fn persist_or_restore(
        &mut self,
        before: CombatLabReportHistory,
        shipped: &CombatSettings,
        paths: &StoragePaths,
    ) -> Result<(), String> {
        match persist_history(&self.history, shipped, paths) {
            Ok(()) => {
                self.error = None;
                Ok(())
            }
            Err(error) => {
                self.history = before;
                self.error = Some(error.clone());
                Err(error)
            }
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<StoragePaths>()
        .init_resource::<CombatLabReportStore>()
        .add_systems(Startup, load_report_history);
}

fn load_report_history(
    paths: Res<StoragePaths>,
    shipped: Option<Res<CombatSettings>>,
    mut store: ResMut<CombatLabReportStore>,
) {
    let Some(shipped) = shipped.as_deref() else {
        store.error = Some("combat reports could not load before shipped rules".to_owned());
        return;
    };
    match read(&paths.combat_reports) {
        Ok(contents) => match ron::from_str::<CombatLabReportHistory>(&contents) {
            Ok(history) => match history.validate(shipped) {
                Ok(()) => {
                    store.history = history;
                    store.error = None;
                }
                Err(error) => {
                    store.error = Some(format!(
                        "combat-reports.ron was preserved but refused: {error}"
                    ));
                }
            },
            Err(error) => {
                store.error = Some(format!(
                    "combat-reports.ron was preserved but could not be parsed: {error}"
                ));
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            store.history = CombatLabReportHistory::default();
            store.error = None;
        }
        Err(error) => {
            store.error = Some(format!("could not read combat-reports.ron: {error}"));
        }
    }
}

fn persist_history(
    history: &CombatLabReportHistory,
    shipped: &CombatSettings,
    paths: &StoragePaths,
) -> Result<(), String> {
    history.validate(shipped)?;
    let serialized = ron::ser::to_string_pretty(history, PrettyConfig::new())
        .map_err(|error| format!("could not serialize combat reports: {error}"))?;
    write_atomic(&paths.combat_reports, &serialized)
        .map_err(|error| format!("could not save combat reports: {error}"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use hex_assets::CombatRulesProfile;
    use hex_combat::CombatEvent;
    use hex_core::{HexCoord, TilePos};

    use super::*;

    fn complete_report() -> CombatLabReport {
        let settings = CombatSettings::default();
        let mut summary = CombatSummary::default();
        summary.outcome = Some(EncounterOutcome::Victory);
        summary.events.push_back(CombatEvent::EncounterResolved {
            outcome: EncounterOutcome::Victory,
        });
        CombatLabReport::new(
            CombatRulesProfile::shipped(&settings),
            CombatLabReportOrigin::Sandbox,
            CombatLabReportMap {
                catalog_id: "flat-arena".to_owned(),
                scenario: "Flat Arena".to_owned(),
                resolved_seed: Some(42),
            },
            99,
            CombatLabReportRosters {
                players: vec![CombatLabReportRosterEntry {
                    unit_id: 1,
                    archetype: "hedge-mage".to_owned(),
                    display_name: "Hedge Mage".to_owned(),
                    controller: CombatLabReportController::Human,
                }],
                hostiles: vec![CombatLabReportRosterEntry {
                    unit_id: 2,
                    archetype: "raider".to_owned(),
                    display_name: "Raider".to_owned(),
                    controller: CombatLabReportController::BaselineAi,
                }],
            },
            CombatLabReportDeployment {
                players: vec![TilePos::new(HexCoord::ORIGIN, 1)],
                hostiles: vec![TilePos::new(HexCoord::from_axial(1, 0), 1)],
            },
            EncounterOutcome::Victory,
            summary,
        )
    }

    fn paths(root: PathBuf) -> StoragePaths {
        StoragePaths {
            preferences: root.join("preferences.ron"),
            resume: root.join("resume.ron"),
            creations: root.join("creations.ron"),
            combat_reports: root.join("combat-reports.ron"),
        }
    }

    #[test]
    fn report_round_trip_and_fingerprint_are_stable() {
        let settings = CombatSettings::default();
        let report = complete_report();
        assert_eq!(report.validate(&settings), Ok(()));
        let fingerprint = report.fingerprint();
        let encoded = ron::to_string(&report).expect("serialize");
        let decoded: CombatLabReport = ron::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, report);
        assert_eq!(decoded.fingerprint(), fingerprint);
    }

    #[test]
    fn report_refuses_duplicate_exact_surface_occupancy() {
        let settings = CombatSettings::default();
        let mut report = complete_report();
        report.deployment.hostiles = report.deployment.players.clone();
        assert!(report.validate(&settings).is_err());
    }

    #[test]
    fn report_refuses_summary_drift() {
        let settings = CombatSettings::default();
        let mut report = complete_report();
        report.summary.downings = 1;
        assert!(report.validate(&settings).is_err());
    }

    #[test]
    fn history_is_bounded_and_separate_by_schema() {
        let settings = CombatSettings::default();
        let report = complete_report();
        let history = CombatLabReportHistory {
            reports: vec![SavedCombatLabReport {
                id: CombatLabReportId(1),
                report,
            }],
            next_id: 2,
            ..CombatLabReportHistory::default()
        };
        assert_eq!(history.validate(&settings), Ok(()));

        let mut invalid = history;
        let duplicate = invalid
            .reports
            .first()
            .expect("history fixture has one report")
            .clone();
        invalid.reports.push(duplicate);
        assert!(invalid.validate(&settings).is_err());
    }

    #[test]
    fn stacked_surfaces_are_distinct_deployment_positions() {
        let settings = CombatSettings::default();
        let mut report = complete_report();
        report.deployment.hostiles = vec![TilePos::new(HexCoord::ORIGIN, 5)];
        assert_eq!(report.validate(&settings), Ok(()));
        assert_eq!(report.rosters.players.len(), 1);
        assert_eq!(report.rosters.hostiles.len(), 1);
    }

    #[test]
    fn explicitly_saved_reports_round_trip_and_confirmed_delete_persists() {
        let root = std::env::temp_dir().join(format!(
            "hex-game-combat-reports-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let paths = paths(root);
        let shipped = CombatSettings::default();
        let mut store = CombatLabReportStore::default();
        let id = store
            .save(complete_report(), &shipped, &paths)
            .expect("explicit save");
        let loaded: CombatLabReportHistory =
            ron::from_str(&read(&paths.combat_reports).expect("read")).expect("parse");
        assert_eq!(loaded, store.history);

        store
            .delete(id, &shipped, &paths)
            .expect("confirmed delete");
        let deleted: CombatLabReportHistory =
            ron::from_str(&read(&paths.combat_reports).expect("read")).expect("parse");
        assert!(deleted.reports.is_empty());
        drop(std::fs::remove_file(&paths.combat_reports));
        if let Some(parent) = paths.combat_reports.parent() {
            drop(std::fs::remove_dir(parent));
        }
    }

    #[test]
    fn corrupt_history_is_preserved_and_never_replaces_valid_memory() {
        let root = std::env::temp_dir().join(format!(
            "hex-game-corrupt-combat-reports-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let paths = paths(root);
        write_atomic(&paths.combat_reports, "not valid ron").expect("write corruption");
        let mut app = App::new();
        app.insert_resource(paths.clone())
            .insert_resource(CombatSettings::default())
            .init_resource::<CombatLabReportStore>()
            .add_systems(Startup, load_report_history);
        app.update();

        let store = app.world().resource::<CombatLabReportStore>();
        assert!(store.history.reports.is_empty());
        assert!(store
            .error
            .as_deref()
            .is_some_and(|error| error.contains("preserved")));
        assert_eq!(
            read(&paths.combat_reports).expect("corruption remains on disk"),
            "not valid ron"
        );
        drop(std::fs::remove_file(&paths.combat_reports));
        if let Some(parent) = paths.combat_reports.parent() {
            drop(std::fs::remove_dir(parent));
        }
    }
}
