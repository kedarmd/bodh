//! Structured analysis data. Publication, coordinates and layout are intentionally absent.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SchemaVersion {
    #[serde(rename = "1.0")]
    V1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewResult {
    pub schema_version: SchemaVersion,
    pub context: ReviewContext,
    pub overview: Overview,
    #[serde(deserialize_with = "crate::bounded_list")]
    pub diagrams: Vec<Diagram>,
    #[serde(deserialize_with = "crate::bounded_list")]
    pub findings: Vec<Finding>,
    #[serde(deserialize_with = "crate::bounded_list")]
    pub follow_up: Vec<FollowUp>,
    pub coverage: Coverage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrIdentity {
    #[serde(deserialize_with = "crate::bounded_text")]
    pub host: String,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub owner: String,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub repository: String,
    pub number: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewContext {
    pub pr: PrIdentity,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub context_id: String,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub base_sha: String,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub head_sha: String,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub merge_base_sha: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    #[serde(deserialize_with = "crate::bounded_text")]
    pub path: String,
    pub revision: Revision,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub source: SourceRef,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Overview {
    #[serde(deserialize_with = "crate::bounded_text")]
    pub implementation_summary: String,
    #[serde(deserialize_with = "crate::bounded_list")]
    pub key_changes: Vec<KeyChange>,
    #[serde(deserialize_with = "crate::bounded_list")]
    pub intent_assessments: Vec<IntentAssessment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyChange {
    #[serde(deserialize_with = "crate::bounded_text")]
    pub summary: String,
    #[serde(deserialize_with = "crate::bounded_list")]
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntentAssessment {
    #[serde(deserialize_with = "crate::bounded_text")]
    pub quote: String,
    pub occurrence: u32,
    pub status: Alignment,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub explanation: String,
    #[serde(deserialize_with = "crate::bounded_list")]
    pub evidence: Vec<Evidence>,
    #[serde(deserialize_with = "crate::bounded_text_list")]
    pub finding_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    #[serde(deserialize_with = "crate::bounded_text_list")]
    pub examined_areas: Vec<String>,
    #[serde(deserialize_with = "crate::bounded_list")]
    pub checks: Vec<Check>,
    #[serde(deserialize_with = "crate::bounded_list")]
    pub excluded_areas: Vec<Exclusion>,
    #[serde(deserialize_with = "crate::bounded_text_list")]
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Check {
    #[serde(deserialize_with = "crate::bounded_text")]
    pub name: String,
    pub status: CheckStatus,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub details: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Exclusion {
    #[serde(deserialize_with = "crate::bounded_text")]
    pub area: String,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    #[serde(deserialize_with = "crate::bounded_text")]
    pub id: String,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub title: String,
    pub severity: Severity,
    pub confidence: Confidence,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub summary: String,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub reason: String,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub impact: String,
    #[serde(deserialize_with = "crate::bounded_list")]
    pub evidence: Vec<Evidence>,
    #[serde(deserialize_with = "crate::bounded_list")]
    pub locations: Vec<Location>,
    #[serde(deserialize_with = "crate::bounded_list")]
    pub suggestions: Vec<Suggestion>,
    #[serde(deserialize_with = "crate::bounded_text_list")]
    pub related_thread_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Location {
    pub source: SourceRef,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FollowUp {
    #[serde(deserialize_with = "crate::bounded_text")]
    pub prior_concern_id: String,
    pub status: FollowUpStatus,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub explanation: String,
    #[serde(deserialize_with = "crate::bounded_list")]
    pub evidence: Vec<Evidence>,
    #[serde(default, deserialize_with = "crate::bounded_optional_text")]
    pub finding_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Diagram {
    #[serde(deserialize_with = "crate::bounded_text")]
    pub id: String,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub title: String,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub description: String,
    #[serde(deserialize_with = "crate::bounded_list")]
    pub participants: Vec<Participant>,
    #[serde(deserialize_with = "crate::bounded_list")]
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Participant {
    #[serde(deserialize_with = "crate::bounded_text")]
    pub id: String,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub label: String,
    pub category: Option<ParticipantCategory>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Branch {
    #[serde(deserialize_with = "crate::bounded_text")]
    pub label: String,
    #[serde(deserialize_with = "crate::bounded_list")]
    pub steps: Vec<SimpleStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Revision {
    Head,
    Base,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    Aligned,
    Partial,
    Mismatch,
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    Failed,
    NotRun,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FollowUpStatus {
    Resolved,
    Unresolved,
    Regressed,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantCategory {
    Actor,
    Service,
    Database,
    Queue,
    External,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Suggestion {
    TextComment {
        #[serde(deserialize_with = "crate::bounded_text")]
        id: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        body: String,
        source: SourceRef,
    },
    CodeReplacement {
        #[serde(deserialize_with = "crate::bounded_text")]
        id: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        body: String,
        source: SourceRef,
        #[serde(deserialize_with = "crate::bounded_text")]
        original_text: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        replacement: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SimpleStep {
    Message {
        #[serde(deserialize_with = "crate::bounded_text")]
        id: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        from: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        to: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        label: String,
        source: Option<SourceRef>,
    },
    Return {
        #[serde(deserialize_with = "crate::bounded_text")]
        id: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        from: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        to: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        label: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        in_reply_to: String,
        source: Option<SourceRef>,
    },
    SelfCall {
        #[serde(deserialize_with = "crate::bounded_text")]
        id: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        participant: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        label: String,
        source: Option<SourceRef>,
    },
    Note {
        #[serde(deserialize_with = "crate::bounded_text")]
        id: String,
        #[serde(deserialize_with = "crate::bounded_text_list")]
        participants: Vec<String>,
        #[serde(deserialize_with = "crate::bounded_text")]
        text: String,
        source: Option<SourceRef>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Message {
        #[serde(deserialize_with = "crate::bounded_text")]
        id: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        from: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        to: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        label: String,
        source: Option<SourceRef>,
    },
    Return {
        #[serde(deserialize_with = "crate::bounded_text")]
        id: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        from: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        to: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        label: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        in_reply_to: String,
        source: Option<SourceRef>,
    },
    SelfCall {
        #[serde(deserialize_with = "crate::bounded_text")]
        id: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        participant: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        label: String,
        source: Option<SourceRef>,
    },
    Note {
        #[serde(deserialize_with = "crate::bounded_text")]
        id: String,
        #[serde(deserialize_with = "crate::bounded_text_list")]
        participants: Vec<String>,
        #[serde(deserialize_with = "crate::bounded_text")]
        text: String,
        source: Option<SourceRef>,
    },
    Alt {
        #[serde(deserialize_with = "crate::bounded_text")]
        id: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        label: String,
        #[serde(deserialize_with = "crate::bounded_list")]
        branches: Vec<Branch>,
    },
    Opt {
        #[serde(deserialize_with = "crate::bounded_text")]
        id: String,
        #[serde(deserialize_with = "crate::bounded_text")]
        label: String,
        #[serde(deserialize_with = "crate::bounded_list")]
        steps: Vec<SimpleStep>,
    },
}
