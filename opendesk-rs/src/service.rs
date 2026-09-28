use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const SERVICE_NAME: &str = "opendesk";
pub const LAUNCHD_LABEL: &str = "com.opendesk.serve";
pub const DEFAULT_PORT: u16 = 8423;

#[derive(Debug)]
pub struct ServiceInstallation {
    pub path: PathBuf,
    pub started: bool,
    pub manager: String, // "systemd" | "launchd" | "schtasks"
}

pub fn install_service(
    port: u16,
    autostart: bool,
    rendezvous: Option<&str>,
    rendezvous_token: Option<&str>,
) -> Result<ServiceInstallation> {
    let exe = std::env::current_exe().context("failed to locate current executable")?;

    #[cfg(target_os = "windows")]
    {
        install_schtasks(&exe, port, autostart, rendezvous, rendezvous_token)
    }

    #[cfg(target_os = "macos")]
    {
        install_launchd(&exe, port, autostart, rendezvous, rendezvous_token)
    }

    #[cfg(target_os = "linux")]
    {
        install_systemd(&exe, port, autostart, rendezvous, rendezvous_token)
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Err(anyhow!("Service install not supported on this OS"))
    }
}

pub fn uninstall_service() -> Result<bool> {
    #[cfg(target_os = "windows")]
    {
        uninstall_schtasks()
    }

    #[cfg(target_os = "macos")]
    {
        uninstall_launchd()
    }

    #[cfg(target_os = "linux")]
    {
        uninstall_systemd()
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Err(anyhow!("Service uninstall not supported on this OS"))
    }
}

// ---------------------------------------------------------------------------
// Windows — Task Scheduler (schtasks)
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn install_schtasks(
    exe: &Path,
    port: u16,
    autostart: bool,
    rendezvous: Option<&str>,
    rendezvous_token: Option<&str>,
) -> Result<ServiceInstallation> {
    let mut tr = format!("\"{}\" serve --port {}", exe.display(), port);
    if let Some(r) = rendezvous {
        tr.push_str(&format!(" --rendezvous \"{}\"", r));
    }
    if let Some(tok) = rendezvous_token {
        tr.push_str(&format!(" --rendezvous-token \"{}\"", tok));
    }

    let status = Command::new("schtasks")
        .args([
            "/create",
            "/tn",
            SERVICE_NAME,
            "/tr",
            &tr,
            "/sc",
            "onlogon",
            "/rl",
            "limited",
            "/f",
        ])
        .output()
        .context("failed to execute schtasks.exe")?;

    if !status.status.success() {
        let err = String::from_utf8_lossy(&status.stderr);
        return Err(anyhow!("schtasks /create failed: {}", err.trim()));
    }

    let mut started = false;
    if autostart {
        let run_status = Command::new("schtasks")
            .args(["/run", "/tn", SERVICE_NAME])
            .output();
        if let Ok(out) = run_status {
            started = out.status.success();
        }
    }

    Ok(ServiceInstallation {
        path: PathBuf::from(format!("TaskScheduler:{}", SERVICE_NAME)),
        started,
        manager: "schtasks".to_string(),
    })
}

#[cfg(target_os = "windows")]
fn uninstall_schtasks() -> Result<bool> {
    let output = Command::new("schtasks")
        .args(["/delete", "/tn", SERVICE_NAME, "/f"])
        .output()
        .context("failed to execute schtasks.exe")?;

    Ok(output.status.success())
}

// ---------------------------------------------------------------------------
// Linux — systemd user unit
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn systemd_unit_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("systemd")
        .join("user")
        .join(format!("{}.service", SERVICE_NAME)))
}

#[cfg(target_os = "linux")]
fn render_systemd_unit(
    exe: &Path,
    port: u16,
    rendezvous: Option<&str>,
    rendezvous_token: Option<&str>,
) -> String {
    let mut args = format!("serve --port {}", port);
    if let Some(r) = rendezvous {
        args.push_str(&format!(" --rendezvous {}", r));
    }
    if let Some(tok) = rendezvous_token {
        args.push_str(&format!(" --rendezvous-token {}", tok));
    }

    format!(
        r#"[Unit]
Description=opendesk serve — control this machine from a paired controller
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart="{}" {}
Restart=always
RestartSec=5s

[Install]
WantedBy=default.target
"#,
        exe.display(),
        args
    )
}

#[cfg(target_os = "linux")]
fn install_systemd(
    exe: &Path,
    port: u16,
    autostart: bool,
    rendezvous: Option<&str>,
    rendezvous_token: Option<&str>,
) -> Result<ServiceInstallation> {
    let path = systemd_unit_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = render_systemd_unit(exe, port, rendezvous, rendezvous_token);
    std::fs::write(&path, content)?;

    let mut started = false;
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output();

    if autostart {
        let out = Command::new("systemctl")
            .args(["--user", "enable", "--now", SERVICE_NAME])
            .output();
        if let Ok(o) = out {
            started = o.status.success();
        }
    }

    Ok(ServiceInstallation {
        path,
        started,
        manager: "systemd".to_string(),
    })
}

#[cfg(target_os = "linux")]
fn uninstall_systemd() -> Result<bool> {
    let path = systemd_unit_path()?;
    let _ = Command::new("systemctl")
        .args(["--user", "disable", "--now", SERVICE_NAME])
        .output();

    let removed = if path.exists() {
        std::fs::remove_file(&path)?;
        true
    } else {
        false
    };

    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output();

    Ok(removed)
}

// ---------------------------------------------------------------------------
// macOS — launchd agent
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn launchd_plist_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{}.plist", LAUNCHD_LABEL)))
}

#[cfg(target_os = "macos")]
fn render_launchd_plist(
    exe: &Path,
    port: u16,
    rendezvous: Option<&str>,
    rendezvous_token: Option<&str>,
) -> String {
    let mut args_xml = format!(
        "<string>{}</string>\n        <string>serve</string>\n        <string>--port</string>\n        <string>{}</string>",
        exe.display(),
        port
    );
    if let Some(r) = rendezvous {
        args_xml.push_str(&format!(
            "\n        <string>--rendezvous</string>\n        <string>{}</string>",
            r
        ));
    }
    if let Some(tok) = rendezvous_token {
        args_xml.push_str(&format!(
            "\n        <string>--rendezvous-token</string>\n        <string>{}</string>",
            tok
        ));
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        {}
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
"#,
        LAUNCHD_LABEL, args_xml
    )
}

#[cfg(target_os = "macos")]
fn install_launchd(
    exe: &Path,
    port: u16,
    autostart: bool,
    rendezvous: Option<&str>,
    rendezvous_token: Option<&str>,
) -> Result<ServiceInstallation> {
    let path = launchd_plist_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = render_launchd_plist(exe, port, rendezvous, rendezvous_token);
    std::fs::write(&path, content)?;

    let mut started = false;
    if autostart {
        let _ = Command::new("launchctl")
            .args(["unload", "-w", &path.to_string_lossy()])
            .output();
        let out = Command::new("launchctl")
            .args(["load", "-w", &path.to_string_lossy()])
            .output();
        if let Ok(o) = out {
            started = o.status.success();
        }
    }

    Ok(ServiceInstallation {
        path,
        started,
        manager: "launchd".to_string(),
    })
}

#[cfg(target_os = "macos")]
fn uninstall_launchd() -> Result<bool> {
    let path = launchd_plist_path()?;
    if !path.exists() {
        return Ok(false);
    }

    let _ = Command::new("launchctl")
        .args(["unload", "-w", &path.to_string_lossy()])
        .output();
    std::fs::remove_file(&path)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_constants() {
        assert_eq!(SERVICE_NAME, "opendesk");
        assert_eq!(LAUNCHD_LABEL, "com.opendesk.serve");
        assert_eq!(DEFAULT_PORT, 8423);
    }
}
