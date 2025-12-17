use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use soulbrowser_kernel::{runtime::RuntimeOptions, Config, GatewayOptions, Kernel};

#[derive(Args, Clone)]
pub struct GatewayArgs {
    /// HTTP listener address (host:port)
    #[arg(long, default_value = "127.0.0.1:8710")]
    pub http: SocketAddr,

    /// Optional gRPC listener (not yet implemented)
    #[arg(long)]
    pub grpc: Option<SocketAddr>,

    /// Optional WebDriver bridge listener (not yet implemented)
    #[arg(long)]
    pub webdriver: Option<SocketAddr>,

    /// Path to adapter policy definition (json/yaml)
    #[arg(long, value_name = "FILE")]
    pub adapter_policy: Option<PathBuf>,

    /// Path to WebDriver bridge policy definition (json/yaml)
    #[arg(long, value_name = "FILE")]
    pub webdriver_policy: Option<PathBuf>,

    /// Optional path to task plan JSON to run immediately when gateway starts
    #[arg(long, value_name = "FILE")]
    pub demo_plan: Option<PathBuf>,
}

pub async fn cmd_gateway(args: GatewayArgs, config: &Config) -> Result<()> {
    let options = GatewayOptions {
        http: args.http,
        grpc: args.grpc,
        webdriver: args.webdriver,
        adapter_policy: args.adapter_policy,
        webdriver_policy: args.webdriver_policy,
        demo_plan: args.demo_plan,
        runtime: RuntimeOptions {
            tenant: "gateway".to_string(),
            websocket_url: None,
            llm_cache_dir: None,
            shared_session_override: None,
        },
    };

    Kernel::new(config.clone()).gateway(options).await
}
