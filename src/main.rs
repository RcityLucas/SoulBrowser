mod cli;

use crate::cli::app;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
