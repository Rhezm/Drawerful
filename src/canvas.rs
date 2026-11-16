use std::collections::HashMap;

//Color

#[derive(Clone, Copy, Debug)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub fn from_u8(c: [u8; 4]) -> Self {
        Self { r: c[0] as f32 / 255.0, g: c[1] as f32 / 255.0, b: c[2] as f32 / 255.0, a: c[3] as f32 / 255.0 }
    }

    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn with_alpha(self, a: f32) -> Self {
        Self { a: self.a * a, ..self }
    }
}

#[inline]
fn pack(r: f32, g: f32, b: f32, a: f32) -> u32 {
    let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    (c(a) << 24) | (c(r) << 16) | (c(g) << 8) | c(b)
}

#[inline]
fn unpack(p: u32) -> [f32; 4] {
    [
        ((p >> 16) & 0xFF) as f32 / 255.0,
        ((p >> 8) & 0xFF) as f32 / 255.0,
        (p & 0xFF) as f32 / 255.0,
        (p >> 24) as f32 / 255.0,
    ]
}

//Imagen premultiplicada BGRA

#[derive(Clone)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u32>,
}

impl Image {
    pub fn from_rgba(w: u32, h: u32, rgba: &[u8]) -> Self {
        let px = rgba
            .chunks_exact(4)
            .map(|c| {
                let a = c[3] as u32;
                let m = |v: u8| (v as u32 * a + 127) / 255;
                (a << 24) | (m(c[0]) << 16) | (m(c[1]) << 8) | m(c[2])
            })
            .collect();
        Self { w, h, px }
    }

    pub fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        let x = x.clamp(0.0, (self.w - 1) as f32);
        let y = y.clamp(0.0, (self.h - 1) as f32);
        let x0 = x.floor() as u32;
        let y0 = y.floor() as u32;
        let x1 = (x0 + 1).min(self.w - 1);
        let y1 = (y0 + 1).min(self.h - 1);
        let fx = x - x0 as f32;
        let fy = y - y0 as f32;
        let p = |xx: u32, yy: u32| unpack(self.px[(yy * self.w + xx) as usize]);
        let (a, b, c, d) = (p(x0, y0), p(x1, y0), p(x0, y1), p(x1, y1));
        let mut out = [0.0; 4];
        for i in 0..4 {
            let top = a[i] + (b[i] - a[i]) * fx;
            let bot = c[i] + (d[i] - c[i]) * fx;
            out[i] = top + (bot - top) * fy;
        }
        out
    }
}

//Lienzo

pub struct Canvas {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u32>,
    clip: (i32, i32, i32, i32),
}

#[inline]
fn rrect_sdf(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (hw - r);
    let qy = (py - cy).abs() - (hh - r);
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Self {
        let w = w.max(1);
        let h = h.max(1);
        Self { w, h, px: vec![0; (w * h) as usize], clip: (0, 0, w, h) }
    }

    pub fn resize(&mut self, w: i32, h: i32) {
        let w = w.max(1);
        let h = h.max(1);
        if w != self.w || h != self.h {
            self.w = w;
            self.h = h;
            self.px = vec![0; (w * h) as usize];
        }
        self.clip = (0, 0, w, h);
    }

    pub fn set_clip(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        self.clip = (x0.max(0), y0.max(0), x1.min(self.w), y1.min(self.h));
    }

    pub fn reset_clip(&mut self) {
        self.clip = (0, 0, self.w, self.h);
    }

    pub fn clear(&mut self, raw: u32) {
        self.px.fill(raw);
    }

    pub fn copy_from(&mut self, other: &Canvas) {
        if other.w == self.w && other.h == self.h {
            self.px.copy_from_slice(&other.px);
        }
    }

    #[inline]
    fn blend(&mut self, x: i32, y: i32, r: f32, g: f32, b: f32, a: f32) {
        let (cx0, cy0, cx1, cy1) = self.clip;
        if a <= 0.0 || x < cx0 || y < cy0 || x >= cx1 || y >= cy1 {
            return;
        }
        let i = (y * self.w + x) as usize;
        let d = unpack(self.px[i]);
        let inv = 1.0 - a;
        self.px[i] = pack(r + d[0] * inv, g + d[1] * inv, b + d[2] * inv, a + d[3] * inv);
    }

    fn bounds(&self, x: f32, y: f32, w: f32, h: f32) -> (i32, i32, i32, i32) {
        let (cx0, cy0, cx1, cy1) = self.clip;
        let x0 = (x.floor() as i32).max(cx0);
        let y0 = (y.floor() as i32).max(cy0);
        let x1 = ((x + w).ceil() as i32).min(cx1);
        let y1 = ((y + h).ceil() as i32).min(cy1);
        (x0, y0, x1, y1)
    }

    pub fn fill_rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Rgba) {
        self.fill_rrect_with(x, y, w, h, r, |_, _| [c.r * c.a, c.g * c.a, c.b * c.a, c.a]);
    }

    pub fn fill_rrect_with(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, mut shade: impl FnMut(i32, i32) -> [f32; 4]) {
        let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
        let (cx, cy, hw, hh) = (x + w * 0.5, y + h * 0.5, w * 0.5, h * 0.5);
        let (x0, y0, x1, y1) = self.bounds(x, y, w, h);
        for py in y0..y1 {
            for px in x0..x1 {
                let d = rrect_sdf(px as f32 + 0.5, py as f32 + 0.5, cx, cy, hw, hh, r);
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let s = shade(px, py);
                    self.blend(px, py, s[0] * cov, s[1] * cov, s[2] * cov, s[3] * cov);
                }
            }
        }
    }

    pub fn stroke_rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, width: f32, c: Rgba) {
        if width <= 0.0 || c.a <= 0.0 {
            return;
        }
        let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
        let (cx, cy, hw, hh) = (x + w * 0.5, y + h * 0.5, w * 0.5, h * 0.5);
        let (x0, y0, x1, y1) = self.bounds(x, y, w, h);
        for py in y0..y1 {
            for px in x0..x1 {
                let d = rrect_sdf(px as f32 + 0.5, py as f32 + 0.5, cx, cy, hw, hh, r);
                let cov = (0.5 - d).clamp(0.0, 1.0) * (d + width + 0.5).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let a = c.a * cov;
                    self.blend(px, py, c.r * a, c.g * a, c.b * a, a);
                }
            }
        }
    }

    pub fn draw_image(&mut self, img: &Image, x: f32, y: f32, w: f32, h: f32, alpha: f32) {
        if img.w == 0 || img.h == 0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let sx = img.w as f32 / w;
        let sy = img.h as f32 / h;
        let exact = (sx - 1.0).abs() < 0.001 && (sy - 1.0).abs() < 0.001 && x.fract() == 0.0 && y.fract() == 0.0;
        let (x0, y0, x1, y1) = self.bounds(x, y, w, h);
        for py in y0..y1 {
            for px in x0..x1 {
                let s = if exact {
                    let ix = (px - x as i32) as u32;
                    let iy = (py - y as i32) as u32;
                    if ix >= img.w || iy >= img.h {
                        continue;
                    }
                    unpack(img.px[(iy * img.w + ix) as usize])
                } else {
                    img.sample((px as f32 + 0.5 - x) * sx - 0.5, (py as f32 + 0.5 - y) * sy - 0.5)
                };
                self.blend(px, py, s[0] * alpha, s[1] * alpha, s[2] * alpha, s[3] * alpha);
            }
        }
    }

    pub fn draw_mask(&mut self, mask: &[u8], mw: usize, mh: usize, x: i32, y: i32, c: Rgba) {
        for my in 0..mh {
            for mx in 0..mw {
                let m = mask[my * mw + mx];
                if m == 0 {
                    continue;
                }
                let a = c.a * m as f32 / 255.0;
                self.blend(x + mx as i32, y + my as i32, c.r * a, c.g * a, c.b * a, a);
            }
        }
    }
}

//Desenfoque de caja (3 pasadas ≈ gaussiano)

pub fn box_blur_rgb(data: &mut [u8], w: usize, h: usize, radius: usize) {
    if radius == 0 || w == 0 || h == 0 {
        return;
    }
    let mut tmp = vec![0u8; data.len()];
    for _ in 0..3 {
        blur_pass(data, &mut tmp, w, h, radius, true);
        blur_pass(&tmp, data, w, h, radius, false);
    }
}

fn blur_pass(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, horizontal: bool) {
    let (len, lines) = if horizontal { (w, h) } else { (h, w) };
    let idx = |line: usize, i: usize| if horizontal { (line * w + i) * 3 } else { (i * w + line) * 3 };
    let win = (2 * r + 1) as u32;
    for line in 0..lines {
        for ch in 0..3 {
            let at = |i: isize| src[idx(line, i.clamp(0, len as isize - 1) as usize) + ch] as u32;
            let mut sum: u32 = (-(r as isize)..=r as isize).map(at).sum();
            for i in 0..len {
                dst[idx(line, i) + ch] = ((sum + win / 2) / win) as u8;
                sum += at(i as isize + r as isize + 1);
                sum -= at(i as isize - r as isize);
            }
        }
    }
}

//Texto

struct Glyph {
    metrics: fontdue::Metrics,
    mask: Vec<u8>,
}

pub struct Text {
    font: fontdue::Font,
    symbols: Option<fontdue::Font>,
    cache: HashMap<(char, u32, bool), Glyph>,
}

fn load_font(candidates: &[&str]) -> Option<fontdue::Font> {
    let dir = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into()).join("Fonts");
    candidates.iter().find_map(|name| {
        let bytes = std::fs::read(dir.join(name)).ok()?;
        fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).ok()
    })
}

pub const ELLIPSIS: char = '…';

impl Text {
    pub fn new() -> Result<Self, String> {
        let font = load_font(&["segoeui.ttf", "arial.ttf", "tahoma.ttf"]).ok_or("No se encontró ninguna fuente del sistema (Segoe UI / Arial)")?;
        let symbols = load_font(&["SegoeIcons.ttf", "segmdl2.ttf"]);
        Ok(Self { font, symbols, cache: HashMap::new() })
    }

    pub fn has_symbols(&self) -> bool {
        self.symbols.is_some()
    }

    fn glyph(&mut self, c: char, px: f32, symbol: bool) -> &Glyph {
        let key = (c, (px * 4.0) as u32, symbol);
        let font = if symbol { self.symbols.as_ref().unwrap_or(&self.font) } else { &self.font };
        self.cache.entry(key).or_insert_with(|| {
            let (metrics, mask) = font.rasterize(c, px);
            Glyph { metrics, mask }
        })
    }

    pub fn trim_cache(&mut self) {
        if self.cache.len() > 4096 {
            self.cache.clear();
        }
    }

    pub fn advance(&mut self, c: char, px: f32) -> f32 {
        self.glyph(c, px, false).metrics.advance_width
    }

    pub fn measure(&mut self, s: &str, px: f32) -> f32 {
        s.chars().map(|c| self.advance(c, px)).sum()
    }

    pub fn line_height(&self, px: f32) -> f32 {
        self.font.horizontal_line_metrics(px).map(|m| m.new_line_size).unwrap_or(px * 1.33)
    }

    fn ascent(&self, px: f32) -> f32 {
        self.font.horizontal_line_metrics(px).map(|m| m.ascent).unwrap_or(px)
    }

    pub fn wrap(&mut self, text: &str, px: f32, max_w: f32, max_lines: usize) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        let mut cur = String::new();
        let mut cur_w = 0.0;
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let a = self.advance(c, px);
            if cur_w + a > max_w && !cur.is_empty() {
                if lines.len() + 1 == max_lines {
                    let rest: String = chars[i..].iter().collect();
                    lines.push(self.ellipsize(&(cur.clone() + &rest), px, max_w));
                    return lines;
                }
                if let Some(sp) = cur.rfind(' ') {
                    let tail = cur[sp + 1..].to_string();
                    cur.truncate(sp);
                    lines.push(std::mem::take(&mut cur));
                    cur = tail;
                } else {
                    lines.push(std::mem::take(&mut cur));
                }
                cur_w = self.measure(&cur, px);
                continue;
            }
            cur.push(c);
            cur_w += a;
            i += 1;
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        lines
    }

    pub fn ellipsize(&mut self, s: &str, px: f32, max_w: f32) -> String {
        if self.measure(s, px) <= max_w {
            return s.to_string();
        }
        let ell = self.advance(ELLIPSIS, px);
        let mut out = String::new();
        let mut w = 0.0;
        for c in s.chars() {
            let a = self.advance(c, px);
            if w + a + ell > max_w {
                break;
            }
            out.push(c);
            w += a;
        }
        out.trim_end().to_string() + "…"
    }

    pub fn draw(&mut self, cv: &mut Canvas, s: &str, x: f32, top: f32, px: f32, color: Rgba, shadow: bool) {
        let base = top + self.ascent(px);
        if shadow {
            self.draw_run(cv, s, x + 1.0, base + 1.0, px, Rgba::new(0.0, 0.0, 0.0, 0.75 * color.a), false);
        }
        self.draw_run(cv, s, x, base, px, color, false);
    }

    pub fn draw_centered(&mut self, cv: &mut Canvas, s: &str, cx: f32, top: f32, px: f32, color: Rgba, shadow: bool) {
        let w = self.measure(s, px);
        self.draw(cv, s, (cx - w * 0.5).round(), top.round(), px, color, shadow);
    }

    fn draw_run(&mut self, cv: &mut Canvas, s: &str, x: f32, base: f32, px: f32, color: Rgba, symbol: bool) {
        let mut pen = x;
        for c in s.chars() {
            let g = self.glyph(c, px, symbol);
            let gx = (pen + g.metrics.xmin as f32).round() as i32;
            let gy = (base - g.metrics.height as f32 - g.metrics.ymin as f32).round() as i32;
            cv.draw_mask(&g.mask, g.metrics.width, g.metrics.height, gx, gy, color);
            pen += g.metrics.advance_width;
        }
    }

    pub fn draw_symbol(&mut self, cv: &mut Canvas, c: char, cx: f32, cy: f32, px: f32, color: Rgba, shadow: bool) {
        let symbol = self.symbols.is_some();
        let m = self.glyph(c, px, symbol).metrics;
        let x = cx - m.width as f32 * 0.5 - m.xmin as f32;
        let base = cy + m.height as f32 * 0.5 + m.ymin as f32;
        let s = c.to_string();
        if shadow {
            self.draw_run(cv, &s, x + 1.0, base + 1.5, px, Rgba::new(0.0, 0.0, 0.0, 0.55 * color.a), symbol);
        }
        self.draw_run(cv, &s, x, base, px, color, symbol);
    }
}
