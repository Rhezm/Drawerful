use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS};
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, GetSystemMetrics, HICON, ICONINFO, SM_CXSMICON};

use crate::canvas::{Canvas, Rgba};
use crate::winutil::WM_APP_TRAY;

//Icono de la bandeja del sistema

pub struct Tray {
    hwnd: HWND,
    icon: HICON,
    added: bool,
}

fn fill(dst: &mut [u16], s: &str) {
    let n = dst.len() - 1;
    let mut i = 0;
    for c in s.encode_utf16().take(n) {
        dst[i] = c;
        i += 1;
    }
    dst[i] = 0;
}

impl Tray {
    pub fn new(hwnd: HWND) -> Self {
        let icon = make_icon().unwrap_or_default();
        Self { hwnd, icon, added: false }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: self.hwnd, uID: 1, ..Default::default() }
    }

    pub fn add(&mut self) {
        let mut d = self.data();
        d.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        d.uCallbackMessage = WM_APP_TRAY;
        d.hIcon = self.icon;
        fill(&mut d.szTip, "Cuadros");
        self.added = unsafe { Shell_NotifyIconW(NIM_ADD, &d).as_bool() };
        if !self.added {
            crate::winutil::log("No se pudo agregar el icono a la bandeja");
        }
    }

    pub fn set_tip(&self, tip: &str) {
        let mut d = self.data();
        d.uFlags = NIF_TIP;
        fill(&mut d.szTip, tip);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
        }
    }

    pub fn balloon(&self, title: &str, text: &str, warning: bool) {
        if !self.added {
            return;
        }
        let mut d = self.data();
        d.uFlags = NIF_INFO;
        d.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO };
        fill(&mut d.szInfoTitle, title);
        fill(&mut d.szInfo, text);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
        }
    }

    pub fn remove(&mut self) {
        if self.added {
            let d = self.data();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &d);
            }
            self.added = false;
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
        if !self.icon.is_invalid() {
            unsafe {
                let _ = DestroyIcon(self.icon);
            }
        }
    }
}

fn icon_canvas(size: i32) -> Canvas {
    let s = size as f32 / 16.0;
    let mut cv = Canvas::new(size, size);
    let white = Rgba::new(1.0, 1.0, 1.0, 1.0);
    cv.fill_rrect(0.5 * s, 0.5 * s, 15.0 * s, 15.0 * s, 3.5 * s, Rgba::new(0.16, 0.18, 0.24, 0.92));
    cv.stroke_rrect(0.5 * s, 0.5 * s, 15.0 * s, 15.0 * s, 3.5 * s, 1.0 * s, Rgba::new(1.0, 1.0, 1.0, 0.55));
    for (x, y) in [(3.5, 3.5), (8.5, 3.5), (3.5, 8.5), (8.5, 8.5)] {
        cv.fill_rrect(x * s, y * s, 4.0 * s, 4.0 * s, 1.0 * s, white);
    }
    cv
}

fn unpremultiply(p: u32) -> u32 {
    let a = p >> 24;
    if a == 0 {
        return 0;
    }
    let un = |v: u32| ((v * 255 + a / 2) / a).min(255);
    (a << 24) | (un((p >> 16) & 0xFF) << 16) | (un((p >> 8) & 0xFF) << 8) | un(p & 0xFF)
}

pub fn icon_rgba(size: i32) -> Vec<u8> {
    icon_canvas(size)
        .px
        .iter()
        .flat_map(|&p| {
            let u = unpremultiply(p);
            [((u >> 16) & 0xFF) as u8, ((u >> 8) & 0xFF) as u8, (u & 0xFF) as u8, (u >> 24) as u8]
        })
        .collect()
}

fn make_icon() -> Option<HICON> {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64);
    let cv = icon_canvas(size);
    unsafe {
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(None, &bi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let dst = std::slice::from_raw_parts_mut(bits.cast::<u32>(), (size * size) as usize);
        for (d, &p) in dst.iter_mut().zip(cv.px.iter()) {
            *d = unpremultiply(p);
        }
        let mask = CreateBitmap(size, size, 1, 1, None);
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}
