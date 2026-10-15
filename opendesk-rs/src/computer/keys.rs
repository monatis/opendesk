//! Key-name parsing and keyboard-layout helpers shared by the input paths.

/// Split a chord such as `ctrl+shift+s` into its keys.
///
/// `+` is the separator; `-` is accepted as a legacy separator when the chord
/// has no `+` (`ctrl-s`). A separator in key position is the key itself, so
/// `ctrl+-`, `ctrl++`, `-` and `+` all work. Returns an empty list only for
/// blank input.
pub fn split_chord(s: &str) -> Vec<String> {
    let s = s.trim();
    if s.chars().count() <= 1 {
        return if s.is_empty() {
            Vec::new()
        } else {
            vec![s.to_string()]
        };
    }
    let sep = if s.contains('+') {
        '+'
    } else if s.contains('-') {
        '-'
    } else {
        return vec![s.to_string()];
    };
    let mut parts = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if c == sep && !cur.trim().is_empty() {
            parts.push(cur.trim().to_string());
            cur.clear();
        } else {
            // A separator with nothing before it is the key itself.
            cur.push(c);
        }
    }
    if !cur.trim().is_empty() {
        parts.push(cur.trim().to_string());
    }
    parts
}

/// Keys and mouse buttons a controller is holding down, so they can be
/// released if the controller goes away mid-gesture.
#[derive(Debug, Default)]
pub struct HeldInput {
    pub keys: Vec<String>,
    pub buttons: Vec<String>,
}

impl HeldInput {
    pub fn press(list: &mut Vec<String>, name: &str) {
        let n = name.trim().to_lowercase();
        if !list.contains(&n) {
            list.push(n);
        }
    }

    pub fn release(list: &mut Vec<String>, name: &str) {
        let n = name.trim().to_lowercase();
        list.retain(|k| *k != n);
    }
}

/// The key (and modifiers) that produce a character on a keyboard layout.
#[cfg(target_os = "windows")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutKey {
    pub vk: u16,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

#[cfg(target_os = "windows")]
mod win {
    use std::ffi::c_void;

    unsafe extern "system" {
        pub fn GetForegroundWindow() -> *mut c_void;
        pub fn GetWindowThreadProcessId(hwnd: *mut c_void, pid: *mut u32) -> u32;
        pub fn GetKeyboardLayout(thread: u32) -> *mut c_void;
        pub fn VkKeyScanExW(ch: u16, hkl: *mut c_void) -> i16;
        pub fn MapVirtualKeyExW(code: u32, map_type: u32, hkl: *mut c_void) -> u32;
        pub fn ToUnicodeEx(
            vk: u32,
            scan: u32,
            key_state: *const u8,
            buf: *mut u16,
            buf_len: i32,
            flags: u32,
            hkl: *mut c_void,
        ) -> i32;
    }

    /// What `vk` types on `hkl`, with or without Shift. Flag 0x4 leaves the
    /// kernel's dead-key state alone (Windows 10 1607+).
    pub fn char_of(vk: u16, shift: bool, hkl: *mut c_void) -> Option<u16> {
        let mut state = [0u8; 256];
        if shift {
            state[0x10] = 0x80;
        }
        let mut buf = [0u16; 4];
        let n = unsafe {
            let scan = MapVirtualKeyExW(vk as u32, 0, hkl);
            ToUnicodeEx(
                vk as u32,
                scan,
                state.as_ptr(),
                buf.as_mut_ptr(),
                4,
                0x4,
                hkl,
            )
        };
        (n == 1).then_some(buf[0])
    }

    /// The layout of the window that will receive the input.
    pub fn foreground_layout() -> *mut c_void {
        unsafe {
            let hwnd = GetForegroundWindow();
            let thread = if hwnd.is_null() {
                0
            } else {
                GetWindowThreadProcessId(hwnd, std::ptr::null_mut())
            };
            GetKeyboardLayout(thread)
        }
    }
}

/// Which key types `c` on the foreground window's layout, e.g. on Turkish-Q
/// `i` is not VK_I (that key types `ı`). `None` if the layout cannot type it.
///
/// A key needing at most Shift is preferred: VkKeyScanExW answers `i` on
/// Turkish-Q with AltGr+I (Ctrl+Alt+I), which a screen reader or app would
/// take as a different command, and it misses `ı` altogether.
#[cfg(target_os = "windows")]
pub fn layout_key(c: char) -> Option<LayoutKey> {
    let mut buf = [0u16; 2];
    let units = c.encode_utf16(&mut buf);
    let &[unit] = &*units else {
        return None;
    };
    let hkl = win::foreground_layout();
    let scanned = match unsafe { win::VkKeyScanExW(unit, hkl) } {
        -1 => None,
        r => {
            let [vk, mods] = (r as u16).to_le_bytes();
            Some(LayoutKey {
                vk: vk as u16,
                shift: mods & 1 != 0,
                ctrl: mods & 2 != 0,
                alt: mods & 4 != 0,
            })
        }
    };
    if let Some(k) = scanned
        && !k.ctrl
        && !k.alt
    {
        return Some(k);
    }
    for shift in [false, true] {
        for vk in 0x20u16..=0xFE {
            if win::char_of(vk, shift, hkl) == Some(unit) {
                return Some(LayoutKey {
                    vk,
                    shift,
                    ctrl: false,
                    alt: false,
                });
            }
        }
    }
    scanned
}

/// Foreground window's keyboard layout handle as a number: the low word is
/// the language id, the high word the layout (e.g. 0x041F041F, Turkish-Q).
#[cfg(target_os = "windows")]
pub fn foreground_layout_id() -> u32 {
    win::foreground_layout() as usize as u32
}

/// Hardware scan code for `vk` on the foreground layout (0 if unknown).
/// Some consumers (screen readers, games) read the scan code, not the VK.
#[cfg(target_os = "windows")]
pub fn scan_code(vk: u16) -> u16 {
    const MAPVK_VK_TO_VSC: u32 = 0;
    unsafe { win::MapVirtualKeyExW(vk as u32, MAPVK_VK_TO_VSC, win::foreground_layout()) as u16 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn plain_chords() {
        assert_eq!(split_chord("ctrl+shift+s"), s(&["ctrl", "shift", "s"]));
        assert_eq!(split_chord(" ctrl + s "), s(&["ctrl", "s"]));
        assert_eq!(split_chord("enter"), s(&["enter"]));
        assert_eq!(split_chord("a"), s(&["a"]));
    }

    #[test]
    fn legacy_dash_separator() {
        assert_eq!(split_chord("ctrl-s"), s(&["ctrl", "s"]));
        assert_eq!(split_chord("ctrl-alt-del"), s(&["ctrl", "alt", "del"]));
    }

    #[test]
    fn separator_as_key() {
        assert_eq!(split_chord("ctrl+-"), s(&["ctrl", "-"]));
        assert_eq!(split_chord("ctrl++"), s(&["ctrl", "+"]));
        assert_eq!(split_chord("ctrl--"), s(&["ctrl", "-"]));
        assert_eq!(split_chord("-"), s(&["-"]));
        assert_eq!(split_chord("+"), s(&["+"]));
        assert_eq!(split_chord("shift+="), s(&["shift", "="]));
    }

    #[test]
    fn blank_is_empty() {
        assert!(split_chord("").is_empty());
        assert!(split_chord("   ").is_empty());
    }

    #[test]
    fn held_input_tracks_unique_keys() {
        let mut h = HeldInput::default();
        HeldInput::press(&mut h.keys, "Ctrl");
        HeldInput::press(&mut h.keys, "ctrl");
        HeldInput::press(&mut h.keys, "shift");
        assert_eq!(h.keys, s(&["ctrl", "shift"]));
        HeldInput::release(&mut h.keys, "CTRL");
        assert_eq!(h.keys, s(&["shift"]));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn layout_key_finds_ascii_letters() {
        // Every Latin layout can type 'a'; the VK may differ (AZERTY), but
        // it must be some key, and never one that needs Ctrl/Alt.
        let k = layout_key('a').unwrap();
        assert!(!k.ctrl && !k.alt, "{k:?}");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn turkish_q_dotted_and_dotless_i() {
        // Only meaningful when the foreground layout is Turkish-Q, where the
        // VK_I key types 'ı' and 'i' sits on its own key.
        let Some(dotless) = layout_key('ı') else {
            return;
        };
        if dotless.vk != 0x49 {
            return;
        }
        assert!(
            !dotless.shift && !dotless.ctrl && !dotless.alt,
            "{dotless:?}"
        );
        let i = layout_key('i').unwrap();
        assert_ne!(i.vk, 0x49, "{i:?}");
        assert!(!i.ctrl && !i.alt, "'i' must not need AltGr: {i:?}");
    }
}
