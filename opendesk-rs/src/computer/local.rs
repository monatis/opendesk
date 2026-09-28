use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use xa11y::{input_sim, App, AppExt, Key, Point, Rect, ScrollDelta};

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
            Some(r) => xa11y::screenshot_region(r).context("region screenshot failed")?,
            None => xa11y::screenshot().context("fullscreen screenshot failed")?,
        };
        shot.to_png().context("PNG encoding failed")
    }

    // -----------------------------------------------------------------------
    // Mouse
    // -----------------------------------------------------------------------

    pub fn mouse_move(&self, x: i32, y: i32) -> Result<()> {
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        sim.mouse().move_to(Point { x, y }).map_err(|e| anyhow!("mouse_move failed: {e}"))
    }

    pub fn mouse_click(&self, x: i32, y: i32, button: Option<&str>) -> Result<()> {
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        let mouse = sim.mouse();
        let target = Point { x, y };

        match button.unwrap_or("left").to_lowercase().as_str() {
            "right" => mouse.right_click(target).map_err(|e| anyhow!("{e}"))?,
            "middle" => {
                mouse.move_to(target).map_err(|e| anyhow!("{e}"))?;
                mouse.down(xa11y::MouseButton::Middle).map_err(|e| anyhow!("{e}"))?;
                mouse.up(xa11y::MouseButton::Middle).map_err(|e| anyhow!("{e}"))?;
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
            .drag(Point { x: from_x, y: from_y }, Point { x: to_x, y: to_y })
            .map_err(|e| anyhow!("drag failed: {e}"))
    }

    pub fn mouse_scroll(&self, x: i32, y: i32, dy: i32) -> Result<()> {
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        sim.mouse()
            .scroll(Point { x, y }, ScrollDelta { dx: 0, dy })
            .map_err(|e| anyhow!("scroll failed: {e}"))
    }

    // -----------------------------------------------------------------------
    // Keyboard
    // -----------------------------------------------------------------------

    pub fn keyboard_type(&self, text: &str) -> Result<()> {
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        sim.keyboard()
            .type_text(text)
            .map_err(|e| anyhow!("type_text failed: {e}"))
    }

    pub fn keyboard_press(&self, key_str: &str) -> Result<()> {
        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        let key = parse_key(key_str).ok_or_else(|| anyhow!("unknown key: {key_str}"))?;
        sim.keyboard().press(key).map_err(|e| anyhow!("press failed: {e}"))
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

    // -----------------------------------------------------------------------
    // App management
    // -----------------------------------------------------------------------

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
        let app = App::by_name(name, Duration::from_secs(3))
            .map_err(|e| anyhow!("app {name} not found: {e}"))?;
        app.as_element().activate().map_err(|e| anyhow!("activate {name} failed: {e}"))
    }

    pub fn app_close(&self, name: &str) -> Result<()> {
        let app = App::by_name(name, Duration::from_secs(3))
            .map_err(|e| anyhow!("app {name} not found: {e}"))?;
        app.as_element().close().map_err(|e| anyhow!("close {name} failed: {e}"))
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
            Some(name) => App::by_name(name, Duration::from_secs(3))
                .map_err(|e| anyhow!("app {name} not found: {e}"))?,
            None => App::foreground(Duration::from_secs(2))
                .map_err(|e| anyhow!("no foreground app: {e}"))?,
        };
        app.dump(max_depth).map_err(|e| anyhow!("dump tree failed: {e}"))
    }

    pub fn ui_click(&self, app_name: Option<&str>, selector: &str) -> Result<()> {
        let app = match app_name {
            Some(name) => App::by_name(name, Duration::from_secs(3))
                .map_err(|e| anyhow!("app {name} not found: {e}"))?,
            None => App::foreground(Duration::from_secs(2))
                .map_err(|e| anyhow!("no foreground app: {e}"))?,
        };
        let locator = app.locator(selector);
        locator.press().map_err(|e| anyhow!("locator press '{selector}' failed: {e}"))
    }

    pub fn ui_type(&self, app_name: Option<&str>, selector: &str, text: &str) -> Result<()> {
        let app = match app_name {
            Some(name) => App::by_name(name, Duration::from_secs(3))
                .map_err(|e| anyhow!("app {name} not found: {e}"))?,
            None => App::foreground(Duration::from_secs(2))
                .map_err(|e| anyhow!("no foreground app: {e}"))?,
        };
        let locator = app.locator(selector);
        locator.type_text(text).map_err(|e| anyhow!("locator type_text '{selector}' failed: {e}"))
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
