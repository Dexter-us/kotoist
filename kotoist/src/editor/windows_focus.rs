//! Give the embedded editor keyboard focus and preserve click coordinates.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winapi::shared::{
    basetsd::{DWORD_PTR, UINT_PTR},
    minwindef::{LPARAM, LRESULT, UINT, WPARAM},
    windef::HWND,
};
use winapi::um::{
    commctrl::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
    winuser::{
        SetFocus, DLGC_WANTALLKEYS, DLGC_WANTARROWS, DLGC_WANTCHARS, DLGC_WANTTAB, WM_GETDLGCODE,
        WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_NCDESTROY, WM_RBUTTONDOWN, WM_XBUTTONDOWN,
    },
};

const SUBCLASS_ID: UINT_PTR = 0x4B4F544F;

pub(crate) fn install(handle: &impl HasWindowHandle) -> Result<(), String> {
    let hwnd = match handle
        .window_handle()
        .map_err(|error| error.to_string())?
        .as_raw()
    {
        RawWindowHandle::Win32(handle) => handle.hwnd.get() as HWND,
        _ => return Err("The editor did not return a Windows window handle.".into()),
    };
    if unsafe { SetWindowSubclass(hwnd, Some(focus_proc), SUBCLASS_ID, 0) } == 0 {
        return Err(format!(
            "Could not install the editor focus handler: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

unsafe extern "system" fn focus_proc(
    hwnd: HWND,
    message: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: UINT_PTR,
    _reference_data: DWORD_PTR,
) -> LRESULT {
    match message {
        WM_LBUTTONDOWN | WM_MBUTTONDOWN | WM_RBUTTONDOWN | WM_XBUTTONDOWN => {
            SetFocus(hwnd);
            // baseview button events use its last pointer position; forward the
            // click's actual client coordinates first to avoid stale hit tests.
            let _ = DefSubclassProc(hwnd, WM_MOUSEMOVE, wparam, lparam);
        }
        WM_GETDLGCODE => {
            return DefSubclassProc(hwnd, message, wparam, lparam)
                | (DLGC_WANTALLKEYS | DLGC_WANTARROWS | DLGC_WANTCHARS | DLGC_WANTTAB);
        }
        WM_NCDESTROY => {
            RemoveWindowSubclass(hwnd, Some(focus_proc), SUBCLASS_ID);
        }
        _ => {}
    }
    DefSubclassProc(hwnd, message, wparam, lparam)
}
