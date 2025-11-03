use assert_cmd::prelude::*;
use serde_json::Value;
use std::path::Path;
use std::process::Command;

#[test]
fn console_command_emits_expected_payload() {
    let input = Path::new("tests/fixtures/console_run_bundle.json");
    assert!(input.exists(), "fixture missing");

    let bin = assert_cmd::cargo::cargo_bin!("soulbrowser");
    let mut cmd = Command::new(bin);
    let assert = cmd
        .args(["console", "--input", input.to_str().unwrap(), "--pretty"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 output");
    let json_slice = extract_json(&stdout);
    let value: Value = serde_json::from_str(json_slice).expect("valid json");

    assert_eq!(value["plans"].as_array().unwrap().len(), 1);
    assert_eq!(value["execution"].as_array().unwrap().len(), 1);

    let summary = &value["artifacts"]["summary"];
    assert_eq!(summary["count"].as_u64(), Some(1));
    assert_eq!(summary["total_bytes"].as_u64(), Some(68));

    let items = value["artifacts"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["label"].as_str(), Some("screenshot"));
    assert_eq!(items[0]["dispatch_label"].as_str(), Some("action"));

    let overlays = value["overlays"].as_array().unwrap();
    assert_eq!(overlays.len(), 1);
    assert_eq!(overlays[0]["content_type"].as_str(), Some("image/png"));

    let state_events = value["state_events"].as_array().unwrap();
    assert_eq!(state_events.len(), 1);
    assert_eq!(state_events[0]["action_id"].as_str(), Some("action-1"));
}

fn extract_json(output: &str) -> &str {
    let start = output.find('{').expect("json start");
    let end = output.rfind('}').expect("json end");
    &output[start..=end]
}
