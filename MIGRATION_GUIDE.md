# Migration Guide: Replacing SoulBrowser Components with Soul-Base

## UPDATE: Phase 2 Complete - Direct Soul-Base Integration

This guide shows how to migrate existing SoulBrowser components to use soul-base modules directly, avoiding reimplementation and leveraging battle-tested components.

## Component Replacement Map

| SoulBrowser Component | Soul-Base Replacement | Status | Benefits |
|-----------------------|----------------------|---------|----------|
| `SoulError` enum | `soulbase-errors` | ✅ Ready | HTTP/gRPC mapping, retry semantics |
| `SoulConfig` | `soulbase-config` | ✅ Ready | Schema validation, hot reload |
| `l5-tools` | `soulbase-tools` | ✅ Ready | Tool manifest, registry protocol |
| Custom auth | `soulbase-auth` | ✅ Adapted | OIDC, quota, consent |
| Interceptors | `soulbase-interceptors` | ✅ Adapted | Staged pipeline, policies |
| Auth policy config | `config/policies/browser_policy.json` | ✅ New | Override via `SOUL_POLICY_PATH`, enable strict mode with `SOUL_STRICT_AUTHZ=true` |
| Basic storage | `soulbase-storage` | ✅ Adapted | Multiple backends, transactions |
| Quota persistence | `FileQuotaStore` | ✅ Enhanced | Tune with `SOUL_QUOTA_PERSIST_MS` / `SOUL_QUOTA_REFRESH_MS` |
| Logging | `soulbase-observe` | ✅ Adapted | Structured logs, metrics, traces |
| Simple cache | `soulbase-cache` | ✅ Ready | Two-level, Redis support |
| File execution | `soulbase-sandbox` | ✅ Ready | Secure execution, budgets |
| - | `soulbase-llm` | ✅ Ready | Chat, embeddings, reranking |
| File storage | `soulbase-blob` | ⏳ Pending | S3, GCS support |
| - | `soulbase-crypto` | ⏳ Pending | Encryption, signing |
| HTTP requests | `soulbase-net` | ⏳ Pending | Retry, failover |
| - | `soulbase-tx` | ⏳ Pending | Saga, outbox pattern |

## Migration Steps

### 1. Error System Migration

**Before (SoulBrowser custom):**
```rust
#[derive(Debug, thiserror::Error)]
pub enum SoulError {
    #[error("Navigation failed: {0}")]
    NavigationError(String),
    #[error("Element not found: {0}")]
    ElementError(String),
}
```

**After (soulbase-errors):**
```rust
use soul_integration::reuse_adapter::{
    BrowserError, BrowserErrorBuilder, BrowserErrorCode
};

// Create rich errors with metadata
let error = BrowserErrorBuilder::new(BrowserErrorCode::NavigationFailed)
    .user_message("Page could not be loaded")
    .dev_message("DNS resolution failed")
    .retryable(true)
    .http_status(502)
    .meta("url", json!(url))
    .meta("attempt", json!(retry_count))
    .build();
```

**Benefits:**
- Structured error metadata
- HTTP/gRPC status mapping
- Built-in retry semantics
- User vs developer messages
- Error correlation tracking

### 2. Configuration Migration

**Before (SoulBrowser basic):**
```rust
struct Config {
    browser_type: String,
    headless: bool,
}

let config = Config {
    browser_type: "chrome".to_string(),
    headless: true,
};
```

**After (soulbase-config):**
```rust
use soul_integration::reuse_adapter::{BrowserConfig, ConfigLoader};

let config = ConfigLoader::new()
    .from_file("config.json")?  // Schema-validated JSON
    .from_env()                 // Override with env vars
    .build();

// Comprehensive typed configuration
println!("Browser: {}", config.browser.browser_type);
println!("Rate limit: {}/min", config.performance.rate_limit_per_minute);
println!("Sandbox: {}", config.security.enable_sandbox);
```

**Benefits:**
- Schema-first validation
- Multiple config sources
- Hot reload support
- Type-safe access
- Environment overrides

### 3. Tools Layer Migration

**Before (l5-tools custom):**
```rust
pub enum Tool {
    Navigate { url: String },
    Click { selector: String },
    Screenshot { filename: String },
}

fn execute_tool(tool: Tool) -> Result<()> {
    match tool {
        Tool::Navigate { url } => { /* ... */ },
        Tool::Click { selector } => { /* ... */ },
        Tool::Screenshot { filename } => { /* ... */ },
    }
}
```

**After (soulbase-tools):**
```rust
use soul_integration::reuse_adapter::{BrowserToolRegistry, ToolManifest};

let mut registry = BrowserToolRegistry::new();

// Register tools with manifest
registry.register(ToolManifest {
    id: "navigate".to_string(),
    name: "Navigate".to_string(),
    version: "1.0.0".to_string(),
    category: ToolCategory::Navigation,
    inputs: vec![
        ToolInput {
            name: "url".to_string(),
            type_: "string".to_string(),
            required: true,
            description: "Target URL".to_string(),
        }
    ],
    outputs: vec![],
    config: HashMap::new(),
});

// Dynamic tool invocation
let tool = registry.get("navigate").unwrap();
let inputs = hashmap!["url" => json!("https://example.com")];
let result = invoker.invoke(tool.id, inputs).await?;
```

**Benefits:**
- Tool discovery and registry
- Versioned tool manifests
- Input/output validation
- Dynamic tool loading
- Tool composition support

### 4. Sandbox Integration

**Before (unsafe execution):**
```rust
// Direct execution without isolation
let result = execute_browser_action(action)?;
```

**After (soulbase-sandbox):**
```rust
use soul_integration::reuse_adapter::{SandboxConfig, SandboxExecutor};

let sandbox_config = SandboxConfig {
    enable_network: true,
    enable_file_system: false,
    max_memory_mb: 256,
    max_cpu_percent: 25,
    timeout_seconds: 10,
    allowed_paths: vec!["/tmp".into()],
    blocked_paths: vec!["/etc".into()],
};

let sandbox = SandboxExecutor::new(sandbox_config);

// Execute with resource limits and isolation
let result = sandbox.execute(|| {
    execute_browser_action(action)
}).await?;
```

**Benefits:**
- Resource isolation
- Memory/CPU limits
- Network control
- File system restrictions
- Evidence recording
- Budget enforcement

### 5. LLM Integration

**Before (no AI support):**
```rust
// Manual element selection
let selector = "#login-button";
```

**After (soulbase-llm):**
```rust
use soul_integration::reuse_adapter::{LLMClient, ChatMessage};

let llm = create_llm_client(config);

// AI-powered element detection
let messages = vec![
    ChatMessage {
        role: "system".to_string(),
        content: "Find element selectors from descriptions".to_string(),
    },
    ChatMessage {
        role: "user".to_string(),
        content: "Find the blue login button in the header".to_string(),
    },
];

let selector = llm.chat(messages).await?;
```

**Benefits:**
- Natural language automation
- Multiple LLM providers
- Streaming support
- Embeddings for similarity
- Reranking for relevance

### 6. Cache System Migration

**Before (basic HashMap):**
```rust
let mut cache = HashMap::new();
cache.insert("key", "value");
```

**After (soulbase-cache):**
```rust
use soul_integration::reuse_adapter::TwoLevelCache;

let mut cache = TwoLevelCache::new(Duration::from_secs(300));

// L1 memory cache with TTL
cache.put("selector_1".to_string(), element.clone());

// Automatic expiration
if let Some(element) = cache.get("selector_1") {
    // Use cached element
}

// Manual invalidation
cache.invalidate("selector_1");
```

**Benefits:**
- Two-level caching (L1/L2)
- TTL support
- Redis backend option
- SingleFlight deduplication
- Cache warming
- Invalidation strategies

## Migration Status (Current Progress)

### ✅ **Phase 1: Core Infrastructure** (COMPLETED)
  - [x] Replaced the custom error system with `soulbase-errors` (`src/errors.rs`)
  - [x] Adopted `soulbase-config` for runtime settings (`src/config.rs`)
  - [x] Reused fundamental types from `soulbase-types` (`src/types.rs`)

### ✅ **Phase 2: Service Components** (COMPLETED)
  - [x] Integrated `soulbase-auth` for authentication/authorization (`src/auth.rs`)
  - [x] Swapped storage for `soulbase-storage` (`src/storage.rs`)
  - [x] Hooked `soulbase-interceptors` into the request pipeline (`src/interceptors.rs`)
  - [x] Replaced tool execution with `soulbase-tools` (`src/tools.rs`)

### ✅ **Phase 3: Cleanup** (COMPLETED)
  - [x] Removed the legacy `soul_integration` and migration shims
  - [x] Pruned unused modules/examples from the pre-soul-base implementation
  - [x] Project now builds directly against soul-base crates

### 🔄 **Phase 4: Future Enhancements**
  - [ ] Add `soulbase-blob` for screenshot storage
  - [ ] Integrate `soulbase-crypto` for security
  - [ ] Use `soulbase-net` for resilient HTTP
  - [ ] Add `soulbase-tx` for transactions
  - [ ] Integrate `soulbase-observe` for logging/metrics
  - [ ] Add `soulbase-cache` for performance
  - [ ] Integrate `soulbase-sandbox` for security
  - [ ] Add `soulbase-llm` for AI capabilities

## Code Organization

### Current Architecture (After Migration)
```
src/
├── main.rs             # CLI entry point
├── app_context.rs      # Shared runtime wiring
├── auth.rs             # soulbase-auth integration
├── browser_impl.rs     # L0/L1 orchestration with soul-base
├── interceptors.rs     # soulbase-interceptors stages
├── policy.rs           # Policy loader utilities
├── storage.rs          # soulbase-storage adapter
├── tools.rs            # soulbase-tools wiring
└── types.rs            # Common types derived from soulbase-types
```

## Benefits Summary

By reusing soul-base components, SoulBrowser gains:

1. **Production-Ready Components**: Battle-tested in enterprise environments
2. **Comprehensive Features**: More capabilities than custom implementations
3. **Maintenance**: Updates and fixes from soul-base team
4. **Standards Compliance**: HTTP/gRPC, OpenTelemetry, OAuth
5. **Performance**: Optimized caching, connection pooling
6. **Security**: Sandboxing, encryption, authentication
7. **Observability**: Structured logging, distributed tracing
8. **Extensibility**: Plugin architecture, custom providers

## Next Steps

1. Complete integration of remaining soul-base components
2. Remove redundant custom implementations
3. Update tests to use soul-base components
4. Document soul-base component configuration
5. Create examples for common use cases

## Support

For questions about soul-base components:
- Check soul-base documentation in `/soul-base-main/`
- Review examples in `examples/soul_reuse_example.rs`
- Consult SOUL_INTEGRATION.md for architecture details
