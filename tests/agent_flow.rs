use assert_cmd::prelude::*;
use serde_json::Value;
use std::process::Command;

#[test]
fn chat_cli_outputs_structured_plan_payload() {
    let bin = assert_cmd::cargo::cargo_bin!("soulbrowser");

    let mut cmd = Command::new(bin);
    let assert = cmd
        .args([
            "--output",
            "json",
            "chat",
            "--prompt",
            "open https://example.com and click pricing",
        ])
        .assert()
        .success();

    let stdout = &assert.get_output().stdout;
    assert!(
        !stdout.is_empty(),
        "chat command should emit structured JSON"
    );

    let stdout_str = String::from_utf8_lossy(stdout);
    let json_slice = extract_json_block(&stdout_str);
    let payload: Value = serde_json::from_str(json_slice).expect("valid JSON payload");

    let plans = payload
        .get("plans")
        .and_then(|v| v.as_array())
        .expect("plans array present");
    assert_eq!(plans.len(), 1, "single plan expected");

    let plan_entry = &plans[0];
    let plan_obj = plan_entry
        .get("plan")
        .and_then(|v| v.as_object())
        .expect("plan object present");
    let task_id = plan_obj
        .get("task_id")
        .and_then(|v| v.as_str())
        .expect("task id present");
    assert!(
        !task_id.is_empty(),
        "planner must emit non-empty task identifier"
    );

    let steps = plan_obj
        .get("steps")
        .and_then(|v| v.as_array())
        .expect("steps array present");
    assert!(
        !steps.is_empty(),
        "at least one plan step should be produced"
    );

    let first_step = steps[0].as_object().expect("step should be an object");
    assert!(
        first_step
            .get("tool")
            .and_then(|t| t.get("kind"))
            .map(|kind| kind.is_object())
            .unwrap_or(false),
        "steps must include structured tool definitions",
    );

    let flow_obj = plan_entry
        .get("flow")
        .and_then(|flow| flow.as_object())
        .expect("flow object present");
    let step_count = flow_obj
        .get("metadata")
        .and_then(|meta| meta.as_object())
        .and_then(|meta| meta.get("step_count"))
        .or_else(|| flow_obj.get("step_count"))
        .and_then(|v| v.as_u64())
        .expect("step_count numeric");
    assert_eq!(
        step_count as usize,
        steps.len(),
        "flow metadata should match plan steps"
    );

    let explanations = plan_entry
        .get("explanations")
        .and_then(|v| v.as_array())
        .expect("explanations array present");
    assert!(
        !explanations.is_empty(),
        "planner must emit rationale entries"
    );

    let overlays = plan_entry
        .get("overlays")
        .and_then(|v| v.as_array())
        .expect("plan overlays array present");
    assert!(
        overlays.iter().all(|entry| entry.is_object()),
        "overlays should be structured objects even if empty"
    );

    let execution = payload
        .get("execution")
        .and_then(|v| v.as_array())
        .expect("execution array present");
    assert!(
        execution.is_empty(),
        "execution report should be empty when --execute is not set"
    );

    let artifacts = payload
        .get("artifacts")
        .and_then(|v| v.as_array())
        .expect("artifact manifest array present");
    assert!(
        artifacts.is_empty(),
        "no artifacts are generated without executing the plan"
    );
}

fn extract_json_block(output: &str) -> &str {
    let start = output
        .find("\n{")
        .map(|idx| idx + 1)
        .or_else(|| {
            output.char_indices().find_map(|(idx, ch)| {
                if ch == '{' {
                    if idx == 0 || output.as_bytes()[idx - 1] == b'\n' {
                        Some(idx)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
        })
        .or_else(|| output.find('{'))
        .unwrap_or_else(|| panic!("missing JSON start: {output}"));
    let mut depth = 0i32;
    let slice = &output[start..];
    for (idx, ch) in slice.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &slice[..=idx];
                }
            }
            _ => {}
        }
    }
    panic!("failed to locate complete JSON payload: {output}");
}
