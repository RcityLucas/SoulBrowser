use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use tempfile::tempdir;

#[test]
fn demo_real_browser_smoke() -> Result<()> {
    const TOGGLE: &str = "SOULBROWSER_SMOKE_DEMO";
    let enabled = env::var(TOGGLE).unwrap_or_default();
    if enabled.is_empty() || enabled == "0" {
        eprintln!(
            "skipping real-browser smoke test (set {TOGGLE}=1 and provide Chrome path to enable)"
        );
        return Ok(());
    }

    let chrome_path = env::var("SOULBROWSER_CHROME")
        .context("SOULBROWSER_CHROME must point to a Chrome/Chromium executable")?;
    let binary = env!("CARGO_BIN_EXE_soulbrowser");
    let tmp = tempdir()?;
    let screenshot_path = tmp.path().join("demo.png");

    let output = Command::new(binary)
        .env("SOULBROWSER_USE_REAL_CHROME", "1")
        .arg("demo")
        .arg("--chrome-path")
        .arg(&chrome_path)
        .arg("--screenshot")
        .arg(&screenshot_path)
        .output()
        .context("failed to execute demo command")?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "demo command failed: status={:?}\nstdout:\n{}\nstderr:\n{}",
            output.status, stdout, stderr
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Input anchor resolved"),
        "stdout did not include perceiver resolution log: {}",
        stdout
    );
    assert!(
        stdout.contains("Submit button clicked"),
        "stdout did not include submit click confirmation: {}",
        stdout
    );
    assert!(
        stdout.contains("Final URL:"),
        "stdout did not print final URL: {}",
        stdout
    );

    assert!(
        screenshot_path.exists(),
        "expected screenshot at {}",
        screenshot_path.display()
    );

    // Retain screenshot for inspection if requested; otherwise remove tempdir automatically
    if env::var("SOULBROWSER_DEMO_KEEP")
        .map(|value| !value.is_empty() && value != "0")
        .unwrap_or(false)
    {
        let target = PathBuf::from(env::var("SOULBROWSER_DEMO_KEEP").unwrap_or_default());
        if !target.as_os_str().is_empty() {
            fs::create_dir_all(&target)?;
            let copy_path = target.join("demo.png");
            fs::copy(&screenshot_path, &copy_path)
                .with_context(|| format!("copying screenshot to {}", copy_path.display()))?;
        }
    }

    Ok(())
}
