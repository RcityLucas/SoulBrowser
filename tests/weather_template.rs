use std::process::Command;

#[test]
fn weather_prompt_execute_does_not_trigger_plan_validation_error() {
    let bin = assert_cmd::cargo::cargo_bin!("soulbrowser");
    let output = Command::new(bin)
        .args([
            "chat",
            "--prompt",
            "帮我打开百度，查询下今天天气",
            "--execute",
        ])
        .output()
        .expect("run chat command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        !combined.contains("plan failed validation"),
        "chat command should not report plan validation failure even if execution stops: {}",
        combined
    );
}
