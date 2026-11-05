use bodh::{
    Alignment, ChangedFile, CommittedGeneratedReview, ContractError, ReviewContext, Revision,
    SourceRef,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommittedOverviewStatus {
    #[default]
    Missing,
    Ready,
    StaleContext,
    LoadFailed,
}

#[derive(Clone, Debug, Default)]
pub struct CommittedOverviewView {
    pub committed: Option<CommittedGeneratedReview>,
    pub status: CommittedOverviewStatus,
    pub error_code: Option<String>,
}

impl CommittedOverviewView {
    pub fn ready(committed: CommittedGeneratedReview) -> Self {
        Self {
            committed: Some(committed),
            status: CommittedOverviewStatus::Ready,
            error_code: None,
        }
    }
}

/// Apply an atomic current-result read while keeping errors from erasing a
/// previously displayed result for the same pinned context.
pub fn apply_committed_overview_load(
    expected: &ReviewContext,
    previous: &CommittedOverviewView,
    loaded: Result<Option<CommittedGeneratedReview>, ContractError>,
) -> CommittedOverviewView {
    match loaded {
        Ok(Some(committed)) if committed_matches_context(&committed, expected) => {
            CommittedOverviewView::ready(committed)
        }
        Ok(Some(_)) => CommittedOverviewView {
            committed: None,
            status: CommittedOverviewStatus::StaleContext,
            error_code: None,
        },
        Ok(None) => CommittedOverviewView::default(),
        Err(error) => CommittedOverviewView {
            committed: previous
                .committed
                .as_ref()
                .filter(|committed| committed_matches_context(committed, expected))
                .cloned(),
            status: CommittedOverviewStatus::LoadFailed,
            error_code: Some(error.code),
        },
    }
}

pub fn committed_matches_context(
    committed: &CommittedGeneratedReview,
    expected: &ReviewContext,
) -> bool {
    committed.context.context == *expected && committed.result.context == *expected
}

pub fn is_significant_assessment(status: &Alignment) -> bool {
    *status != Alignment::Aligned
}

pub fn evidence_file_index(changed_files: &[ChangedFile], source: &SourceRef) -> Option<usize> {
    changed_files.iter().position(|file| {
        (match source.revision {
            Revision::Base => file.old_path.as_deref(),
            Revision::Head => file.new_path.as_deref(),
        }) == Some(source.path.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CommittedOverviewStatus, CommittedOverviewView, apply_committed_overview_load,
        evidence_file_index, is_significant_assessment,
    };
    use bodh::{
        Alignment, ChangedFile, CommittedGeneratedReview, PreparedEvidenceContext, ReviewContext,
        ReviewResult, Revision, SourceRef,
    };

    fn context(id: &str) -> ReviewContext {
        serde_json::from_value(serde_json::json!({
            "pr": {"host":"github.com","owner":"acme","repository":"widget","number":7},
            "context_id":id,
            "base_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "head_sha":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "merge_base_sha":"cccccccccccccccccccccccccccccccccccccccc"
        }))
        .unwrap()
    }

    fn committed(context: ReviewContext) -> CommittedGeneratedReview {
        let mut result: ReviewResult =
            serde_json::from_str(include_str!("../../../contracts/v1/fixtures/empty.json"))
                .unwrap();
        result.context = context.clone();
        CommittedGeneratedReview {
            result,
            context: PreparedEvidenceContext {
                context,
                original_description: Some("Original PR intent".into()),
                discussions: Default::default(),
                prior_concerns: None,
            },
            generation: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn overview_only_exposes_a_commit_bound_to_the_displayed_context() {
        let expected = context("current");
        let stale = committed(context("old"));
        let view = apply_committed_overview_load(
            &expected,
            &CommittedOverviewView::default(),
            Ok(Some(stale)),
        );
        assert_eq!(view.status, CommittedOverviewStatus::StaleContext);
        assert!(view.committed.is_none());
    }

    #[test]
    fn a_failed_refresh_keeps_the_last_committed_view_for_the_same_context() {
        let expected = context("current");
        let previous = CommittedOverviewView::ready(committed(expected.clone()));
        let view = apply_committed_overview_load(
            &expected,
            &previous,
            Err(bodh::ContractError::new(
                "STATE_UNAVAILABLE",
                "Review state is unavailable",
                "",
            )),
        );
        assert_eq!(view.status, CommittedOverviewStatus::LoadFailed);
        assert!(view.committed.is_some());
    }

    #[test]
    fn non_aligned_assessments_are_significant_and_evidence_uses_exact_side_paths() {
        assert!(!is_significant_assessment(&Alignment::Aligned));
        for status in [
            Alignment::Partial,
            Alignment::Mismatch,
            Alignment::Unverified,
        ] {
            assert!(is_significant_assessment(&status));
        }
        let files = vec![ChangedFile {
            old_path: Some("old/name.rs".into()),
            new_path: Some("src/name.rs".into()),
            old_mode: "100644".into(),
            new_mode: "100644".into(),
            old_oid: "a".repeat(40),
            new_oid: "b".repeat(40),
            status: "R100".into(),
            unsupported_reason: None,
            hunks: Vec::new(),
        }];
        let head_source = SourceRef {
            path: "src/name.rs".into(),
            revision: Revision::Head,
            start_line: 4,
            end_line: 4,
        };
        assert_eq!(evidence_file_index(&files, &head_source), Some(0));
        let wrong_side = SourceRef {
            revision: Revision::Base,
            ..head_source
        };
        assert_eq!(evidence_file_index(&files, &wrong_side), None);
    }
}
