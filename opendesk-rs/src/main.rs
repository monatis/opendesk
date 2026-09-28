use clap::{Parser, Subcommand};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use opendesk_rs::mcp::server::McpServer;
use opendesk_rs::protocol::identity::generate_pairing_code;
use opendesk_rs::protocol::storage::{
    TrustedPeers, clear_description, fingerprint, read_description, write_description,
};
use opendesk_rs::remote::client::{connect as remote_connect, pair_with};
use opendesk_rs::remote::rendezvous::RendezvousServer;
use opendesk_rs::remote::server::OpendeskServer;

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
    /// Run as an MCP (Model Context Protocol) stdio server for agents
    Mcp,

    /// Run quick diagnostic test of local accessibility and input
    Test,

    /// Register opendesk MCP server with Claude Code
    Install {
        #[arg(long, default_value = "user")]
        scope: String,
    },

    /// Remove opendesk MCP server from Claude Code
    Uninstall,

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
        #[arg(long)]
        home: Option<PathBuf>,
        #[arg(long)]
        no_mdns: bool,
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
        #[arg(long)]
        home: Option<PathBuf>,
    },

    /// Serve the desktop for remote control over WebSocket or Rendezvous
    Serve {
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        #[arg(long, default_value_t = 8423)]
        port: u16,
        #[arg(long)]
        home: Option<PathBuf>,
        #[arg(long)]
        no_mdns: bool,
        #[arg(long)]
        no_audit: bool,
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
        #[arg(long)]
        home: Option<PathBuf>,
    },

    /// List opendesk peers visible on the LAN or rendezvous
    Discover {
        #[arg(long, default_value_t = 2.0)]
        timeout: f64,
        #[arg(long)]
        rendezvous: Option<String>,
        #[arg(long)]
        rendezvous_token: Option<String>,
    },

    /// Launch the local opendesk UI on http://127.0.0.1:8424
    App {
        #[arg(long, default_value_t = 8424)]
        port: u16,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long)]
        home: Option<PathBuf>,
        #[arg(long)]
        no_browser: bool,
    },

    /// Verify platform permissions (macOS Accessibility / Screen Recording)
    Check {
        #[arg(long)]
        open: bool,
        #[arg(long)]
        no_open: bool,
    },

    /// Print the server-side audit log (controlled machine)
    Audit {
        #[arg(long)]
        date: Option<String>,
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(short, long)]
        follow: bool,
        #[arg(long)]
        home: Option<PathBuf>,
    },

    /// Show the active controller session (controlled machine)
    Sessions {
        #[arg(long)]
        home: Option<PathBuf>,
    },

    /// Ask the active controller to leave (cooperative; sends session.evicted PUSH and closes)
    Disconnect {
        #[arg(long)]
        home: Option<PathBuf>,
    },

    /// Revoke a paired controller (and disconnect them if active)
    Unpair {
        name: String,
        #[arg(long)]
        home: Option<PathBuf>,
    },

    /// List, remove, rename, or set default trusted peer
    Peers {
        #[arg(long)]
        home: Option<PathBuf>,
        #[command(subcommand)]
        subcmd: Option<PeersCommands>,
    },

    /// Read / set / clear the broadcast description of this machine
    Describe {
        text: Option<String>,
        #[arg(long)]
        clear: bool,
        #[arg(long)]
        home: Option<PathBuf>,
    },

    /// (WSL only) Print or apply Windows-side port forwarding for WSL
    #[command(name = "wsl-setup")]
    WslSetup {
        #[arg(long, default_value_t = 8423)]
        port: u16,
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        undo: bool,
    },

    /// Manage the background task scheduler
    Scheduler {
        scheduler_cmd: String,
        #[arg(long)]
        dir: Option<PathBuf>,
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
    #[command(name = "install-service")]
    InstallService {
        #[arg(long, default_value_t = 8423)]
        port: u16,
        #[arg(long)]
        home: Option<PathBuf>,
        #[arg(long)]
        no_start: bool,
        #[arg(long)]
        rendezvous: Option<String>,
        #[arg(long)]
        rendezvous_token: Option<String>,
    },

    /// Uninstall the user-scoped opendesk OS service
    #[command(name = "uninstall-service")]
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
    /// Get / set the rendezvous server URL for a trusted peer
    Rendezvous { name: String, url: Option<String> },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "off".into()),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

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

        Commands::Install { scope } => {
            cmd_install(&scope)?;
        }

        Commands::Uninstall => {
            cmd_uninstall()?;
        }

        Commands::Pair {
            host,
            port,
            code,
            timeout,
            home,
            no_mdns,
        } => {
            let mut server = OpendeskServer::new(&host, port, home.as_deref(), vec![], None)?;
            server.set_advertise_mdns(!no_mdns);
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
            home,
        } => {
            let (_, server_pub) = pair_with(
                host.as_deref(),
                Some(port),
                &code,
                name.as_deref(),
                rendezvous.as_deref(),
                rendezvous_token.as_deref(),
                target_pubkey.as_deref(),
                home.as_deref(),
            )
            .await?;

            let fp = fingerprint(&server_pub);
            let display_name = name.unwrap_or_else(|| {
                format!("peer-{}", &data_encoding::HEXLOWER.encode(&server_pub)[..6])
            });
            println!("✓ Paired with {} ({})", display_name, fp);
            println!("  Now reachable as: opendesk connect {}", display_name);
        }

        Commands::Serve {
            host,
            port,
            home,
            no_mdns,
            no_audit,
            rendezvous,
            rendezvous_token,
        } => {
            let mut server =
                OpendeskServer::new(&host, port, home.as_deref(), rendezvous, rendezvous_token)?;
            server.set_advertise_mdns(!no_mdns);
            server.set_no_audit(no_audit);
            server.serve_forever().await?;
        }

        Commands::Connect {
            peer,
            rendezvous,
            rendezvous_token,
            screenshot,
            home,
        } => {
            println!("Connecting to peer...");
            let remote = remote_connect(
                peer.as_deref(),
                rendezvous.as_deref(),
                rendezvous_token.as_deref(),
                home.as_deref(),
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

        Commands::Discover {
            timeout,
            rendezvous,
            rendezvous_token,
        } => {
            if let Some(r_url) = rendezvous {
                let client = opendesk_rs::remote::rendezvous::RendezvousClient::new(
                    &r_url,
                    rendezvous_token.as_deref(),
                );
                let peers = client.list_peers(Duration::from_secs_f64(timeout)).await?;
                if peers.is_empty() {
                    println!("No online opendesk peers found on rendezvous {}.", r_url);
                    return Ok(());
                }
                println!(
                    "{:<24}  {:<22}  {:<22}  DESCRIPTION",
                    "NAME", "ADDR", "FINGERPRINT"
                );
                for p in peers {
                    let desc = if p.description.len() > 80 {
                        &p.description[..80]
                    } else {
                        &p.description
                    };
                    println!(
                        "{:<24}  {:<22}  {:<22}  {}",
                        p.name, "rendezvous", p.fingerprint, desc
                    );
                }
                return Ok(());
            }

            let peers =
                opendesk_rs::remote::discovery::discover(Duration::from_secs_f64(timeout)).await?;
            if peers.is_empty() {
                println!("No opendesk peers found on the LAN.");
                return Ok(());
            }
            println!(
                "{:<24}  {:<22}  {:<22}  DESCRIPTION",
                "NAME", "ADDR", "FINGERPRINT"
            );
            for p in peers {
                let desc = if p.description.len() > 80 {
                    &p.description[..80]
                } else {
                    &p.description
                };
                let addr = format!("{}:{}", p.host, p.port);
                println!(
                    "{:<24}  {:<22}  {:<22}  {}",
                    p.name, addr, p.fingerprint, desc
                );
            }
        }

        Commands::App {
            port,
            host,
            home,
            no_browser,
        } => {
            opendesk_rs::app::run_app(home.as_deref(), &host, port, !no_browser).await?;
        }

        Commands::Check { r#open, no_open } => {
            let statuses = opendesk_rs::computer::permissions::check_all();
            if statuses.is_empty() {
                println!("No platform permissions to check on this OS.");
                return Ok(());
            }
            println!("opendesk permission check:");
            let ok = opendesk_rs::computer::permissions::report(&statuses);
            if ok {
                println!("\nAll required permissions are granted.");
                return Ok(());
            }
            println!();
            if r#open && !no_open {
                for s in &statuses {
                    if !s.granted && !s.settings_url.is_empty() {
                        opendesk_rs::computer::permissions::open_settings(&s.settings_url);
                        break;
                    }
                }
                println!(
                    "Opened the relevant System Settings pane.  Re-run `opendesk check` after granting."
                );
            }
            std::process::exit(1);
        }

        Commands::Audit {
            date,
            peer,
            limit,
            follow,
            home,
        } => {
            let audit = opendesk_rs::remote::audit::AuditLog::new(home.as_deref());
            let matches_peer = |entry: &Value| -> bool {
                if let Some(p_filter) = peer.as_deref() {
                    let p_obj = entry.get("peer").unwrap_or(&Value::Null);
                    let name = p_obj.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let fp = p_obj.get("fp").and_then(|v| v.as_str()).unwrap_or("");
                    name.contains(p_filter) || fp.contains(p_filter)
                } else {
                    true
                }
            };

            if !follow {
                let mut entries = audit.iter_entries(date.as_deref());
                if let Some(lim) = limit
                    && entries.len() > lim
                {
                    entries = entries.split_off(entries.len() - lim);
                }
                for e in entries {
                    if matches_peer(&e) {
                        println!("{}", opendesk_rs::remote::audit::format_audit_entry(&e));
                    }
                }
                return Ok(());
            }

            // Follow mode: poll every 500ms
            let mut entries = audit.iter_entries(date.as_deref());
            let mut seen = entries.len();
            if let Some(lim) = limit
                && entries.len() > lim
            {
                entries = entries.split_off(entries.len() - lim);
            }
            for e in entries {
                if matches_peer(&e) {
                    println!("{}", opendesk_rs::remote::audit::format_audit_entry(&e));
                }
            }

            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let current = audit.iter_entries(date.as_deref());
                if current.len() > seen {
                    for e in &current[seen..] {
                        if matches_peer(e) {
                            println!("{}", opendesk_rs::remote::audit::format_audit_entry(e));
                        }
                    }
                    seen = current.len();
                }
            }
        }

        Commands::Sessions { home } => {
            let mut client =
                match opendesk_rs::remote::admin::AdminClient::connect(home.as_deref()).await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("ERROR: {}", e);
                        std::process::exit(1);
                    }
                };
            let sessions = client.list_sessions().await?;
            if sessions.is_empty() {
                println!("No active session.");
                return Ok(());
            }
            println!("{:<22}  {:<22}  {:<8}  ID", "PEER", "FROM", "AGE");
            for s in sessions {
                let age = opendesk_rs::remote::admin::format_age(s.age_seconds);
                println!(
                    "{:<22}  {:<22}  {:<8}  {}",
                    s.peer_name, s.remote_addr, age, s.id
                );
            }
        }

        Commands::Disconnect { home } => {
            let mut client =
                match opendesk_rs::remote::admin::AdminClient::connect(home.as_deref()).await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("ERROR: {}", e);
                        std::process::exit(1);
                    }
                };
            let n = client.kill_all().await?;
            if n == 0 {
                println!("No active session to disconnect.");
            } else {
                println!("Disconnected the active controller.");
            }
        }

        Commands::Unpair { name, home } => {
            let trusted = TrustedPeers::new(home.as_deref());
            do_unpair(&trusted, &name, home.as_deref()).await?;
        }

        Commands::Peers { home, subcmd } => {
            let trusted = TrustedPeers::new(home.as_deref());
            match subcmd.unwrap_or(PeersCommands::List) {
                PeersCommands::List => {
                    let peers = trusted.list();
                    if peers.is_empty() {
                        println!("No trusted peers.");
                        return Ok(());
                    }
                    let default = trusted.get_default();
                    println!(
                        "{:<22}  {:<22}  {:<22}  DESCRIPTION",
                        "NAME", "FINGERPRINT", "LAST ENDPOINT"
                    );
                    for p in peers {
                        let marker = if Some(&p.name) == default.as_ref() {
                            "  [default]"
                        } else {
                            ""
                        };
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
                        println!(
                            "{:<22}  {:<22}  {:<22}  {}{}",
                            p.name,
                            p.fingerprint(),
                            endpoint,
                            short_desc,
                            marker
                        );
                    }
                }
                PeersCommands::Remove { target } => {
                    do_unpair(&trusted, &target, home.as_deref()).await?;
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
                PeersCommands::Rendezvous { name, url } => {
                    let peer = trusted.find_by_name(&name);
                    if peer.is_none() {
                        eprintln!("No trusted peer named '{}'.", name);
                        std::process::exit(1);
                    }
                    let p = peer.unwrap();
                    if let Some(u) = url {
                        trusted.cache_rendezvous(&p.public_bytes()?, &u)?;
                        println!("Rendezvous URL for {} set to: {}", name, u);
                    } else if !p.rendezvous_url.is_empty() {
                        println!("{}", p.rendezvous_url);
                    } else {
                        println!("(no rendezvous URL set)");
                    }
                }
            }
        }

        Commands::Describe { text, clear, home } => {
            if clear {
                if clear_description(home.as_deref())? {
                    println!("Description cleared.");
                } else {
                    println!("No description was set.");
                }
            } else if let Some(t) = text {
                write_description(home.as_deref(), &t)?;
                println!("Description saved.  Next session will broadcast it.");
            } else {
                let current = read_description(home.as_deref());
                if current.is_empty() {
                    println!("(no description set)");
                } else {
                    println!("{}", current);
                }
            }
        }

        Commands::WslSetup { port, apply, undo } => {
            if undo {
                println!(
                    "netsh interface portproxy delete v4tov4 listenport={port} listenaddress=0.0.0.0"
                );
                println!("Remove-NetFirewallRule -DisplayName 'opendesk inbound {port}'");
            } else {
                println!(
                    "netsh interface portproxy add v4tov4 listenport={port} listenaddress=0.0.0.0 connectport={port} connectaddress=127.0.0.1"
                );
                println!(
                    "New-NetFirewallRule -DisplayName 'opendesk inbound {port}' -Direction Inbound -LocalPort {port} -Protocol TCP -Action Allow -Profile Private"
                );
            }
            if apply {
                println!(
                    "Note: To apply, run the above commands in an elevated PowerShell prompt."
                );
            }
        }

        Commands::Scheduler { scheduler_cmd, dir } => {
            let project_dir = dir
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            match scheduler_cmd.as_str() {
                "start" => {
                    opendesk_rs::automation::scheduler::run_scheduler_daemon(&project_dir).await?;
                }
                "list" => {
                    let store =
                        opendesk_rs::automation::scheduler::ScheduleStore::new(&project_dir);
                    let entries = store.all();
                    if entries.is_empty() {
                        println!("No schedules.");
                    } else {
                        for e in entries {
                            let status = if e.enabled { "on" } else { "off" };
                            println!("  [{status}] {}  ({})  →  {}", e.name, e.timing, e.task);
                        }
                    }
                }
                _ => {
                    println!("Usage: opendesk scheduler start|list");
                }
            }
        }

        Commands::Rendezvous { host, port, token } => {
            let server = RendezvousServer::new(&host, port, token);
            server.serve_forever().await?;
        }

        Commands::InstallService {
            port,
            home: _,
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
                    println!(
                        "✓ Service installed ({}): {}",
                        result.manager,
                        result.path.display()
                    );
                    if result.started {
                        println!("  Started. It will also run automatically on next login.");
                    } else if no_start {
                        println!(
                            "  Not started (--no-start). Activate manually or rerun without the flag."
                        );
                    } else {
                        println!(
                            "  WARNING: Service registered but could not be started immediately."
                        );
                    }
                }
                Err(e) => {
                    eprintln!("ERROR: Failed to install service: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::UninstallService => match opendesk_rs::service::uninstall_service() {
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
        },
    }

    Ok(())
}

async fn do_unpair(
    trusted: &TrustedPeers,
    target: &str,
    home: Option<&Path>,
) -> anyhow::Result<()> {
    let entry = trusted.find_by_name(target);
    let peer_name_for_kick = entry
        .as_ref()
        .map(|p| p.name.clone())
        .unwrap_or_else(|| target.to_string());
    if !trusted.remove(target)? {
        eprintln!("No peer matched '{}'.", target);
        std::process::exit(1);
    }

    let mut kicked = false;
    if let Ok(mut client) = opendesk_rs::remote::admin::AdminClient::connect(home).await
        && let Ok(sessions) = client.list_sessions().await
    {
        for s in sessions {
            if s.peer_name == peer_name_for_kick {
                if client.kill(&s.id).await.unwrap_or(false) {
                    kicked = true;
                }
                break;
            }
        }
    }

    if kicked {
        println!("Unpaired {} and disconnected the active session.", target);
    } else {
        println!("Unpaired {}.", target);
    }
    Ok(())
}

fn cmd_install(scope: &str) -> anyhow::Result<()> {
    let claude_bin = which("claude");
    if claude_bin.is_none() {
        eprintln!(
            "ERROR: 'claude' command not found.\nInstall Claude Code first: https://claude.ai/code"
        );
        std::process::exit(1);
    }
    let claude = claude_bin.unwrap();
    let current_exe = std::env::current_exe()?;
    let mcp_bin = current_exe.with_file_name(if cfg!(windows) {
        "opendesk-mcp.exe"
    } else {
        "opendesk-mcp"
    });
    let mcp_path = if mcp_bin.exists() {
        mcp_bin
    } else {
        current_exe
    };

    let _ = std::process::Command::new(&claude)
        .args(["mcp", "remove", "opendesk"])
        .output();

    let output = std::process::Command::new(&claude)
        .args([
            "mcp",
            "add",
            "opendesk",
            &format!("--scope={scope}"),
            "--",
            &mcp_path.to_string_lossy(),
        ])
        .output()?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        eprintln!("ERROR: {}", err.trim());
        std::process::exit(1);
    }

    println!("opendesk MCP server registered ({}).", scope);
    println!("  Binary: {}", mcp_path.display());
    println!("Start a Claude Code conversation and say 'take a screenshot' to verify.");
    Ok(())
}

fn cmd_uninstall() -> anyhow::Result<()> {
    let claude_bin = which("claude");
    if claude_bin.is_none() {
        eprintln!("ERROR: 'claude' command not found.");
        std::process::exit(1);
    }
    let claude = claude_bin.unwrap();
    let output = std::process::Command::new(&claude)
        .args(["mcp", "remove", "opendesk"])
        .output()?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        eprintln!("ERROR: {}", err.trim());
        std::process::exit(1);
    }

    println!("opendesk MCP server removed from Claude Code.");
    Ok(())
}

fn which(cmd: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for p in std::env::split_paths(&path_var) {
        let candidate = p.join(cmd);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let candidate_exe = p.join(format!("{}.exe", cmd));
            if candidate_exe.is_file() {
                return Some(candidate_exe);
            }
            let candidate_cmd = p.join(format!("{}.cmd", cmd));
            if candidate_cmd.is_file() {
                return Some(candidate_cmd);
            }
        }
    }
    None
}
