//! Hide a widget from Alt+Tab while retaining its taskbar button.
//!
//! Call `install(window.hwnd()?.0 as isize)` on the native window thread, after
//! creating the main window and before its first show when possible. Failure
//! restores the original switcher styles; the caller can log and continue.
//! Native subclass messages maintain the policy without a worker or polling.

#[cfg(windows)]
mod windows {
    use std::{cell::Cell, ffi::c_void, ptr, rc::Rc};

    type Hwnd = *mut c_void;
    type Hresult = i32;
    type SubclassProc = unsafe extern "system" fn(Hwnd, u32, usize, isize, usize, usize) -> isize;
    const GWL_EXSTYLE: i32 = -20;
    const APPWINDOW: u32 = 0x0004_0000;
    const TOOLWINDOW: u32 = 0x0000_0080;
    const SWITCHER_BITS: u32 = APPWINDOW | TOOLWINDOW;
    const SUBCLASS_ID: usize = 0x414c_5454;
    const WM_SIZE: u32 = 0x0005;
    const WM_SHOWWINDOW: u32 = 0x0018;
    const WM_WINDOWPOSCHANGED: u32 = 0x0047;
    const WM_STYLECHANGING: u32 = 0x007c;
    const WM_NCDESTROY: u32 = 0x0082;
    const SWP_SHOWWINDOW: u32 = 0x0040;
    const SWP_HIDEWINDOW: u32 = 0x0080;
    const FRAME_REFRESH: u32 = 0x0020 | 0x0001 | 0x0002 | 0x0004 | 0x0010;

    #[repr(C)]
    struct StyleStruct {
        old: u32,
        new: u32,
    }
    #[repr(C)]
    struct WindowPos {
        hwnd: Hwnd,
        after: Hwnd,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    }
    #[repr(C)]
    struct Guid(u32, u16, u16, [u8; 8]);
    const CLSID_TASKBAR: Guid = Guid(
        0x56fdf344,
        0xfd6d,
        0x11d0,
        [0x95, 0x8a, 0, 0x60, 0x97, 0xc9, 0xa0, 0x90],
    );
    const IID_TASKBAR: Guid = Guid(
        0x56fdf342,
        0xfd6d,
        0x11d0,
        [0x95, 0x8a, 0, 0x60, 0x97, 0xc9, 0xa0, 0x90],
    );
    #[repr(C)]
    struct Taskbar {
        vtable: *const TaskbarVtable,
    }
    #[repr(C)]
    struct TaskbarVtable {
        query_interface:
            unsafe extern "system" fn(*mut Taskbar, *const Guid, *mut *mut c_void) -> Hresult,
        add_ref: unsafe extern "system" fn(*mut Taskbar) -> u32,
        release: unsafe extern "system" fn(*mut Taskbar) -> u32,
        init: unsafe extern "system" fn(*mut Taskbar) -> Hresult,
        add_tab: unsafe extern "system" fn(*mut Taskbar, Hwnd) -> Hresult,
        delete_tab: unsafe extern "system" fn(*mut Taskbar, Hwnd) -> Hresult,
        activate_tab: unsafe extern "system" fn(*mut Taskbar, Hwnd) -> Hresult,
        set_active_alt: unsafe extern "system" fn(*mut Taskbar, Hwnd) -> Hresult,
    }

    #[link(name = "user32")]
    extern "system" {
        fn IsWindow(hwnd: Hwnd) -> i32;
        fn IsWindowVisible(hwnd: Hwnd) -> i32;
        fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
        fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
        fn SetWindowLongPtrW(hwnd: Hwnd, index: i32, value: isize) -> isize;
        fn SetWindowPos(
            hwnd: Hwnd,
            after: Hwnd,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
        fn RegisterWindowMessageW(name: *const u16) -> u32;
        fn PostMessageW(hwnd: Hwnd, message: u32, wparam: usize, lparam: isize) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThreadId() -> u32;
        fn GetCurrentProcessId() -> u32;
        fn SetLastError(error: u32);
        fn GetLastError() -> u32;
    }
    #[link(name = "comctl32")]
    extern "system" {
        fn SetWindowSubclass(hwnd: Hwnd, proc: Option<SubclassProc>, id: usize, data: usize)
            -> i32;
        fn GetWindowSubclass(
            hwnd: Hwnd,
            proc: Option<SubclassProc>,
            id: usize,
            data: *mut usize,
        ) -> i32;
        fn RemoveWindowSubclass(hwnd: Hwnd, proc: Option<SubclassProc>, id: usize) -> i32;
        fn DefSubclassProc(hwnd: Hwnd, message: u32, wparam: usize, lparam: isize) -> isize;
    }
    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> Hresult;
        fn CoUninitialize();
        fn CoCreateInstance(
            clsid: *const Guid,
            outer: *mut c_void,
            context: u32,
            iid: *const Guid,
            object: *mut *mut c_void,
        ) -> Hresult;
    }

    fn widget_style(style: u32) -> u32 {
        (style | TOOLWINDOW) & !APPWINDOW
    }
    fn error(operation: &str, value: u32) -> String {
        format!("{operation} failed (0x{value:08X})")
    }
    fn succeeded(result: Hresult, operation: &str) -> Result<(), String> {
        if result < 0 {
            Err(error(operation, result as u32))
        } else {
            Ok(())
        }
    }
    unsafe fn create_taskbar() -> Result<*mut Taskbar, String> {
        let mut result = ptr::null_mut();
        succeeded(
            CoCreateInstance(
                &CLSID_TASKBAR,
                ptr::null_mut(),
                1,
                &IID_TASKBAR,
                &mut result,
            ),
            "Create taskbar interface",
        )?;
        let taskbar = result as *mut Taskbar;
        if taskbar.is_null() {
            return Err("Taskbar interface is null".into());
        }
        if let Err(error) = succeeded(
            ((*(*taskbar).vtable).init)(taskbar),
            "Initialize taskbar interface",
        ) {
            ((*(*taskbar).vtable).release)(taskbar);
            return Err(error);
        }
        Ok(taskbar)
    }
    unsafe fn set_style(hwnd: Hwnd, style: u32) -> Result<(), String> {
        SetLastError(0);
        let previous = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style as isize);
        let code = GetLastError();
        if previous == 0 && code != 0 {
            return Err(error("Set widget window style", code));
        }
        if SetWindowPos(hwnd, ptr::null_mut(), 0, 0, 0, 0, FRAME_REFRESH) == 0 {
            return Err(error("Refresh widget frame", GetLastError()));
        }
        Ok(())
    }
    unsafe fn registered_message(name: &str) -> Result<u32, String> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let message = RegisterWindowMessageW(wide.as_ptr());
        if message == 0 {
            Err(error("Register widget message", GetLastError()))
        } else {
            Ok(message)
        }
    }

    struct State {
        hwnd: Hwnd,
        original_bits: u32,
        taskbar_created: u32,
        sync_message: u32,
        taskbar: Cell<*mut Taskbar>,
        enabled: Cell<bool>,
        registered: Cell<bool>,
        explorer_changed: Cell<bool>,
        queued: Cell<bool>,
        busy: Cell<bool>,
        closed: Cell<bool>,
        size_state: Cell<usize>,
    }
    impl State {
        unsafe fn queue(&self) {
            if !self.closed.get()
                && !self.queued.replace(true)
                && PostMessageW(self.hwnd, self.sync_message, 0, 0) == 0
            {
                self.queued.set(false);
            }
        }
        unsafe fn delete_tab(&self) {
            if self.registered.replace(false) {
                let taskbar = self.taskbar.get();
                if !taskbar.is_null() {
                    ((*(*taskbar).vtable).delete_tab)(taskbar, self.hwnd);
                }
            }
        }
        unsafe fn sync(&self) -> Result<(), String> {
            if self.closed.get() || self.busy.replace(true) {
                return Ok(());
            }
            struct Reset<'a>(&'a Cell<bool>);
            impl Drop for Reset<'_> {
                fn drop(&mut self) {
                    self.0.set(false);
                }
            }
            let _reset = Reset(&self.busy);
            if IsWindowVisible(self.hwnd) == 0 {
                self.delete_tab();
                return Ok(());
            }
            if self.explorer_changed.replace(false) {
                self.registered.set(false);
                let old = self.taskbar.replace(ptr::null_mut());
                if !old.is_null() {
                    ((*(*old).vtable).release)(old);
                }
                self.taskbar.set(create_taskbar()?);
            }
            if !self.registered.get() {
                let taskbar = self.taskbar.get();
                if taskbar.is_null() {
                    return Err("Taskbar interface is unavailable".into());
                }
                succeeded(
                    ((*(*taskbar).vtable).add_tab)(taskbar, self.hwnd),
                    "Keep widget taskbar button",
                )?;
                // A COM call can dispatch nested window messages. Keep the State
                // alive through it and remove a just-added tab if destruction ran.
                if self.closed.get() {
                    ((*(*taskbar).vtable).delete_tab)(taskbar, self.hwnd);
                } else {
                    self.registered.set(true);
                }
            }
            if self.explorer_changed.get() {
                self.queue();
            }
            Ok(())
        }
        unsafe fn restore_style(&self) -> Result<(), String> {
            self.enabled.set(false);
            if self.closed.get() || IsWindow(self.hwnd) == 0 {
                return Ok(());
            }
            let current = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32;
            set_style(self.hwnd, (current & !SWITCHER_BITS) | self.original_bits)
        }
    }
    impl Drop for State {
        fn drop(&mut self) {
            unsafe {
                self.delete_tab();
                let taskbar = self.taskbar.replace(ptr::null_mut());
                if !taskbar.is_null() {
                    ((*(*taskbar).vtable).release)(taskbar);
                }
                CoUninitialize();
            }
        }
    }

    unsafe extern "system" fn subclass(
        hwnd: Hwnd,
        message: u32,
        wparam: usize,
        lparam: isize,
        _id: usize,
        data: usize,
    ) -> isize {
        let raw = data as *const State;
        // One Rc belongs to the installed subclass; each invocation holds a
        // temporary Rc across calls that may dispatch nested native messages.
        Rc::increment_strong_count(raw);
        let state = Rc::from_raw(raw);
        if message == WM_NCDESTROY && !state.closed.replace(true) {
            state.enabled.set(false);
            state.delete_tab();
            RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID);
            drop(Rc::from_raw(raw));
            return DefSubclassProc(hwnd, message, wparam, lparam);
        }
        if state.enabled.get() && !state.closed.get() {
            if message == WM_STYLECHANGING && wparam as isize == GWL_EXSTYLE as isize && lparam != 0
            {
                let styles = &mut *(lparam as *mut StyleStruct);
                styles.new = widget_style(styles.new);
            } else if message == state.sync_message {
                state.queued.set(false);
                if let Err(error) = state.sync() {
                    let restored = state.restore_style();
                    eprintln!("Widget Alt+Tab exclusion disabled: {error}; restore={restored:?}");
                }
                return 0;
            } else if message == state.taskbar_created {
                state.registered.set(false);
                state.explorer_changed.set(true);
                state.queue();
            } else if message == WM_SHOWWINDOW {
                if wparam == 0 {
                    state.delete_tab();
                }
                state.queue();
            } else if message == WM_SIZE {
                if state.size_state.replace(wparam) != wparam {
                    state.registered.set(false);
                }
                state.queue();
            } else if message == WM_WINDOWPOSCHANGED && lparam != 0 {
                let position = &*(lparam as *const WindowPos);
                if position.flags & SWP_HIDEWINDOW != 0 {
                    state.delete_tab();
                }
                if position.flags & (SWP_SHOWWINDOW | SWP_HIDEWINDOW) != 0 {
                    state.queue();
                }
            }
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }

    pub fn install(handle: isize) -> Result<(), String> {
        unsafe {
            let hwnd = handle as Hwnd;
            let mut pid = 0;
            let thread = GetWindowThreadProcessId(hwnd, &mut pid);
            if IsWindow(hwnd) == 0 || pid != GetCurrentProcessId() || thread != GetCurrentThreadId()
            {
                return Err("Install widget Alt+Tab exclusion on its native window thread".into());
            }
            let mut existing = 0;
            if GetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, &mut existing) != 0 {
                let state = &*(existing as *const State);
                if state.enabled.get() {
                    state.queue();
                    return Ok(());
                }
                return Err("Widget Alt+Tab exclusion was disabled after a taskbar error".into());
            }
            succeeded(
                CoInitializeEx(ptr::null_mut(), 2),
                "Initialize widget COM apartment",
            )?;
            let taskbar = match create_taskbar() {
                Ok(taskbar) => taskbar,
                Err(error) => {
                    CoUninitialize();
                    return Err(error);
                }
            };
            let state = Rc::new(State {
                hwnd,
                original_bits: GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & SWITCHER_BITS,
                taskbar_created: 0,
                sync_message: 0,
                taskbar: Cell::new(taskbar),
                enabled: Cell::new(true),
                registered: Cell::new(false),
                explorer_changed: Cell::new(false),
                queued: Cell::new(false),
                busy: Cell::new(false),
                closed: Cell::new(false),
                size_state: Cell::new(usize::MAX),
            });
            // Fill registered messages before exposing the state to native code.
            let mut state = Rc::try_unwrap(state)
                .ok()
                .expect("new widget state is unique");
            state.taskbar_created = registered_message("TaskbarCreated")?;
            state.sync_message = registered_message("HanChangHun.WidgetAltTab.Sync.v1")?;
            let state = Rc::new(state);
            let raw = Rc::into_raw(state.clone());
            if SetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, raw as usize) == 0 {
                drop(Rc::from_raw(raw));
                return Err(error("Install widget window hook", GetLastError()));
            }
            let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            let result = set_style(hwnd, widget_style(current)).and_then(|_| state.sync());
            if let Err(error) = result {
                let restored = state.restore_style();
                state.delete_tab();
                if RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID) != 0 {
                    drop(Rc::from_raw(raw));
                }
                return Err(format!("{error}; restore={restored:?}"));
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn widget_bits_preserve_unrelated_window_behaviour() {
            for style in [0, APPWINDOW, 0x8, 0x0800_0000, 0x8008_0130, u32::MAX] {
                let next = widget_style(style);
                assert_eq!(next & SWITCHER_BITS, TOOLWINDOW);
                assert_eq!(next & !SWITCHER_BITS, style & !SWITCHER_BITS);
            }
        }

        #[repr(C)]
        struct Message {
            hwnd: Hwnd,
            message: u32,
            wparam: usize,
            lparam: isize,
            time: u32,
            point: [i32; 2],
            private: u32,
        }
        #[link(name = "user32")]
        extern "system" {
            fn CreateWindowExW(
                ex_style: u32,
                class: *const u16,
                title: *const u16,
                style: u32,
                x: i32,
                y: i32,
                width: i32,
                height: i32,
                parent: Hwnd,
                menu: Hwnd,
                instance: Hwnd,
                parameter: *mut c_void,
            ) -> Hwnd;
            fn DestroyWindow(hwnd: Hwnd) -> i32;
            fn SendMessageW(hwnd: Hwnd, message: u32, wparam: usize, lparam: isize) -> isize;
            fn PeekMessageW(
                message: *mut Message,
                hwnd: Hwnd,
                first: u32,
                last: u32,
                flags: u32,
            ) -> i32;
            fn DispatchMessageW(message: *const Message) -> isize;
        }

        #[test]
        fn hidden_native_window_lifecycle_preserves_styles_and_thread_ownership() {
            // Create only our own hidden off-screen window. Never enumerate,
            // show, focus, or modify any of the user's running widget windows.
            unsafe {
                let class: Vec<u16> = "STATIC".encode_utf16().chain(Some(0)).collect();
                let title: Vec<u16> = "Widget AltTab hidden test"
                    .encode_utf16()
                    .chain(Some(0))
                    .collect();
                let hwnd = CreateWindowExW(
                    APPWINDOW,
                    class.as_ptr(),
                    title.as_ptr(),
                    0x00cf0000,
                    -30000,
                    -30000,
                    320,
                    240,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                assert!(!hwnd.is_null());
                struct Destroy(Hwnd);
                impl Drop for Destroy {
                    fn drop(&mut self) {
                        unsafe {
                            DestroyWindow(self.0);
                        }
                    }
                }
                let destroy = Destroy(hwnd);
                assert_eq!(IsWindowVisible(hwnd), 0);
                install(hwnd as isize).unwrap();
                let mut reference = 0;
                assert_ne!(
                    GetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, &mut reference),
                    0
                );
                assert_ne!(reference, 0);
                let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
                assert_eq!(style & SWITCHER_BITS, TOOLWINDOW);
                assert_eq!(style & 0x0800_0000, 0, "NOACTIVATE must not be added");
                assert_eq!(IsWindowVisible(hwnd), 0);
                install(hwnd as isize).unwrap();
                let mut repeated = 0;
                assert_ne!(
                    GetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, &mut repeated),
                    0
                );
                assert_eq!(reference, repeated, "install is idempotent");

                // Tao rewrites its complete extended style on pin/decoration
                // changes. The native style-changing hook must keep our policy
                // while preserving the new unrelated style flags.
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (APPWINDOW | 0x0008_0000) as isize);
                let updated = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
                assert_eq!(updated & SWITCHER_BITS, TOOLWINDOW);
                assert_ne!(updated & 0x0008_0000, 0);

                let handle = hwnd as isize;
                assert!(std::thread::spawn(move || install(handle))
                    .join()
                    .unwrap()
                    .is_err());
                let state = &*(reference as *const State);
                SendMessageW(hwnd, state.taskbar_created, 0, 0);
                let mut message = std::mem::MaybeUninit::<Message>::uninit();
                let mut count = 0;
                while PeekMessageW(message.as_mut_ptr(), hwnd, 0, 0, 1) != 0 {
                    DispatchMessageW(message.as_ptr());
                    count += 1;
                    assert!(count < 20, "event-driven sync must not poll itself");
                }
                assert_eq!(IsWindowVisible(hwnd), 0);
                assert!(
                    !state.registered.get(),
                    "hidden windows must have no manual taskbar tab"
                );
                drop(destroy);
                assert_eq!(IsWindow(hwnd), 0);
            }
        }
    }
}

#[cfg(windows)]
pub use self::windows::install;

#[cfg(not(windows))]
pub fn install(_handle: isize) -> Result<(), String> {
    Ok(())
}
