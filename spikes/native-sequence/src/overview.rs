use bodh::{
    Alignment, ChangedFile, CommittedGeneratedReview, ContractError, Finding, FollowUp,
    FollowUpStatus, PriorConcern, ReviewContext, Revision, SourceRef,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowUpProminence {
    Unassessed,
    Unresolved,
    Regressed,
    Uncertain,
    Resolved,
}

impl FollowUpProminence {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unassessed => "Not assessed",
            Self::Unresolved => "Unresolved",
            Self::Regressed => "Regressed",
            Self::Uncertain => "Uncertain",
            Self::Resolved => "Resolved",
        }
    }

    pub fn is_prominent(self) -> bool {
        !matches!(self, Self::Resolved)
    }
}

pub struct FollowUpConcernRow<'a> {
    pub concern: &'a PriorConcern,
    pub assessment: Option<&'a FollowUp>,
    pub current_finding: Option<&'a Finding>,
    pub prominence: FollowUpProminence,
}

pub fn follow_up_concerns(
    committed: &CommittedGeneratedReview,
) -> Option<Vec<FollowUpConcernRow<'_>>> {
    let prior = committed.context.prior_concerns.as_ref()?;
    let mut rows = prior
        .iter()
        .map(|concern| {
            let assessment = committed
                .result
                .follow_up
                .iter()
                .find(|item| item.prior_concern_id == concern.id);
            let current_finding = assessment
                .and_then(|item| item.finding_id.as_deref())
                .and_then(|id| {
                    committed
                        .result
                        .findings
                        .iter()
                        .find(|finding| finding.id == id)
                });
            let prominence = match assessment.map(|item| &item.status) {
                Some(FollowUpStatus::Unresolved) => FollowUpProminence::Unresolved,
                Some(FollowUpStatus::Regressed) => FollowUpProminence::Regressed,
                Some(FollowUpStatus::Uncertain) => FollowUpProminence::Uncertain,
                Some(FollowUpStatus::Resolved) => FollowUpProminence::Resolved,
                None => FollowUpProminence::Unassessed,
            };
            FollowUpConcernRow {
                concern,
                assessment,
                current_finding,
                prominence,
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| match row.prominence {
        FollowUpProminence::Unassessed
        | FollowUpProminence::Unresolved
        | FollowUpProminence::Regressed => 0,
        FollowUpProminence::Uncertain => 1,
        FollowUpProminence::Resolved => 2,
    });
    Some(rows)
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
        CommittedOverviewStatus, CommittedOverviewView, FollowUpProminence,
        apply_committed_overview_load, evidence_file_index, follow_up_concerns,
        is_significant_assessment,
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

    #[test]
    fn follow_up_rows_order_actionable_uncertain_and_resolved_and_resolve_current_findings() {
        let mut review = committed(context("current"));
        review.context.prior_concerns = Some(
            [
                "resolved",
                "unresolved",
                "regressed",
                "uncertain",
                "unassessed",
            ]
            .into_iter()
            .map(|id| bodh::PriorConcern {
                id: id.into(),
                title: format!("Prior {id}"),
                summary: format!("Summary {id}"),
                evidence: Vec::new(),
            })
            .collect(),
        );
        review.result.findings = serde_json::from_value(serde_json::json!([
            {"id":"F-unresolved","title":"Unresolved","severity":"high","confidence":"high","summary":"s","reason":"r","impact":"i","evidence":[],"locations":[],"suggestions":[],"related_thread_ids":[]},
            {"id":"F-regressed","title":"Regressed","severity":"high","confidence":"high","summary":"s","reason":"r","impact":"i","evidence":[],"locations":[],"suggestions":[],"related_thread_ids":[]}
        ]))
        .unwrap();
        review.result.follow_up = serde_json::from_value(serde_json::json!([
            {"prior_concern_id":"resolved","status":"resolved","explanation":"Fixed","evidence":[],"finding_id":null},
            {"prior_concern_id":"unresolved","status":"unresolved","explanation":"Open","evidence":[],"finding_id":"F-unresolved"},
            {"prior_concern_id":"regressed","status":"regressed","explanation":"Back again","evidence":[],"finding_id":"F-regressed"},
            {"prior_concern_id":"uncertain","status":"uncertain","explanation":"Cannot confirm","evidence":[],"finding_id":null}
        ]))
        .unwrap();

        let rows = follow_up_concerns(&review).unwrap();
        assert_eq!(
            rows.iter().map(|row| row.prominence).collect::<Vec<_>>(),
            [
                FollowUpProminence::Unresolved,
                FollowUpProminence::Regressed,
                FollowUpProminence::Unassessed,
                FollowUpProminence::Uncertain,
                FollowUpProminence::Resolved,
            ]
        );
        assert_eq!(rows[0].current_finding.unwrap().id, "F-unresolved");
        assert_eq!(rows[1].current_finding.unwrap().id, "F-regressed");
        assert!(rows[3].current_finding.is_none());
        assert!(!rows[4].prominence.is_prominent());
        assert!(follow_up_concerns(&committed(context("without-prior"))).is_none());
    }
}
