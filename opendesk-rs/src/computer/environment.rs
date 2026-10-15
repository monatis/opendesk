//! Facts about a machine that change how it must be driven: its language and
//! keyboard layout (which characters keys produce), display layout and
//! scaling (where clicks land), and clock (timestamps in its logs).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Display {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Display scaling, 1.0 = 100 % (96 DPI).
    pub scale: f64,
    pub primary: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Environment {
    pub os: String,
    pub os_version: String,
    pub hostname: String,
    /// User locale, e.g. `tr-TR`.
    pub locale: String,
    pub timezone: String,
    pub utc_offset_minutes: i32,
    /// Keyboard layout of the foreground window, e.g. `041F041F (tr-TR)`.
    pub keyboard_layout: String,
    pub displays: Vec<Display>,
    /// The machine's clock when this was read (Unix time, milliseconds).
    pub time_unix_ms: i64,
}

impl Environment {
    pub fn current() -> Self {
        let now = chrono::Local::now();
        let mut env = Self {
            os: std::env::consts::OS.to_string(),
            hostname: std::env::var("COMPUTERNAME")
                .or_else(|_| std::env::var("HOSTNAME"))
                .unwrap_or_default(),
            utc_offset_minutes: now.offset().local_minus_utc() / 60,
            time_unix_ms: now.timestamp_millis(),
            ..Default::default()
        };
        #[cfg(target_os = "windows")]
        win::fill(&mut env);
        #[cfg(not(target_os = "windows"))]
        {
            env.locale = std::env::var("LANG").unwrap_or_default();
            env.timezone = std::env::var("TZ").unwrap_or_default();
        }
        env
    }

    /// Human-readable summary, one fact per line.
    pub fn describe(&self, clock_offset_ms: Option<i64>) -> String {
        let mut lines = vec![
            format!("OS: {} {}", self.os, self.os_version)
                .trim_end()
                .to_string(),
            format!("Hostname: {}", self.hostname),
            format!("Locale: {}", self.locale),
            format!("Keyboard layout: {}", self.keyboard_layout),
            format!(
                "Time zone: {} (UTC{:+03}:{:02})",
                self.timezone,
                self.utc_offset_minutes / 60,
                (self.utc_offset_minutes % 60).abs()
            ),
        ];
        if let Some(off) = clock_offset_ms {
            lines.push(format!(
                "Clock: {} this machine by {:.1} s",
                if off >= 0 { "ahead of" } else { "behind" },
                off.unsigned_abs() as f64 / 1000.0
            ));
        }
        for (i, d) in self.displays.iter().enumerate() {
            lines.push(format!(
                "Display {}: {}x{} at ({}, {}), scale {:.0}%{}",
                i + 1,
                d.width,
                d.height,
                d.x,
                d.y,
                d.scale * 100.0,
                if d.primary { " (primary)" } else { "" }
            ));
        }
        lines.join("\n")
    }
}

#[cfg(target_os = "windows")]
#[allow(non_snake_case)]
mod win {
    use super::{Display, Environment};
    use std::ffi::c_void;

    #[repr(C)]
    struct OsVersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        csd: [u16; 128],
    }

    #[repr(C)]
    struct SystemTime([u16; 8]);

    #[repr(C)]
    struct DynamicTimeZoneInformation {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: SystemTime,
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: SystemTime,
        daylight_bias: i32,
        key_name: [u16; 128],
        dynamic_daylight_disabled: u8,
    }

    #[repr(C)]
    struct RectL {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct MonitorInfo {
        size: u32,
        monitor: RectL,
        work: RectL,
        flags: u32,
    }

    type MonitorEnumProc =
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut RectL, isize) -> i32;

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfo) -> i32;
    }
    #[link(name = "shcore")]
    unsafe extern "system" {
        fn GetDpiForMonitor(monitor: *mut c_void, kind: u32, x: *mut u32, y: *mut u32) -> i32;
    }
    unsafe extern "system" {
        fn GetUserDefaultLocaleName(buf: *mut u16, len: i32) -> i32;
        fn LCIDToLocaleName(lcid: u32, buf: *mut u16, len: i32, flags: u32) -> i32;
        fn GetDynamicTimeZoneInformation(info: *mut DynamicTimeZoneInformation) -> u32;
        fn EnumDisplayMonitors(
            hdc: *mut c_void,
            clip: *const RectL,
            callback: MonitorEnumProc,
            data: isize,
        ) -> i32;
        fn GetMonitorInfoW(monitor: *mut c_void, info: *mut MonitorInfo) -> i32;
        fn SetThreadDpiAwarenessContext(context: isize) -> isize;
    }

    fn wide(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    fn locale_of(lcid: u32) -> String {
        let mut buf = [0u16; 85];
        let n = unsafe { LCIDToLocaleName(lcid, buf.as_mut_ptr(), buf.len() as i32, 0) };
        if n > 0 { wide(&buf) } else { String::new() }
    }

    unsafe extern "system" fn on_monitor(
        monitor: *mut c_void,
        _hdc: *mut c_void,
        _rect: *mut RectL,
        data: isize,
    ) -> i32 {
        let out = unsafe { &mut *(data as *mut Vec<Display>) };
        let mut info = MonitorInfo {
            size: std::mem::size_of::<MonitorInfo>() as u32,
            monitor: RectL {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            work: RectL {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            flags: 0,
        };
        if unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 {
            let (mut dx, mut dy) = (96u32, 96u32);
            // MDT_EFFECTIVE_DPI = 0
            let scale = if unsafe { GetDpiForMonitor(monitor, 0, &mut dx, &mut dy) } == 0 {
                dx as f64 / 96.0
            } else {
                1.0
            };
            let r = &info.monitor;
            out.push(Display {
                x: r.left,
                y: r.top,
                width: r.right - r.left,
                height: r.bottom - r.top,
                scale,
                primary: info.flags & 1 != 0,
            });
        }
        1
    }

    pub fn fill(env: &mut Environment) {
        unsafe {
            let mut v = OsVersionInfo {
                size: std::mem::size_of::<OsVersionInfo>() as u32,
                major: 0,
                minor: 0,
                build: 0,
                platform: 0,
                csd: [0; 128],
            };
            if RtlGetVersion(&mut v) == 0 {
                env.os_version = format!("{}.{}.{}", v.major, v.minor, v.build);
            }

            let mut buf = [0u16; 85];
            if GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) > 0 {
                env.locale = wide(&buf);
            }

            let mut tz: DynamicTimeZoneInformation = std::mem::zeroed();
            if GetDynamicTimeZoneInformation(&mut tz) != u32::MAX {
                env.timezone = wide(&tz.key_name);
            }

            let hkl = crate::computer::keys::foreground_layout_id();
            let lang = hkl & 0xFFFF;
            env.keyboard_layout = format!("{hkl:08X} ({})", locale_of(lang));

            // Read real pixels and DPI: a DPI-unaware thread sees scaled-down
            // sizes and always 96 DPI. PER_MONITOR_AWARE_V2 = -4.
            let previous = SetThreadDpiAwarenessContext(-4);
            let mut displays: Vec<Display> = Vec::new();
            EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                on_monitor,
                &mut displays as *mut Vec<Display> as isize,
            );
            if previous != 0 {
                SetThreadDpiAwarenessContext(previous);
            }
            env.displays = displays;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_has_basics() {
        let env = Environment::current();
        assert!(!env.os.is_empty());
        assert!(env.time_unix_ms > 1_700_000_000_000);
        #[cfg(target_os = "windows")]
        {
            assert!(!env.os_version.is_empty(), "{env:?}");
            assert!(!env.locale.is_empty(), "{env:?}");
            assert!(!env.displays.is_empty(), "{env:?}");
            assert!(env.displays.iter().any(|d| d.primary), "{env:?}");
        }
    }

    #[test]
    fn describe_reports_clock_offset() {
        let env = Environment {
            os: "windows".into(),
            utc_offset_minutes: 180,
            ..Default::default()
        };
        let text = env.describe(Some(-1_200_000));
        assert!(text.contains("UTC+03:00"), "{text}");
        assert!(text.contains("behind this machine by 1200.0 s"), "{text}");
    }
}
