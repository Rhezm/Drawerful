use std::collections::HashMap;
use std::rc::Rc;
use std::time::SystemTime;

use image::{imageops, imageops::FilterType, Rgb, RgbImage};
use windows::Win32::Graphics::Gdi::{GetSysColor, COLOR_DESKTOP};
use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};

use crate::canvas::box_blur_rgb;
use crate::winutil::{from_wide, read_reg_str};

//Fondo desenfocado listo para muestrear

pub struct Backdrop {
    w: u32,
    h: u32,
    factor: f32,
    data: Vec<u8>,
}

impl Backdrop {
    pub fn sample(&self, sx: f32, sy: f32) -> [f32; 3] {
        let x = (sx / self.factor - 0.5).clamp(0.0, (self.w - 1) as f32);
        let y = (sy / self.factor - 0.5).clamp(0.0, (self.h - 1) as f32);
        let x0 = x as u32;
        let y0 = y as u32;
        let x1 = (x0 + 1).min(self.w - 1);
        let y1 = (y0 + 1).min(self.h - 1);
        let fx = x - x0 as f32;
        let fy = y - y0 as f32;
        let at = |xx: u32, yy: u32, c: usize| self.data[((yy * self.w + xx) * 3) as usize + c] as f32;
        let mut out = [0.0; 3];
        for (c, o) in out.iter_mut().enumerate() {
            let top = at(x0, y0, c) + (at(x1, y0, c) - at(x0, y0, c)) * fx;
            let bot = at(x0, y1, c) + (at(x1, y1, c) - at(x0, y1, c)) * fx;
            *o = (top + (bot - top) * fy) / 255.0;
        }
        out
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Style {
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
}

#[derive(PartialEq, Clone)]
struct Key {
    path: String,
    mtime: Option<SystemTime>,
    style: String,
    tile: String,
}

pub struct Wallpaper {
    key: Option<Key>,
    source: Option<RgbImage>,
    style: Style,
    color: Rgb<u8>,
    cache: HashMap<(i32, i32, u32), Rc<Backdrop>>,
}

fn current_key() -> Key {
    let mut buf = [0u16; 1024];
    unsafe {
        let _ = SystemParametersInfoW(SPI_GETDESKWALLPAPER, buf.len() as u32, Some(buf.as_mut_ptr().cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
    let path = from_wide(&buf);
    let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    Key {
        path,
        mtime,
        style: read_reg_str("Control Panel\\Desktop", "WallpaperStyle").unwrap_or_default(),
        tile: read_reg_str("Control Panel\\Desktop", "TileWallpaper").unwrap_or_default(),
    }
}

fn desktop_color() -> Rgb<u8> {
    let c = unsafe { GetSysColor(COLOR_DESKTOP) };
    Rgb([(c & 0xFF) as u8, ((c >> 8) & 0xFF) as u8, ((c >> 16) & 0xFF) as u8])
}

impl Wallpaper {
    pub fn new() -> Self {
        let mut w = Self { key: None, source: None, style: Style::Fill, color: Rgb([0, 0, 0]), cache: HashMap::new() };
        w.refresh();
        w
    }

    pub fn refresh(&mut self) {
        let key = current_key();
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.cache.clear();
        self.color = desktop_color();
        self.style = match (key.style.trim(), key.tile.trim()) {
            ("6", _) => Style::Fit,
            ("2", _) => Style::Stretch,
            ("0", "1") => Style::Tile,
            ("0", _) => Style::Center,
            _ => Style::Fill,
        };
        self.source = if key.path.is_empty() { None } else { load_source(&key.path) };
        self.key = Some(key);
    }

    pub fn backdrop(&mut self, mon_w: i32, mon_h: i32, radius: u32) -> Rc<Backdrop> {
        let k = (mon_w, mon_h, radius);
        if let Some(b) = self.cache.get(&k) {
            return b.clone();
        }
        let factor = (radius / 2).clamp(1, 4);
        let tw = (mon_w.max(1) as u32).div_ceil(factor);
        let th = (mon_h.max(1) as u32).div_ceil(factor);
        let mut img = match &self.source {
            Some(src) => compose(src, tw, th, self.style, self.color, factor),
            None => RgbImage::from_pixel(tw, th, self.color),
        };
        box_blur_rgb(&mut img, tw as usize, th as usize, (radius / factor).max(1) as usize);
        let b = Rc::new(Backdrop { w: tw, h: th, factor: factor as f32, data: img.into_raw() });
        if self.cache.len() > 8 {
            self.cache.clear();
        }
        self.cache.insert(k, b.clone());
        b
    }
}

fn load_source(path: &str) -> Option<RgbImage> {
    let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let img = if img.width() > 2560 { img.resize(2560, 2560, FilterType::Triangle) } else { img };
    Some(img.to_rgb8())
}

fn compose(src: &RgbImage, tw: u32, th: u32, style: Style, bg: Rgb<u8>, factor: u32) -> RgbImage {
    let (iw, ih) = (src.width() as f32, src.height() as f32);
    let mut out = RgbImage::from_pixel(tw, th, bg);
    match style {
        Style::Stretch => return imageops::resize(src, tw, th, FilterType::Triangle),
        Style::Fill | Style::Fit => {
            let s = if style == Style::Fill { (tw as f32 / iw).max(th as f32 / ih) } else { (tw as f32 / iw).min(th as f32 / ih) };
            let rw = ((iw * s).round() as u32).max(1);
            let rh = ((ih * s).round() as u32).max(1);
            let scaled = imageops::resize(src, rw, rh, FilterType::Triangle);
            imageops::overlay(&mut out, &scaled, (tw as i64 - rw as i64) / 2, (th as i64 - rh as i64) / 2);
        }
        Style::Center | Style::Tile => {
            let rw = ((iw / factor as f32).round() as u32).max(1);
            let rh = ((ih / factor as f32).round() as u32).max(1);
            let scaled = imageops::resize(src, rw, rh, FilterType::Triangle);
            if style == Style::Center {
                imageops::overlay(&mut out, &scaled, (tw as i64 - rw as i64) / 2, (th as i64 - rh as i64) / 2);
            } else {
                let mut y = 0;
                while y < th {
                    let mut x = 0;
                    while x < tw {
                        imageops::overlay(&mut out, &scaled, x as i64, y as i64);
                        x += rw;
                    }
                    y += rh;
                }
            }
        }
    }
    out
}
