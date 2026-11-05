//! Shared headless review contract. Structural validation does not certify evidence.
mod acquisition;
mod diagrams;
mod diff;
mod discussion_acquisition;
mod discussions;
mod drafts;
mod evidence;
pub use diff::*;
pub use discussion_acquisition::*;
pub use discussions::*;
pub use drafts::*;
mod findings;
pub use findings::*;
mod follow_up;
mod model;
mod preparation;
pub use evidence::*;
pub use model::*;
pub use preparation::*;
use serde::Serialize;
use serde_json::{Value, json};

pub const PROTOCOL_VERSION: &str = "1.0";
pub const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_STRING_CHARS: usize = 65_536;
pub const MAX_COLLECTION_ITEMS: usize = 1_024;

/// Validates the complete schema v1 result against its trusted prepared context.
pub fn validate_complete(
    input: &[u8],
    prepared: &PreparedEvidenceContext,
    repository: &std::path::Path,
    control: &OperationControl,
) -> Result<ReviewResult, ContractError> {
    let result = validate_findings(input, prepared, repository, control)?;
    diagrams::validate_diagrams(&result, &prepared.context, repository, control)?;
    follow_up::validate_follow_up(&result, prepared, repository, control)?;
    Ok(result)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContractError {
    pub code: String,
    pub message: String,
    pub pointer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
}
impl ContractError {
    pub fn new(code: &str, message: &str, pointer: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            pointer: pointer.into(),
            line: None,
            column: None,
        }
    }
}

pub fn schema() -> Value {
    let mut schema =
        serde_json::to_value(schemars::schema_for!(ReviewResult)).expect("schema serializes");
    bound_schema(&mut schema);
    schema
}

pub fn validate_structure(input: &[u8]) -> Result<ReviewResult, ContractError> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(ContractError::new(
            "INPUT_TOO_LARGE",
            "Input exceeds 8 MiB",
            "",
        ));
    }
    let value: Value = serde_json::from_slice(input).map_err(|error| {
        let mut failure = ContractError::new("MALFORMED_JSON", "Expected one JSON document", "");
        failure.line = Some(error.line());
        failure.column = Some(error.column());
        failure
    })?;
    if value
        .get("schema_version")
        .and_then(Value::as_str)
        .is_some_and(|v| v != "1.0")
    {
        return Err(ContractError::new(
            "UNSUPPORTED_SCHEMA_VERSION",
            "Supported schema version is 1.0",
            "/schema_version",
        ));
    }
    let validator = jsonschema::validator_for(&schema()).expect("generated schema compiles");
    if let Err(error) = validator.validate(&value) {
        return Err(ContractError::new(
            "CONTRACT_INVALID",
            "Submission does not match the review schema",
            &error.instance_path().to_string(),
        ));
    }
    serde_json::from_slice(input).map_err(|_| {
        ContractError::new(
            "CONTRACT_INVALID",
            "Submission does not match strict Rust representation",
            "",
        )
    })
}

pub fn describe() -> Value {
    json!({"schema_version":"1.0","status":"frozen_contract_fixture_validation_complete","validation":"complete_fixture_context","semantic_validation_complete":true,"production_preparation_validation_available":true,"limits":{"input_bytes":MAX_INPUT_BYTES,"string_characters":MAX_STRING_CHARS,"collection_items":MAX_COLLECTION_ITEMS},"commands":["contract describe","contract schema","prepare --repo <checkout> --pr <number> --state <database> [--remote <name>]","inspect --prepared <preparation-id> --kind <manifest|description|files|diff|source|discussions|previous> --state <database>","draft list --prepared <preparation-id> --state <database>","draft save --prepared <preparation-id> --state <database> --input <json-file|->","draft discard --prepared <preparation-id> --state <database> --input <json-file|->","validate --input <file> --structural","validate --input <file> --evidence-only --context <fixture.json> --repository <checkout>","validate --input <file> --findings-only --context <fixture.json> --repository <checkout>","validate --input <file> --complete-fixture --context <fixture.json> --repository <checkout>"],"reserved_commands":["validate --prepared <preparation-id> --input <file>","commit --prepared <preparation-id> --input <file> [--approval <grant-id>]","authorize-replacement --prepared <preparation-id> [...human interaction]","open --repo <checkout> --pr <number>"],"exit_categories":{"success":0,"input_contract":2,"stale_state":3,"auth_access":4,"network_uncertain":5,"internal":10}})
}

fn bound_schema(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if object.get("type").is_some_and(|t| {
                t == "string"
                    || t.as_array()
                        .is_some_and(|a| a.iter().any(|v| v == "string"))
            }) {
                object.insert("maxLength".into(), json!(MAX_STRING_CHARS));
            }
            if object.get("type").is_some_and(|t| t == "array") {
                object.insert("maxItems".into(), json!(MAX_COLLECTION_ITEMS));
            }
            for child in object.values_mut() {
                bound_schema(child);
            }
        }
        Value::Array(array) => {
            for child in array {
                bound_schema(child);
            }
        }
        _ => (),
    }
}

fn bounded_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let text = <String as serde::Deserialize>::deserialize(d)?;
    if text.chars().count() > MAX_STRING_CHARS {
        return Err(serde::de::Error::custom("string exceeds character limit"));
    }
    Ok(text)
}
fn bounded_list<'de, T: serde::Deserialize<'de>, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<T>, D::Error> {
    let list = <Vec<T> as serde::Deserialize>::deserialize(d)?;
    if list.len() > MAX_COLLECTION_ITEMS {
        return Err(serde::de::Error::custom("collection exceeds item limit"));
    }
    Ok(list)
}
fn bounded_text_list<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let list: Vec<String> = bounded_list(d)?;
    if list.iter().any(|s| s.chars().count() > MAX_STRING_CHARS) {
        return Err(serde::de::Error::custom("string exceeds character limit"));
    }
    Ok(list)
}
fn bounded_optional_text<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<String>, D::Error> {
    let text = <Option<String> as serde::Deserialize>::deserialize(d)?;
    if text
        .as_ref()
        .is_some_and(|s| s.chars().count() > MAX_STRING_CHARS)
    {
        return Err(serde::de::Error::custom("string exceeds character limit"));
    }
    Ok(text)
}
