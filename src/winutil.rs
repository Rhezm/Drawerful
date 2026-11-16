use std::path::PathBuf;

use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, SystemParametersInfoForDpi};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{FOLDERID_Desktop, FileOpenDialog, IFileOpenDialog, SHGetKnownFolderPath, SIGDN_FILESYSPATH, KF_FLAG_DEFAULT};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, MessageBoxW, PostMessageW, MB_ICONERROR, MB_ICONQUESTION, MB_ICONWARNING, MB_OK,
    MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO, IDYES, SPI_GETICONTITLELOGFONT, SPI_ICONHORIZONTALSPACING, SPI_ICONVERTICALSPACING,
};
use windows::Win32::Graphics::Gdi::LOGFONTW;

pub const HOST_CLASS: PCWSTR = w!("Cuadros.Host");
pub const WM_APP_RELOAD: u32 = 0x8000 + 1;
pub const WM_APP_SETTINGS: u32 = 0x8000 + 2;
pub const WM_APP_ICON_READY: u32 = 0x8000 + 3;
pub const WM_APP_TRAY: u32 = 0x8000 + 4;

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("Cuadros");

//Cadenas

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

pub fn hs(s: &str) -> HSTRING {
    HSTRING::from(s)
}

pub fn loword(v: isize) -> i32 {
    (v & 0xFFFF) as u16 as i16 as i32
}

pub fn hiword(v: isize) -> i32 {
    ((v >> 16) & 0xFFFF) as u16 as i16 as i32
}

//Mensajes al usuario

pub fn error_box(text: &str) {
    unsafe {
        MessageBoxW(None, &hs(text), w!("Cuadros"), MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST);
    }
}

pub fn warn_box(text: &str) {
    unsafe {
        MessageBoxW(None, &hs(text), w!("Cuadros"), MB_OK | MB_ICONWARNING | MB_SETFOREGROUND | MB_TOPMOST);
    }
}

pub fn confirm_box(text: &str) -> bool {
    unsafe { MessageBoxW(None, &hs(text), w!("Cuadros"), MB_YESNO | MB_ICONQUESTION | MB_SETFOREGROUND | MB_TOPMOST) == IDYES }
}

pub fn log(msg: &str) {
    use std::io::Write;
    let dir = crate::config::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("cuadros.log");
    if std::fs::metadata(&path).map(|m| m.len() > 512 * 1024).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("cuadros.old.log"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "[{}] {msg}", crate::config::now_secs());
    }
}

//Instancia única

pub struct SingleInstance(HANDLE);

impl SingleInstance {
    pub fn acquire(name: &str) -> Option<Self> {
        unsafe {
            let h = CreateMutexW(None, false, &hs(name)).ok()?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(h);
                return None;
            }
            Some(Self(h))
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub fn post_to_host(msg: u32, wparam: usize) -> bool {
    unsafe {
        match FindWindowW(HOST_CLASS, PCWSTR::null()) {
            Ok(h) if !h.is_invalid() => PostMessageW(Some(h), msg, WPARAM(wparam), LPARAM(0)).is_ok(),
            _ => false,
        }
    }
}

//Inicio con Windows

fn exe_command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", exe.to_string_lossy()))
}

fn read_reg_string(subkey: PCWSTR, value: PCWSTR) -> Option<String> {
    unsafe {
        let mut buf = vec![0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let r = RegGetValueW(HKEY_CURRENT_USER, subkey, value, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size));
        r.is_ok().then(|| from_wide(&buf))
    }
}

pub fn read_reg_dword(subkey: &str, value: &str) -> Option<u32> {
    unsafe {
        let mut v = 0u32;
        let mut size = 4u32;
        let r = RegGetValueW(HKEY_CURRENT_USER, &hs(subkey), &hs(value), RRF_RT_REG_DWORD, None, Some((&mut v as *mut u32).cast()), Some(&mut size));
        r.is_ok().then_some(v)
    }
}

pub fn read_reg_str(subkey: &str, value: &str) -> Option<String> {
    read_reg_string(PCWSTR(hs(subkey).as_ptr()), PCWSTR(hs(value).as_ptr()))
}

pub fn autostart_enabled() -> bool {
    read_reg_string(RUN_KEY, RUN_VALUE).is_some()
}

pub fn set_autostart(on: bool) -> Result<(), String> {
    unsafe {
        if on {
            let cmd = exe_command().ok_or("No se pudo obtener la ruta del ejecutable")?;
            let data = wide(&cmd);
            let r = RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, REG_SZ.0, Some(data.as_ptr().cast()), (data.len() * 2) as u32);
            if r.is_err() {
                return Err(format!("No se pudo registrar el inicio automático (código {})", r.0));
            }
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
        }
    }
    Ok(())
}

pub fn refresh_autostart_path() {
    if let (Some(current), Some(expected)) = (read_reg_string(RUN_KEY, RUN_VALUE), exe_command()) {
        if !current.eq_ignore_ascii_case(&expected) {
            let _ = set_autostart(true);
        }
    }
}

//Carpetas del sistema

pub fn desktop_dir() -> PathBuf {
    unsafe {
        if let Ok(p) = SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None) {
            let s = p.to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0 as *const _));
            if !s.is_empty() {
                return PathBuf::from(s);
            }
        }
    }
    std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Desktop")).unwrap_or_else(|| PathBuf::from("."))
}

//Métricas de escritorio

pub struct DesktopMetrics {
    pub scale: f32,
    pub icon: u32,
    pub cell_w: u32,
    pub cell_h: u32,
    pub font_px: f32,
}

pub fn dpi_of(hwnd: HWND) -> u32 {
    let d = unsafe { GetDpiForWindow(hwnd) };
    if d == 0 { 96 } else { d }
}

pub fn desktop_metrics(dpi: u32) -> DesktopMetrics {
    let scale = dpi as f32 / 96.0;
    let logical_icon = read_reg_dword("Software\\Microsoft\\Windows\\Shell\\Bags\\1\\Desktop", "IconSize").unwrap_or(48).clamp(16, 256);
    let icon = (logical_icon as f32 * scale).round() as u32;
    let mut h = 0i32;
    let mut v = 0i32;
    let mut lf = LOGFONTW::default();
    unsafe {
        let _ = SystemParametersInfoForDpi(SPI_ICONHORIZONTALSPACING.0, 0, Some((&mut h as *mut i32).cast()), 0, dpi);
        let _ = SystemParametersInfoForDpi(SPI_ICONVERTICALSPACING.0, 0, Some((&mut v as *mut i32).cast()), 0, dpi);
        let _ = SystemParametersInfoForDpi(SPI_GETICONTITLELOGFONT.0, std::mem::size_of::<LOGFONTW>() as u32, Some((&mut lf as *mut LOGFONTW).cast()), 0, dpi);
    }
    let font_px = if lf.lfHeight != 0 { lf.lfHeight.unsigned_abs() as f32 } else { 12.0 * scale };
    let min_w = icon + (28.0 * scale) as u32;
    let min_h = icon + (font_px * 2.0 + 16.0 * scale) as u32;
    DesktopMetrics {
        scale,
        icon,
        cell_w: (h.max(0) as u32).max(min_w),
        cell_h: (v.max(0) as u32).max(min_h),
        font_px,
    }
}

//Monitores

pub fn work_area_at(pt: POINT) -> (RECT, RECT) {
    unsafe {
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        (mi.rcWork, mi.rcMonitor)
    }
}

//Diálogo de archivos

pub fn pick_image(owner: Option<HWND>) -> Option<PathBuf> {
    unsafe {
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let name = wide("Imágenes");
        let spec = wide("*.png;*.jpg;*.jpeg;*.ico;*.webp;*.bmp");
        let filters = [COMDLG_FILTERSPEC { pszName: PCWSTR(name.as_ptr()), pszSpec: PCWSTR(spec.as_ptr()) }];
        let _ = dlg.SetFileTypes(&filters);
        let _ = dlg.SetTitle(w!("Elegir icono del cuadro"));
        dlg.Show(owner).ok()?;
        let item = dlg.GetResult().ok()?;
        let p: PWSTR = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}
