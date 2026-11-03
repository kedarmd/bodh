//! Structured analysis data. Publication, coordinates and layout are intentionally absent.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SchemaVersion { #[serde(rename = "1.0")] V1 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewResult {
    pub schema_version: SchemaVersion,
    pub context: ReviewContext,
    pub overview: Overview,
    pub diagrams: Vec<Diagram>,
    pub findings: Vec<Finding>,
    pub follow_up: Vec<FollowUp>,
    pub coverage: Coverage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrIdentity {
    pub host: String,
    pub owner: String,
    pub repository: String,
    pub number: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewContext {
    pub pr: PrIdentity,
    pub context_id: String,
    pub base_sha: String,
    pub head_sha: String,
    pub merge_base_sha: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    pub path: String,
    pub revision: Revision,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub source: SourceRef,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Overview {
    pub implementation_summary: String,
    pub key_changes: Vec<KeyChange>,
    pub intent_assessments: Vec<IntentAssessment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyChange {
    pub summary: String,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntentAssessment {
    pub quote: String,
    pub occurrence: u32,
    pub status: Alignment,
    pub explanation: String,
    pub evidence: Vec<Evidence>,
    pub finding_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub examined_areas: Vec<String>,
    pub checks: Vec<Check>,
    pub excluded_areas: Vec<Exclusion>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub name: String,
    pub status: CheckStatus,
    pub details: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Exclusion {
    pub area: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub id: String,
    pub title: String,
    pub severity: Severity,
    pub confidence: Confidence,
    pub summary: String,
    pub reason: String,
    pub impact: String,
    pub evidence: Vec<Evidence>,
    pub locations: Vec<Location>,
    pub suggestions: Vec<Suggestion>,
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
    pub prior_concern_id: String,
    pub status: FollowUpStatus,
    pub explanation: String,
    pub evidence: Vec<Evidence>,
    pub finding_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Diagram {
    pub id: String,
    pub title: String,
    pub description: String,
    pub participants: Vec<Participant>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Participant {
    pub id: String,
    pub label: String,
    pub category: Option<ParticipantCategory>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Branch {
    pub label: String,
    pub steps: Vec<SimpleStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Revision { Head, Base }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Alignment { Aligned, Partial, Mismatch, Unverified }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus { Passed, Failed, NotRun, Unavailable }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity { Critical, High, Medium, Low }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Confidence { High, Medium, Low }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FollowUpStatus { Resolved, Unresolved, Regressed, Uncertain }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantCategory { Actor, Service, Database, Queue, External }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Suggestion {
    TextComment { id: String, body: String, source: SourceRef },
    CodeReplacement { id: String, body: String, source: SourceRef, original_text: String, replacement: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SimpleStep {
    Message {
    id: String,
    from: String,
    to: String,
    label: String,
    source: Option<SourceRef>,
    },
    Return {
    id: String,
    from: String,
    to: String,
    label: String,
    in_reply_to: String,
    source: Option<SourceRef>,
    },
    SelfCall {
    id: String,
    participant: String,
    label: String,
    source: Option<SourceRef>,
    },
    Note {
    id: String,
    participants: Vec<String>,
    text: String,
    source: Option<SourceRef>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Message {
    id: String,
    from: String,
    to: String,
    label: String,
    source: Option<SourceRef>,
    },
    Return {
    id: String,
    from: String,
    to: String,
    label: String,
    in_reply_to: String,
    source: Option<SourceRef>,
    },
    SelfCall {
    id: String,
    participant: String,
    label: String,
    source: Option<SourceRef>,
    },
    Note {
    id: String,
    participants: Vec<String>,
    text: String,
    source: Option<SourceRef>,
    },
    Alt {
    id: String,
    label: String,
    branches: Vec<Branch>,
    },
    Opt {
    id: String,
    label: String,
    steps: Vec<SimpleStep>,
    },
}

