//! Basic navigation example for SoulBrowser
//!
//! Run with: cargo run --example basic_navigation

use anyhow::Result;
use soulbrowser::{BrowserBuilder, Soul};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("🌟 SoulBrowser Basic Navigation Example");
    println!("========================================\n");

    // Create a Soul instance
    println!("Initializing SoulBrowser...");
    let soul = Soul::new().await?;

    // Launch a browser
    println!("Launching browser...");
    let mut browser = soul.launch_browser().await?;

    // Navigate to a website
    println!("Navigating to example.com...");
    browser.goto("https://example.com").await?;

    // Wait for page to load
    println!("Waiting for page to load...");
    browser.wait_for_selector("h1").await?;

    // Get page content
    println!("Getting page content...");
    let content = browser.content().await?;

    // Print first 500 chars of content
    println!("\nPage content (first 500 chars):");
    println!("{}", &content[..500.min(content.len())]);

    // Take a screenshot
    println!("\nTaking screenshot...");
    let screenshot = browser.screenshot().await?;
    println!("Screenshot captured: {} bytes", screenshot.len());

    // Close the browser
    println!("\nClosing browser...");
    browser.close().await?;

    // Shutdown Soul
    soul.shutdown().await?;

    println!("\n✨ Example completed successfully!");

    Ok(())
}
