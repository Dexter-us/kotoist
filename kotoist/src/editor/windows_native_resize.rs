//! Keep the embedded editor's native client exactly aligned with its host.

use super::windows_geometry::{self, DpiScope};
use baseview::WindowContext;
use std::cell::Cell;
use winapi::{
    shared::windef::{HWND, POINT, RECT},
    um::{
        libloaderapi::{GetModuleHandleW, GetProcAddress},
        winuser::*,
    },
};

thread_local! {
    static PARENT_SYNC: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn is_parent_sync() -> bool {
    PARENT_SYNC.with(Cell::get)
}

struct ParentSync(bool);

impl ParentSync {
    fn enter() -> Self {
        Self(PARENT_SYNC.with(|flag| flag.replace(true)))
    }
}

impl Drop for ParentSync {
    fn drop(&mut self) {
        PARENT_SYNC.with(|flag| flag.set(self.0));
    }
}

fn dimensions(rect: RECT) -> (u32, u32) {
    (
        (rect.right - rect.left) as u32,
        (rect.bottom - rect.top) as u32,
    )
}

pub(crate) fn before_draw(window: &WindowContext) {
    if is_parent_sync() {
        return;
    }
    let cached = window.size().physical;
    if let Ok(hwnd) = windows_geometry::hwnd(window) {
        let _ = synchronize(hwnd, (cached.width, cached.height));
    }
}

fn synchronize(hwnd: HWND, cached: (u32, u32)) -> Result<(), String> {
    let _dpi = DpiScope::for_window(hwnd);
    let parent = windows_geometry::parent(hwnd);
    if parent.is_null() {
        return Ok(());
    }
    let target = windows_geometry::client_rect(parent)?;
    let desired = dimensions(target);
    if desired.0 == 0 || desired.1 == 0 || desired.0 > 8192 || desired.1 > 8192 {
        return Ok(());
    }
    let actual = windows_geometry::client_rect(hwnd)?;
    let mut origin = POINT { x: 0, y: 0 };
    if unsafe { ClientToScreen(hwnd, &mut origin) } == 0
        || unsafe { ScreenToClient(parent, &mut origin) } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }

    let _sync = ParentSync::enter();
    if dimensions(actual) != desired || origin.x != 0 || origin.y != 0 {
        let mut outer = target;
        let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
        let exstyle = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        let module =
            unsafe { GetModuleHandleW("user32.dll\0".encode_utf16().collect::<Vec<_>>().as_ptr()) };
        let adjust =
            unsafe { GetProcAddress(module, b"AdjustWindowRectExForDpi\0".as_ptr().cast()) };
        let ok = if adjust.is_null() {
            unsafe { AdjustWindowRectEx(&mut outer, style, 0, exstyle) }
        } else {
            let adjust: unsafe extern "system" fn(*mut RECT, u32, i32, u32, u32) -> i32 =
                unsafe { std::mem::transmute(adjust) };
            unsafe {
                adjust(
                    &mut outer,
                    style,
                    0,
                    exstyle,
                    (windows_geometry::dpi(hwnd) * 96.0).round() as u32,
                )
            }
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                outer.left,
                outer.top,
                outer.right - outer.left,
                outer.bottom - outer.top,
                SWP_NOACTIVATE | SWP_NOZORDER,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }

    let actual = dimensions(windows_geometry::client_rect(hwnd)?);
    if actual != desired {
        return Err(format!(
            "Editor client is {}x{} but host client is {}x{}",
            actual.0, actual.1, desired.0, desired.1
        ));
    }
    if actual != cached {
        unsafe {
            SendMessageW(
                hwnd,
                WM_SIZE,
                SIZE_RESTORED,
                (actual.0 | actual.1 << 16) as isize,
            );
        }
    }
    Ok(())
}
