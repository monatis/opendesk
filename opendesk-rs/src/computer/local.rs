use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use xa11y::{App, AppExt, Key, Point, Rect, ScrollDelta, TreeNode, input_sim};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeyboardStep {
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub keys: Option<Vec<String>>,
    #[serde(default)]
    pub hotkey: Option<serde_json::Value>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub delay_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub success: bool,
    /// The command hit its timeout; its process tree was killed and the
    /// output is whatever it printed before that.
    #[serde(default)]
    pub timed_out: bool,
}

#[cfg(target_os = "windows")]
#[allow(non_snake_case, non_camel_case_types)]
pub mod win_input {
    pub const INPUT_KEYBOARD: u32 = 1;
    pub const KEYEVENTF_EXTENDEDKEY: u32 = 0x0001;
    pub const KEYEVENTF_KEYUP: u32 = 0x0002;
    pub const KEYEVENTF_UNICODE: u32 = 0x0004;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct KEYBDINPUT {
        pub w_vk: u16,
        pub w_scan: u16,
        pub dw_flags: u32,
        pub time: u32,
        pub dw_extra_info: usize,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct MOUSEINPUT {
        pub dx: i32,
        pub dy: i32,
        pub mouse_data: u32,
        pub dw_flags: u32,
        pub time: u32,
        pub dw_extra_info: usize,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct HARDWAREINPUT {
        pub u_msg: u32,
        pub w_param_l: u16,
        pub w_param_h: u16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub union INPUT_UNION {
        pub mi: MOUSEINPUT,
        pub ki: KEYBDINPUT,
        pub hi: HARDWAREINPUT,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct INPUT {
        pub r#type: u32,
        pub u: INPUT_UNION,
    }

    unsafe extern "system" {
        pub fn SendInput(c_inputs: u32, p_inputs: *const INPUT, cb_size: i32) -> u32;
    }
}

#[derive(Clone)]
pub struct LocalComputer {
    pub privacy: std::sync::Arc<crate::computer::privacy::PrivacyController>,
}

impl Default for LocalComputer {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalComputer {
    pub fn new() -> Self {
        Self {
            privacy: std::sync::Arc::new(crate::computer::privacy::PrivacyController::new()),
        }
    }

    pub fn set_privacy(
        &self,
        lock_input: bool,
        blackout: bool,
    ) -> Result<crate::computer::privacy::PrivacyState> {
        self.privacy.set_privacy(lock_input, blackout)
    }

    pub fn get_privacy(&self) -> crate::computer::privacy::PrivacyState {
        self.privacy.get_privacy()
    }

    pub fn reset_privacy(&self) {
        self.privacy.reset();
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

        if let Some(max_d) = max_dim
            && (shot.width > max_d || shot.height > max_d)
        {
            let scale = (max_d as f64) / (shot.width.max(shot.height) as f64);
            let new_w = (shot.width as f64 * scale).round().max(1.0) as u32;
            let new_h = (shot.height as f64 * scale).round().max(1.0) as u32;
            if let Ok(resized) = shot.resize(new_w, new_h) {
                shot = resized;
            }
        }

        let width = shot.width;
        let height = shot.height;

        match format.to_lowercase().as_str() {
            "jpeg" | "jpg" => {
                let q = if quality == 0 { 75 } else { quality.min(100) };
                let mut rgb = Vec::with_capacity((shot.width * shot.height * 3) as usize);
                for chunk in shot.pixels.as_chunks::<4>().0 {
                    rgb.extend_from_slice(&chunk[0..3]);
                }
                let mut out = Vec::new();
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q);
                encoder
                    .encode(
                        &rgb,
                        shot.width,
                        shot.height,
                        image::ExtendedColorType::Rgb8,
                    )
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

        #[cfg(target_os = "windows")]
        {
            // Pure SendInput Unicode injection (KEYEVENTF_UNICODE).
            // Does NOT clobber the system clipboard and correctly handles international Unicode.
            let mut chars = text.chars().peekable();
            while let Some(ch) = chars.next() {
                if ch == '\r' {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    win_send_vk(0x0D, false, true)?;
                    win_send_vk(0x0D, false, false)?;
                    continue;
                }
                if ch == '\n' {
                    win_send_vk(0x0D, false, true)?;
                    win_send_vk(0x0D, false, false)?;
                    continue;
                }
                if ch == '\t' {
                    win_send_vk(0x09, false, true)?;
                    win_send_vk(0x09, false, false)?;
                    continue;
                }

                let mut utf16_buf = [0u16; 2];
                let encoded = ch.encode_utf16(&mut utf16_buf);
                for &code_unit in &*encoded {
                    let down = win_input::INPUT {
                        r#type: win_input::INPUT_KEYBOARD,
                        u: win_input::INPUT_UNION {
                            ki: win_input::KEYBDINPUT {
                                w_vk: 0,
                                w_scan: code_unit,
                                dw_flags: win_input::KEYEVENTF_UNICODE,
                                time: 0,
                                dw_extra_info: 0,
                            },
                        },
                    };
                    let up = win_input::INPUT {
                        r#type: win_input::INPUT_KEYBOARD,
                        u: win_input::INPUT_UNION {
                            ki: win_input::KEYBDINPUT {
                                w_vk: 0,
                                w_scan: code_unit,
                                dw_flags: win_input::KEYEVENTF_UNICODE | win_input::KEYEVENTF_KEYUP,
                                time: 0,
                                dw_extra_info: 0,
                            },
                        },
                    };
                    let sent = unsafe {
                        win_input::SendInput(
                            2,
                            [down, up].as_ptr(),
                            std::mem::size_of::<win_input::INPUT>() as i32,
                        )
                    };
                    if sent != 2 {
                        bail!(
                            "SendInput unicode injection failed for code unit 0x{:X}",
                            code_unit
                        );
                    }
                }
            }
            return Ok(());
        }

        #[cfg(not(target_os = "windows"))]
        {
            let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
            sim.keyboard()
                .type_text(text)
                .map_err(|e| anyhow!("type_text failed: {e}"))
        }
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

        #[cfg(target_os = "windows")]
        {
            if let Some((vk, ext)) = parse_key_vk(key_str) {
                win_send_vk(vk, ext, true)?;
                std::thread::sleep(Duration::from_millis(15));
                win_send_vk(vk, ext, false)?;
                return Ok(());
            }
        }

        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        let key = parse_key(key_str).ok_or_else(|| anyhow!("unknown key: {key_str}"))?;
        sim.keyboard()
            .press(key)
            .map_err(|e| anyhow!("press failed: {e}"))
    }

    pub fn keyboard_down(&self, key_str: &str) -> Result<()> {
        #[cfg(target_os = "windows")]
        {
            if let Some((vk, ext)) = parse_key_vk(key_str) {
                return win_send_vk(vk, ext, true);
            }
        }

        let sim = input_sim().map_err(|e| anyhow!("input_sim unavailable: {e}"))?;
        let key = parse_key(key_str).ok_or_else(|| anyhow!("unknown key: {key_str}"))?;
        sim.keyboard()
            .down(key)
            .map_err(|e| anyhow!("down failed: {e}"))
    }

    pub fn keyboard_up(&self, key_str: &str) -> Result<()> {
        #[cfg(target_os = "windows")]
        {
            if let Some((vk, ext)) = parse_key_vk(key_str) {
                return win_send_vk(vk, ext, false);
            }
        }

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

        #[cfg(target_os = "windows")]
        {
            let mut vks = Vec::with_capacity(flat_keys.len());
            let mut all_vk = true;
            for k in &flat_keys {
                if let Some(vk) = parse_key_vk(k) {
                    vks.push(vk);
                } else {
                    all_vk = false;
                    break;
                }
            }
            if all_vk {
                // Press down in sequence
                for &(vk, ext) in &vks {
                    win_send_vk(vk, ext, true)?;
                }
                std::thread::sleep(Duration::from_millis(20));
                // Release in reverse order
                for &(vk, ext) in vks.iter().rev() {
                    win_send_vk(vk, ext, false)?;
                }
                return Ok(());
            }
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
        self.keyboard_down(key_str)?;
        std::thread::sleep(Duration::from_secs_f64(duration_secs.max(0.0)));
        self.keyboard_up(key_str)?;
        Ok(())
    }

    pub fn keyboard_sequence(&self, steps: &[KeyboardStep]) -> Result<()> {
        for step in steps {
            if let Some(ref hotkey_val) = step.hotkey {
                match hotkey_val {
                    serde_json::Value::Array(arr) => {
                        let keys: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
                        self.keyboard_hotkey(&keys)?;
                    }
                    serde_json::Value::String(s) => {
                        let parts: Vec<&str> = s
                            .split(['+', '-'])
                            .map(|p| p.trim())
                            .filter(|p| !p.is_empty())
                            .collect();
                        self.keyboard_hotkey(&parts)?;
                    }
                    _ => {}
                }
            } else if let Some(ref keys) = step.keys {
                let key_strs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
                self.keyboard_hotkey(&key_strs)?;
            } else if let Some(ref key) = step.key {
                self.keyboard_press(key)?;
            } else if let Some(ref text) = step.text {
                self.keyboard_type(text)?;
            } else if let Some(ref action) = step.action {
                match action.as_str() {
                    "press" => {
                        if let Some(ref k) = step.key {
                            self.keyboard_press(k)?;
                        }
                    }
                    "type" => {
                        if let Some(ref t) = step.text {
                            self.keyboard_type(t)?;
                        }
                    }
                    "hotkey" => {
                        if let Some(ref ks) = step.keys {
                            let key_strs: Vec<&str> = ks.iter().map(|s| s.as_str()).collect();
                            self.keyboard_hotkey(&key_strs)?;
                        }
                    }
                    _ => {}
                }
            }

            let delay = step.delay_ms.unwrap_or(50);
            if delay > 0 {
                std::thread::sleep(Duration::from_millis(delay));
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Command execution
    // -----------------------------------------------------------------------

    pub fn execute_command(
        &self,
        command: &str,
        shell: Option<&str>,
        timeout_secs: Option<u64>,
    ) -> Result<ExecResult> {
        let timeout = Duration::from_secs(timeout_secs.unwrap_or(30));

        #[cfg(target_os = "windows")]
        let (mut cmd, temp_script) = {
            let shell_choice = shell.unwrap_or("powershell").to_lowercase();
            if shell_choice == "cmd" || shell_choice == "cmd.exe" {
                let mut c = std::process::Command::new("cmd.exe");
                c.args(["/C", command]);
                (c, None)
            } else {
                super::process::powershell_command(command)?
            }
        };

        #[cfg(not(target_os = "windows"))]
        let (mut cmd, temp_script): (_, Option<std::path::PathBuf>) = {
            let shell_choice = shell.unwrap_or("sh");
            let mut c = std::process::Command::new(shell_choice);
            c.args(["-c", command]);
            (c, None)
        };

        let res = super::process::run_with_timeout(&mut cmd, timeout);
        if let Some(path) = temp_script {
            let _ = std::fs::remove_file(path);
        }
        res
    }

    pub fn execute_argv(&self, argv: &[&str], timeout_secs: Option<u64>) -> Result<ExecResult> {
        if argv.is_empty() {
            bail!("empty argv");
        }
        let timeout = Duration::from_secs(timeout_secs.unwrap_or(30));
        let mut cmd = std::process::Command::new(argv[0]);
        if argv.len() > 1 {
            cmd.args(&argv[1..]);
        }
        super::process::run_with_timeout(&mut cmd, timeout)
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

#[cfg(target_os = "windows")]
pub fn win_send_vk(vk: u16, extended: bool, down: bool) -> Result<()> {
    let mut flags = 0u32;
    if extended {
        flags |= win_input::KEYEVENTF_EXTENDEDKEY;
    }
    if !down {
        flags |= win_input::KEYEVENTF_KEYUP;
    }
    let input = win_input::INPUT {
        r#type: win_input::INPUT_KEYBOARD,
        u: win_input::INPUT_UNION {
            ki: win_input::KEYBDINPUT {
                w_vk: vk,
                w_scan: 0,
                dw_flags: flags,
                time: 0,
                dw_extra_info: 0,
            },
        },
    };
    let sent =
        unsafe { win_input::SendInput(1, &input, std::mem::size_of::<win_input::INPUT>() as i32) };
    if sent != 1 {
        bail!("SendInput failed for VK 0x{:X}", vk);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn parse_key_vk(s: &str) -> Option<(u16, bool)> {
    let lower = s.trim().to_lowercase();
    match lower.as_str() {
        // Lock and modifier keys (JAWS / NVDA laptop & desktop modifiers)
        "capslock" | "caps_lock" | "capital" | "caps" => Some((0x14, false)),
        "numlock" | "num_lock" | "num" => Some((0x90, true)),
        "scrolllock" | "scroll_lock" | "scroll" => Some((0x91, false)),
        "printscreen" | "prtsc" | "prtscn" | "snapshot" => Some((0x2C, true)),
        "pause" | "break" => Some((0x13, false)),
        "apps" | "menu" | "contextmenu" | "app" => Some((0x5D, true)),
        "insert" | "ins" => Some((0x2D, true)),
        "delete" | "del" => Some((0x2E, true)),
        "home" => Some((0x24, true)),
        "end" => Some((0x23, true)),
        "pageup" | "pgup" => Some((0x21, true)),
        "pagedown" | "pgdn" => Some((0x22, true)),
        "arrowup" | "up" => Some((0x26, true)),
        "arrowdown" | "down" => Some((0x28, true)),
        "arrowleft" | "left" => Some((0x25, true)),
        "arrowright" | "right" => Some((0x27, true)),
        "enter" | "return" => Some((0x0D, false)),
        "escape" | "esc" => Some((0x1B, false)),
        "backspace" => Some((0x08, false)),
        "tab" => Some((0x09, false)),
        "space" | "spacebar" => Some((0x20, false)),
        "shift" => Some((0x10, false)),
        "ctrl" | "control" => Some((0x11, false)),
        "alt" | "option" => Some((0x12, false)),
        "meta" | "cmd" | "command" | "win" | "windows" | "winleft" | "winright" | "super" => {
            Some((0x5B, true))
        }
        "numpad0" => Some((0x60, false)),
        "numpad1" => Some((0x61, false)),
        "numpad2" => Some((0x62, false)),
        "numpad3" => Some((0x63, false)),
        "numpad4" => Some((0x64, false)),
        "numpad5" => Some((0x65, false)),
        "numpad6" => Some((0x66, false)),
        "numpad7" => Some((0x67, false)),
        "numpad8" => Some((0x68, false)),
        "numpad9" => Some((0x69, false)),
        "multiply" => Some((0x6A, false)),
        "add" => Some((0x6B, false)),
        "separator" => Some((0x6C, false)),
        "subtract" => Some((0x6D, false)),
        "decimal" => Some((0x6E, false)),
        "divide" => Some((0x6F, true)),
        s if s.starts_with('f') && s.len() > 1 => {
            if let Ok(n) = s[1..].parse::<u16>() {
                if (1..=24).contains(&n) {
                    return Some((0x70 + n - 1, false));
                }
            }
            None
        }
        ";" => Some((0xBA, false)),
        "/" => Some((0xBF, false)),
        "`" => Some((0xC0, false)),
        "[" => Some((0xDB, false)),
        "\\" => Some((0xDC, false)),
        "]" => Some((0xDD, false)),
        "'" => Some((0xDE, false)),
        "," => Some((0xBC, false)),
        "." => Some((0xBE, false)),
        "-" => Some((0xBD, false)),
        "=" => Some((0xBB, false)),
        other if other.len() == 1 => {
            let c = other.chars().next().unwrap();
            if c.is_ascii_alphabetic() {
                Some((c.to_ascii_uppercase() as u16, false))
            } else if c.is_ascii_digit() {
                Some((c as u16, false))
            } else {
                None
            }
        }
        _ => None,
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

    #[cfg(target_os = "windows")]
    #[test]
    fn test_parse_key_vk_lock_keys() {
        // CapsLock / Capital (VK_CAPITAL = 0x14)
        assert_eq!(parse_key_vk("capslock"), Some((0x14, false)));
        assert_eq!(parse_key_vk("caps_lock"), Some((0x14, false)));
        assert_eq!(parse_key_vk("capital"), Some((0x14, false)));

        // NumLock (VK_NUMLOCK = 0x90, extended)
        assert_eq!(parse_key_vk("numlock"), Some((0x90, true)));
        assert_eq!(parse_key_vk("num_lock"), Some((0x90, true)));

        // ScrollLock (VK_SCROLL = 0x91)
        assert_eq!(parse_key_vk("scrolllock"), Some((0x91, false)));
        assert_eq!(parse_key_vk("scroll_lock"), Some((0x91, false)));

        // PrintScreen (VK_SNAPSHOT = 0x2C, extended)
        assert_eq!(parse_key_vk("printscreen"), Some((0x2C, true)));
        assert_eq!(parse_key_vk("prtsc"), Some((0x2C, true)));

        // Insert (VK_INSERT = 0x2D, extended)
        assert_eq!(parse_key_vk("insert"), Some((0x2D, true)));
        assert_eq!(parse_key_vk("ins"), Some((0x2D, true)));
    }

    #[test]
    fn test_keyboard_step_deserialization() {
        let json_seq = serde_json::json!({
            "action": "sequence",
            "steps": [
                { "hotkey": ["insert", "space"], "delay_ms": 100 },
                { "hotkey": ["shift", "h"], "delay_ms": 500 },
                { "key": "h", "delay_ms": 800 },
                { "hotkey": ["insert", "space"], "delay_ms": 100 },
                { "hotkey": ["ctrl", "h"] }
            ]
        });
        let steps: Vec<KeyboardStep> = serde_json::from_value(json_seq["steps"].clone()).unwrap();
        assert_eq!(steps.len(), 5);
        assert_eq!(steps[0].delay_ms, Some(100));
        assert_eq!(steps[2].key, Some("h".to_string()));
    }

    #[test]
    fn test_execute_command_local() {
        let computer = LocalComputer::new();
        #[cfg(target_os = "windows")]
        let res = computer
            .execute_command(
                "Write-Output 'HELLO_OPENDESK_EXEC'",
                Some("powershell"),
                Some(10),
            )
            .unwrap();
        #[cfg(not(target_os = "windows"))]
        let res = computer
            .execute_command("echo HELLO_OPENDESK_EXEC", Some("sh"), Some(10))
            .unwrap();
        assert!(res.success);
        assert!(res.stdout.contains("HELLO_OPENDESK_EXEC"));
        assert_eq!(res.exit_code, 0);
    }

    #[test]
    fn test_keyboard_type_preserves_clipboard() {
        let computer = LocalComputer::new();
        let initial_clip = "SECRET_SAVED_CLIPBOARD_DATA_12345";
        let _ = computer.clipboard_write(initial_clip);
        let _ = computer.keyboard_type("Typed test string");
        let after_clip = computer.clipboard_read().unwrap_or_default();
        assert_eq!(after_clip, initial_clip);
    }
}
