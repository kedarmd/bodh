//! Finding evidence is readable source; suggestion locations must additionally be inline anchors.
use crate::{ContractError, OperationControl, PreparedEvidenceContext, ReviewResult, read_source};
use std::{collections::HashSet, path::Path};

pub fn validate_findings(
    input: &[u8],
    prepared: &PreparedEvidenceContext,
    repository: &Path,
    control: &OperationControl,
) -> Result<ReviewResult, ContractError> {
    let result = crate::validate_evidence(input, prepared, repository, control)?;
    let mut ids = HashSet::new();
    for (i, f) in result.findings.iter().enumerate() {
        let p = format!("/findings/{i}");
        require_id(&f.id, &format!("{p}/id"))?;
        if !ids.insert(f.id.as_str()) {
            return Err(ContractError::new(
                "ID_DUPLICATE",
                "Finding IDs must be unique",
                &format!("{p}/id"),
            ));
        }
        for (field, text) in [
            ("title", &f.title),
            ("summary", &f.summary),
            ("reason", &f.reason),
            ("impact", &f.impact),
        ] {
            require_text(text, &format!("{p}/{field}"))?;
        }
        if f.evidence.is_empty() {
            return Err(ContractError::new(
                "FINDING_EVIDENCE_REQUIRED",
                "Finding needs explained source evidence",
                &format!("{p}/evidence"),
            ));
        }
        for (j, e) in f.evidence.iter().enumerate() {
            require_text(&e.explanation, &format!("{p}/evidence/{j}/explanation"))?;
            read_source(repository, &prepared.context, &e.source, control)
                .map_err(|e| at(e, &format!("{p}/evidence/{j}/source")))?;
        }
        if f.locations.iter().filter(|l| l.primary).count() != 1 {
            return Err(ContractError::new(
                "PRIMARY_LOCATION_INVALID",
                "Exactly one finding location must be primary",
                &format!("{p}/locations"),
            ));
        }
        for (j, l) in f.locations.iter().enumerate() {
            read_source(repository, &prepared.context, &l.source, control)
                .map_err(|e| at(e, &format!("{p}/locations/{j}/source")))?;
        }
        let mut choices = HashSet::new();
        for (j, suggestion) in f.suggestions.iter().enumerate() {
            let (id, body) = match suggestion {
                crate::Suggestion::TextComment { id, body, .. }
                | crate::Suggestion::CodeReplacement { id, body, .. } => (id, body),
            };
            require_id(id, &format!("{p}/suggestions/{j}/id"))?;
            if !choices.insert(id.as_str()) {
                return Err(ContractError::new(
                    "ID_DUPLICATE",
                    "Alternative IDs must be unique within a finding",
                    &format!("{p}/suggestions/{j}/id"),
                ));
            }
            require_text(body, &format!("{p}/suggestions/{j}/body"))?;
        }
    }

    for (i, assessment) in result.overview.intent_assessments.iter().enumerate() {
        let mut referenced = HashSet::new();
        for (j, id) in assessment.finding_ids.iter().enumerate() {
            if !ids.contains(id.as_str()) || !referenced.insert(id.as_str()) {
                return Err(ContractError::new(
                    "FINDING_REFERENCE_INVALID",
                    "Reference must name a distinct current finding",
                    &format!("/overview/intent_assessments/{i}/finding_ids/{j}"),
                ));
            }
        }
    }
    control.check()?;
    Ok(result)
}
pub(crate) fn require_id(id: &str, pointer: &str) -> Result<(), ContractError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err(ContractError::new(
            "ID_INVALID",
            "IDs use 1–128 ASCII letters, digits, underscores or hyphens",
            pointer,
        ));
    }
    Ok(())
}
fn require_text(text: &str, pointer: &str) -> Result<(), ContractError> {
    if text.trim().is_empty() {
        return Err(ContractError::new(
            "FINDING_CONTENT_INVALID",
            "Expected nonempty explanatory text",
            pointer,
        ));
    }
    Ok(())
}
fn at(mut error: ContractError, prefix: &str) -> ContractError {
    error.pointer = format!("{prefix}{}", error.pointer);
    error
}
