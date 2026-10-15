//! The foreground window, and guarding input so it only reaches the window
//! the caller expects.
//!
//! Synthetic input goes to whatever window is in front. If focus moved (a
//! notification, a chat window, the agent's own terminal), keystrokes meant
//! for one app land in another. A [`WindowGuard`] is checked on the machine
//! receiving the input immediately before it is sent.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Foreground {
    pub pid: u32,
    /// Window title.
    pub title: String,
    /// Executable file name, e.g. `chrome.exe`.
    pub process: String,
}

/// Input is only sent while the foreground window matches every given
/// field. Matching is case-insensitive: `title` is a substring of the
/// window title, `process` the executable name with or without `.exe`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowGuard {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
}

fn strip_exe(s: &str) -> String {
    let l = s.trim().to_lowercase();
    l.strip_suffix(".exe").map(str::to_string).unwrap_or(l)
}

impl WindowGuard {
    pub fn is_empty(&self) -> bool {
        self.title.as_deref().is_none_or(|t| t.trim().is_empty())
            && self.process.as_deref().is_none_or(|p| p.trim().is_empty())
    }

    pub fn matches(&self, fg: &Foreground) -> bool {
        let title_ok = self.title.as_deref().is_none_or(|t| {
            t.trim().is_empty() || fg.title.to_lowercase().contains(&t.trim().to_lowercase())
        });
        let process_ok = self
            .process
            .as_deref()
            .is_none_or(|p| p.trim().is_empty() || strip_exe(p) == strip_exe(&fg.process));
        title_ok && process_ok
    }

    /// Fail unless the current foreground window matches. Fails closed when
    /// the foreground window cannot be determined.
    pub fn check(&self) -> Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        match foreground() {
            Some(fg) if self.matches(&fg) => Ok(()),
            Some(fg) => bail!(
                "input refused: the foreground window is '{}' ({}), not the expected {}",
                fg.title,
                fg.process,
                self.describe()
            ),
            None => bail!(
                "input refused: cannot determine the foreground window to match {}",
                self.describe()
            ),
        }
    }

    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(t) = self.title.as_deref().filter(|t| !t.trim().is_empty()) {
            parts.push(format!("title containing '{t}'"));
        }
        if let Some(p) = self.process.as_deref().filter(|p| !p.trim().is_empty()) {
            parts.push(format!("process '{p}'"));
        }
        parts.join(" and ")
    }

    /// Read a guard from a JSON tool argument: `{"title": .., "process": ..}`.
    pub fn from_json(v: Option<&serde_json::Value>) -> Option<Self> {
        let g: Self = serde_json::from_value(v?.clone()).ok()?;
        (!g.is_empty()).then_some(g)
    }

    /// Read a guard from an RPC parameter map.
    pub fn from_rmpv(v: Option<&rmpv::Value>) -> Option<Self> {
        let get = |k: &str| {
            crate::protocol::frames::value_get(v?, k)
                .and_then(|x| x.as_str())
                .map(str::to_string)
        };
        let g = Self {
            title: get("title"),
            process: get("process"),
        };
        (!g.is_empty()).then_some(g)
    }

    pub fn to_rmpv(&self) -> rmpv::Value {
        let mut m = Vec::new();
        if let Some(t) = &self.title {
            m.push((rmpv::Value::from("title"), rmpv::Value::from(t.as_str())));
        }
        if let Some(p) = &self.process {
            m.push((rmpv::Value::from("process"), rmpv::Value::from(p.as_str())));
        }
        rmpv::Value::Map(m)
    }
}

/// The window currently in front, or `None` if there is none or it cannot
/// be inspected.
#[cfg(target_os = "windows")]
pub fn foreground() -> Option<Foreground> {
    use std::ffi::c_void;

    unsafe extern "system" {
        fn GetForegroundWindow() -> *mut c_void;
        fn GetWindowTextW(hwnd: *mut c_void, buf: *mut u16, max: i32) -> i32;
        fn GetWindowThreadProcessId(hwnd: *mut c_void, pid: *mut u32) -> u32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn QueryFullProcessImageNameW(
            process: *mut c_void,
            flags: u32,
            buf: *mut u16,
            size: *mut u32,
        ) -> i32;
        fn CloseHandle(h: *mut c_void) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
        let title = String::from_utf16_lossy(&buf[..n.min(buf.len())]);

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return None;
        }
        let mut process = String::new();
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if !h.is_null() {
            let mut path = [0u16; 1024];
            let mut size = path.len() as u32;
            if QueryFullProcessImageNameW(h, 0, path.as_mut_ptr(), &mut size) != 0 {
                let full = String::from_utf16_lossy(&path[..(size as usize).min(path.len())]);
                process = full
                    .rsplit(['\\', '/'])
                    .next()
                    .unwrap_or_default()
                    .to_string();
            }
            CloseHandle(h);
        }
        Some(Foreground {
            pid,
            title,
            process,
        })
    }
}

#[cfg(not(target_os = "windows"))]
pub fn foreground() -> Option<Foreground> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fg(title: &str, process: &str) -> Foreground {
        Foreground {
            pid: 1,
            title: title.into(),
            process: process.into(),
        }
    }

    #[test]
    fn title_is_a_case_insensitive_substring() {
        let g = WindowGuard {
            title: Some("notepad".into()),
            process: None,
        };
        assert!(g.matches(&fg("notes.txt - Notepad", "notepad.exe")));
        assert!(!g.matches(&fg("WhatsApp", "WhatsApp.exe")));
    }

    #[test]
    fn process_ignores_case_and_exe() {
        let g = WindowGuard {
            title: None,
            process: Some("Chrome".into()),
        };
        assert!(g.matches(&fg("x", "chrome.exe")));
        assert!(!g.matches(&fg("x", "chromedriver.exe")));
    }

    #[test]
    fn all_fields_must_match() {
        let g = WindowGuard {
            title: Some("signup".into()),
            process: Some("chrome.exe".into()),
        };
        assert!(g.matches(&fg("Signup form - Chrome", "chrome.exe")));
        assert!(!g.matches(&fg("Signup form - Edge", "msedge.exe")));
    }

    #[test]
    fn empty_guard_allows_everything() {
        let g = WindowGuard::default();
        assert!(g.is_empty());
        assert!(g.check().is_ok());
        assert!(WindowGuard::from_json(Some(&serde_json::json!({"title": " "}))).is_none());
    }

    #[test]
    fn guard_round_trips_through_rpc_params() {
        let g = WindowGuard {
            title: Some("Calc".into()),
            process: Some("calc.exe".into()),
        };
        assert_eq!(WindowGuard::from_rmpv(Some(&g.to_rmpv())), Some(g));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn a_guard_for_a_window_that_is_not_in_front_refuses() {
        let g = WindowGuard {
            title: Some("opendesk-no-such-window-7f3a".into()),
            process: None,
        };
        let err = g.check().unwrap_err().to_string();
        assert!(err.contains("input refused"), "{err}");
    }
}
