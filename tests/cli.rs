use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};
fn run(args: &[&str], input: &[u8]) -> (i32, Value, String) {
    let mut process = Command::new(env!("CARGO_BIN_EXE_bodh"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    process.stdin.take().unwrap().write_all(input).unwrap();
    let output = process.wait_with_output().unwrap();
    (
        output.status.code().unwrap(),
        serde_json::from_slice(&output.stdout).expect("one stdout envelope"),
        String::from_utf8(output.stderr).unwrap(),
    )
}
#[test]
fn cli_discovers_schema_and_rejects_bad_input_without_a_gui() {
    let (exit, body, stderr) = run(&["--json", "contract", "describe"], b"");
    assert_eq!(exit, 0, "{stderr}");
    assert_eq!(body["protocol_version"], "1.0");
    assert_eq!(body["data"]["validation"], "structural_only");
    let (exit, schema, _) = run(&["contract", "schema", "--json"], b"");
    assert_eq!(exit, 0);
    assert_eq!(schema["data"]["additionalProperties"], false);
    let (exit, body, stderr) = run(
        &["validate", "--input", "-", "--structural", "--json"],
        b"{ broken",
    );
    assert_eq!(exit, 2);
    assert_eq!(body["ok"], false);
    assert_eq!(body["error"]["code"], "MALFORMED_JSON");
    assert!(!stderr.is_empty());
    let (_, body, _) = run(
        &["validate", "--input", "/definitely/missing", "--structural"],
        b"",
    );
    assert_eq!(body["error"]["code"], "INPUT_UNREADABLE");
    for args in [
        vec!["unknown"],
        vec!["validate", "--force"],
        vec!["validate", "--input", "-"],
    ] {
        let (exit, body, _) = run(&args, b"");
        assert_eq!(exit, 2);
        assert_eq!(body["error"]["code"], "INVALID_ARGUMENTS");
    }
}

#[test]
fn malformed_input_includes_location_without_echoing_untrusted_text() {
    let (exit, body, stderr) = run(
        &["validate", "--input", "-", "--structural"],
        b"{\nsecret-token",
    );
    assert_eq!(exit, 2);
    assert_eq!(body["error"]["line"], 2);
    assert!(body["error"]["column"].as_u64().unwrap() > 0);
    assert!(!stderr.contains("secret-token"));
    assert!(!body.to_string().contains("secret-token"));
}
