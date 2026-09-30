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
        self.screenshot_format(region, "png", 0, None)
            .map(|(bytes, _, _, _)| bytes)
    }

    pub fn screenshot_format(
        &self,
        region: Option<Rect>,
        format: &str,
        quality: u8,
        max_dim: Option<u32>,
    ) -> Result<(Vec<u8>, String, u32, u32)> {
        let mut shot = match region {
            Some(r) => {
                xa11y::screenshot_region(r).map_err(|e| anyhow!("region screenshot failed: {e}"))?
            }
            None => {
                xa11y::screenshot().map_err(|e| anyhow!("fullscreen screenshot failed: {e}"))?
            }
        };

        if let Some(max_d) = max_dim {
            if shot.width > max_d || shot.height > max_d {
                let scale = (max_d as f64) / (shot.width.max(shot.height) as f64);
                let new_w = (shot.width as f64 * scale).round().max(1.0) as u32;
                let new_h = (shot.height as f64 * scale).round().max(1.0) as u32;
                if let Ok(resized) = shot.resize(new_w, new_h) {
                    shot = resized;
                }
            }
        }

        let width = shot.width;
        let height = shot.height;

        match format.to_lowercase().as_str() {
            "jpeg" | "jpg" => {
                let q = if quality == 0 { 75 } else { quality.min(100) };
                let mut rgb = Vec::with_capacity((shot.width * shot.height * 3) as usize);
                for chunk in shot.pixels.chunks_exact(4) {
                    rgb.extend_from_slice(&chunk[0..3]);
                }
                let mut out = Vec::new();
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q);
                encoder
                    .encode(&rgb, shot.width, shot.height, image::ExtendedColorType::Rgb8)
                    .context("JPEG encoding failed")?;
                Ok((out, "image/jpeg".to_string(), width, height))
            }
            _ => {
                let png_bytes = shot.to_png().context("PNG encoding failed")?;
                Ok((png_bytes, "image/png".to_string(), width, height))
            }
        }
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

        // Like Python OpenDesk reference implementation (_text_sync): insert text via
        // clipboard-paste for full Unicode support and to prevent WinUI 3 / XAML,
        // Chromium web forms, and desktop GUI key-drops.
        self.clipboard_write(text)?;
        std::thread::sleep(Duration::from_millis(50));
        #[cfg(target_os = "macos")]
        self.keyboard_hotkey(&["command", "v"])?;
        #[cfg(not(target_os = "macos"))]
        self.keyboard_hotkey(&["ctrl", "v"])?;
        std::thread::sleep(Duration::from_millis(50));
        Ok(())
    }

    pub fn keyboard_press(&self, key_str: &str) -> Result<()> {
        if key_str.contains('+') || key_str.contains('-') {
            let parts: Vec<&str> = key_str
                .split(['+', '-'])
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            return self.keyboard_hotkey(&parts);
        }
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        let key = parse_key(key_str).ok_or_else(|| anyhow!("unknown key: {key_str}"))?;
        sim.keyboard()
            .press(key)
            .map_err(|e| anyhow!("press failed: {e}"))
    }

    pub fn keyboard_down(&self, key_str: &str) -> Result<()> {
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        let key = parse_key(key_str).ok_or_else(|| anyhow!("unknown key: {key_str}"))?;
        sim.keyboard()
            .down(key)
            .map_err(|e| anyhow!("down failed: {e}"))
    }

    pub fn keyboard_up(&self, key_str: &str) -> Result<()> {
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        let key = parse_key(key_str).ok_or_else(|| anyhow!("unknown key: {key_str}"))?;
        sim.keyboard()
            .up(key)
            .map_err(|e| anyhow!("up failed: {e}"))
    }

    pub fn keyboard_hotkey(&self, keys: &[&str]) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        // Flatten any composite tokens like ["ctrl+s"] or ["ctrl", "shift+s"]
        let mut flat_keys = Vec::new();
        for k in keys {
            for part in k.split(['+', '-']) {
                let trimmed = part.trim();
                if !trimmed.is_empty() {
                    flat_keys.push(trimmed);
                }
            }
        }
        if flat_keys.is_empty() {
            return Ok(());
        }

        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        let parsed_keys: Vec<Key> = flat_keys
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
        "space" | "spacebar" => Some(Key::Space),
        "delete" | "del" => Some(Key::Delete),
        "insert" | "ins" => Some(Key::Insert),
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
        "alt" | "option" => Some(Key::Alt),
        "meta" | "cmd" | "command" | "win" | "windows" | "winleft" | "winright" | "super" => {
            Some(Key::Meta)
        }
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
        assert_eq!(parse_key("command"), Some(Key::Meta));
        assert_eq!(parse_key("option"), Some(Key::Alt));
        assert_eq!(parse_key("windows"), Some(Key::Meta));
        assert_eq!(parse_key("spacebar"), Some(Key::Space));
        assert_eq!(parse_key("f5"), Some(Key::F(5)));
        assert_eq!(parse_key("a"), Some(Key::Char('a')));
        assert_eq!(parse_key("unknown_key_xyz"), None);
    }
}
