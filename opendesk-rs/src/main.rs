use std::path::PathBuf;
use clap::{Parser, Subcommand};
use opendesk_rs::mcp::server::McpServer;
use opendesk_rs::protocol::identity::generate_pairing_code;
use opendesk_rs::protocol::storage::{
    clear_description, fingerprint, read_description, write_description, TrustedPeers,
};
use opendesk_rs::remote::client::{connect as remote_connect, pair_with};
use opendesk_rs::remote::rendezvous::RendezvousServer;
use opendesk_rs::remote::server::OpendeskServer;
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

    /// Accept one new controller (prints a code; controller types it on `pair-with`)
    Pair {
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        #[arg(long, default_value_t = 8423)]
        port: u16,
        #[arg(long)]
        code: Option<String>,
        #[arg(long, default_value_t = 300)]
        timeout: u64,
    },

    /// Pair this machine with a peer running `opendesk pair`
    #[command(name = "pair-with")]
    PairWith {
        host: Option<String>,
        code: String,
        #[arg(long, default_value_t = 8423)]
        port: u16,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        rendezvous: Option<String>,
        #[arg(long)]
        target_pubkey: Option<String>,
        #[arg(long)]
        rendezvous_token: Option<String>,
    },

    /// Serve the desktop for remote control over WebSocket or Rendezvous
    Serve {
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        #[arg(long, default_value_t = 8423)]
        port: u16,
        #[arg(long)]
        rendezvous: Vec<String>,
        #[arg(long)]
        rendezvous_token: Option<String>,
    },

    /// Open a paired peer and confirm it works
    Connect {
        peer: Option<String>,
        #[arg(long)]
        rendezvous: Option<String>,
        #[arg(long)]
        rendezvous_token: Option<String>,
        #[arg(long)]
        screenshot: Option<PathBuf>,
    },

    /// List, remove, rename, or set default trusted peer
    Peers {
        #[command(subcommand)]
        subcmd: Option<PeersCommands>,
    },

    /// Read / set / clear the broadcast description of this machine
    Describe {
        text: Option<String>,
        #[arg(long)]
        clear: bool,
    },

    /// Run standalone OpenDesk Rendezvous & Relay Server
    Rendezvous {
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        #[arg(long, default_value_t = 8424)]
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

#[derive(Subcommand)]
enum PeersCommands {
    /// List trusted peers (default action)
    List,
    /// Forget a trusted peer
    Remove { target: String },
    /// Rename a trusted peer
    Rename { target: String, new_name: String },
    /// Get / set / clear persistent default peer
    Default {
        name: Option<String>,
        #[arg(long)]
        clear: bool,
    },
    /// Set / clear / show controller description override for a peer
    Describe {
        name: String,
        text: Option<String>,
        #[arg(long)]
        clear: bool,
    },
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

        Commands::Pair {
            host,
            port,
            code,
            timeout,
        } => {
            let server = OpendeskServer::new(&host, port, None, vec![], None)?;
            let pair_code = code.unwrap_or_else(|| generate_pairing_code(6));
            server.run_pair(&pair_code, timeout).await?;
        }

        Commands::PairWith {
            host,
            code,
            port,
            name,
            rendezvous,
            target_pubkey,
            rendezvous_token,
        } => {
            let (_, server_pub) = pair_with(
                host.as_deref(),
                Some(port),
                &code,
                name.as_deref(),
                rendezvous.as_deref(),
                rendezvous_token.as_deref(),
                target_pubkey.as_deref(),
                None,
            )
            .await?;

            let fp = fingerprint(&server_pub);
            let display_name = name.unwrap_or_else(|| format!("peer-{}", &data_encoding::HEXLOWER.encode(&server_pub)[..6]));
            println!("✓ Paired with {} ({})", display_name, fp);
            println!("  Now reachable as: opendesk connect {}", display_name);
        }

        Commands::Serve {
            host,
            port,
            rendezvous,
            rendezvous_token,
        } => {
            let server = OpendeskServer::new(&host, port, None, rendezvous, rendezvous_token)?;
            server.serve_forever().await?;
        }

        Commands::Connect {
            peer,
            rendezvous,
            rendezvous_token,
            screenshot,
        } => {
            println!("Connecting to peer...");
            let remote = remote_connect(
                peer.as_deref(),
                rendezvous.as_deref(),
                rendezvous_token.as_deref(),
                None,
            )
            .await?;

            println!("✓ Successfully connected to peer!");
            let caps = remote.capabilities();
            println!("Capabilities: {:?}", caps);

            if let Some(shot_path) = screenshot {
                println!("Capturing screenshot from peer...");
                let b64 = remote.screenshot("png", None).await?;
                let png_bytes = data_encoding::BASE64.decode(b64.as_bytes())?;
                std::fs::write(&shot_path, png_bytes)?;
                println!("✓ Screenshot saved to: {}", shot_path.display());
            }
        }

        Commands::Peers { subcmd } => {
            let trusted = TrustedPeers::new(None);
            match subcmd.unwrap_or(PeersCommands::List) {
                PeersCommands::List => {
                    let peers = trusted.list();
                    if peers.is_empty() {
                        println!("No trusted peers. Run `opendesk pair` or `opendesk pair-with`.");
                        return Ok(());
                    }
                    let default = trusted.get_default();
                    println!("{:<22}  {:<22}  {:<22}  DESCRIPTION", "NAME", "FINGERPRINT", "LAST ENDPOINT");
                    for p in peers {
                        let marker = if Some(&p.name) == default.as_ref() { "  [default]" } else { "" };
                        let endpoint = if !p.rendezvous_url.is_empty() {
                            format!("rendezvous ({})", p.rendezvous_url)
                        } else if !p.last_host.is_empty() {
                            format!("{}:{}", p.last_host, p.last_port)
                        } else {
                            "(unknown)".to_string()
                        };
                        let desc = p.effective_description();
                        let desc_line = desc.lines().next().unwrap_or("");
                        let short_desc = if desc_line.len() > 60 {
                            format!("{}…", &desc_line[..60])
                        } else {
                            desc_line.to_string()
                        };
                        println!("{:<22}  {:<22}  {:<22}  {}{}", p.name, p.fingerprint(), endpoint, short_desc, marker);
                    }
                }
                PeersCommands::Remove { target } => {
                    if trusted.remove(&target)? {
                        println!("✓ Removed peer '{}'.", target);
                    } else {
                        eprintln!("No peer matched '{}'.", target);
                        std::process::exit(1);
                    }
                }
                PeersCommands::Rename { target, new_name } => {
                    if trusted.rename(&target, &new_name)? {
                        println!("✓ Renamed {} → {}.", target, new_name);
                    } else {
                        eprintln!("No peer matched '{}'.", target);
                        std::process::exit(1);
                    }
                }
                PeersCommands::Default { name, clear } => {
                    if clear {
                        if trusted.clear_default()? {
                            println!("Default peer cleared.");
                        } else {
                            println!("No default peer was set.");
                        }
                    } else if let Some(n) = name {
                        if trusted.set_default(&n)? {
                            println!("Default peer is now: {}", n);
                        } else {
                            eprintln!("No trusted peer named '{}'.", n);
                            std::process::exit(1);
                        }
                    } else {
                        match trusted.get_default() {
                            Some(d) => println!("{}", d),
                            None => println!("No default peer set."),
                        }
                    }
                }
                PeersCommands::Describe { name, text, clear } => {
                    if clear {
                        if trusted.clear_description_override(&name)? {
                            println!("Description override for {} cleared.", name);
                        } else {
                            eprintln!("No peer named '{}'.", name);
                            std::process::exit(1);
                        }
                    } else if let Some(t) = text {
                        if trusted.set_description_override(&name, &t)? {
                            println!("Description override saved for {}.", name);
                        } else {
                            eprintln!("No peer named '{}'.", name);
                            std::process::exit(1);
                        }
                    } else if let Some(p) = trusted.find_by_name(&name) {
                        if !p.description_override.is_empty() {
                            println!("override: {}", p.description_override);
                        }
                        if !p.description.is_empty() {
                            println!("broadcast: {}", p.description);
                        }
                        if p.description_override.is_empty() && p.description.is_empty() {
                            println!("(no description set)");
                        }
                    } else {
                        eprintln!("No peer named '{}'.", name);
                        std::process::exit(1);
                    }
                }
            }
        }

        Commands::Describe { text, clear } => {
            if clear {
                if clear_description(None)? {
                    println!("Description cleared.");
                } else {
                    println!("No description was set.");
                }
            } else if let Some(t) = text {
                write_description(None, &t)?;
                println!("Description saved. Next session will broadcast it.");
            } else {
                let current = read_description(None);
                if current.is_empty() {
                    println!("(no description set)");
                } else {
                    println!("{}", current);
                }
            }
        }

        Commands::Rendezvous { host, port, token } => {
            let server = RendezvousServer::new(&host, port, token);
            server.serve_forever().await?;
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
