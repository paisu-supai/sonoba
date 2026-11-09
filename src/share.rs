//! A window for sharing one window in Discord, Meet or OBS: the translated window as DWM draws it,
//! with the overlay on top, both as thumbnails, so it costs next to nothing. The overlay is a
//! window of its own, which a share of the original window leaves out. This one sits off every
//! screen, out of the way, and stays after a job so that a share does not end with it.

use crate::i18n::t;
use std::cell::Cell;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::w;

const TIMER: usize = 1;
const TIMER_MS: u32 = 50; // the windows move and change size without telling this one
const TITLE: &str = "Sonoba 共有用";

thread_local! {
    static WINDOW: Cell<HWND> = const { Cell::new(null_mut()) };
    static TARGET: Cell<HWND> = const { Cell::new(null_mut()) };
    static OVERLAY: Cell<HWND> = const { Cell::new(null_mut()) };
    static THUMBS: Cell<(isize, isize)> = const { Cell::new((0, 0)) };
    static PLACED: Cell<[i32; 11]> = const { Cell::new([0; 11]) };
}

/// The share window shows `target` from now on, made the first time it is needed.
pub fn follow(target: HWND, overlay: HWND) {
    unsafe {
        let mut hwnd = WINDOW.get();
        if hwnd.is_null() {
            hwnd = CreateWindowExW(WS_EX_APPWINDOW, w!("SonobaShare"), null(), WS_POPUP, 0, 0, 1, 1, null_mut(), null_mut(), GetModuleHandleW(null()), null());
            WINDOW.set(hwnd);
            SetTimer(hwnd, TIMER, TIMER_MS, None);
        }
        let title: Vec<u16> = t(TITLE).encode_utf16().chain([0]).collect();
        SetWindowTextW(hwnd, title.as_ptr());
        if TARGET.get() == target && OVERLAY.get() == overlay {
            return;
        }
        unregister();
        // The later thumbnail is drawn on top.
        let (mut below, mut above) = (0, 0);
        DwmRegisterThumbnail(hwnd, target, &mut below);
        DwmRegisterThumbnail(hwnd, overlay, &mut above);
        THUMBS.set((below, above));
        TARGET.set(target);
        OVERLAY.set(overlay);
        PLACED.set([0; 11]);
        place();
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
}

/// The setting went off.
pub fn close() {
    let hwnd = WINDOW.get();
    if !hwnd.is_null() {
        unsafe { DestroyWindow(hwnd) };
    }
}

fn unregister() {
    let (below, above) = THUMBS.replace((0, 0));
    for thumb in [below, above].into_iter().filter(|&thumb| thumb != 0) {
        unsafe { DwmUnregisterThumbnail(thumb) };
    }
    TARGET.set(null_mut());
}

/// Size and thumbnails after the windows, and the position off every screen, which can change.
unsafe fn place() {
    unsafe {
        let (hwnd, target, overlay) = (WINDOW.get(), TARGET.get(), OVERLAY.get());
        let Some(bounds) = crate::frame_bounds(target) else { return }; // gone, or for now not there
        let mut whole: RECT = std::mem::zeroed();
        GetWindowRect(target, &mut whole);
        let mut over: RECT = std::mem::zeroed();
        GetWindowRect(overlay, &mut over);
        let (w, h) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN) + GetSystemMetrics(SM_CXVIRTUALSCREEN) + 100;
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let shown = IsWindowVisible(overlay);
        let (sx, sy, ox, oy) = (bounds.left - whole.left, bounds.top - whole.top, over.left - bounds.left, over.top - bounds.top);
        let now = [x, y, w, h, sx, sy, ox, oy, over.right - over.left, over.bottom - over.top, shown];
        if PLACED.replace(now) == now {
            return;
        }
        SetWindowPos(hwnd, null_mut(), x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
        let (below, above) = THUMBS.get();
        // The window without its invisible resize border, as a share of the window itself shows it.
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_RECTSOURCE | DWM_TNP_VISIBLE | DWM_TNP_OPACITY,
            rcDestination: RECT { left: 0, top: 0, right: w, bottom: h },
            rcSource: RECT { left: sx, top: sy, right: sx + w, bottom: sy + h },
            opacity: 255,
            fVisible: 1,
            fSourceClientAreaOnly: 0,
        };
        DwmUpdateThumbnailProperties(below, &props);
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY,
            rcDestination: RECT { left: ox, top: oy, right: ox + over.right - over.left, bottom: oy + over.bottom - over.top },
            fVisible: shown,
            ..props
        };
        DwmUpdateThumbnailProperties(above, &props);
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER => {
                place();
                0
            }
            // A minimised window has nothing for the share to show.
            WM_SYSCOMMAND if w & 0xFFF0 == SC_MINIMIZE as usize => 0,
            WM_DESTROY => {
                unregister();
                WINDOW.set(null_mut());
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

pub fn register_class() {
    unsafe {
        let mut class: WNDCLASSEXW = std::mem::zeroed();
        class.cbSize = size_of::<WNDCLASSEXW>() as u32;
        class.lpfnWndProc = Some(proc);
        class.hInstance = GetModuleHandleW(null());
        class.lpszClassName = w!("SonobaShare");
        class.hbrBackground = GetStockObject(BLACK_BRUSH) as HBRUSH;
        class.hIcon = crate::app_icon(Some(48));
        class.hIconSm = crate::app_icon(None);
        RegisterClassExW(&class);
    }
}
