//! Platform permission self-check.
//!
//! On macOS, controlling mouse/keyboard requires the Accessibility entitlement
//! and capturing the screen requires the Screen Recording entitlement.
//! Linux and Windows have no equivalent permission gate; the checks return
//! "not required" so the same code path works cross-platform.

use std::process::Command;

#[derive(Clone, Debug)]
pub struct PermissionStatus {
    pub name: String,
    pub granted: bool,
    pub reason: String,
    pub settings_url: String,
    pub install_hint: String,
}

pub fn check_all() -> Vec<PermissionStatus> {
    #[cfg(target_os = "macos")]
    {
        vec![check_accessibility(), check_screen_recording()]
    }
    #[cfg(not(target_os = "macos"))]
    {
        vec![]
    }
}

#[cfg(target_os = "macos")]
fn check_accessibility() -> PermissionStatus {
    let output = Command::new("osascript")
        .args(["-e", "tell application \"System Events\" to get name of first process"])
        .output();

    match output {
        Ok(out) => {
            let combined = format!(
                "{} {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
            .to_lowercase();

            if out.status.success() && !combined.contains("not allowed") && !combined.contains("-1002") {
                PermissionStatus {
                    name: "Accessibility".into(),
                    granted: true,
                    reason: String::new(),
                    settings_url: String::new(),
                    install_hint: String::new(),
                }
            } else {
                PermissionStatus {
                    name: "Accessibility".into(),
                    granted: false,
                    reason: "System Events refused the probe (missing Accessibility access).".into(),
                    settings_url: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility".into(),
                    install_hint: "Grant Accessibility permissions in System Settings -> Privacy & Security -> Accessibility".into(),
                }
            }
        }
        Err(e) => PermissionStatus {
            name: "Accessibility".into(),
            granted: false,
            reason: format!("osascript probe failed: {e}"),
            settings_url: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility".into(),
            install_hint: "Grant Accessibility permissions in System Settings -> Privacy & Security -> Accessibility".into(),
        },
    }
}

#[cfg(target_os = "macos")]
fn check_screen_recording() -> PermissionStatus {
    // Try capturing screen pixel
    let computer = crate::computer::local::LocalComputer::new();
    match computer.screenshot("png", None) {
        Ok(_) => PermissionStatus {
            name: "Screen Recording".into(),
            granted: true,
            reason: String::new(),
            settings_url: String::new(),
            install_hint: String::new(),
        },
        Err(e) => PermissionStatus {
            name: "Screen Recording".into(),
            granted: false,
            reason: format!("Capture failed: {e}"),
            settings_url: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture".into(),
            install_hint: "Grant Screen Recording permissions in System Settings -> Privacy & Security -> Screen & System Audio Recording".into(),
        },
    }
}

pub fn open_settings(url: &str) {
    if url.is_empty() {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(url).spawn();
    }
    #[cfg(windows)]
    {
        let _ = Command::new("cmd").args(["/c", "start", url]).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("xdg-open").arg(url).spawn();
    }
}

pub fn report(statuses: &[PermissionStatus]) -> bool {
    if statuses.is_empty() {
        return true;
    }
    let mut all_ok = true;
    for s in statuses {
        let mark = if s.granted { "✓" } else { "✗" };
        println!("  {mark} {}", s.name);
        if !s.granted {
            all_ok = false;
            if !s.reason.is_empty() {
                println!("      {}", s.reason);
            }
            if !s.install_hint.is_empty() {
                for line in s.install_hint.lines() {
                    println!("      {line}");
                }
            }
        }
    }
    all_ok
}
