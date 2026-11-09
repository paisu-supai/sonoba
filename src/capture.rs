//! Reads a rectangle of the screen, whichever window happens to be showing there.

use std::cell::{Cell, RefCell};
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
    window_dc: HDC, // what PrintWindow draws into
    window_old: Cell<HGDIOBJ>,
    // One kept bitmap per window read: a new bitmap each time costs the window too.
    window_bitmaps: RefCell<Vec<(HWND, HBITMAP, (i32, i32))>>,
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
            let (window_old, window_bitmaps) = (Cell::new(null_mut()), RefCell::new(Vec::new()));
            Some(Self { screen, mem, dib, old, bits: bits as *const u8, w, h, window_dc, window_old, window_bitmaps })
        }
    }

    /// BGRA pixels of the rectangle at (x, y), top-down and tightly packed.
    pub fn grab(&self, x: i32, y: i32) -> &[u8] {
        unsafe {
            // CAPTUREBLT also picks up layered windows such as subtitle overlays.
            BitBlt(self.mem, 0, 0, self.w, self.h, self.screen, x, y, SRCCOPY | CAPTUREBLT);
            GdiFlush();
        }
        self.pixels()
    }

    /// What the last grab or reads left, in the same form.
    pub fn pixels(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.bits, (self.w * self.h * 4) as usize) }
    }

    /// The screen rectangle (x, y, w, h) as the window itself draws it, whatever lies on top, put
    /// at (to_x, to_y) in the picture: the overlay can then stay visible to screen sharing. False
    /// where the window cannot be read so.
    pub fn read_window(&self, window: HWND, (x, y, w, h): (i32, i32, i32, i32), (to_x, to_y): (i32, i32)) -> bool {
        const PW_RENDERFULLCONTENT: u32 = 2; // what DWM shows, GPU-drawn content included
        unsafe {
            let mut r: RECT = std::mem::zeroed();
            GetWindowRect(window, &mut r);
            if x < r.left || y < r.top || x + w > r.right || y + h > r.bottom {
                return false;
            }
            // A window that Windows scales up for its monitor draws itself at its own, smaller size.
            let (mut dpi, mut unused) = (0, 0);
            GetDpiForMonitor(MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST), MDT_EFFECTIVE_DPI, &mut dpi, &mut unused);
            if GetDpiForWindow(window) != dpi {
                return false;
            }
            // It draws from the window's corner and stops at the bitmap's edge, so only that far is read.
            let (dx, dy) = (x - r.left, y - r.top);
            let size = (dx + w, dy + h);
            let mut bitmaps = self.window_bitmaps.borrow_mut();
            let index = match bitmaps.iter().position(|(owner, _, _)| *owner == window) {
                Some(index) => index,
                None => {
                    bitmaps.push((window, null_mut(), (0, 0)));
                    bitmaps.len() - 1
                }
            };
            if bitmaps[index].2 != size {
                let bitmap = CreateCompatibleBitmap(self.screen, size.0, size.1);
                if bitmap.is_null() {
                    return false;
                }
                let old = SelectObject(self.window_dc, bitmap);
                if self.window_old.get().is_null() {
                    self.window_old.set(old);
                }
                if !bitmaps[index].1.is_null() {
                    DeleteObject(bitmaps[index].1);
                }
                bitmaps[index] = (window, bitmap, size);
            }
            SelectObject(self.window_dc, bitmaps[index].1);
            let drawn = PrintWindow(window, self.window_dc, PW_RENDERFULLCONTENT) != 0;
            if drawn {
                BitBlt(self.mem, to_x, to_y, w, h, self.window_dc, dx, dy, SRCCOPY);
                GdiFlush();
            }
            drawn
        }
    }
}

impl Drop for Grabber {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.mem, self.old);
            DeleteObject(self.dib);
            DeleteDC(self.mem);
            if !self.window_old.get().is_null() {
                SelectObject(self.window_dc, self.window_old.get());
            }
            for (_, bitmap, _) in self.window_bitmaps.borrow().iter() {
                DeleteObject(*bitmap);
            }
            DeleteDC(self.window_dc);
            ReleaseDC(null_mut(), self.screen);
        }
    }
}
