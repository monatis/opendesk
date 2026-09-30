use anyhow::Result;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, Default, PartialEq, Eq)]
pub struct PrivacyState {
    pub lock_input: bool,
    pub blackout: bool,
    pub supported: bool,
}

#[cfg(target_os = "windows")]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use std::ptr::null_mut;
    use std::sync::mpsc::channel;
    use std::thread::JoinHandle;
    use tracing::warn;

    type HWND = *mut c_void;
    type HHOOK = *mut c_void;
    type HINSTANCE = *mut c_void;
    type HGDIOBJ = *mut c_void;
    type HBRUSH = *mut c_void;
    type HMENU = *mut c_void;
    type HICON = *mut c_void;
    type HCURSOR = *mut c_void;
    type BOOL = i32;

    #[repr(C)]
    struct POINT {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    #[allow(non_snake_case)]
    struct MSG {
        hwnd: HWND,
        message: u32,
        wParam: usize,
        lParam: isize,
        time: u32,
        pt: POINT,
        lPrivate: u32,
    }

    #[repr(C)]
    struct KBDLLHOOKSTRUCT {
        vk_code: u32,
        scan_code: u32,
        flags: u32,
        time: u32,
        dw_extra_info: usize,
    }

    #[repr(C)]
    struct MSLLHOOKSTRUCT {
        pt: POINT,
        mouse_data: u32,
        flags: u32,
        time: u32,
        dw_extra_info: usize,
    }

    #[repr(C)]
    struct WNDCLASSW {
        style: u32,
        lpfn_wnd_proc: Option<unsafe extern "system" fn(HWND, u32, usize, isize) -> isize>,
        cb_cls_extra: i32,
        cb_wnd_extra: i32,
        h_instance: HINSTANCE,
        h_icon: HICON,
        h_cursor: HCURSOR,
        hbr_background: HBRUSH,
        lpsz_menu_name: *const u16,
        lpsz_class_name: *const u16,
    }

    unsafe extern "system" {
        fn SetWindowsHookExW(
            id_hook: i32,
            lpfn: unsafe extern "system" fn(i32, usize, isize) -> isize,
            hmod: HINSTANCE,
            dw_thread_id: u32,
        ) -> HHOOK;
        fn UnhookWindowsHookEx(hhk: HHOOK) -> BOOL;
        fn CallNextHookEx(hhk: HHOOK, n_code: i32, w_param: usize, l_param: isize) -> isize;
        fn GetMessageW(
            lp_msg: *mut MSG,
            h_wnd: HWND,
            w_msg_filter_min: u32,
            w_msg_filter_max: u32,
        ) -> BOOL;
        fn TranslateMessage(lp_msg: *const MSG) -> BOOL;
        fn DispatchMessageW(lp_msg: *const MSG) -> isize;
        fn PostThreadMessageW(id_thread: u32, msg: u32, w_param: usize, l_param: isize) -> BOOL;
        fn CreateWindowExW(
            dw_ex_style: u32,
            lp_class_name: *const u16,
            lp_window_name: *const u16,
            dw_style: u32,
            x: i32,
            y: i32,
            n_width: i32,
            n_height: i32,
            h_wnd_parent: HWND,
            h_menu: HMENU,
            h_instance: HINSTANCE,
            lp_param: *mut c_void,
        ) -> HWND;
        fn DestroyWindow(h_wnd: HWND) -> BOOL;
        fn ShowWindow(h_wnd: HWND, n_cmd_show: i32) -> BOOL;
        fn RegisterClassW(lp_wnd_class: *const WNDCLASSW) -> u16;
        fn DefWindowProcW(h_wnd: HWND, msg: u32, w_param: usize, l_param: isize) -> isize;
        fn SetWindowDisplayAffinity(h_wnd: HWND, dw_affinity: u32) -> BOOL;
        fn GetSystemMetrics(n_index: i32) -> i32;
        fn GetStockObject(i: i32) -> HGDIOBJ;
        fn GetModuleHandleW(lp_module_name: *const u16) -> HINSTANCE;
        fn GetCurrentThreadId() -> u32;
    }

    const WH_KEYBOARD_LL: i32 = 13;
    const WH_MOUSE_LL: i32 = 14;
    const WM_QUIT: u32 = 0x0012;
    const WM_CLOSE: u32 = 0x0010;
    const WM_KEYDOWN: usize = 0x0100;
    const WM_SYSKEYDOWN: usize = 0x0104;
    const VK_ESCAPE: u32 = 0x1B;
    const SM_XVIRTUALSCREEN: i32 = 76;
    const SM_YVIRTUALSCREEN: i32 = 77;
    const SM_CXVIRTUALSCREEN: i32 = 78;
    const SM_CYVIRTUALSCREEN: i32 = 79;
    const WS_POPUP: u32 = 0x80000000;
    const WS_VISIBLE: u32 = 0x10000000;
    const WS_EX_TOPMOST: u32 = 0x00000008;
    const WS_EX_TOOLWINDOW: u32 = 0x00000080;
    const WS_EX_NOACTIVATE: u32 = 0x08000000;
    const WDA_EXCLUDEFROMCAPTURE: u32 = 0x00000011;
    const BLACK_BRUSH: i32 = 4;
    const SW_SHOW: i32 = 5;

    static ACTIVE_HOOK_KBD: AtomicBool = AtomicBool::new(false);
    static ACTIVE_HOOK_MOUSE: AtomicBool = AtomicBool::new(false);
    static ESCAPE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static LAST_ESCAPE_TIME_MS: AtomicU64 = AtomicU64::new(0);
    static EMERGENCY_UNLOCKED: AtomicBool = AtomicBool::new(false);

    fn check_emergency_escape() -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let prev = LAST_ESCAPE_TIME_MS.swap(now, Ordering::SeqCst);
        if now.saturating_sub(prev) <= 1500 {
            let cnt = ESCAPE_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
            if cnt >= 5 {
                ESCAPE_COUNT.store(0, Ordering::SeqCst);
                return true;
            }
        } else {
            ESCAPE_COUNT.store(1, Ordering::SeqCst);
        }
        false
    }

    unsafe extern "system" fn low_level_keyboard_proc(
        code: i32,
        w_param: usize,
        l_param: isize,
    ) -> isize {
        if code >= 0 && ACTIVE_HOOK_KBD.load(Ordering::Relaxed) {
            let kbd = unsafe { &*(l_param as *const KBDLLHOOKSTRUCT) };
            // Injected events have LLKHF_INJECTED (0x01) or LLKHF_LOWER_IL_INJECTED (0x02)
            let is_injected = (kbd.flags & 1) != 0 || (kbd.flags & 2) != 0;
            if is_injected {
                return unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
            }

            // Physical keystroke! Check for emergency escape (5x Esc)
            if kbd.vk_code == VK_ESCAPE && (w_param == WM_KEYDOWN || w_param == WM_SYSKEYDOWN) {
                if check_emergency_escape() {
                    warn!("Emergency physical unlock triggered via 5x Escape sequence!");
                    EMERGENCY_UNLOCKED.store(true, Ordering::SeqCst);
                    ACTIVE_HOOK_KBD.store(false, Ordering::SeqCst);
                    ACTIVE_HOOK_MOUSE.store(false, Ordering::SeqCst);
                    return unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
                }
            }

            // Drop physical input
            return 1;
        }
        unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) }
    }

    unsafe extern "system" fn low_level_mouse_proc(
        code: i32,
        w_param: usize,
        l_param: isize,
    ) -> isize {
        if code >= 0 && ACTIVE_HOOK_MOUSE.load(Ordering::Relaxed) {
            let ms = unsafe { &*(l_param as *const MSLLHOOKSTRUCT) };
            let is_injected = (ms.flags & 1) != 0 || (ms.flags & 2) != 0;
            if is_injected {
                return unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
            }
            // Drop physical input
            return 1;
        }
        unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) }
    }

    unsafe extern "system" fn blackout_window_proc(
        hwnd: HWND,
        msg: u32,
        w_param: usize,
        l_param: isize,
    ) -> isize {
        match msg {
            WM_CLOSE => {
                let _ = unsafe { DestroyWindow(hwnd) };
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, w_param, l_param) },
        }
    }

    struct WorkerHandle {
        thread_id: u32,
        join_handle: Option<JoinHandle<()>>,
    }

    pub struct WindowsPrivacyManager {
        state: Mutex<PrivacyState>,
        worker: Mutex<Option<WorkerHandle>>,
    }

    impl WindowsPrivacyManager {
        pub fn new() -> Self {
            Self {
                state: Mutex::new(PrivacyState {
                    lock_input: false,
                    blackout: false,
                    supported: true,
                }),
                worker: Mutex::new(None),
            }
        }

        pub fn get_state(&self) -> PrivacyState {
            if EMERGENCY_UNLOCKED.swap(false, Ordering::SeqCst) {
                let mut st = self.state.lock().unwrap();
                st.lock_input = false;
                st.blackout = false;
                self.stop_worker();
            }
            *self.state.lock().unwrap()
        }

        fn stop_worker(&self) {
            ACTIVE_HOOK_KBD.store(false, Ordering::SeqCst);
            ACTIVE_HOOK_MOUSE.store(false, Ordering::SeqCst);
            let mut w = self.worker.lock().unwrap();
            if let Some(mut handle) = w.take() {
                unsafe {
                    PostThreadMessageW(handle.thread_id, WM_QUIT, 0, 0);
                }
                if let Some(jh) = handle.join_handle.take() {
                    let _ = jh.join();
                }
            }
        }

        pub fn set_privacy(&self, lock_input: bool, blackout: bool) -> Result<PrivacyState> {
            self.stop_worker();

            if lock_input || blackout {
                let (ready_tx, ready_rx) = channel::<u32>();
                let jh = std::thread::Builder::new()
                    .name("opendesk-privacy-worker".to_string())
                    .spawn(move || {
                        let tid = unsafe { GetCurrentThreadId() };
                        let _ = ready_tx.send(tid);

                        let mut kbd_hook: HHOOK = null_mut();
                        let mut mouse_hook: HHOOK = null_mut();
                        let mut blackout_hwnd: HWND = null_mut();

                        unsafe {
                            if lock_input {
                                ACTIVE_HOOK_KBD.store(true, Ordering::SeqCst);
                                ACTIVE_HOOK_MOUSE.store(true, Ordering::SeqCst);
                                kbd_hook = SetWindowsHookExW(
                                    WH_KEYBOARD_LL,
                                    low_level_keyboard_proc,
                                    GetModuleHandleW(null_mut()),
                                    0,
                                );
                                mouse_hook = SetWindowsHookExW(
                                    WH_MOUSE_LL,
                                    low_level_mouse_proc,
                                    GetModuleHandleW(null_mut()),
                                    0,
                                );
                            }

                            if blackout {
                                let class_name: Vec<u16> =
                                    "OpenDeskBlackoutScreen\0".encode_utf16().collect();
                                let wnd_class = WNDCLASSW {
                                    style: 0,
                                    lpfn_wnd_proc: Some(blackout_window_proc),
                                    cb_cls_extra: 0,
                                    cb_wnd_extra: 0,
                                    h_instance: GetModuleHandleW(null_mut()),
                                    h_icon: null_mut(),
                                    h_cursor: null_mut(),
                                    hbr_background: GetStockObject(BLACK_BRUSH) as HBRUSH,
                                    lpsz_menu_name: null_mut(),
                                    lpsz_class_name: class_name.as_ptr(),
                                };
                                RegisterClassW(&wnd_class);

                                let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
                                let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
                                let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
                                let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);

                                blackout_hwnd = CreateWindowExW(
                                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                                    class_name.as_ptr(),
                                    class_name.as_ptr(),
                                    WS_POPUP | WS_VISIBLE,
                                    vx,
                                    vy,
                                    vw,
                                    vh,
                                    null_mut(),
                                    null_mut(),
                                    GetModuleHandleW(null_mut()),
                                    null_mut(),
                                );

                                if !blackout_hwnd.is_null() {
                                    // Make invisible to DXGI Desktop Duplication & screen capture
                                    SetWindowDisplayAffinity(blackout_hwnd, WDA_EXCLUDEFROMCAPTURE);
                                    ShowWindow(blackout_hwnd, SW_SHOW);
                                }
                            }

                            let mut msg: MSG = std::mem::zeroed();
                            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                                if msg.message == WM_QUIT {
                                    break;
                                }
                                TranslateMessage(&msg);
                                DispatchMessageW(&msg);
                            }

                            if !kbd_hook.is_null() {
                                UnhookWindowsHookEx(kbd_hook);
                            }
                            if !mouse_hook.is_null() {
                                UnhookWindowsHookEx(mouse_hook);
                            }
                            if !blackout_hwnd.is_null() {
                                DestroyWindow(blackout_hwnd);
                            }
                        }
                    })?;

                let tid = ready_rx.recv().unwrap_or(0);
                let mut w = self.worker.lock().unwrap();
                *w = Some(WorkerHandle {
                    thread_id: tid,
                    join_handle: Some(jh),
                });
            }

            let mut st = self.state.lock().unwrap();
            st.lock_input = lock_input;
            st.blackout = blackout;
            st.supported = true;
            Ok(*st)
        }

        pub fn reset(&self) {
            self.stop_worker();
            let mut st = self.state.lock().unwrap();
            st.lock_input = false;
            st.blackout = false;
        }
    }

    impl Drop for WindowsPrivacyManager {
        fn drop(&mut self) {
            self.reset();
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use super::*;

    pub struct NonWindowsPrivacyManager {
        state: std::sync::Mutex<PrivacyState>,
    }

    impl NonWindowsPrivacyManager {
        pub fn new() -> Self {
            Self {
                state: std::sync::Mutex::new(PrivacyState {
                    lock_input: false,
                    blackout: false,
                    supported: false,
                }),
            }
        }

        pub fn get_state(&self) -> PrivacyState {
            *self.state.lock().unwrap()
        }

        pub fn set_privacy(&self, _lock_input: bool, _blackout: bool) -> Result<PrivacyState> {
            anyhow::bail!(
                "Privacy screen and physical input locking are currently supported on Windows hosts only."
            )
        }

        pub fn reset(&self) {
            let mut st = self.state.lock().unwrap();
            st.lock_input = false;
            st.blackout = false;
        }
    }
}

pub struct PrivacyController {
    #[cfg(target_os = "windows")]
    inner: imp::WindowsPrivacyManager,
    #[cfg(not(target_os = "windows"))]
    inner: imp::NonWindowsPrivacyManager,
}

impl PrivacyController {
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "windows")]
            inner: imp::WindowsPrivacyManager::new(),
            #[cfg(not(target_os = "windows"))]
            inner: imp::NonWindowsPrivacyManager::new(),
        }
    }

    pub fn set_privacy(&self, lock_input: bool, blackout: bool) -> Result<PrivacyState> {
        self.inner.set_privacy(lock_input, blackout)
    }

    pub fn get_privacy(&self) -> PrivacyState {
        self.inner.get_state()
    }

    pub fn reset(&self) {
        self.inner.reset();
    }
}

impl Default for PrivacyController {
    fn default() -> Self {
        Self::new()
    }
}
