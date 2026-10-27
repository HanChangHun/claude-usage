//! Hide a widget from Alt+Tab while retaining its taskbar button.
//!
//! Call `install(window.hwnd()?.0 as isize)` on the native window thread, after
//! creating the main window and before its first show when possible. Failure
//! restores the original owner and switcher styles; log failures and continue.
//! An invisible ordinary top-level owner suppresses Alt+Tab without TOOLWINDOW,
//! so the main window remains eligible for FancyZones. Native messages maintain
//! the policy and the manual taskbar button without a worker or polling.

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
        #[cfg(test)]
        if let Some(taskbar) = tests::mock_taskbar() {
            return Ok(taskbar);
        }
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
            if self.closed.get() {
                return Ok(());
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
                if self.closed.get() || IsWindowVisible(self.hwnd) == 0 {
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
                self.delete_tab();
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
            state.delete_tab();
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
                    let restored = state.restore_configuration();
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
            if GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & CHILD != 0 {
                return Err("Widget Alt+Tab exclusion requires a top-level window".into());
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
                original_owner: GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT) as Hwnd,
                owner: Cell::new(ptr::null_mut()),
                owner_cleanup: 0,
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
                .and_then(|_| set_style(hwnd, widget_style(current)))
                .and_then(|_| state.sync());
            if let Err(error) = result {
                let restored = state.restore_configuration();
                state.delete_tab();
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
        use std::cell::RefCell;

        #[test]
        fn widget_bits_preserve_unrelated_window_behaviour() {
            for style in [
                0,
                APPWINDOW,
                TOOLWINDOW,
                0x8,
                0x0800_0000,
                0x8008_0130,
                u32::MAX,
            ] {
                let next = widget_style(style);
                assert_eq!(next & SWITCHER_BITS, 0);
                assert_eq!(next & !SWITCHER_BITS, style & !SWITCHER_BITS);
            }
        }

        #[derive(Default)]
        struct Metrics {
            creates: Cell<usize>,
            adds: Cell<usize>,
            deletes: Cell<usize>,
            releases: Cell<usize>,
            fail_add: Cell<bool>,
        }
        thread_local! {
            static MOCK: RefCell<Option<Rc<Metrics>>> = RefCell::new(None);
        }
        #[repr(C)]
        struct MockTaskbar {
            base: Taskbar,
            metrics: Rc<Metrics>,
            references: Cell<u32>,
        }
        unsafe extern "system" fn mock_query(
            _: *mut Taskbar,
            _: *const Guid,
            _: *mut *mut c_void,
        ) -> Hresult {
            -1
        }
        unsafe extern "system" fn mock_add_ref(taskbar: *mut Taskbar) -> u32 {
            let mock = &*(taskbar as *mut MockTaskbar);
            let references = mock.references.get() + 1;
            mock.references.set(references);
            references
        }
        unsafe extern "system" fn mock_release(taskbar: *mut Taskbar) -> u32 {
            let mock = &*(taskbar as *mut MockTaskbar);
            let references = mock.references.get() - 1;
            mock.references.set(references);
            if references == 0 {
                mock.metrics.releases.set(mock.metrics.releases.get() + 1);
                drop(Box::from_raw(taskbar as *mut MockTaskbar));
            }
            references
        }
        unsafe extern "system" fn mock_init(_: *mut Taskbar) -> Hresult {
            0
        }
        unsafe extern "system" fn mock_add(taskbar: *mut Taskbar, _: Hwnd) -> Hresult {
            let metrics = &(*(taskbar as *mut MockTaskbar)).metrics;
            metrics.adds.set(metrics.adds.get() + 1);
            if metrics.fail_add.get() {
                0x8000_4005u32 as i32
            } else {
                0
            }
        }
        unsafe extern "system" fn mock_delete(taskbar: *mut Taskbar, _: Hwnd) -> Hresult {
            let metrics = &(*(taskbar as *mut MockTaskbar)).metrics;
            metrics.deletes.set(metrics.deletes.get() + 1);
            0
        }
        unsafe extern "system" fn mock_activate(_: *mut Taskbar, _: Hwnd) -> Hresult {
            0
        }
        static MOCK_VTABLE: TaskbarVtable = TaskbarVtable {
            query_interface: mock_query,
            add_ref: mock_add_ref,
            release: mock_release,
            init: mock_init,
            add_tab: mock_add,
            delete_tab: mock_delete,
            activate_tab: mock_activate,
            set_active_alt: mock_activate,
        };
        pub(super) fn mock_taskbar() -> Option<*mut Taskbar> {
            MOCK.with(|slot| slot.borrow().clone()).map(|metrics| {
                metrics.creates.set(metrics.creates.get() + 1);
                Box::into_raw(Box::new(MockTaskbar {
                    base: Taskbar {
                        vtable: &MOCK_VTABLE,
                    },
                    metrics,
                    references: Cell::new(1),
                })) as *mut Taskbar
            })
        }
        fn with_mock(test: impl FnOnce(Rc<Metrics>)) {
            let metrics = Rc::new(Metrics::default());
            MOCK.with(|slot| *slot.borrow_mut() = Some(metrics.clone()));
            struct Reset;
            impl Drop for Reset {
                fn drop(&mut self) {
                    MOCK.with(|slot| *slot.borrow_mut() = None);
                }
            }
            let _reset = Reset;
            test(metrics);
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

        #[test]
        fn owner_styles_geometry_and_native_thread_are_preserved() {
            // All native HWNDs belong to this test. No enumeration or input.
            with_mock(|metrics| unsafe {
                let main = window(APPWINDOW, ptr::null_mut());
                let hwnd = main.0;
                let foreground = GetForegroundWindow();
                let original_rect = rect(hwnd);
                let normal_style = GetWindowLongPtrW(hwnd, GWL_STYLE);
                install(hwnd as isize).unwrap();
                let state = reference(hwnd);
                let owner = state.owner.get();
                assert!(!owner.is_null());
                assert_eq!(GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT), owner as isize);
                assert_eq!(GetAncestor(hwnd, 2), hwnd, "main retains GA_ROOT identity");
                assert_eq!(GetAncestor(owner, 2), owner, "owner is ordinary top-level");
                assert_eq!(GetWindowLongPtrW(owner, GWLP_HWNDPARENT), 0);
                assert_eq!(IsWindowVisible(owner), 0);
                let owner_rect = rect(owner);
                assert_eq!(owner_rect.right - owner_rect.left, 0);
                assert_eq!(owner_rect.bottom - owner_rect.top, 0);
                assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE), normal_style);
                assert_eq!(rect(hwnd), original_rect);
                assert_eq!(
                    GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & (SWITCHER_BITS | 0x0800_0000),
                    0
                );
                assert_eq!(GetForegroundWindow(), foreground);
                install(hwnd as isize).unwrap();
                assert!(
                    Rc::ptr_eq(&state, &reference(hwnd)),
                    "install is idempotent"
                );

                // Tao can replace the entire extended style during pinning.
                SetWindowLongPtrW(
                    hwnd,
                    GWL_EXSTYLE,
                    (APPWINDOW | TOOLWINDOW | 0x0008_0000) as isize,
                );
                let updated = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
                assert_eq!(updated & SWITCHER_BITS, 0);
                assert_ne!(updated & 0x0008_0000, 0);
                assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE), normal_style);
                let handle = hwnd as isize;
                assert!(std::thread::spawn(move || install(handle))
                    .join()
                    .unwrap()
                    .is_err());
                assert_eq!(
                    metrics.adds.get(),
                    0,
                    "hidden window never gets a taskbar tab"
                );

                state.restore_configuration().unwrap();
                assert_eq!(GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT), 0);
                assert_eq!(IsWindow(owner), 0);
                assert_eq!(
                    GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & SWITCHER_BITS,
                    APPWINDOW
                );
                assert_ne!(GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & 0x0008_0000, 0);
                drop(main);
                assert!(state.closed.get());
                drop(state);
                assert_eq!(metrics.creates.get(), metrics.releases.get());
            });
        }

        #[test]
        fn show_hide_minimize_explorer_restart_and_destroy_are_event_driven() {
            with_mock(|metrics| unsafe {
                let main = window(APPWINDOW, ptr::null_mut());
                let hwnd = main.0;
                let foreground = GetForegroundWindow();
                install(hwnd as isize).unwrap();
                let state = reference(hwnd);
                let owner = state.owner.get();
                // The only visible HWND is synthetic, off-screen, owned by the
                // invisible helper, and shown without activation. Mock COM never
                // creates an actual Explorer taskbar button.
                ShowWindow(hwnd, 4);
                pump(hwnd);
                assert_ne!(IsWindowVisible(hwnd), 0);
                assert!(state.registered.get());
                assert_eq!(metrics.adds.get(), 1);
                assert_eq!(
                    GetWindowLongPtrW(hwnd, GWL_STYLE) as u32,
                    0x14cf_0000,
                    "same visible normal style as the installed widgets"
                );
                let bounds = rect(hwnd);
                let point = |x: i32, y: i32| -> isize {
                    ((x as u16 as u32) | ((y as u16 as u32) << 16)) as isize
                };
                assert_eq!(
                    SendMessageW(hwnd, 0x0084, 0, point(bounds.left + 50, bounds.top + 20)),
                    2,
                    "native caption dragging remains available"
                );
                assert_eq!(
                    SendMessageW(hwnd, 0x0084, 0, point(bounds.right - 2, bounds.bottom - 2)),
                    17,
                    "native resize hit testing remains available"
                );
                pump(hwnd);
                assert_eq!(metrics.adds.get(), 1, "no repeated AddTab while stable");
                assert_eq!(IsWindowVisible(owner), 0);
                assert_eq!(GetForegroundWindow(), foreground);
                ShowWindow(hwnd, 7);
                pump(hwnd);
                assert_ne!(IsIconic(hwnd), 0);
                assert_ne!(IsWindowVisible(hwnd), 0);
                assert!(
                    state.registered.get(),
                    "minimized window retains taskbar button"
                );
                assert_eq!(GetForegroundWindow(), foreground);
                ShowWindow(hwnd, 4);
                pump(hwnd);
                assert_eq!(IsIconic(hwnd), 0);
                assert!(state.registered.get());
                ShowWindow(hwnd, 0);
                pump(hwnd);
                assert!(!state.registered.get());
                assert_eq!(metrics.deletes.get(), 1);
                ShowWindow(hwnd, 4);
                pump(hwnd);
                let before = metrics.adds.get();
                SendMessageW(hwnd, state.taskbar_created, 0, 0);
                pump(hwnd);
                assert_eq!(
                    metrics.creates.get(),
                    2,
                    "Explorer restart recreates COM proxy"
                );
                assert_eq!(metrics.releases.get(), 1);
                assert_eq!(metrics.adds.get(), before + 1);
                assert!(state.registered.get());
                ShowWindow(hwnd, 0);
                pump(hwnd);
                SendMessageW(hwnd, state.taskbar_created, 0, 0);
                pump(hwnd);
                assert_eq!(
                    metrics.creates.get(),
                    2,
                    "hidden recovery waits until shown"
                );
                ShowWindow(hwnd, 4);
                pump(hwnd);
                assert_eq!(metrics.creates.get(), 3);
                assert_eq!(GetForegroundWindow(), foreground);
                ShowWindow(hwnd, 0);
                pump(hwnd);
                drop(main);
                assert_eq!(IsWindow(hwnd), 0);
                assert_eq!(
                    IsWindow(owner),
                    0,
                    "NCDESTROY cleans helper without recursion"
                );
                assert!(state.closed.get());
                drop(state);
                assert_eq!(metrics.creates.get(), metrics.releases.get());
            });
        }

        #[test]
        fn taskbar_failure_restores_original_owner_and_cleans_helper() {
            with_mock(|metrics| unsafe {
                // Keep fallback owned by another hidden test window, so even
                // the deliberate failure cannot create a real shell entry.
                let original = window(0, ptr::null_mut());
                let main = window(0, original.0);
                let hwnd = main.0;
                install(hwnd as isize).unwrap();
                let state = reference(hwnd);
                let owner = state.owner.get();
                metrics.fail_add.set(true);
                ShowWindow(hwnd, 4);
                pump(hwnd);
                assert!(!state.enabled.get());
                assert_eq!(
                    GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT),
                    original.0 as isize
                );
                assert_eq!(IsWindow(owner), 0);
                assert_eq!(
                    GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & SWITCHER_BITS,
                    0
                );
                assert_ne!(IsWindow(hwnd), 0, "fallback never destroys main");
                assert_ne!(IsWindow(original.0), 0, "original owner is never destroyed");
                ShowWindow(hwnd, 0);
                pump(hwnd);
                drop(main);
                drop(state);
                drop(original);
                assert_eq!(metrics.creates.get(), metrics.releases.get());
            });
        }
    }
}

#[cfg(windows)]
pub use self::windows::install;

#[cfg(not(windows))]
pub fn install(_handle: isize) -> Result<(), String> {
    Ok(())
}
