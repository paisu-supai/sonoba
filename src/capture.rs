//! Reads a rectangle of the screen, whichever window happens to be showing there.

use std::cell::Cell;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Storage::Xps::PrintWindow;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;

pub struct Grabber {
    screen: HDC,
    mem: HDC,
    dib: HBITMAP,
    old: HGDIOBJ,
    bits: *const u8,
    w: i32,
    h: i32,
    window_dc: HDC, // what PrintWindow draws into, kept: a new bitmap each time costs the window too
    window_bitmap: Cell<HBITMAP>,
    window_old: Cell<HGDIOBJ>,
    window_size: Cell<(i32, i32)>,
}

impl Grabber {
    pub fn new(w: i32, h: i32) -> Option<Self> {
        unsafe {
            let screen = GetDC(null_mut());
            let mem = CreateCompatibleDC(screen);
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w;
            info.bmiHeader.biHeight = -h; // negative means top-down rows
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            let mut bits = null_mut();
            let dib = CreateDIBSection(mem, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if dib.is_null() || bits.is_null() {
                DeleteDC(mem);
                ReleaseDC(null_mut(), screen);
                return None;
            }
            let old = SelectObject(mem, dib);
            let window_dc = CreateCompatibleDC(screen);
            let (window_bitmap, window_old, window_size) = (Cell::new(null_mut()), Cell::new(null_mut()), Cell::new((0, 0)));
            Some(Self { screen, mem, dib, old, bits: bits as *const u8, w, h, window_dc, window_bitmap, window_old, window_size })
        }
    }

    /// BGRA pixels of the rectangle at (x, y), top-down and tightly packed.
    pub fn grab(&self, x: i32, y: i32) -> &[u8] {
        unsafe {
            // CAPTUREBLT also picks up layered windows such as subtitle overlays.
            BitBlt(self.mem, 0, 0, self.w, self.h, self.screen, x, y, SRCCOPY | CAPTUREBLT);
            GdiFlush();
            std::slice::from_raw_parts(self.bits, (self.w * self.h * 4) as usize)
        }
    }

    /// The same rectangle as the window itself draws it, whatever lies on top: the overlay can then
    /// stay visible to screen sharing. None where the window cannot be read so.
    pub fn grab_window(&self, window: HWND, x: i32, y: i32) -> Option<&[u8]> {
        const PW_RENDERFULLCONTENT: u32 = 2; // what DWM shows, GPU-drawn content included
        unsafe {
            let mut r: RECT = std::mem::zeroed();
            GetWindowRect(window, &mut r);
            if x < r.left || y < r.top || x + self.w > r.right || y + self.h > r.bottom {
                return None;
            }
            // A window that Windows scales up for its monitor draws itself at its own, smaller size.
            let (mut dpi, mut unused) = (0, 0);
            GetDpiForMonitor(MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST), MDT_EFFECTIVE_DPI, &mut dpi, &mut unused);
            if GetDpiForWindow(window) != dpi {
                return None;
            }
            // It draws from the window's corner and stops at the bitmap's edge, so only that far is read.
            let (dx, dy) = (x - r.left, y - r.top);
            let size = (dx + self.w, dy + self.h);
            if self.window_size.get() != size {
                let bitmap = CreateCompatibleBitmap(self.screen, size.0, size.1);
                if bitmap.is_null() {
                    return None;
                }
                let old = SelectObject(self.window_dc, bitmap);
                let previous = self.window_bitmap.replace(bitmap);
                if previous.is_null() {
                    self.window_old.set(old);
                } else {
                    DeleteObject(previous);
                }
                self.window_size.set(size);
            }
            let drawn = PrintWindow(window, self.window_dc, PW_RENDERFULLCONTENT) != 0;
            if drawn {
                BitBlt(self.mem, 0, 0, self.w, self.h, self.window_dc, dx, dy, SRCCOPY);
                GdiFlush();
            }
            drawn.then(|| std::slice::from_raw_parts(self.bits, (self.w * self.h * 4) as usize))
        }
    }
}

impl Drop for Grabber {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.mem, self.old);
            DeleteObject(self.dib);
            DeleteDC(self.mem);
            if !self.window_bitmap.get().is_null() {
                SelectObject(self.window_dc, self.window_old.get());
                DeleteObject(self.window_bitmap.get());
            }
            DeleteDC(self.window_dc);
            ReleaseDC(null_mut(), self.screen);
        }
    }
}
