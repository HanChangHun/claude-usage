//! Tray widget window policy for Windows.
//!
//! Install on the native UI thread before the first show. An ordinary hidden
//! owner preserves FancyZones eligibility; ITaskbarList::DeleteTab explicitly
//! removes the visible widget from taskbar and Alt+Tab registration.
//! Never re-add the widget with ITaskbarList::AddTab, and never set TOOLWINDOW
//! or Tauri skip_taskbar: FancyZones rejects tool windows. The tray provides
//! the independent way to restore the window. No polling, input simulation, or global hooks.

#[cfg(windows)]
mod windows {
    use std::{cell::Cell, ffi::c_void, ptr, rc::Rc};

    type Hwnd = *mut c_void;
    type Hresult = i32;
    type SubclassProc = unsafe extern "system" fn(Hwnd, u32, usize, isize, usize, usize) -> isize;
    const GWL_EXSTYLE: i32 = -20;
    const GWL_STYLE: i32 = -16;
    const GWLP_HWNDPARENT: i32 = -8;
    const CHILD: u32 = 0x4000_0000;
    const APPWINDOW: u32 = 0x0004_0000;
    const TOOLWINDOW: u32 = 0x0000_0080;
    const SWITCHER_BITS: u32 = APPWINDOW | TOOLWINDOW;
    const SUBCLASS_ID: usize = 0x414c_5454;
    const WM_SHOWWINDOW: u32 = 0x0018;
    const WM_SIZE: u32 = 0x0005;
    const WM_STYLECHANGED: u32 = 0x007d;
    const WM_WINDOWPOSCHANGED: u32 = 0x0047;
    const SWP_SHOWWINDOW: u32 = 0x0040;
    const SWP_HIDEWINDOW: u32 = 0x0080;
    const WM_STYLECHANGING: u32 = 0x007c;
    const WM_NCDESTROY: u32 = 0x0082;
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
        style & !SWITCHER_BITS
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
    unsafe fn set_owner(hwnd: Hwnd, owner: Hwnd) -> Result<(), String> {
        SetLastError(0);
        let previous = SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner as isize);
        let code = GetLastError();
        if previous == 0 && code != 0 {
            Err(error("Set widget owner", code))
        } else {
            Ok(())
        }
    }

    // This helper has no State pointer. If detachment ever fails during native
    // destruction, its single deferred message can safely finish after the main
    // HWND has gone away. Never destroy an owner that still owns a live main HWND.
    unsafe extern "system" fn owner_subclass(
        hwnd: Hwnd,
        message: u32,
        wparam: usize,
        lparam: isize,
        id: usize,
        main: usize,
    ) -> isize {
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(owner_subclass), id);
        } else if message == id as u32 {
            let main = main as Hwnd;
            if IsWindow(main) == 0 || GetWindowLongPtrW(main, GWLP_HWNDPARENT) != hwnd as isize {
                DestroyWindow(hwnd);
            } else {
                eprintln!("Widget hidden owner retained because native detachment failed");
            }
            return 0;
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }

    unsafe fn create_owner(main: Hwnd, cleanup_message: u32) -> Result<Hwnd, String> {
        let class: Vec<u16> = "STATIC".encode_utf16().chain(Some(0)).collect();
        let title = [0u16];
        // An ordinary, unowned top-level HWND. Do not use HWND_MESSAGE or
        // SetParent, and never show/minimize/activate this zero-sized owner.
        let owner = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            0x8000_0000, // WS_POPUP permits a truly zero-sized ordinary HWND.
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        );
        if owner.is_null() {
            return Err(error("Create hidden widget owner", GetLastError()));
        }
        if SetWindowSubclass(
            owner,
            Some(owner_subclass),
            cleanup_message as usize,
            main as usize,
        ) == 0
        {
            let code = GetLastError();
            DestroyWindow(owner);
            return Err(error("Install hidden owner cleanup", code));
        }
        Ok(owner)
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
        original_owner: Hwnd,
        owner: Cell<Hwnd>,
        owner_cleanup: u32,
        taskbar_created: u32,
        sync_message: u32,
        taskbar: Cell<*mut Taskbar>,
        removed: Cell<bool>,
        explorer_changed: Cell<bool>,
        queued: Cell<bool>,
        busy: Cell<bool>,
        size_state: Cell<usize>,
        enabled: Cell<bool>,
        closed: Cell<bool>,
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
                return Ok(());
            }
            if self.explorer_changed.replace(false) {
                self.removed.set(false);
                let old = self.taskbar.replace(ptr::null_mut());
                if !old.is_null() {
                    ((*(*old).vtable).release)(old);
                }
            }
            if self.taskbar.get().is_null() {
                self.taskbar.set(create_taskbar()?);
            }
            if !self.removed.get() && !self.closed.get() {
                let taskbar = self.taskbar.get();
                succeeded(
                    ((*(*taskbar).vtable).delete_tab)(taskbar, self.hwnd),
                    "Exclude widget from taskbar and Alt+Tab",
                )?;
                // COM can dispatch nested window messages; the callback's Rc
                // holds the interface alive until this operation returns.
                if !self.closed.get() {
                    self.removed.set(true);
                }
            }
            if self.explorer_changed.get() {
                self.queue();
            }
            Ok(())
        }
        unsafe fn release_owner(&self) -> Result<(), String> {
            let owner = self.owner.get();
            if owner.is_null() {
                return Ok(());
            }
            if IsWindow(self.hwnd) != 0
                && GetWindowLongPtrW(self.hwnd, GWLP_HWNDPARENT) == owner as isize
            {
                let original = if IsWindow(self.original_owner) != 0 {
                    self.original_owner
                } else {
                    ptr::null_mut()
                };
                set_owner(self.hwnd, original)?;
                // Verify detachment before destroying an owner: DestroyWindow
                // would otherwise recursively destroy its owned main window.
                if GetWindowLongPtrW(self.hwnd, GWLP_HWNDPARENT) == owner as isize {
                    return Err("Native widget owner did not detach".into());
                }
            }
            if IsWindow(owner) != 0 && DestroyWindow(owner) == 0 {
                return Err(error("Destroy hidden widget owner", GetLastError()));
            }
            self.owner.set(ptr::null_mut());
            Ok(())
        }
        unsafe fn restore_configuration(&self) -> Result<(), String> {
            self.enabled.set(false);
            if self.closed.get() || IsWindow(self.hwnd) == 0 {
                return Ok(());
            }
            let restored_owner = self.release_owner();
            let current = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32;
            let mut restored_style = (current & !SWITCHER_BITS) | self.original_bits;
            if restored_owner.is_err() {
                // If Windows refuses to detach, keep the widget reachable in
                // both switchers despite the remaining invisible owner.
                restored_style = (restored_style | APPWINDOW) & !TOOLWINDOW;
            }
            let restored_style = set_style(self.hwnd, restored_style);
            restored_owner.and(restored_style)
        }
    }
    impl Drop for State {
        fn drop(&mut self) {
            unsafe {
                if let Err(error) = self.release_owner() {
                    eprintln!("Widget hidden owner cleanup deferred: {error}");
                    let owner = self.owner.replace(ptr::null_mut());
                    if !owner.is_null() {
                        PostMessageW(owner, self.owner_cleanup, 0, 0);
                    }
                }
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
            // Detach while main HWND is still valid. The State remains alive
            // through DefSubclassProc and any nested COM/window callbacks.
            if let Err(error) = state.release_owner() {
                eprintln!("Widget hidden owner detachment deferred: {error}");
            }
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
                    // The tray remains available. Retry on the next lifecycle
                    // event instead of polling or changing FancyZones styles.
                    eprintln!("Widget shell exclusion unavailable: {error}");
                }
                return 0;
            } else if message == state.taskbar_created {
                state.explorer_changed.set(true);
                state.removed.set(false);
                state.queue();
            } else if message == WM_SHOWWINDOW || message == WM_STYLECHANGED {
                state.removed.set(false);
                state.queue();
            } else if message == WM_SIZE {
                if state.size_state.replace(wparam) != wparam {
                    state.removed.set(false);
                    state.queue();
                }
            } else if message == WM_WINDOWPOSCHANGED && lparam != 0 {
                let position = &*(lparam as *const WindowPos);
                if position.flags & (SWP_SHOWWINDOW | SWP_HIDEWINDOW) != 0 {
                    state.removed.set(false);
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
            if GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & CHILD != 0 {
                return Err("Widget Alt+Tab exclusion requires a top-level window".into());
            }
            let mut existing = 0;
            if GetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, &mut existing) != 0 {
                let state = &*(existing as *const State);
                if state.enabled.get() {
                    return Ok(());
                }
                return Err(
                    "Widget Alt+Tab exclusion was disabled after a native window error".into(),
                );
            }
            if IsWindowVisible(hwnd) != 0 {
                return Err("Install tray widget policy before showing the window".into());
            }
            succeeded(
                CoInitializeEx(ptr::null_mut(), 2),
                "Initialize widget COM apartment",
            )?;
            let state = Rc::new(State {
                hwnd,
                original_bits: GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & SWITCHER_BITS,
                original_owner: GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT) as Hwnd,
                owner: Cell::new(ptr::null_mut()),
                owner_cleanup: 0,
                taskbar_created: 0,
                sync_message: 0,
                taskbar: Cell::new(ptr::null_mut()),
                removed: Cell::new(false),
                explorer_changed: Cell::new(false),
                queued: Cell::new(false),
                busy: Cell::new(false),
                size_state: Cell::new(usize::MAX),
                enabled: Cell::new(true),
                closed: Cell::new(false),
            });
            // Fill registered messages before exposing the state to native code.
            let mut state = Rc::try_unwrap(state)
                .ok()
                .expect("new widget state is unique");
            state.taskbar_created = registered_message("TaskbarCreated")?;
            state.sync_message = registered_message("HanChangHun.WidgetTray.Sync.v2")?;
            state.owner_cleanup = registered_message("HanChangHun.WidgetAltTab.OwnerCleanup.v1")?;
            state.owner.set(create_owner(hwnd, state.owner_cleanup)?);
            let state = Rc::new(state);
            let raw = Rc::into_raw(state.clone());
            if SetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, raw as usize) == 0 {
                drop(Rc::from_raw(raw));
                return Err(error("Install widget window hook", GetLastError()));
            }
            let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            let result = set_owner(hwnd, state.owner.get())
                .and_then(|_| set_style(hwnd, widget_style(current)));
            if let Err(error) = result {
                let restored = state.restore_configuration();
                // Keep the disabled subclass alive until NCDESTROY if native
                // detachment failed, so it still owns and cleans the helper.
                if state.owner.get().is_null()
                    && RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID) != 0
                {
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
        #[repr(C)]
        #[derive(Debug, PartialEq)]
        struct Rect {
            left: i32,
            top: i32,
            right: i32,
            bottom: i32,
        }
        #[repr(C)]
        struct WindowClass {
            style: u32,
            proc: Option<unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize>,
            class_extra: i32,
            window_extra: i32,
            instance: Hwnd,
            icon: Hwnd,
            cursor: Hwnd,
            background: Hwnd,
            menu: *const u16,
            name: *const u16,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetModuleHandleW(name: *const u16) -> Hwnd;
        }
        #[link(name = "user32")]
        extern "system" {
            fn RegisterClassW(class: *const WindowClass) -> u16;
            fn DefWindowProcW(hwnd: Hwnd, message: u32, wparam: usize, lparam: isize) -> isize;
            fn SendMessageW(hwnd: Hwnd, message: u32, wparam: usize, lparam: isize) -> isize;
            fn PeekMessageW(
                message: *mut Message,
                hwnd: Hwnd,
                first: u32,
                last: u32,
                flags: u32,
            ) -> i32;
            fn DispatchMessageW(message: *const Message) -> isize;
            fn GetAncestor(hwnd: Hwnd, flags: u32) -> Hwnd;
            fn GetWindowRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
            fn GetForegroundWindow() -> Hwnd;
            fn ShowWindow(hwnd: Hwnd, command: i32) -> i32;
            fn IsIconic(hwnd: Hwnd) -> i32;
        }
        struct Destroy(Hwnd);
        impl Drop for Destroy {
            fn drop(&mut self) {
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }
        unsafe fn window(ex_style: u32, owner: Hwnd) -> Destroy {
            let class: Vec<u16> = "HanChangHun.WidgetAltTab.NativeTests"
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let instance = GetModuleHandleW(ptr::null());
            let definition = WindowClass {
                style: 0,
                proc: Some(DefWindowProcW),
                class_extra: 0,
                window_extra: 0,
                instance,
                icon: ptr::null_mut(),
                cursor: ptr::null_mut(),
                background: ptr::null_mut(),
                menu: ptr::null(),
                name: class.as_ptr(),
            };
            if RegisterClassW(&definition) == 0 {
                assert_eq!(GetLastError(), 1410, "test class registration");
            }
            let title: Vec<u16> = "Isolated widget native test"
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let hwnd = CreateWindowExW(
                ex_style,
                class.as_ptr(),
                title.as_ptr(),
                0x00cf0000,
                -30000,
                -30000,
                320,
                240,
                owner,
                ptr::null_mut(),
                instance,
                ptr::null_mut(),
            );
            assert!(!hwnd.is_null());
            Destroy(hwnd)
        }
        unsafe fn rect(hwnd: Hwnd) -> Rect {
            let mut rect = Rect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            assert_ne!(GetWindowRect(hwnd, &mut rect), 0);
            rect
        }
        unsafe fn reference(hwnd: Hwnd) -> Rc<State> {
            let mut reference = 0;
            assert_ne!(
                GetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, &mut reference),
                0
            );
            Rc::increment_strong_count(reference as *const State);
            Rc::from_raw(reference as *const State)
        }
        unsafe fn pump(hwnd: Hwnd) {
            let mut message = std::mem::MaybeUninit::<Message>::uninit();
            let mut count = 0;
            while PeekMessageW(message.as_mut_ptr(), hwnd, 0, 0, 1) != 0 {
                DispatchMessageW(message.as_ptr());
                count += 1;
                assert!(count < 30, "event-driven native sync must not poll");
            }
        }

        // Same relevant eligibility checks as PowerToys 0.102.0. The window
        // remains a root, resizable window with no visible owner or tool flag.
        unsafe fn assert_zone_candidate(hwnd: Hwnd) {
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            assert_ne!(IsWindowVisible(hwnd), 0);
            assert_eq!(ex & (TOOLWINDOW | APPWINDOW), 0);
            assert_eq!(GetAncestor(hwnd, 2), hwnd);
            let owner = GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT) as Hwnd;
            assert!(!owner.is_null());
            assert_eq!(IsWindowVisible(owner), 0);
            assert_ne!(style & 0x0004_0000, 0, "native resize frame");
            assert!(style & 0x8000_0000 == 0 || style & 0x00cf_0000 != 0);
        }

        #[test]
        fn policy_preserves_unrelated_styles_and_rejects_invalid_handles() {
            for style in [0, APPWINDOW, TOOLWINDOW, 8, 0x0800_0000, u32::MAX] {
                assert_eq!(widget_style(style) & SWITCHER_BITS, 0);
                assert_eq!(widget_style(style) & !SWITCHER_BITS, style & !SWITCHER_BITS);
            }
            assert!(install(0).is_err());
        }

        #[test]
        fn native_owner_and_fancyzones_survive_restore_minimize_and_pin() {
            unsafe {
                // Real user32 windows, no mocked taskbar COM. All windows belong
                // to this test and stay off-screen without activating anything.
                let main = window(APPWINDOW, ptr::null_mut());
                let hwnd = main.0;
                let foreground = GetForegroundWindow();
                let original = rect(hwnd);
                install(hwnd as isize).unwrap();
                let state = reference(hwnd);
                let owner = state.owner.get();
                assert_eq!(GetAncestor(owner, 2), owner);
                assert_eq!(GetWindowLongPtrW(owner, GWLP_HWNDPARENT), 0);
                let bounds = rect(owner);
                assert_eq!(
                    (bounds.right - bounds.left, bounds.bottom - bounds.top),
                    (0, 0)
                );
                assert_eq!(rect(hwnd), original);
                install(hwnd as isize).unwrap();
                assert!(Rc::ptr_eq(&state, &reference(hwnd)));
                let handle = hwnd as isize;
                assert!(std::thread::spawn(move || install(handle))
                    .join()
                    .unwrap()
                    .is_err());

                for _ in 0..3 {
                    ShowWindow(hwnd, 4);
                    pump(hwnd);
                    assert_zone_candidate(hwnd);
                    // Tao rewrites these styles during always-on-top changes.
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (APPWINDOW | TOOLWINDOW | 0x100) as isize);
                    assert_zone_candidate(hwnd);
                    assert_ne!(GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & 0x100, 0);
                    let bounds = rect(hwnd);
                    let point = |x: i32, y: i32| -> isize {
                        ((x as u16 as u32) | ((y as u16 as u32) << 16)) as isize
                    };
                    assert_eq!(
                        SendMessageW(hwnd, 0x84, 0, point(bounds.left + 50, bounds.top + 20)),
                        2
                    );
                    assert_eq!(
                        SendMessageW(hwnd, 0x84, 0, point(bounds.right - 2, bounds.bottom - 2)),
                        17
                    );
                    ShowWindow(hwnd, 7);
                    pump(hwnd);
                    assert_ne!(IsIconic(hwnd), 0);
                    ShowWindow(hwnd, 4);
                    pump(hwnd);
                    assert_eq!(IsIconic(hwnd), 0);
                    assert_zone_candidate(hwnd);
                    // Explorer recreation does not change owner/style policy.
                    SendMessageW(hwnd, registered_message("TaskbarCreated").unwrap(), 0, 0);
                    pump(hwnd);
                    assert_zone_candidate(hwnd);
                    ShowWindow(hwnd, 0);
                    pump(hwnd);
                    assert_eq!(IsWindowVisible(hwnd), 0);
                    assert_eq!(rect(hwnd), original);
                }
                assert_eq!(GetForegroundWindow(), foreground);
                drop(main);
                assert!(state.closed.get());
                assert_eq!(IsWindow(owner), 0);
            }
        }

        #[test]
        fn rollback_restores_previous_owner_without_destroying_main() {
            unsafe {
                let original = window(0, ptr::null_mut());
                let main = window(APPWINDOW, original.0);
                install(main.0 as isize).unwrap();
                let state = reference(main.0);
                let owner = state.owner.get();
                state.restore_configuration().unwrap();
                assert_eq!(IsWindow(owner), 0);
                assert_ne!(IsWindow(main.0), 0);
                assert_ne!(IsWindow(original.0), 0);
                assert_eq!(
                    GetWindowLongPtrW(main.0, GWLP_HWNDPARENT),
                    original.0 as isize
                );
                assert_eq!(
                    GetWindowLongPtrW(main.0, GWL_EXSTYLE) as u32 & SWITCHER_BITS,
                    APPWINDOW
                );
            }
        }
    }
}

#[cfg(windows)]
pub use self::windows::install;
