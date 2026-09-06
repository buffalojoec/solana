//! What a run produced.

use crate::{
    entry::Entry,
    extraction::ExtractionRecord,
    invariants::{Severity, Violation},
};

pub struct Report {
    pub violations: Vec<Violation>,
    pub extractions: Vec<ExtractionRecord>,
    pub fingerprint: Vec<Entry>,
}

impl Report {
    pub fn failed(&self) -> bool {
        self.violations
            .iter()
            .any(|violation| violation.severity == Severity::Critical)
    }

    pub fn describe(&self) -> String {
        self.violations
            .iter()
            .map(|violation| {
                format!(
                    "[{:?}] {}: {}",
                    violation.severity, violation.invariant, violation.detail
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn assert_clean(&self) {
        assert!(!self.failed(), "invariants violated:\n{}", self.describe());
    }
}
