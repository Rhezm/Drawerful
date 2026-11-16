use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender};

use windows::Win32::Foundation::{HWND, LPARAM, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_RESIZETOFIT};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::canvas::Image;
use crate::winutil::{hs, WM_APP_ICON_READY};

type Key = (String, u32);

struct Done {
    key: Key,
    image: Option<Image>,
}

//Carga de iconos del shell en un hilo aparte

pub struct IconLoader {
    tx: Sender<Key>,
    rx: Receiver<Done>,
    pending: HashSet<Key>,
    cache: HashMap<Key, Option<Rc<Image>>>,
    custom: HashMap<String, Option<Rc<Image>>>,
}

impl IconLoader {
    pub fn new(notify: HWND) -> Self {
        let (tx, worker_rx) = channel::<Key>();
        let (worker_tx, rx) = channel::<Done>();
        let target = notify.0 as isize;
        std::thread::spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
            while let Ok(key) = worker_rx.recv() {
                let image = extract(&key.0, key.1);
                if worker_tx.send(Done { key, image }).is_err() {
                    break;
                }
                unsafe {
                    let _ = PostMessageW(Some(HWND(target as *mut _)), WM_APP_ICON_READY, WPARAM(0), LPARAM(0));
                }
            }
            unsafe { CoUninitialize() };
        });
        Self { tx, rx, pending: HashSet::new(), cache: HashMap::new(), custom: HashMap::new() }
    }

    pub fn get(&mut self, path: &str, size: u32) -> Option<Rc<Image>> {
        let key = (path.to_string(), size);
        if let Some(v) = self.cache.get(&key) {
            return v.clone();
        }
        if self.pending.insert(key.clone()) && self.tx.send(key.clone()).is_err() {
            self.pending.remove(&key);
        }
        None
    }

    pub fn collect(&mut self) -> bool {
        let mut any = false;
        while let Ok(done) = self.rx.try_recv() {
            self.pending.remove(&done.key);
            self.cache.insert(done.key, done.image.map(Rc::new));
            any = true;
        }
        any
    }

    pub fn retain_paths(&mut self, keep: &HashSet<String>) {
        self.cache.retain(|k, _| keep.contains(&k.0));
    }

    pub fn invalidate(&mut self, path: &str) {
        self.cache.retain(|k, _| k.0 != path);
    }

    pub fn custom(&mut self, file: &str) -> Option<Rc<Image>> {
        if let Some(v) = self.custom.get(file) {
            return v.clone();
        }
        let path = crate::config::icons_dir().join(file);
        let img = image::open(&path).ok().map(|i| {
            let rgba = i.to_rgba8();
            Rc::new(Image::from_rgba(rgba.width(), rgba.height(), rgba.as_raw()))
        });
        if img.is_none() {
            crate::winutil::log(&format!("No se pudo cargar el icono personalizado {}", path.display()));
        }
        self.custom.insert(file.to_string(), img.clone());
        img
    }

    pub fn clear_custom(&mut self) {
        self.custom.clear();
    }
}

fn extract(path: &str, size: u32) -> Option<Image> {
    unsafe {
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&hs(path), None).ok()?;
        let hbm = factory.GetImage(SIZE { cx: size as i32, cy: size as i32 }, SIIGBF_RESIZETOFIT).ok()?;
        let img = bitmap_to_image(hbm);
        let _ = DeleteObject(hbm.into());
        img
    }
}

unsafe fn bitmap_to_image(hbm: HBITMAP) -> Option<Image> {
    let mut bm = BITMAP::default();
    if GetObjectW(hbm.into(), std::mem::size_of::<BITMAP>() as i32, Some((&mut bm as *mut BITMAP).cast())) == 0 {
        return None;
    }
    let (w, h) = (bm.bmWidth.max(0) as u32, bm.bmHeight.unsigned_abs());
    if w == 0 || h == 0 {
        return None;
    }
    let mut bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut px = vec![0u32; (w * h) as usize];
    let dc = GetDC(None);
    let lines = GetDIBits(dc, hbm, 0, h, Some(px.as_mut_ptr().cast()), &mut bi, DIB_RGB_COLORS);
    ReleaseDC(None, dc);
    if lines == 0 {
        return None;
    }
    normalize_alpha(&mut px);
    Some(Image { w, h, px })
}

fn normalize_alpha(px: &mut [u32]) {
    if px.iter().all(|p| p >> 24 == 0) {
        px.iter_mut().for_each(|p| *p |= 0xFF00_0000);
        return;
    }
    let straight = px.iter().any(|&p| {
        let a = p >> 24;
        ((p >> 16) & 0xFF) > a || ((p >> 8) & 0xFF) > a || (p & 0xFF) > a
    });
    if straight {
        for p in px.iter_mut() {
            let a = *p >> 24;
            let m = |v: u32| (v * a + 127) / 255;
            *p = (a << 24) | (m((*p >> 16) & 0xFF) << 16) | (m((*p >> 8) & 0xFF) << 8) | m(*p & 0xFF);
        }
    }
}
