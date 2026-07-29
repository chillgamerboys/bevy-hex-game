//! Snapshot-based undo and redo for editor documents.
//!
//! A transaction stores one baseline snapshot. Any number of model operations may
//! run before commit, so a drag stroke becomes one undo step rather than one step per
//! voxel.

use std::collections::VecDeque;
use std::fmt;

/// Default number of document snapshots retained in each history direction.
pub const DEFAULT_HISTORY_LIMIT: usize = 128;

/// An invalid history operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryError {
    message: String,
}

impl HistoryError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Human-readable failure detail.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for HistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HistoryError {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HistoryEntry<T> {
    label: String,
    snapshot: T,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenTransaction<T> {
    label: String,
    baseline: T,
}

/// Bounded snapshot history with explicit transaction grouping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotHistory<T> {
    limit: usize,
    undo: VecDeque<HistoryEntry<T>>,
    redo: VecDeque<HistoryEntry<T>>,
    transaction: Option<OpenTransaction<T>>,
}

impl<T> Default for SnapshotHistory<T> {
    fn default() -> Self {
        Self {
            limit: DEFAULT_HISTORY_LIMIT,
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            transaction: None,
        }
    }
}

impl<T> SnapshotHistory<T>
where
    T: Clone + PartialEq,
{
    /// Creates an empty history with the requested nonzero snapshot limit.
    pub fn new(limit: usize) -> Result<Self, HistoryError> {
        if limit == 0 {
            return Err(HistoryError::new(
                "history limit must retain at least one snapshot",
            ));
        }
        Ok(Self {
            limit,
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            transaction: None,
        })
    }

    /// Maximum snapshots retained in each direction.
    #[must_use]
    pub const fn limit(&self) -> usize {
        self.limit
    }

    /// Whether a grouped edit is currently open.
    #[must_use]
    pub const fn is_transaction_open(&self) -> bool {
        self.transaction.is_some()
    }

    /// Label of the next undo action.
    #[must_use]
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.back().map(|entry| entry.label.as_str())
    }

    /// Label of the next redo action.
    #[must_use]
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.back().map(|entry| entry.label.as_str())
    }

    /// Begins a grouped transaction at `current`.
    pub fn begin(&mut self, label: impl Into<String>, current: &T) -> Result<(), HistoryError> {
        if self.transaction.is_some() {
            return Err(HistoryError::new(
                "cannot begin an editor transaction while another is open",
            ));
        }
        let label = label.into();
        validate_label(&label)?;
        self.transaction = Some(OpenTransaction {
            label,
            baseline: current.clone(),
        });
        Ok(())
    }

    /// Commits the open transaction as at most one undo step.
    ///
    /// Returns `true` when the document changed from the transaction baseline.
    pub fn commit(&mut self, current: &T) -> Result<bool, HistoryError> {
        let Some(transaction) = self.transaction.take() else {
            return Err(HistoryError::new(
                "cannot commit because no editor transaction is open",
            ));
        };
        if transaction.baseline == *current {
            return Ok(false);
        }
        self.push_undo(HistoryEntry {
            label: transaction.label,
            snapshot: transaction.baseline,
        });
        self.redo.clear();
        Ok(true)
    }

    /// Cancels the open transaction and returns its baseline snapshot.
    pub fn cancel(&mut self) -> Result<T, HistoryError> {
        self.transaction
            .take()
            .map(|transaction| transaction.baseline)
            .ok_or_else(|| HistoryError::new("cannot cancel because no editor transaction is open"))
    }

    /// Records one already-applied atomic edit.
    ///
    /// Returns `true` when `current` differs from `before`. Calls made inside an open
    /// transaction deliberately do not add an intermediate history entry.
    pub fn record_atomic(
        &mut self,
        label: impl Into<String>,
        before: T,
        current: &T,
    ) -> Result<bool, HistoryError> {
        if before == *current {
            return Ok(false);
        }
        if self.transaction.is_some() {
            return Ok(true);
        }
        let label = label.into();
        validate_label(&label)?;
        self.push_undo(HistoryEntry {
            label,
            snapshot: before,
        });
        self.redo.clear();
        Ok(true)
    }

    /// Restores the preceding snapshot, or returns `None` when history is empty.
    pub fn undo(&mut self, current: &T) -> Result<Option<T>, HistoryError> {
        if self.transaction.is_some() {
            return Err(HistoryError::new(
                "cannot undo while an editor transaction is open",
            ));
        }
        let Some(entry) = self.undo.pop_back() else {
            return Ok(None);
        };
        self.push_redo(HistoryEntry {
            label: entry.label,
            snapshot: current.clone(),
        });
        Ok(Some(entry.snapshot))
    }

    /// Restores the next snapshot, or returns `None` when redo history is empty.
    pub fn redo(&mut self, current: &T) -> Result<Option<T>, HistoryError> {
        if self.transaction.is_some() {
            return Err(HistoryError::new(
                "cannot redo while an editor transaction is open",
            ));
        }
        let Some(entry) = self.redo.pop_back() else {
            return Ok(None);
        };
        self.push_undo(HistoryEntry {
            label: entry.label,
            snapshot: current.clone(),
        });
        Ok(Some(entry.snapshot))
    }

    /// Drops all undo, redo, and transaction state.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.transaction = None;
    }

    fn push_undo(&mut self, entry: HistoryEntry<T>) {
        self.undo.push_back(entry);
        trim_to_limit(&mut self.undo, self.limit);
    }

    fn push_redo(&mut self, entry: HistoryEntry<T>) {
        self.redo.push_back(entry);
        trim_to_limit(&mut self.redo, self.limit);
    }
}

fn trim_to_limit<T>(entries: &mut VecDeque<T>, limit: usize) {
    while entries.len() > limit {
        entries.pop_front();
    }
}

fn validate_label(label: &str) -> Result<(), HistoryError> {
    if label.trim().is_empty() {
        return Err(HistoryError::new("editor history labels must be non-empty"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_groups_many_changes_into_one_step() {
        let mut history = SnapshotHistory::default();
        let mut document = vec![1];

        assert!(history.begin("Paint stroke", &document).is_ok());
        let before_first = document.clone();
        document.push(2);
        assert!(history
            .record_atomic("Place voxel", before_first, &document)
            .is_ok());
        let before_second = document.clone();
        document.push(3);
        assert!(history
            .record_atomic("Place voxel", before_second, &document)
            .is_ok());
        assert_eq!(history.undo_label(), None);
        assert_eq!(history.commit(&document), Ok(true));
        assert_eq!(history.undo_label(), Some("Paint stroke"));

        let Ok(Some(restored)) = history.undo(&document) else {
            unreachable!("the stroke should create one undo entry")
        };
        assert_eq!(restored, [1]);
        let Ok(Some(redone)) = history.redo(&restored) else {
            unreachable!("the stroke should create one redo entry")
        };
        assert_eq!(redone, [1, 2, 3]);
    }

    #[test]
    fn cancelled_transaction_returns_the_baseline_without_history() {
        let mut history = SnapshotHistory::default();
        let document = String::from("before");
        assert!(history.begin("Change", &document).is_ok());

        let Ok(restored) = history.cancel() else {
            unreachable!("the transaction is open")
        };
        assert_eq!(restored, "before");
        assert_eq!(history.undo_label(), None);
        assert_eq!(history.redo_label(), None);
    }

    #[test]
    fn atomic_edits_clear_redo_and_history_is_bounded() {
        let Ok(mut history) = SnapshotHistory::new(2) else {
            unreachable!("a two-entry limit is valid")
        };
        let mut document = 0;
        for next in 1..=3 {
            let before = document;
            document = next;
            assert!(history
                .record_atomic(format!("Set {next}"), before, &document)
                .is_ok());
        }

        let Ok(Some(document)) = history.undo(&document) else {
            unreachable!("latest edit should be undoable")
        };
        assert_eq!(document, 2);
        let Ok(Some(document)) = history.undo(&document) else {
            unreachable!("second retained edit should be undoable")
        };
        assert_eq!(document, 1);
        assert_eq!(history.undo(&document), Ok(None));

        let before = document;
        let document = 9;
        assert!(history.record_atomic("Branch", before, &document).is_ok());
        assert_eq!(history.redo(&document), Ok(None));
    }

    #[test]
    fn nested_transactions_and_undo_while_open_are_rejected() {
        let mut history = SnapshotHistory::default();
        assert!(history.begin("Outer", &0).is_ok());
        assert!(history.begin("Nested", &0).is_err());
        assert!(history.undo(&0).is_err());
        assert!(history.redo(&0).is_err());
        assert!(history.commit(&0).is_ok());
        assert!(history.commit(&0).is_err());
    }
}
