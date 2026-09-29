use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use xa11y::{App, AppExt, Key, Point, Rect, ScrollDelta, TreeNode, input_sim};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    pub pid: Option<u32>,
}

#[derive(Default)]
pub struct LocalComputer;

impl LocalComputer {
    pub fn new() -> Self {
        Self
    }

    // -----------------------------------------------------------------------
    // Display / Screenshot
    // -----------------------------------------------------------------------

    pub fn screenshot(&self, region: Option<Rect>) -> Result<Vec<u8>> {
        let shot = match region {
            Some(r) => {
                xa11y::screenshot_region(r).map_err(|e| anyhow!("region screenshot failed: {e}"))?
            }
            None => {
                xa11y::screenshot().map_err(|e| anyhow!("fullscreen screenshot failed: {e}"))?
            }
        };
        shot.to_png().context("PNG encoding failed")
    }

    // -----------------------------------------------------------------------
    // Mouse
    // -----------------------------------------------------------------------

    pub fn mouse_move(&self, x: i32, y: i32) -> Result<()> {
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        sim.mouse()
            .move_to(Point { x, y })
            .map_err(|e| anyhow!("mouse_move failed: {e}"))
    }

    pub fn mouse_click(&self, x: i32, y: i32, button: Option<&str>) -> Result<()> {
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        let mouse = sim.mouse();
        let target = Point { x, y };

        match button.unwrap_or("left").to_lowercase().as_str() {
            "right" => mouse.right_click(target).map_err(|e| anyhow!("{e}"))?,
            "middle" => {
                mouse.move_to(target).map_err(|e| anyhow!("{e}"))?;
                mouse
                    .down(xa11y::MouseButton::Middle)
                    .map_err(|e| anyhow!("{e}"))?;
                mouse
                    .up(xa11y::MouseButton::Middle)
                    .map_err(|e| anyhow!("{e}"))?;
            }
            _ => mouse.click(target).map_err(|e| anyhow!("{e}"))?,
        }
        Ok(())
    }

    pub fn mouse_double_click(&self, x: i32, y: i32) -> Result<()> {
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        sim.mouse()
            .double_click(Point { x, y })
            .map_err(|e| anyhow!("double_click failed: {e}"))
    }

    pub fn mouse_drag(&self, from_x: i32, from_y: i32, to_x: i32, to_y: i32) -> Result<()> {
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        sim.mouse()
            .drag(
                Point {
                    x: from_x,
                    y: from_y,
                },
                Point { x: to_x, y: to_y },
            )
            .map_err(|e| anyhow!("drag failed: {e}"))
    }

    pub fn mouse_scroll(&self, x: i32, y: i32, dy: i32) -> Result<()> {
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        sim.mouse()
            .scroll(Point { x, y }, ScrollDelta { dx: 0, dy })
            .map_err(|e| anyhow!("scroll failed: {e}"))
    }

    pub fn mouse_down(&self, button: Option<&str>) -> Result<()> {
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        let btn = match button.unwrap_or("left").to_lowercase().as_str() {
            "right" => xa11y::MouseButton::Right,
            "middle" => xa11y::MouseButton::Middle,
            _ => xa11y::MouseButton::Left,
        };
        sim.mouse()
            .down(btn)
            .map_err(|e| anyhow!("mouse_down failed: {e}"))
    }

    pub fn mouse_up(&self, button: Option<&str>) -> Result<()> {
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        let btn = match button.unwrap_or("left").to_lowercase().as_str() {
            "right" => xa11y::MouseButton::Right,
            "middle" => xa11y::MouseButton::Middle,
            _ => xa11y::MouseButton::Left,
        };
        sim.mouse()
            .up(btn)
            .map_err(|e| anyhow!("mouse_up failed: {e}"))
    }

    pub fn mouse_triple_click(&self, x: i32, y: i32) -> Result<()> {
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        let mouse = sim.mouse();
        let target = Point { x, y };
        mouse.click(target).map_err(|e| anyhow!("{e}"))?;
        mouse.click(target).map_err(|e| anyhow!("{e}"))?;
        mouse.click(target).map_err(|e| anyhow!("{e}"))?;
        Ok(())
    }

    pub fn cursor_position(&self) -> Result<(i32, i32)> {
        #[cfg(target_os = "windows")]
        {
            #[allow(clippy::upper_case_acronyms)]
            #[repr(C)]
            struct POINT {
                x: i32,
                y: i32,
            }
            unsafe extern "system" {
                fn GetCursorPos(lpPoint: *mut POINT) -> i32;
            }
            let mut pt = POINT { x: 0, y: 0 };
            unsafe {
                if GetCursorPos(&mut pt) != 0 {
                    Ok((pt.x, pt.y))
                } else {
                    Err(anyhow!("GetCursorPos failed"))
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            Ok((0, 0))
        }
    }

    // -----------------------------------------------------------------------
    // Keyboard
    // -----------------------------------------------------------------------

    pub fn keyboard_type(&self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }

        // For multiline text, strings with newlines, or longer text blocks (> 10 chars),
        // use clipboard paste for maximum reliability and Unicode/formatting fidelity.
        // This avoids key drops and buffer queue overflows in WinUI 3/XAML, web browsers,
        // and modern desktop GUI frameworks (matching Python OpenDesk reference behavior).
        if text.contains('\n') || text.contains('\r') || text.chars().count() > 10 {
            self.clipboard_write(text)?;
            std::thread::sleep(Duration::from_millis(50));
            #[cfg(target_os = "macos")]
            self.keyboard_hotkey(&["command", "v"])?;
            #[cfg(not(target_os = "macos"))]
            self.keyboard_hotkey(&["ctrl", "v"])?;
            std::thread::sleep(Duration::from_millis(50));
            return Ok(());
        }

        // For short single-line strings, type character-by-character with a small
        // debounce/inter-key delay so input queues don't drop events.
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        for ch in text.chars() {
            if ch == '\n' || ch == '\r' {
                self.keyboard_press("enter")?;
            } else {
                let s = ch.to_string();
                sim.keyboard()
                    .type_text(&s)
                    .map_err(|e| anyhow!("type_text failed: {e}"))?;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        Ok(())
    }

    pub fn keyboard_press(&self, key_str: &str) -> Result<()> {
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        let key = parse_key(key_str).ok_or_else(|| anyhow!("unknown key: {key_str}"))?;
        sim.keyboard()
            .press(key)
            .map_err(|e| anyhow!("press failed: {e}"))
    }

    pub fn keyboard_hotkey(&self, keys: &[&str]) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        let parsed_keys: Vec<Key> = keys
            .iter()
            .map(|k| parse_key(k).ok_or_else(|| anyhow!("unknown key: {k}")))
            .collect::<Result<_, _>>()?;

        let (target, modifiers) = parsed_keys.split_last().unwrap();
        sim.keyboard()
            .chord(target.clone(), modifiers)
            .map_err(|e| anyhow!("chord failed: {e}"))
    }

    pub fn keyboard_hold(&self, key_str: &str, duration_secs: f64) -> Result<()> {
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        let key = parse_key(key_str).ok_or_else(|| anyhow!("unknown key: {key_str}"))?;
        sim.keyboard()
            .down(key.clone())
            .map_err(|e| anyhow!("down failed: {e}"))?;
        std::thread::sleep(Duration::from_secs_f64(duration_secs.max(0.0)));
        sim.keyboard()
            .up(key)
            .map_err(|e| anyhow!("up failed: {e}"))?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // App management
    // -----------------------------------------------------------------------

    fn find_app(name: &str) -> Result<App> {
        if let Ok(app) = App::by_name(name, Duration::from_millis(800)) {
            return Ok(app);
        }
        let lower = name.trim().to_lowercase();
        let lower_no_exe = lower.strip_suffix(".exe").unwrap_or(&lower);
        App::find(Duration::from_millis(800), |d| {
            if let Some(ref n) = d.name {
                let nl = n.to_lowercase();
                let nl_no_exe = nl.strip_suffix(".exe").unwrap_or(&nl);
                nl == lower || nl_no_exe == lower_no_exe || nl.contains(lower_no_exe)
            } else {
                false
            }
        })
        .map_err(|e| anyhow!("app '{name}' not found: {e}"))
    }

    pub fn app_open(&self, name: &str) -> Result<()> {
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("cmd")
                .args(["/c", "start", "", name])
                .spawn()
                .context("failed to spawn application")?;
        }
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .args(["-a", name])
                .spawn()
                .context("failed to open application")?;
        }
        #[cfg(target_os = "linux")]
        {
            std::process::Command::new("xdg-open")
                .arg(name)
                .spawn()
                .context("failed to open application")?;
        }
        Ok(())
    }

    pub fn app_focus(&self, name: &str) -> Result<()> {
        let app = Self::find_app(name)?;

        // Top-level windows implement WindowPattern / activate on Windows/macOS/Linux.
        // Trying to activate the root Application node directly fails on Windows UIA
        // with "Action activate not supported on application".
        if let Ok(windows) = app.windows() {
            for win in windows {
                if win.activate().is_ok() {
                    return Ok(());
                }
            }
        }

        // Fallback: try activating the application element directly
        if app.as_element().activate().is_ok() {
            return Ok(());
        }

        #[cfg(target_os = "windows")]
        {
            if let Some(pid) = app.pid {
                let script = format!(
                    "$p = Get-Process -Id {} -ErrorAction SilentlyContinue; if ($p -and $p.MainWindowHandle -ne 0) {{ $w = Add-Type -MemberDefinition '[DllImport(\"user32.dll\")] public static extern bool SetForegroundWindow(IntPtr hWnd);' -Name W32 -Namespace W32 -PassThru; $w::SetForegroundWindow($p.MainWindowHandle) }}",
                    pid
                );
                let _ = std::process::Command::new("powershell")
                    .args(["-NoProfile", "-Command", &script])
                    .output();
                return Ok(());
            }
        }

        bail!("could not activate application '{name}'")
    }

    pub fn app_close(&self, name: &str) -> Result<()> {
        let app = Self::find_app(name)?;

        // Gracefully attempt to close top-level windows first
        let mut closed_any = false;
        if let Ok(windows) = app.windows() {
            for win in windows {
                if win.close().is_ok() {
                    closed_any = true;
                }
            }
        }

        if closed_any {
            std::thread::sleep(Duration::from_millis(100));
            return Ok(());
        }

        // If closing windows was not supported or failed, fallback to closing app element
        if app.as_element().close().is_ok() {
            return Ok(());
        }

        // Fallback to process termination (matching Python OpenDesk behavior)
        #[cfg(target_os = "windows")]
        {
            if let Some(pid) = app.pid {
                let r = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .output();
                if let Ok(out) = r
                    && out.status.success()
                {
                    return Ok(());
                }
            }
            let im = if name.to_lowercase().ends_with(".exe") {
                name.to_string()
            } else {
                format!("{name}.exe")
            };
            let r = std::process::Command::new("taskkill")
                .args(["/IM", &im, "/F"])
                .output();
            if let Ok(out) = r
                && out.status.success()
            {
                return Ok(());
            }
            bail!("could not close application '{name}'")
        }

        #[cfg(target_os = "macos")]
        {
            if let Some(pid) = app.pid {
                let _ = std::process::Command::new("kill")
                    .args(["-9", &pid.to_string()])
                    .output();
                return Ok(());
            }
            let _ = std::process::Command::new("killall").arg(name).output();
            Ok(())
        }

        #[cfg(target_os = "linux")]
        {
            if let Some(pid) = app.pid {
                let _ = std::process::Command::new("kill")
                    .args(["-9", &pid.to_string()])
                    .output();
                return Ok(());
            }
            let _ = std::process::Command::new("pkill")
                .args(["-f", name])
                .output();
            Ok(())
        }
    }

    pub fn app_list(&self) -> Result<Vec<AppInfo>> {
        let apps = App::list().map_err(|e| anyhow!("list apps failed: {e}"))?;
        Ok(apps
            .into_iter()
            .map(|a| AppInfo {
                name: a.name,
                pid: a.pid,
            })
            .collect())
    }

    // -----------------------------------------------------------------------
    // UI Automation via xa11y
    // -----------------------------------------------------------------------

    pub fn ui_tree(&self, app_name: Option<&str>, max_depth: Option<usize>) -> Result<String> {
        let app = match app_name {
            Some(name) => Self::find_app(name)?,
            None => App::foreground(Duration::from_secs(2))
                .map_err(|e| anyhow!("no foreground app: {e}"))?,
        };
        let tree_node = app
            .as_element()
            .tree(max_depth)
            .map_err(|e| anyhow!("dump tree failed: {e}"))?;
        let mut out = String::new();
        format_tree_node(&tree_node, 0, &mut out);
        Ok(out)
    }

    pub fn ui_click(&self, app_name: Option<&str>, selector: &str) -> Result<()> {
        let app = match app_name {
            Some(name) => Self::find_app(name)?,
            None => App::foreground(Duration::from_secs(2))
                .map_err(|e| anyhow!("no foreground app: {e}"))?,
        };
        let locator = app.locator(selector);
        locator
            .press()
            .map_err(|e| anyhow!("locator press '{selector}' failed: {e}"))
    }

    pub fn ui_type(&self, app_name: Option<&str>, selector: &str, text: &str) -> Result<()> {
        let app = match app_name {
            Some(name) => Self::find_app(name)?,
            None => App::foreground(Duration::from_secs(2))
                .map_err(|e| anyhow!("no foreground app: {e}"))?,
        };
        let locator = app.locator(selector);
        locator
            .type_text(text)
            .map_err(|e| anyhow!("locator type_text '{selector}' failed: {e}"))
    }

    // -----------------------------------------------------------------------
    // Clipboard
    // -----------------------------------------------------------------------

    pub fn clipboard_read(&self) -> Result<String> {
        let mut cb = arboard::Clipboard::new().context("failed to access clipboard")?;
        cb.get_text().context("failed to read clipboard text")
    }

    pub fn clipboard_write(&self, text: &str) -> Result<()> {
        let mut cb = arboard::Clipboard::new().context("failed to access clipboard")?;
        cb.set_text(text).context("failed to write clipboard text")
    }
}

fn format_tree_node(node: &TreeNode, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    out.push_str(&indent);
    out.push_str(&node.role);
    if let Some(ref n) = node.name {
        let escaped = n
            .replace('\\', "\\\\")
            .replace('\r', "")
            .replace('\n', "\\n")
            .replace('"', "\\\"");
        out.push_str(" \"");
        out.push_str(&escaped);
        out.push('"');
    }
    if let Some(ref v) = node.value {
        let escaped = v
            .replace('\\', "\\\\")
            .replace('\r', "")
            .replace('\n', "\\n")
            .replace('"', "\\\"");
        out.push_str(" value=\"");
        out.push_str(&escaped);
        out.push('"');
    }
    out.push('\n');
    for child in &node.children {
        format_tree_node(child, depth + 1, out);
    }
}

pub fn parse_key(s: &str) -> Option<Key> {
    match s.trim().to_lowercase().as_str() {
        "enter" | "return" => Some(Key::Enter),
        "esc" | "escape" => Some(Key::Escape),
        "backspace" => Some(Key::Backspace),
        "tab" => Some(Key::Tab),
        "space" => Some(Key::Space),
        "delete" | "del" => Some(Key::Delete),
        "insert" => Some(Key::Insert),
        "up" | "arrowup" => Some(Key::ArrowUp),
        "down" | "arrowdown" => Some(Key::ArrowDown),
        "left" | "arrowleft" => Some(Key::ArrowLeft),
        "right" | "arrowright" => Some(Key::ArrowRight),
        "home" => Some(Key::Home),
        "end" => Some(Key::End),
        "pageup" | "pgup" => Some(Key::PageUp),
        "pagedown" | "pgdn" => Some(Key::PageDown),
        "shift" => Some(Key::Shift),
        "ctrl" | "control" => Some(Key::Ctrl),
        "alt" => Some(Key::Alt),
        "meta" | "cmd" | "win" | "super" => Some(Key::Meta),
        "f1" => Some(Key::F(1)),
        "f2" => Some(Key::F(2)),
        "f3" => Some(Key::F(3)),
        "f4" => Some(Key::F(4)),
        "f5" => Some(Key::F(5)),
        "f6" => Some(Key::F(6)),
        "f7" => Some(Key::F(7)),
        "f8" => Some(Key::F(8)),
        "f9" => Some(Key::F(9)),
        "f10" => Some(Key::F(10)),
        "f11" => Some(Key::F(11)),
        "f12" => Some(Key::F(12)),
        other if other.len() == 1 => {
            let c = other.chars().next().unwrap();
            Some(Key::Char(c))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key() {
        assert_eq!(parse_key("enter"), Some(Key::Enter));
        assert_eq!(parse_key("Ctrl"), Some(Key::Ctrl));
        assert_eq!(parse_key("f5"), Some(Key::F(5)));
        assert_eq!(parse_key("a"), Some(Key::Char('a')));
        assert_eq!(parse_key("unknown_key_xyz"), None);
    }
}
