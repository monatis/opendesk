use clap::{Parser, Subcommand};
use opendesk_rs::mcp::server::McpServer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(
    name = "opendesk",
    version = "0.3.0",
    about = "Ultra-lightweight native computer-use framework for AI agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run as an MCP (Model Context Protocol) stdio server for agents (Claude, Cursor, isanagent)
    Mcp,

    /// Run quick diagnostic test of local accessibility and input
    Test,

    /// Serve the desktop for remote control over WebSocket or Rendezvous
    Serve {
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        #[arg(long, default_value_t = 8423)]
        port: u16,
        #[arg(long)]
        rendezvous: Option<String>,
        #[arg(long)]
        rendezvous_token: Option<String>,
    },

    /// Run standalone OpenDesk Rendezvous & Relay Server
    Rendezvous {
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        #[arg(long, default_value_t = 8765)]
        port: u16,
        #[arg(long)]
        token: Option<String>,
    },

    /// Install opendesk serve as a user-scoped OS service (systemd, launchd, or Task Scheduler)
    #[command(alias = "install")]
    InstallService {
        #[arg(long, default_value_t = 8423)]
        port: u16,
        #[arg(long)]
        no_start: bool,
        #[arg(long)]
        rendezvous: Option<String>,
        #[arg(long)]
        rendezvous_token: Option<String>,
    },

    /// Uninstall the user-scoped opendesk OS service
    #[command(alias = "uninstall")]
    UninstallService,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // If logging is enabled via RUST_LOG, write logs to stderr so stdout stays clean for MCP JSON-RPC
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "off".into()))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    // Check if the binary was invoked as `opendesk-mcp` or with `mcp`
    let exe_name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_default();

    if exe_name == "opendesk-mcp" {
        let server = McpServer::new();
        return server.run_stdio().await;
    }

    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Mcp) {
        Commands::Mcp => {
            let server = McpServer::new();
            server.run_stdio().await?;
        }

        Commands::Test => {
            println!("=== OpenDesk Native Diagnostic Test ===");
            let computer = opendesk_rs::computer::local::LocalComputer::new();
            match computer.app_list() {
                Ok(apps) => println!("Accessible apps count: {}", apps.len()),
                Err(e) => println!("App list diagnostic note: {e}"),
            }
            match computer.clipboard_read() {
                Ok(text) => println!("Clipboard text length: {} chars", text.len()),
                Err(e) => println!("Clipboard diagnostic note: {e}"),
            }
            println!("Diagnostic test completed.");
        }

        Commands::Serve {
            host,
            port,
            rendezvous,
            rendezvous_token: _,
        } => {
            println!("opendesk-rs serve starting on {host}:{port}");
            if let Some(r) = rendezvous {
                println!("  Connecting outbound to rendezvous: {r}");
            }
            println!("(Remote agent daemon loop will listen here)");
        }

        Commands::Rendezvous { host, port, token } => {
            println!("opendesk-rs rendezvous listening on {host}:{port}");
            if token.is_some() {
                println!("  Token authentication enabled.");
            }
            println!("(Rendezvous signaling & relay loop will listen here)");
        }

        Commands::InstallService {
            port,
            no_start,
            rendezvous,
            rendezvous_token,
        } => {
            match opendesk_rs::service::install_service(
                port,
                !no_start,
                rendezvous.as_deref(),
                rendezvous_token.as_deref(),
            ) {
                Ok(result) => {
                    println!("✓ Service installed ({}): {}", result.manager, result.path.display());
                    if result.started {
                        println!("  Started. It will also run automatically on next login.");
                    } else if no_start {
                        println!("  Not started (--no-start). Activate manually or rerun without the flag.");
                    } else {
                        println!("  WARNING: Service registered but could not be started immediately.");
                    }
                }
                Err(e) => {
                    eprintln!("ERROR: Failed to install service: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::UninstallService => {
            match opendesk_rs::service::uninstall_service() {
                Ok(removed) => {
                    if removed {
                        println!("✓ Service uninstalled.");
                    } else {
                        println!("No opendesk service was installed.");
                    }
                }
                Err(e) => {
                    eprintln!("ERROR: Failed to uninstall service: {e}");
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}
