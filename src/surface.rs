use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

use crate::canvas::Canvas;

//Superficie DIB para ventanas con transparencia por píxel

pub struct Surface {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    w: i32,
    h: i32,
}

impl Surface {
    fn create(w: i32, h: i32) -> Result<Self, String> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return Err("CreateCompatibleDC falló".into());
            }
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bmp = match CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(b) if !bits.is_null() => b,
                _ => {
                    let _ = DeleteDC(dc);
                    return Err(format!("No se pudo crear un bitmap de {w}x{h}"));
                }
            };
            let old = SelectObject(dc, bmp.into());
            Ok(Self { dc, bmp, old, bits: bits.cast(), w, h })
        }
    }

    pub fn present(slot: &mut Option<Surface>, hwnd: HWND, cv: &Canvas, x: i32, y: i32, alpha: u8) -> Result<(), String> {
        let needs_new = slot.as_ref().map(|s| s.w != cv.w || s.h != cv.h).unwrap_or(true);
        if needs_new {
            *slot = None;
            *slot = Some(Self::create(cv.w, cv.h)?);
        }
        let s = slot.as_ref().unwrap();
        unsafe {
            std::ptr::copy_nonoverlapping(cv.px.as_ptr(), s.bits, cv.px.len());
        }
        s.update(hwnd, x, y, alpha)
    }

    pub fn update(&self, hwnd: HWND, x: i32, y: i32, alpha: u8) -> Result<(), String> {
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: alpha, AlphaFormat: AC_SRC_ALPHA as u8 };
        let pos = POINT { x, y };
        let size = SIZE { cx: self.w, cy: self.h };
        let src = POINT { x: 0, y: 0 };
        unsafe {
            UpdateLayeredWindow(hwnd, None, Some(&pos), Some(&size), Some(self.dc), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA)
                .map_err(|e| format!("UpdateLayeredWindow: {e}"))
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bmp.into());
            let _ = DeleteDC(self.dc);
        }
    }
}
