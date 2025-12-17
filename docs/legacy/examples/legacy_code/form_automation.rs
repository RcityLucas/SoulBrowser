//! Form automation example for SoulBrowser
//!
//! Run with: cargo run --example form_automation

use anyhow::Result;
use soulbrowser::{Browser, Soul};
use tokio::time::{sleep, Duration};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("🌟 SoulBrowser Form Automation Example");
    println!("=======================================\n");

    // Create a Soul instance
    let soul = Soul::new().await?;

    // Launch a browser
    let mut browser = soul.launch_browser().await?;

    // Navigate to a form page (using a test form)
    println!("Navigating to form page...");
    browser
        .goto("https://www.w3schools.com/html/html_forms.asp")
        .await?;

    // Wait for form to be present
    println!("Waiting for form elements...");
    browser.wait_for_selector("input[type='text']").await?;

    // Fill in form fields
    println!("Filling form fields...");

    // Type in first name field
    browser
        .type_text("input[name='fname']", "Soul")
        .await
        .unwrap_or_else(|e| println!("Note: fname field might not exist: {}", e));

    // Type in last name field
    browser
        .type_text("input[name='lname']", "Browser")
        .await
        .unwrap_or_else(|e| println!("Note: lname field might not exist: {}", e));

    // Click submit button
    println!("Attempting to submit form...");
    browser
        .click("input[type='submit']")
        .await
        .unwrap_or_else(|e| println!("Note: Submit might not work on demo page: {}", e));

    // Wait a bit to see results
    sleep(Duration::from_secs(2)).await;

    // Take a screenshot of the result
    println!("Taking screenshot of form state...");
    let screenshot = browser.screenshot().await?;
    println!("Screenshot captured: {} bytes", screenshot.len());

    // Close browser
    browser.close().await?;
    soul.shutdown().await?;

    println!("\n✨ Form automation example completed!");

    Ok(())
}
