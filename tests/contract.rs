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

#[test]
fn unsupported_versions_have_a_stable_error_and_pointer() {
    let mut value = empty();
    value["schema_version"] = json!("2.0");
    let error = bodh::validate_structure(&serde_json::to_vec(&value).unwrap()).unwrap_err();
    assert_eq!(error.code, "UNSUPPORTED_SCHEMA_VERSION");
    assert_eq!(error.pointer, "/schema_version");
}

#[test]
fn complete_fixture_corpus_agrees_with_schema_rust_service_and_cli() {
    let schema = bodh::schema();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for name in ["empty", "all-variants"] {
        let path = format!(
            "{}/contracts/v1/fixtures/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let input = std::fs::read(&path).unwrap();
        let value: Value = serde_json::from_slice(&input).unwrap();
        assert!(validator.is_valid(&value), "{name}");
        let result = bodh::validate_structure(&input).unwrap();
        let strict: bodh::ReviewResult = serde_json::from_slice(&input).unwrap();
        assert_eq!(result, strict);
        let process = std::process::Command::new(env!("CARGO_BIN_EXE_bodh"))
            .args(["validate", "--input", &path, "--structural", "--json"])
            .output()
            .unwrap();
        assert!(
            process.status.success(),
            "{}",
            String::from_utf8_lossy(&process.stderr)
        );
        let envelope: Value = serde_json::from_slice(&process.stdout).unwrap();
        assert_eq!(envelope["data"]["validation"], "structural_only");
        let mut paths = Vec::new();
        object_paths(&value, "", &mut paths);
        for pointer in paths {
            let mut invalid = value.clone();
            invalid
                .pointer_mut(&pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("unsupported_field".into(), json!("reject"));
            assert!(
                !validator.is_valid(&invalid),
                "schema accepted unknown at {pointer}"
            );
            assert!(
                serde_json::from_value::<bodh::ReviewResult>(invalid.clone()).is_err(),
                "Rust accepted unknown at {pointer}"
            );
            assert!(
                bodh::validate_structure(&serde_json::to_vec(&invalid).unwrap()).is_err(),
                "service accepted unknown at {pointer}"
            );
        }
    }
}
fn object_paths(value: &Value, pointer: &str, paths: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            paths.push(pointer.into());
            for (key, value) in object {
                object_paths(value, &format!("{pointer}/{key}"), paths);
            }
        }
        Value::Array(array) => {
            for (index, value) in array.iter().enumerate() {
                object_paths(value, &format!("{pointer}/{index}"), paths);
            }
        }
        _ => (),
    }
}

#[test]
fn negative_contract_corpus_agrees_across_all_public_interfaces() {
    let all: Value =
        serde_json::from_str(include_str!("../contracts/v1/fixtures/all-variants.json")).unwrap();
    let validator = jsonschema::validator_for(&bodh::schema()).unwrap();
    let cases = [
        ("/schema_version", json!("2.0")),
        ("/diagrams/0/steps/0/kind", json!("mermaid")),
        ("/findings/0/severity", json!("urgent")),
        ("/findings/0/confidence", json!(0.99)),
        ("/findings/0/suggestions/0/kind", json!("html")),
        ("/coverage/checks/0/status", json!("maybe")),
        ("/overview/intent_assessments/0/status", json!("false")),
        ("/follow_up/0/status", json!("dismissed")),
        ("/context/pr/number", json!(-1)),
        ("/findings/0/locations/0/source/start_line", json!(1.5)),
        ("/diagrams/0/participants/0/category", json!("server")),
        (
            "/diagrams/0/steps/4/branches/0/steps/0",
            json!({"kind":"opt","id":"nested","label":"bad","steps":[]}),
        ),
    ];
    for (pointer, replacement) in cases {
        let mut invalid = all.clone();
        *invalid.pointer_mut(pointer).unwrap() = replacement;
        assert!(!validator.is_valid(&invalid), "{pointer}");
        assert!(
            serde_json::from_value::<bodh::ReviewResult>(invalid.clone()).is_err(),
            "{pointer}"
        );
        assert!(
            bodh::validate_structure(&serde_json::to_vec(&invalid).unwrap()).is_err(),
            "{pointer}"
        );
        assert_cli_rejects(&invalid);
    }
    for field in [
        "context",
        "overview",
        "diagrams",
        "findings",
        "follow_up",
        "coverage",
    ] {
        let mut invalid = all.clone();
        invalid.as_object_mut().unwrap().remove(field);
        assert!(!validator.is_valid(&invalid));
        assert!(serde_json::from_value::<bodh::ReviewResult>(invalid.clone()).is_err());
        assert!(bodh::validate_structure(&serde_json::to_vec(&invalid).unwrap()).is_err());
        assert_cli_rejects(&invalid);
    }
}

#[test]
fn exact_limits_accept_without_truncation_and_oversized_documents_fail() {
    let mut value = empty();
    value["overview"]["implementation_summary"] = json!("é".repeat(65_536));
    value["coverage"]["limitations"] = json!(vec!["x"; 1_024]);
    let input = serde_json::to_vec(&value).unwrap();
    let result = bodh::validate_structure(&input).unwrap();
    assert_eq!(
        result.overview.implementation_summary.chars().count(),
        65_536
    );
    assert_eq!(result.coverage.limitations.len(), 1_024);
    let error = bodh::validate_structure(&vec![b' '; 8 * 1024 * 1024 + 1]).unwrap_err();
    assert_eq!(error.code, "INPUT_TOO_LARGE");
}

#[test]
fn committed_canonical_schema_cannot_drift_from_rust() {
    let committed: Value =
        serde_json::from_str(include_str!("../contracts/v1/review.schema.json")).unwrap();
    assert_eq!(bodh::schema(), committed);
}

fn assert_cli_rejects(value: &Value) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut process = Command::new(env!("CARGO_BIN_EXE_bodh"))
        .args(["validate", "--input", "-", "--structural", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    process
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(value).unwrap())
        .unwrap();
    let output = process.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["ok"], false);
    assert!(envelope["error"]["pointer"].is_string());
}
