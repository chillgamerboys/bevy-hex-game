//! Pure Combat Lab editing, report-selection, and re-entry state.

use bevy_ecs::prelude::Resource;
use hex_core::TilePos;
use serde::{Deserialize, Serialize};

/// Maximum units on either Combat Lab side.
pub const MAX_COMBAT_LAB_ROSTER: usize = 6;

/// Top-level Lab surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LabTab {
    /// Human-composed sandbox.
    #[default]
    Sandbox,
    /// Immutable deterministic fixtures.
    Fixtures,
    /// Saved deterministic reports.
    Reports,
}

/// Ordered Sandbox editing step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SandboxStep {
    /// Select a packaged map.
    #[default]
    Map,
    /// Compose ordered sides.
    Rosters,
    /// Select or tune a profile.
    Rules,
}

/// Saved-report presentation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReportMode {
    /// Outcome, duration, and aggregate action summary.
    #[default]
    Overview,
    /// Per-unit results.
    Units,
    /// Spell and persistent-effect activity.
    SpellsEffects,
    /// Ordered event transcript.
    Timeline,
    /// Comparison against one independently selected saved report.
    Compare,
}

impl ReportMode {
    /// Stable UI order and human labels for report surfaces.
    pub const ALL: [(Self, &'static str); 5] = [
        (Self::Overview, "Overview"),
        (Self::Units, "Units"),
        (Self::SpellsEffects, "Spells & Effects"),
        (Self::Timeline, "Timeline"),
        (Self::Compare, "Compare"),
    ];
}

/// Pure saved-report presentation selection.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct ReportViewModel<ReportId> {
    /// Active report surface.
    pub mode: ReportMode,
    /// Saved report selected for comparison.
    pub compare_report: Option<ReportId>,
}

impl<ReportId> Default for ReportViewModel<ReportId> {
    fn default() -> Self {
        Self {
            mode: ReportMode::Overview,
            compare_report: None,
        }
    }
}

impl<ReportId> ReportViewModel<ReportId> {
    /// Selects one presentation surface without changing comparison identity.
    pub fn select_mode(&mut self, mode: ReportMode) {
        self.mode = mode;
    }

    /// Selects one comparison and enters Compare in the same transition.
    pub fn compare_with(&mut self, id: ReportId) {
        self.mode = ReportMode::Compare;
        self.compare_report = Some(id);
    }
}

/// Outcome action whose launch fidelity is a pure gameplay concern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabRunAction {
    /// Requeue the exact retained scenario input.
    RetryExact,
    /// Restore the completed report into editable Sandbox state.
    TuneAgain,
    /// Copy an immutable fixture report into editable Sandbox state.
    CopyToSandbox,
}

/// Validated transition produced by one outcome action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabRunTransition<Scenario, Report> {
    /// Retry the exact scenario, including its resolved seed and encounter.
    RetryExact(Scenario),
    /// Restore the exact frozen report into Sandbox.
    RestoreSandbox(Report),
}

/// Typed reason an outcome action cannot be performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabRunFailure {
    /// The exact scenario launch input was not retained.
    MissingScenario,
    /// The frozen report was not retained.
    MissingReport,
    /// Copy is fixture-only; Sandbox results use Tune instead.
    CopyRequiresFixture,
}

/// Resolves outcome routing without observing widgets, resources, or navigation.
pub fn resolve_lab_run<Scenario: Clone, Report: Clone>(
    action: LabRunAction,
    scenario: Option<&Scenario>,
    report: Option<&Report>,
    is_fixture: impl FnOnce(&Report) -> bool,
) -> Result<LabRunTransition<Scenario, Report>, LabRunFailure> {
    match action {
        LabRunAction::RetryExact => scenario
            .cloned()
            .map(LabRunTransition::RetryExact)
            .ok_or(LabRunFailure::MissingScenario),
        LabRunAction::TuneAgain => report
            .cloned()
            .map(LabRunTransition::RestoreSandbox)
            .ok_or(LabRunFailure::MissingReport),
        LabRunAction::CopyToSandbox => {
            let report = report.ok_or(LabRunFailure::MissingReport)?;
            if !is_fixture(report) {
                return Err(LabRunFailure::CopyRequiresFixture);
            }
            Ok(LabRunTransition::RestoreSandbox(report.clone()))
        }
    }
}

/// One ordered roster choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RosterChoice<CustomId> {
    /// Packaged runtime archetype key.
    Template(String),
    /// User-authored local character.
    Custom(CustomId),
    /// Fixture-packaged character using the same stable id vocabulary.
    Packaged(CustomId),
}

/// Exact ordered deployment frozen by a run or edited deployment.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CombatLabReportDeployment {
    /// Player surfaces corresponding one-to-one with the player roster.
    pub players: Vec<TilePos>,
    /// Hostile surfaces corresponding one-to-one with the hostile roster.
    pub hostiles: Vec<TilePos>,
}

/// Stable local report id.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CombatLabReportId(
    /// Monotonic file-local numeric identity.
    pub u64,
);

/// Primitive restore payload produced from an immutable report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxRestore<Profile, CustomId> {
    /// Stable map catalog id.
    pub map: String,
    /// Ordered player side.
    pub players: Vec<RosterChoice<CustomId>>,
    /// Ordered hostile side.
    pub hostiles: Vec<RosterChoice<CustomId>>,
    /// Frozen validated profile.
    pub rules: Profile,
    /// Exact deployment.
    pub deployment: CombatLabReportDeployment,
}

/// Pure mutations that need no catalog, filesystem, or navigation side effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CombatLabEdit<CustomId> {
    /// Switch top-level surface.
    Tab(LabTab),
    /// Switch Sandbox step.
    SandboxStep(SandboxStep),
    /// Select a packaged map.
    SelectMap(String),
    /// Append one player choice.
    AddPlayer(RosterChoice<CustomId>),
    /// Append one hostile choice.
    AddHostile(RosterChoice<CustomId>),
    /// Remove one player slot.
    RemovePlayer(usize),
    /// Remove one hostile slot.
    RemoveHostile(usize),
    /// Move one player slot by a signed adjacent delta.
    MovePlayer(usize, i8),
    /// Move one hostile slot by a signed adjacent delta.
    MoveHostile(usize, i8),
    /// Choose the left saved comparison.
    SelectCompareLeft(CombatLabReportId),
    /// Choose the right saved comparison.
    SelectCompareRight(CombatLabReportId),
    /// Open destructive confirmation for one report.
    RequestReportDelete(CombatLabReportId),
    /// Cancel destructive confirmation.
    CancelReportDelete,
}

/// Authoritative editable Combat Lab state.
#[derive(Resource, Debug)]
pub struct CombatLabModel<Profile, CustomId> {
    /// Active top-level surface.
    pub tab: LabTab,
    /// Active Sandbox step.
    pub sandbox_step: SandboxStep,
    /// Stable packaged map id.
    pub map: String,
    /// Ordered player side.
    pub players: Vec<RosterChoice<CustomId>>,
    /// Ordered hostile side.
    pub hostiles: Vec<RosterChoice<CustomId>>,
    /// Fixture search text.
    pub fixture_filter: String,
    /// Whether navigation began in Creator.
    pub creator_origin: bool,
    /// Selected frozen rules.
    pub rules: Option<Profile>,
    /// Deployment retained only while all launch-defining inputs remain unchanged.
    pub preserved_deployment: Option<CombatLabReportDeployment>,
    /// Independently selected left comparison.
    pub compare_left: Option<CombatLabReportId>,
    /// Independently selected right comparison.
    pub compare_right: Option<CombatLabReportId>,
    /// Report awaiting destructive confirmation.
    pub pending_report_delete: Option<CombatLabReportId>,
    /// Current actionable notice.
    pub notice: String,
    /// Monotonic view invalidation token.
    pub revision: u64,
}

impl<Profile, CustomId> Default for CombatLabModel<Profile, CustomId> {
    fn default() -> Self {
        Self {
            tab: LabTab::Sandbox,
            sandbox_step: SandboxStep::Map,
            map: "flat-arena".to_owned(),
            players: vec![RosterChoice::Template("hedge-mage".to_owned())],
            hostiles: vec![RosterChoice::Template("raider".to_owned())],
            fixture_filter: String::new(),
            creator_origin: false,
            rules: None,
            preserved_deployment: None,
            compare_left: None,
            compare_right: None,
            pending_report_delete: None,
            notice: String::new(),
            revision: 1,
        }
    }
}

impl<Profile, CustomId> CombatLabModel<Profile, CustomId> {
    /// Applies one pure edit and invalidates the immutable view projection once.
    pub fn apply(&mut self, edit: CombatLabEdit<CustomId>) {
        match edit {
            CombatLabEdit::Tab(tab) => {
                self.tab = tab;
                self.pending_report_delete = None;
            }
            CombatLabEdit::SandboxStep(step) => {
                self.sandbox_step = step;
                self.notice.clear();
            }
            CombatLabEdit::SelectMap(map) => {
                self.map = map;
                self.invalidate_deployment();
            }
            CombatLabEdit::AddPlayer(choice) => {
                if self.players.len() < MAX_COMBAT_LAB_ROSTER {
                    self.players.push(choice);
                    self.invalidate_deployment();
                }
            }
            CombatLabEdit::AddHostile(choice) => {
                if self.hostiles.len() < MAX_COMBAT_LAB_ROSTER {
                    self.hostiles.push(choice);
                    self.invalidate_deployment();
                }
            }
            CombatLabEdit::RemovePlayer(index) => {
                remove_at(&mut self.players, index);
                self.invalidate_deployment();
            }
            CombatLabEdit::RemoveHostile(index) => {
                remove_at(&mut self.hostiles, index);
                self.invalidate_deployment();
            }
            CombatLabEdit::MovePlayer(index, delta) => {
                move_at(&mut self.players, index, delta);
                self.invalidate_deployment();
            }
            CombatLabEdit::MoveHostile(index, delta) => {
                move_at(&mut self.hostiles, index, delta);
                self.invalidate_deployment();
            }
            CombatLabEdit::SelectCompareLeft(id) => {
                self.compare_left = Some(id);
                self.notice = format!("Report {} selected as the left comparison.", id.0);
            }
            CombatLabEdit::SelectCompareRight(id) => {
                self.compare_right = Some(id);
                self.notice = format!("Report {} selected as the right comparison.", id.0);
            }
            CombatLabEdit::RequestReportDelete(id) => {
                self.pending_report_delete = Some(id);
                self.notice = format!("Confirm deletion of saved report {} or cancel.", id.0);
            }
            CombatLabEdit::CancelReportDelete => {
                self.pending_report_delete = None;
                self.notice = "Report deletion cancelled.".to_owned();
            }
        }
        self.bump();
    }

    /// Restores all launch-defining report facts into Sandbox exactly once.
    pub fn restore_sandbox(&mut self, restore: SandboxRestore<Profile, CustomId>) {
        self.tab = LabTab::Sandbox;
        self.sandbox_step = SandboxStep::Rules;
        self.map = restore.map;
        self.players = restore.players;
        self.hostiles = restore.hostiles;
        self.rules = Some(restore.rules);
        self.preserved_deployment = Some(restore.deployment);
        self.notice =
            "Frozen map, rosters, profile, and exact deployment copied to Sandbox.".to_owned();
        self.creator_origin = false;
        self.bump();
    }

    /// Records successful persistent deletion and removes stale selections.
    pub fn confirm_report_deleted(&mut self, id: CombatLabReportId) {
        self.pending_report_delete = None;
        if self.compare_left == Some(id) {
            self.compare_left = None;
        }
        if self.compare_right == Some(id) {
            self.compare_right = None;
        }
        self.notice = format!("Saved report {} deleted.", id.0);
        self.bump();
    }

    /// Invalidates immutable UI projection after an effectful adapter transition.
    pub fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn invalidate_deployment(&mut self) {
        self.preserved_deployment = None;
    }
}

fn remove_at<T>(items: &mut Vec<T>, index: usize) {
    if index < items.len() {
        items.remove(index);
    }
}

fn move_at<T>(items: &mut [T], index: usize, delta: i8) {
    let other = if delta < 0 {
        index.checked_sub(1)
    } else {
        index.checked_add(1)
    };
    if let Some(other) = other.filter(|other| *other < items.len()) {
        items.swap(index, other);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::{HexCoord, TilePos};

    fn deployment() -> CombatLabReportDeployment {
        CombatLabReportDeployment {
            players: vec![TilePos::new(HexCoord::ORIGIN, 1)],
            hostiles: vec![TilePos::new(HexCoord::from_axial(2, 0), 3)],
        }
    }

    #[test]
    fn roster_edits_are_bounded_ordered_and_invalidate_deployment() {
        let mut model: CombatLabModel<(), u64> = CombatLabModel {
            preserved_deployment: Some(deployment()),
            ..Default::default()
        };
        model.apply(CombatLabEdit::AddPlayer(RosterChoice::Template(
            "second".to_owned(),
        )));
        assert_eq!(
            model.players,
            [
                RosterChoice::Template("hedge-mage".to_owned()),
                RosterChoice::Template("second".to_owned())
            ]
        );
        assert!(model.preserved_deployment.is_none());

        for index in 0..10 {
            model.apply(CombatLabEdit::AddPlayer(RosterChoice::Template(format!(
                "unit-{index}"
            ))));
        }
        assert_eq!(model.players.len(), MAX_COMBAT_LAB_ROSTER);
        model.apply(CombatLabEdit::MovePlayer(5, -1));
        assert_eq!(
            model.players.get(4),
            Some(&RosterChoice::Template("unit-3".to_owned()))
        );
    }

    #[test]
    fn report_comparisons_are_independent_and_deletion_clears_only_the_deleted_id() {
        let mut model: CombatLabModel<(), u64> = CombatLabModel::default();
        let left = CombatLabReportId(0);
        let right = CombatLabReportId(9);
        model.apply(CombatLabEdit::SelectCompareLeft(left));
        model.apply(CombatLabEdit::SelectCompareRight(right));
        assert_eq!(model.compare_left, Some(left));
        assert_eq!(model.compare_right, Some(right));

        model.apply(CombatLabEdit::RequestReportDelete(left));
        model.confirm_report_deleted(left);
        assert_eq!(model.compare_left, None);
        assert_eq!(model.compare_right, Some(right));
        assert_eq!(model.pending_report_delete, None);
    }

    #[test]
    fn sandbox_restore_preserves_exact_stacked_deployment_and_profile() {
        let profile = 2_u32;
        let expected = deployment();
        let mut model: CombatLabModel<u32, u64> = CombatLabModel::default();
        model.restore_sandbox(SandboxRestore {
            map: "stacked".to_owned(),
            players: vec![RosterChoice::Custom(0)],
            hostiles: vec![RosterChoice::Template("raider".to_owned())],
            rules: profile,
            deployment: expected.clone(),
        });
        assert_eq!(model.map, "stacked");
        assert_eq!(model.rules, Some(profile));
        assert_eq!(model.preserved_deployment, Some(expected));
        assert_eq!(model.players, [RosterChoice::Custom(0)]);
    }

    #[test]
    fn report_view_selection_preserves_valid_zero_identity() {
        let mut view = ReportViewModel::default();
        view.compare_with(CombatLabReportId(0));
        assert_eq!(view.mode, ReportMode::Compare);
        assert_eq!(view.compare_report, Some(CombatLabReportId(0)));
        view.select_mode(ReportMode::Timeline);
        assert_eq!(view.mode, ReportMode::Timeline);
        assert_eq!(view.compare_report, Some(CombatLabReportId(0)));
    }

    #[test]
    fn retry_tune_and_copy_preserve_identity_and_fail_closed() {
        #[derive(Debug, Clone, PartialEq, Eq)]
        struct Scenario {
            seed: u64,
        }
        #[derive(Debug, Clone, PartialEq, Eq)]
        struct Report {
            fingerprint: u64,
            fixture: bool,
        }

        let scenario = Scenario { seed: 9_001 };
        let fixture = Report {
            fingerprint: 44,
            fixture: true,
        };
        let sandbox = Report {
            fingerprint: 45,
            fixture: false,
        };
        let is_fixture = |report: &Report| report.fixture;

        assert_eq!(
            resolve_lab_run(
                LabRunAction::RetryExact,
                Some(&scenario),
                Some(&fixture),
                is_fixture
            ),
            Ok(LabRunTransition::RetryExact(scenario))
        );
        assert_eq!(
            resolve_lab_run(
                LabRunAction::TuneAgain,
                None::<&Scenario>,
                Some(&sandbox),
                is_fixture
            ),
            Ok(LabRunTransition::RestoreSandbox(sandbox.clone()))
        );
        assert_eq!(
            resolve_lab_run(
                LabRunAction::CopyToSandbox,
                None::<&Scenario>,
                Some(&sandbox),
                is_fixture
            ),
            Err(LabRunFailure::CopyRequiresFixture)
        );
        assert_eq!(
            resolve_lab_run::<Scenario, Report>(
                LabRunAction::RetryExact,
                None,
                Some(&fixture),
                is_fixture
            ),
            Err(LabRunFailure::MissingScenario)
        );
        assert_eq!(
            resolve_lab_run::<Scenario, Report>(
                LabRunAction::TuneAgain,
                Some(&Scenario { seed: 1 }),
                None,
                is_fixture
            ),
            Err(LabRunFailure::MissingReport)
        );
    }
}
