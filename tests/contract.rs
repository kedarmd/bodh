use serde_json::{Value, json};

fn empty() -> Value {
    json!({"schema_version":"1.0", "context":{"pr":{"host":"github.com","owner":"example","repository":"demo","number":7},"context_id":"fixture","base_sha":"a".repeat(40),"head_sha":"b".repeat(40),"merge_base_sha":"c".repeat(40)},"overview":{"implementation_summary":"No semantic assertions yet","key_changes":[],"intent_assessments":[]},"diagrams":[],"findings":[],"follow_up":[],"coverage":{"examined_areas":[],"checks":[],"excluded_areas":[],"limitations":[]}})
}

#[test]
fn discover_and_validate_an_empty_structural_result_without_a_workspace() {
    let result = bodh::validate_structure(&serde_json::to_vec(&empty()).unwrap()).unwrap();
    assert_eq!(result.schema_version, bodh::SchemaVersion::V1);
    assert_eq!(bodh::schema()["additionalProperties"], false);
}

#[test]
fn schema_and_strict_rust_reject_oversized_strings_and_collections() {
    let mut value = empty();
    value["overview"]["implementation_summary"] = json!("é".repeat(65_537));
    assert!(bodh::validate_structure(&serde_json::to_vec(&value).unwrap()).is_err());
    assert!(serde_json::from_value::<bodh::ReviewResult>(value.clone()).is_err());
    let validator = jsonschema::validator_for(&bodh::schema()).unwrap();
    assert!(!validator.is_valid(&value));
    value = empty();
    value["coverage"]["limitations"] = json!(vec!["x"; 1_025]);
    assert!(!validator.is_valid(&value));
    assert!(serde_json::from_value::<bodh::ReviewResult>(value).is_err());
}
