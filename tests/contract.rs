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
