//! Shared headless review contract. Structural validation does not certify evidence.
mod model;
pub use model::*;
use serde::Serialize;
use serde_json::{Value, json};

pub const PROTOCOL_VERSION: &str = "1.0";
pub const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_STRING_CHARS: usize = 65_536;
pub const MAX_COLLECTION_ITEMS: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContractError {
    pub code: String,
    pub message: String,
    pub pointer: String,
}
impl ContractError {
    pub fn new(code: &str, message: &str, pointer: &str) -> Self {
        Self { code: code.into(), message: message.into(), pointer: pointer.into() }
    }
}

pub fn schema() -> Value {
    serde_json::to_value(schemars::schema_for!(ReviewResult)).expect("schema serializes")
}

pub fn validate_structure(input: &[u8]) -> Result<ReviewResult, ContractError> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(ContractError::new("INPUT_TOO_LARGE", "Input exceeds 8 MiB", ""));
    }
    let value: Value = serde_json::from_slice(input).map_err(|_| ContractError::new("MALFORMED_JSON", "Expected one JSON document", ""))?;
    let validator = jsonschema::validator_for(&schema()).expect("generated schema compiles");
    if let Err(error) = validator.validate(&value) {
        return Err(ContractError::new("CONTRACT_INVALID", "Submission does not match the review schema", &error.instance_path().to_string()));
    }
    serde_json::from_slice(input).map_err(|_| ContractError::new("CONTRACT_INVALID", "Submission does not match strict Rust representation", ""))
}

pub fn describe() -> Value {
    json!({"schema_version":"1.0","status":"provisional","validation":"structural_only","semantic_validation_available":false,"limits":{"input_bytes":MAX_INPUT_BYTES,"string_characters":MAX_STRING_CHARS,"collection_items":MAX_COLLECTION_ITEMS},"commands":["contract describe","contract schema","validate --input <file> --structural"],"reserved_commands":["prepare","inspect","validate --prepared <preparation-id>","commit","authorize-replacement","open"],"exit_categories":{"success":0,"input_contract":2,"stale_state":3,"auth_access":4,"network_uncertain":5,"internal":10}})
}
