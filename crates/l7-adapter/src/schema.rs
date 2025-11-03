/// Placeholder for adapter schema generation (OpenAPI / Proto / MCP specs).
/// The final implementation will render schemas at build time by reading the
/// ToolSpec/FlowSpec registry.
#[allow(dead_code)]
pub fn emit_openapi() -> String {
    "openapi: 3.1.0\ninfo:\n  title: SoulBrowser Adapter\n  version: v0".to_string()
}
