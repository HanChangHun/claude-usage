// Diagnostic only. The undocumented read-only shell query is never shipped.
// Interface layout reference: MScholtes/VirtualDesktop, VirtualDesktop11.cs.
#[path = "../app/src-tauri/src/alt_tab.rs"]
mod current;
use std::{
    ffi::c_void,
    ptr,
    time::{Duration, Instant},
};
type Hwnd = *mut c_void;
type Com = *mut *const usize;
#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);
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
        ex: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: Hwnd,
        menu: Hwnd,
        instance: Hwnd,
        param: Hwnd,
    ) -> Hwnd;
    fn ShowWindow(hwnd: Hwnd, how: i32) -> i32;
    fn DestroyWindow(hwnd: Hwnd) -> i32;
    fn PeekMessageW(message: *mut Message, hwnd: Hwnd, first: u32, last: u32, remove: u32) -> i32;
    fn DispatchMessageW(message: *const Message) -> isize;
    fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
    fn SetWindowLongPtrW(hwnd: Hwnd, index: i32, value: isize) -> isize;
    fn IsWindowVisible(hwnd: Hwnd) -> i32;
    fn RegisterWindowMessageW(name: *const u16) -> u32;
    fn SendMessageW(hwnd: Hwnd, message: u32, wparam: usize, lparam: isize) -> isize;
    fn SetActiveWindow(hwnd: Hwnd) -> Hwnd;
    fn GetForegroundWindow() -> Hwnd;
    fn GetAncestor(hwnd: Hwnd, flags: u32) -> Hwnd;
}
#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(reserved: Hwnd, flags: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(
        clsid: *const Guid,
        outer: Hwnd,
        context: u32,
        iid: *const Guid,
        object: *mut Com,
    ) -> i32;
}
const SHELL: Guid = Guid(
    0xc2f03a33,
    0x21f5,
    0x47fa,
    [0xb4, 0xbb, 0x15, 0x63, 0x62, 0xa2, 0xf2, 0x39],
);
const SERVICE: Guid = Guid(
    0x6d5140c1,
    0x7436,
    0x11ce,
    [0x80, 0x34, 0, 0xaa, 0, 0x60, 0x09, 0xfa],
);
const VIEWS: Guid = Guid(
    0x1841c6d7,
    0x4f9d,
    0x42c0,
    [0xaf, 0x41, 0x87, 0x47, 0x53, 0x8f, 0x10, 0xe5],
);
unsafe fn release(p: Com) {
    if !p.is_null() {
        let f: unsafe extern "system" fn(Com) -> u32 = std::mem::transmute(*(*p).add(2));
        f(p);
    }
}
unsafe fn pump() {
    let until = Instant::now() + Duration::from_millis(1000);
    while Instant::now() < until {
        let mut m = std::mem::MaybeUninit::uninit();
        while PeekMessageW(m.as_mut_ptr(), ptr::null_mut(), 0, 0, 1) != 0 {
            DispatchMessageW(m.as_ptr());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
unsafe fn window(title: &str) -> Hwnd {
    let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
    let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    let h = CreateWindowExW(
        0x40000,
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
    assert!(!h.is_null());
    h
}
unsafe fn switcher(collection: Com, h: Hwnd) -> Result<Option<i32>, String> {
    let get: unsafe extern "system" fn(Com, Hwnd, *mut Com) -> i32 =
        std::mem::transmute(*(*collection).add(6));
    let mut view = ptr::null_mut();
    let result = get(collection, h, &mut view);
    if result < 0 {
        return Ok(None);
    }
    if view.is_null() {
        return Err("Shell returned an empty view".into());
    }
    let query: unsafe extern "system" fn(Com, *mut i32) -> i32 =
        std::mem::transmute(*(*view).add(27));
    let mut flag = -1;
    let hr = query(view, &mut flag);
    release(view);
    if hr < 0 {
        Err(format!("GetShowInSwitchers failed {hr:x}"))
    } else {
        Ok(Some(flag))
    }
}
unsafe fn sample(collection: Com, label: &str, h: Hwnd) -> Option<i32> {
    pump();
    let flag = switcher(collection, h).unwrap();
    let owner = GetWindowLongPtrW(h, -8) as Hwnd;
    let ex = GetWindowLongPtrW(h, -20) as u32;
    println!(
        "{label}: switcher={flag:?}, ex=0x{ex:X}, owner_visible={}, root={}",
        IsWindowVisible(owner),
        GetAncestor(h, 2) == h
    );
    flag
}
fn main() {
    unsafe {
        assert!(CoInitializeEx(ptr::null_mut(), 2) >= 0);
        let mut shell = ptr::null_mut();
        assert!(
            CoCreateInstance(&SHELL, ptr::null_mut(), 4, &SERVICE, &mut shell) >= 0,
            "Shell connection"
        );
        let query: unsafe extern "system" fn(Com, *const Guid, *const Guid, *mut Com) -> i32 =
            std::mem::transmute(*(*shell).add(3));
        let mut collection = ptr::null_mut();
        assert!(
            query(shell, &VIEWS, &VIEWS, &mut collection) >= 0,
            "Shell views"
        );
        let normal = window("Widget QA normal control");
        let new = window("Widget QA tray policy");
        current::install(new as isize).unwrap();
        for h in [normal, new] {
            ShowWindow(h, 4);
        }
        let control = sample(collection, "normal", normal);
        let first = sample(collection, "tray initial", new);
        let mut observations = vec![first];
        let foreground = GetForegroundWindow();
        SetActiveWindow(new);
        observations.push(sample(collection, "tray thread activation", new));
        assert_eq!(
            GetForegroundWindow(),
            foreground,
            "Do not take foreground from the user"
        );
        ShowWindow(new, 0);
        pump();
        ShowWindow(new, 4);
        observations.push(sample(collection, "tray restored", new));
        SetWindowLongPtrW(new, -20, 0x40180);
        observations.push(sample(collection, "tray style rewrite", new));
        ShowWindow(new, 7);
        pump();
        ShowWindow(new, 4);
        observations.push(sample(collection, "tray unminimized", new));
        let name: Vec<u16> = "TaskbarCreated\0".encode_utf16().collect();
        let event = RegisterWindowMessageW(name.as_ptr());
        SendMessageW(new, event, 0, 0);
        observations.push(sample(collection, "tray Explorer recreated", new));
        ShowWindow(new, 0);
        SendMessageW(new, event, 0, 0);
        pump();
        ShowWindow(new, 4);
        observations.push(sample(collection, "tray hidden Explorer recreated", new));
        for h in [normal, new] {
            DestroyWindow(h);
        }
        pump();
        release(collection);
        release(shell);
        CoUninitialize();
        assert_eq!(
            control,
            Some(1),
            "Positive control must be shown by the actual shell"
        );
        assert!(
            observations.iter().all(|x| matches!(x, None | Some(0))),
            "Tray window must be excluded by actual shell"
        );
        println!("PASS: positive control included; tray policy excluded by actual Windows shell in all seven states.");
    }
}
