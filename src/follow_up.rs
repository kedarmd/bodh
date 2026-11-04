//! Follow-up assessments link a validated result to the actual prior review context.
use crate::{
    ContractError, FollowUpStatus, OperationControl, PreparedEvidenceContext, PriorConcern,
    ReviewResult, read_source,
};
use std::{collections::HashSet, path::Path};

pub(crate) fn validate_follow_up(
    result: &ReviewResult,
    prepared: &PreparedEvidenceContext,
    repository: &Path,
    control: &OperationControl,
) -> Result<(), ContractError> {
    let mut prior_ids = HashSet::new();
    if let Some(prior_concerns) = prepared.prior_concerns.as_ref() {
        for (index, concern) in prior_concerns.iter().enumerate() {
            validate_prior_concern(concern, &format!("/prior_concerns/{index}"))?;
            if !prior_ids.insert(concern.id.as_str()) {
                return Err(follow_up_error(
                    "PRIOR_CONCERN_REFERENCE_INVALID",
                    "Prepared prior-concern IDs must be unique",
                    &format!("/prior_concerns/{index}/id"),
                ));
            }
        }
    }

    if !result.follow_up.is_empty() && prepared.prior_concerns.is_none() {
        return Err(follow_up_error(
            "PRIOR_RESULT_UNAVAILABLE",
            "Follow-up assessments require a prepared prior result",
            "/follow_up",
        ));
    }

    let current_ids: HashSet<&str> = result
        .findings
        .iter()
        .map(|finding| finding.id.as_str())
        .collect();
    let mut assessed = HashSet::new();
    for (index, assessment) in result.follow_up.iter().enumerate() {
        let pointer = format!("/follow_up/{index}");
        let id_pointer = format!("{pointer}/prior_concern_id");
        crate::findings::require_id(&assessment.prior_concern_id, &id_pointer)?;
        if !prior_ids.contains(assessment.prior_concern_id.as_str()) {
            return Err(follow_up_error(
                "PRIOR_CONCERN_REFERENCE_INVALID",
                "Assessment must reference a concern in the prepared prior result",
                &id_pointer,
            ));
        }
        if !assessed.insert(assessment.prior_concern_id.as_str()) {
            return Err(follow_up_error(
                "FOLLOW_UP_DUPLICATE",
                "Each prior concern may be assessed only once",
                &id_pointer,
            ));
        }
        if assessment.explanation.trim().is_empty() {
            return Err(follow_up_error(
                "FOLLOW_UP_EVIDENCE_REQUIRED",
                "Follow-up assessments need an explanation",
                &format!("{pointer}/explanation"),
            ));
        }
        if assessment.evidence.is_empty() {
            return Err(follow_up_error(
                "FOLLOW_UP_EVIDENCE_REQUIRED",
                "Follow-up assessments need current source evidence",
                &format!("{pointer}/evidence"),
            ));
        }
        for (evidence_index, evidence) in assessment.evidence.iter().enumerate() {
            let evidence_pointer = format!("{pointer}/evidence/{evidence_index}");
            if evidence.explanation.trim().is_empty() {
                return Err(follow_up_error(
                    "FOLLOW_UP_EVIDENCE_REQUIRED",
                    "Follow-up source evidence needs an explanation",
                    &format!("{evidence_pointer}/explanation"),
                ));
            }
            read_source(repository, &prepared.context, &evidence.source, control).map_err(
                |mut error| {
                    error.pointer = format!("{evidence_pointer}/source{}", error.pointer);
                    error
                },
            )?;
        }

        match (assessment.status.clone(), assessment.finding_id.as_deref()) {
            (FollowUpStatus::Resolved, None) => {}
            (FollowUpStatus::Resolved, Some(_)) => {
                return Err(follow_up_error(
                    "FOLLOW_UP_RESOLUTION_INVALID",
                    "Resolved concerns cannot point to a current finding",
                    &format!("{pointer}/finding_id"),
                ));
            }
            (FollowUpStatus::Unresolved | FollowUpStatus::Regressed, None) => {
                return Err(follow_up_error(
                    "FOLLOW_UP_FINDING_REQUIRED",
                    "Unresolved and regressed concerns need a current actionable finding",
                    &format!("{pointer}/finding_id"),
                ));
            }
            (
                FollowUpStatus::Unresolved | FollowUpStatus::Regressed | FollowUpStatus::Uncertain,
                Some(finding_id),
            ) => {
                if !current_ids.contains(finding_id) {
                    return Err(follow_up_error(
                        "FOLLOW_UP_FINDING_INVALID",
                        "Follow-up may only reference a validated current finding",
                        &format!("{pointer}/finding_id"),
                    ));
                }
            }
            (FollowUpStatus::Uncertain, None) => {}
        }
        control.check()?;
    }
    Ok(())
}

fn validate_prior_concern(concern: &PriorConcern, pointer: &str) -> Result<(), ContractError> {
    crate::findings::require_id(&concern.id, &format!("{pointer}/id"))?;
    if concern.title.trim().is_empty() || concern.summary.trim().is_empty() {
        return Err(follow_up_error(
            "PREPARED_CONTEXT_INVALID",
            "Prepared prior concerns need a title and summary",
            pointer,
        ));
    }
    Ok(())
}

fn follow_up_error(code: &str, message: &str, pointer: &str) -> ContractError {
    ContractError::new(code, message, pointer)
}
