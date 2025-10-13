//! Example demonstrating proper reuse of soul-base components
//!
//! This shows how SoulBrowser can leverage soul-base modules directly
//! instead of reimplementing them.

use std::collections::HashMap;
use std::time::Duration;

// Include the soul integration modules
include!("../src/soul_integration/mod.rs");
include!("../src/soul_integration/reuse_adapter.rs");

use soul_integration::reuse_adapter::{
    BrowserConfig, BrowserError, BrowserErrorBuilder, BrowserErrorCode, BrowserToolRegistry,
    ChatMessage, ConfigLoader, LLMClient, MockLLMClient, SandboxConfig, SandboxExecutor,
    ToolManifest, TwoLevelCache,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🎯 Soul-Base Component Reuse Demonstration\n");
    println!("This demonstrates how SoulBrowser properly reuses soul-base components");
    println!("instead of reimplementing them.\n");

    // ========================================================================
    // 1. SOULBASE-CONFIG: Schema-driven configuration
    // ========================================================================
    println!("1️⃣ Configuration System (soulbase-config)");
    println!("----------------------------------------");

    let config = ConfigLoader::new().from_env().build();

    println!("✅ Loaded configuration:");
    println!(
        "   Browser: {} (headless: {})",
        config.browser.browser_type, config.browser.headless
    );
    println!("   Session timeout: {}ms", config.session.timeout_ms);
    println!("   Storage backend: {}", config.storage.backend);
    println!(
        "   Rate limit: {}/min",
        config.performance.rate_limit_per_minute
    );
    println!(
        "   Security: sandbox={}, auth={}",
        config.security.enable_sandbox, config.security.enable_auth
    );
    println!();

    // ========================================================================
    // 2. SOULBASE-ERRORS: Unified error system
    // ========================================================================
    println!("2️⃣ Error System (soulbase-errors)");
    println!("-----------------------------------");

    // Create different types of errors
    let nav_error = BrowserErrorBuilder::new(BrowserErrorCode::NavigationFailed)
        .user_message("Failed to navigate to the requested page")
        .dev_message("DNS resolution failed for domain")
        .retryable(true)
        .http_status(502)
        .meta("url", serde_json::json!("https://example.com"))
        .meta("attempt", serde_json::json!(1))
        .build();

    println!("✅ Navigation error created:");
    println!("   Code: {:?}", nav_error.code);
    println!("   User message: {}", nav_error.user_message);
    println!("   Retryable: {}", nav_error.retryable);
    println!("   HTTP status: {}", nav_error.http_status);

    let auth_error = BrowserErrorBuilder::new(BrowserErrorCode::AuthenticationFailed)
        .user_message("Invalid credentials")
        .retryable(false)
        .http_status(401)
        .build();

    println!("✅ Auth error created:");
    println!("   Code: {:?}", auth_error.code);
    println!("   HTTP status: {}", auth_error.http_status);
    println!();

    // ========================================================================
    // 3. SOULBASE-TOOLS: Tool registry and invocation
    // ========================================================================
    println!("3️⃣ Tool System (soulbase-tools)");
    println!("--------------------------------");

    let mut tool_registry = BrowserToolRegistry::new();

    // Register custom tool
    tool_registry.register(ToolManifest {
        id: "screenshot".to_string(),
        name: "Screenshot".to_string(),
        version: "1.0.0".to_string(),
        description: "Capture screenshot".to_string(),
        category: soul_integration::reuse_adapter::tools::ToolCategory::Utility,
        inputs: vec![soul_integration::reuse_adapter::tools::ToolInput {
            name: "filename".to_string(),
            type_: "string".to_string(),
            required: false,
            description: "Output filename".to_string(),
        }],
        outputs: vec![soul_integration::reuse_adapter::tools::ToolOutput {
            name: "path".to_string(),
            type_: "string".to_string(),
            description: "Screenshot file path".to_string(),
        }],
        config: HashMap::new(),
    });

    println!(
        "✅ Tool registry initialized with {} tools:",
        tool_registry.list().len()
    );
    for tool in tool_registry.list() {
        println!(
            "   - {} ({}): {}",
            tool.name, tool.version, tool.description
        );
    }
    println!();

    // ========================================================================
    // 4. SOULBASE-SANDBOX: Secure execution
    // ========================================================================
    println!("4️⃣ Sandbox System (soulbase-sandbox)");
    println!("-------------------------------------");

    let sandbox_config = SandboxConfig {
        enable_network: true,
        enable_file_system: false,
        max_memory_mb: 256,
        max_cpu_percent: 25,
        timeout_seconds: 10,
        ..Default::default()
    };

    let sandbox = SandboxExecutor::new(sandbox_config);

    println!("✅ Sandbox configured:");
    println!("   Network: enabled");
    println!("   File system: disabled");
    println!("   Memory limit: 256MB");
    println!("   CPU limit: 25%");

    // Execute in sandbox
    let result = sandbox.execute(|| "Executed in sandbox").await?;

    println!("✅ Sandbox execution: {}", result);
    println!();

    // ========================================================================
    // 5. SOULBASE-LLM: AI capabilities
    // ========================================================================
    println!("5️⃣ LLM System (soulbase-llm)");
    println!("-----------------------------");

    let llm_client = MockLLMClient;

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: "You are a browser automation assistant".to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: "Find the login button".to_string(),
        },
    ];

    let response = llm_client.chat(messages).await?;
    println!("✅ LLM response: {}", response);

    let embedding = llm_client.embed("login button").await?;
    println!(
        "✅ Text embedding: {:?}",
        &embedding[..3.min(embedding.len())]
    );
    println!();

    // ========================================================================
    // 6. SOULBASE-CACHE: Two-level caching
    // ========================================================================
    println!("6️⃣ Cache System (soulbase-cache)");
    println!("---------------------------------");

    let mut cache: TwoLevelCache<String> = TwoLevelCache::new(Duration::from_secs(60));

    // Put items in cache
    cache.put("selector_1".to_string(), "#login-button".to_string());
    cache.put("selector_2".to_string(), ".submit-btn".to_string());

    println!("✅ Cached 2 selectors");

    // Retrieve from cache
    if let Some(selector) = cache.get("selector_1") {
        println!("✅ Retrieved from cache: {}", selector);
    }

    // Invalidate cache
    cache.invalidate("selector_1");
    println!("✅ Invalidated cache entry");
    println!();

    // ========================================================================
    // 7. Integration Summary
    // ========================================================================
    println!("📊 Soul-Base Component Reuse Summary");
    println!("====================================");
    println!("✅ soulbase-config: Schema-driven configuration management");
    println!("✅ soulbase-errors: Unified error system with retry semantics");
    println!("✅ soulbase-tools: Tool registry and invocation protocol");
    println!("✅ soulbase-sandbox: Secure execution environment");
    println!("✅ soulbase-llm: AI capabilities for automation");
    println!("✅ soulbase-cache: Two-level caching for performance");
    println!();

    println!("🎯 Components not yet integrated:");
    println!("⏳ soulbase-tx: Transaction and saga infrastructure");
    println!("⏳ soulbase-blob: Object storage for screenshots");
    println!("⏳ soulbase-crypto: Encryption and signing");
    println!("⏳ soulbase-net: HTTP client with retry/failover");
    println!();

    println!("✨ By properly reusing soul-base components, SoulBrowser gains:");
    println!("   • Enterprise-grade error handling");
    println!("   • Schema-driven configuration");
    println!("   • Secure sandboxed execution");
    println!("   • AI-powered automation");
    println!("   • High-performance caching");
    println!("   • Extensible tool system");

    Ok(())
}
