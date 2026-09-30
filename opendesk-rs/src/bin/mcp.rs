use clap::Parser;
use opendesk_rs::mcp::server::McpServer;
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "opendesk-mcp", version = "0.3.0", about = "OpenDesk MCP server")]
struct McpCli {
    #[arg(long)]
    rendezvous: Option<String>,
    #[arg(long)]
    rendezvous_token: Option<String>,
    #[arg(long)]
    home: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "off".into()),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    let cli = McpCli::parse();
    let server = McpServer::with_config(cli.home, cli.rendezvous, cli.rendezvous_token);
    server.run_stdio().await
}
