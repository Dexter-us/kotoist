//! Give the embedded editor keyboard focus and preserve click coordinates.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winapi::shared::{
    basetsd::{DWORD_PTR, UINT_PTR},
    minwindef::{LPARAM, LRESULT, UINT, WPARAM},
    windef::{HWND, RECT},
};
use winapi::um::{
    commctrl::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
    winuser::{
        GetClientRect, GetUpdateRect, GetWindowRect, IsWindow, SetFocus, ValidateRect, DLGC_WANTALLKEYS, DLGC_WANTARROWS, DLGC_WANTCHARS, DLGC_WANTTAB, WM_GETDLGCODE,
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_NCDESTROY,
        WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
    },
};

const SUBCLASS_ID: UINT_PTR = 0x4B4F544F;

unsafe fn paint_bounds(hwnd: HWND) -> Option<[i32; 6]> {
    let mut outer: RECT = std::mem::zeroed();
    let mut client: RECT = std::mem::zeroed();
    if GetWindowRect(hwnd, &mut outer) == 0 || GetClientRect(hwnd, &mut client) == 0 {
        return None;
    }
    Some([
        outer.left,
        outer.top,
        outer.right,
        outer.bottom,
        client.right - client.left,
        client.bottom - client.top,
    ])
}

pub(crate) fn install(handle: &impl HasWindowHandle) -> Result<(), String> {
    let hwnd = match handle
        .window_handle()
        .map_err(|error| error.to_string())?
        .as_raw()
    {
        RawWindowHandle::Win32(handle) => handle.hwnd.get() as HWND,
        _ => return Err("The editor did not return a Windows window handle.".into()),
    };
    install_hwnd(hwnd)
}

fn install_hwnd(hwnd: HWND) -> Result<(), String> {
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
        WM_PAINT => {
            // egui renders the full child window. Some hosts leave a stale
            // partial update region after the GL paint, which can generate a
            // paint storm and starve mouse and keyboard messages.
            let mut update = std::mem::zeroed();
            let had_damage = GetUpdateRect(hwnd, &mut update, 0) != 0;
            let before = paint_bounds(hwnd);
            let result = DefSubclassProc(hwnd, message, wparam, lparam);
            if had_damage
                && IsWindow(hwnd) != 0
                && before.is_some()
                && before == paint_bounds(hwnd)
            {
                ValidateRect(hwnd, std::ptr::null());
            }
            return result;
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_RBUTTONDOWN
        | WM_RBUTTONUP | WM_XBUTTONDOWN | WM_XBUTTONUP => {
            if matches!(
                message,
                WM_LBUTTONDOWN | WM_MBUTTONDOWN | WM_RBUTTONDOWN | WM_XBUTTONDOWN
            ) {
                // Focus before forwarding the click. Some plugin hosts restore
                // parent focus while processing the child window's default
                // mouse handler, so setting focus afterward is too late.
                SetFocus(hwnd);
            }

            // baseview button events reuse its last cursor position. Refresh it
            // with the click's real client coordinates before sending the
            // button event so the widget hit-test and click land together.
            DefSubclassProc(hwnd, WM_MOUSEMOVE, wparam, lparam);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::null_mut;
    use winapi::um::winuser::{
        CreateWindowExW, DestroyWindow, GetFocus, SendMessageW, WS_CHILD, WS_OVERLAPPEDWINDOW,
    };

    #[test]
    fn first_click_moves_keyboard_focus_to_the_editor_child() {
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let title: Vec<u16> = "Kotoist focus test\0".encode_utf16().collect();
        unsafe {
            let parent = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                120,
                100,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            );
            assert!(!parent.is_null(), "Could not create test parent");

            let child = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_CHILD,
                0,
                0,
                100,
                80,
                parent,
                null_mut(),
                null_mut(),
                null_mut(),
            );
            assert!(!child.is_null(), "Could not create test child");

            install_hwnd(child).unwrap();
            SetFocus(parent);
            SendMessageW(child, WM_LBUTTONDOWN, 0, 0);
            assert_eq!(GetFocus(), child);

            DestroyWindow(parent);
        }
    }
    unsafe extern "system" fn partial_paint(
        hwnd: HWND,
        message: UINT,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: UINT_PTR,
        _data: DWORD_PTR,
    ) -> LRESULT {
        if message == WM_PAINT {
            let mut client: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut client);
            client.right = client.right.min(8);
            ValidateRect(hwnd, &client);
            return 0;
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }

    #[test]
    fn full_frame_paint_clears_stale_partial_damage() {
        use winapi::um::winuser::{
            CreateWindowExW, GetUpdateRect, InvalidateRect, SendMessageW, WS_CHILD,
            WS_OVERLAPPEDWINDOW, WS_VISIBLE,
        };
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        unsafe {
            let parent = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                0,
                0,
                160,
                120,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            assert!(!parent.is_null());
            let child = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                WS_CHILD | WS_VISIBLE,
                0,
                0,
                120,
                80,
                parent,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            assert!(!child.is_null());
            assert_ne!(SetWindowSubclass(child, Some(partial_paint), SUBCLASS_ID + 1, 0), 0);
            install_hwnd(child).unwrap();
            InvalidateRect(child, std::ptr::null(), 0);
            let mut update: RECT = std::mem::zeroed();
            assert_ne!(GetUpdateRect(child, &mut update, 0), 0);
            SendMessageW(child, WM_PAINT, 0, 0);
            assert_eq!(GetUpdateRect(child, &mut update, 0), 0);
            DestroyWindow(parent);
        }
    }

}
