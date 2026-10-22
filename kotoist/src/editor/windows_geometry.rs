//! Measure Windows client geometry without changing the host process DPI mode.

use baseview::WindowContext;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winapi::shared::windef::{HWND, RECT};
use winapi::um::{
    libloaderapi::{GetModuleHandleW, GetProcAddress},
    winuser::{GetClientRect, GetParent},
};

type SetDpiContext = unsafe extern "system" fn(isize) -> isize;

pub(crate) struct DpiScope(Option<(SetDpiContext, isize)>);

impl DpiScope {
    pub(crate) fn for_window(hwnd: HWND) -> Self {
        unsafe {
            let module =
                GetModuleHandleW("user32.dll\0".encode_utf16().collect::<Vec<_>>().as_ptr());
            let get = GetProcAddress(module, b"GetWindowDpiAwarenessContext\0".as_ptr().cast());
            let set = GetProcAddress(module, b"SetThreadDpiAwarenessContext\0".as_ptr().cast());
            if get.is_null() || set.is_null() {
                return Self(None);
            }
            let get: unsafe extern "system" fn(HWND) -> isize = std::mem::transmute(get);
            let set: SetDpiContext = std::mem::transmute(set);
            let context = get(hwnd);
            if context == 0 {
                return Self(None);
            }
            let old = set(context);
            Self(if old == 0 { None } else { Some((set, old)) })
        }
    }
}

impl Drop for DpiScope {
    fn drop(&mut self) {
        if let Some((set, old)) = self.0 {
            unsafe {
                set(old);
            }
        }
    }
}

pub(crate) fn hwnd(window: &WindowContext) -> Result<HWND, String> {
    match window
        .window_handle()
        .map_err(|error| error.to_string())?
        .as_raw()
    {
        RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get() as HWND),
        _ => Err("A native Windows editor window is required.".into()),
    }
}

pub(crate) fn client_rect(hwnd: HWND) -> Result<RECT, String> {
    let mut rect: RECT = unsafe { std::mem::zeroed() };
    if hwnd.is_null() || unsafe { GetClientRect(hwnd, &mut rect) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(rect)
}

pub(crate) fn parent(hwnd: HWND) -> HWND {
    unsafe { GetParent(hwnd) }
}

pub(crate) fn dpi(hwnd: HWND) -> f32 {
    unsafe {
        let module = GetModuleHandleW("user32.dll\0".encode_utf16().collect::<Vec<_>>().as_ptr());
        if module.is_null() {
            return 1.0;
        }
        let proc = GetProcAddress(module, b"GetDpiForWindow\0".as_ptr().cast());
        if proc.is_null() {
            return 1.0;
        }
        let get: unsafe extern "system" fn(HWND) -> u32 = std::mem::transmute(proc);
        let value = get(hwnd);
        if value == 0 {
            1.0
        } else {
            value as f32 / 96.0
        }
    }
}
