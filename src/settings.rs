use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Margin, RichText, Stroke, TextureHandle, Vec2};
use windows::core::PCWSTR;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE};

use crate::config::{self, Config, GroupIcon, PopupStyle, SortMode, StorageMode, Trigger};
use crate::presets::PRESETS;
use crate::storage;
use crate::winutil::*;

const TITLE: &str = "Cuadros — Configuración";
const ACCENT: Color32 = Color32::from_rgb(96, 165, 250);

#[derive(Clone, Copy, PartialEq)]
enum Page {
    General,
    Group(u64),
}

struct App {
    cfg: Config,
    loaded: Config,
    page: Page,
    mtime: Option<SystemTime>,
    last_poll: Instant,
    dirty_since: Option<Instant>,
    status: Option<(String, bool, Instant)>,
    autostart: bool,
    textures: HashMap<String, Option<TextureHandle>>,
    has_symbols: bool,
}

pub fn run(group: Option<u64>) -> i32 {
    let Some(_instance) = SingleInstance::acquire("Local\\Cuadros.Settings") else {
        unsafe {
            if let Ok(h) = FindWindowW(PCWSTR::null(), &hs(TITLE)) {
                let _ = ShowWindow(h, SW_RESTORE);
                let _ = SetForegroundWindow(h);
            }
        }
        return 0;
    };
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let icon = egui::IconData { rgba: crate::tray::icon_rgba(64), width: 64, height: 64 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_inner_size([940.0, 680.0])
            .with_min_inner_size([760.0, 500.0])
            .with_icon(Arc::new(icon)),
        ..Default::default()
    };
    let result = eframe::run_native(
        TITLE,
        options,
        Box::new(move |cc| {
            let has_symbols = setup_style(&cc.egui_ctx);
            Ok(Box::new(App::new(group, has_symbols)))
        }),
    );
    match result {
        Ok(()) => 0,
        Err(e) => {
            let msg = format!("No se pudo abrir la ventana de configuración: {e}");
            log(&msg);
            error_box(&msg);
            1
        }
    }
}

//Estilo visual

fn font_bytes(names: &[&str]) -> Option<Vec<u8>> {
    let dir = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into()).join("Fonts");
    names.iter().find_map(|n| std::fs::read(dir.join(n)).ok())
}

fn setup_style(ctx: &egui::Context) -> bool {
    let mut fonts = FontDefinitions::default();
    if let Some(ui_font) = font_bytes(&["segoeui.ttf"]) {
        fonts.font_data.insert("segoe".into(), Arc::new(FontData::from_owned(ui_font)));
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "segoe".into());
    }
    let has_symbols = match font_bytes(&["SegoeIcons.ttf", "segmdl2.ttf"]) {
        Some(sym) => {
            fonts.font_data.insert("icons".into(), Arc::new(FontData::from_owned(sym)));
            fonts.families.insert(FontFamily::Name("icons".into()), vec!["icons".into()]);
            true
        }
        None => false,
    };
    ctx.set_fonts(fonts);
    let mut style = (*ctx.global_style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.selection.bg_fill = ACCENT.linear_multiply(0.55);
    style.visuals.hyperlink_color = ACCENT;
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same(6);
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same(6);
    style.visuals.widgets.active.corner_radius = CornerRadius::same(6);
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.spacing.button_padding = Vec2::new(10.0, 5.0);
    style.spacing.slider_width = 220.0;
    ctx.set_global_style(style);
    has_symbols
}

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(32, 34, 40))
        .stroke(Stroke::new(1.0, Color32::from_rgb(52, 55, 64)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(15.0).strong());
            ui.add_space(4.0);
            add(ui);
        });
    ui.add_space(10.0);
}

fn auto_slider(ui: &mut egui::Ui, label: &str, value: &mut u32, range: std::ops::RangeInclusive<u32>, default: u32, suffix: &str) {
    ui.horizontal(|ui| {
        let mut auto = *value == 0;
        ui.add_sized([150.0, 20.0], egui::Label::new(label));
        if ui.checkbox(&mut auto, "Automático").changed() {
            *value = if auto { 0 } else { default };
        }
        if !auto {
            ui.add(egui::Slider::new(value, range).suffix(suffix));
        }
    });
}

fn color_row(ui: &mut egui::Ui, label: &str, c: &mut [u8; 4]) {
    ui.horizontal(|ui| {
        ui.add_sized([150.0, 20.0], egui::Label::new(label));
        let mut col = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
        if egui::color_picker::color_edit_button_srgba(ui, &mut col, egui::color_picker::Alpha::OnlyBlend).changed() {
            *c = col.to_srgba_unmultiplied();
        }
        ui.weak(format!("#{:02X}{:02X}{:02X} · {}%", c[0], c[1], c[2], (c[3] as u32 * 100 + 127) / 255));
    });
}

//Aplicación

impl App {
    fn new(group: Option<u64>, has_symbols: bool) -> Self {
        let (cfg, status) = match config::load() {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some((e, true, Instant::now()))),
        };
        let page = match group {
            Some(g) if cfg.group(g).is_some() => Page::Group(g),
            _ => cfg.groups.first().map(|g| Page::Group(g.id)).unwrap_or(Page::General),
        };
        Self {
            loaded: cfg.clone(),
            cfg,
            page,
            mtime: config::config_mtime(),
            last_poll: Instant::now(),
            dirty_since: None,
            status,
            autostart: autostart_enabled(),
            textures: HashMap::new(),
            has_symbols,
        }
    }

    fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some((text.into(), error, Instant::now()));
    }

    fn poll_disk(&mut self) {
        if self.last_poll.elapsed() < Duration::from_millis(700) {
            return;
        }
        self.last_poll = Instant::now();
        let m = config::config_mtime();
        if m != self.mtime && self.dirty_since.is_none() {
            if let Ok(c) = config::load() {
                self.cfg = c.clone();
                self.loaded = c;
                self.mtime = m;
                if let Page::Group(g) = self.page {
                    if self.cfg.group(g).is_none() {
                        self.page = Page::General;
                    }
                }
            }
        }
    }

    fn save_now(&mut self) {
        if let Ok(disk) = config::load() {
            for g in &mut self.cfg.groups {
                let Some(d) = disk.group(g.id) else { continue };
                g.x = d.x;
                g.y = d.y;
                let untouched = self.loaded.group(g.id).map(|l| l.items == g.items).unwrap_or(false);
                if untouched {
                    g.items = d.items.clone();
                }
            }
            self.cfg.next_id = self.cfg.next_id.max(disk.next_id);
        }
        match config::save(&self.cfg) {
            Ok(()) => {
                self.loaded = self.cfg.clone();
                self.mtime = config::config_mtime();
                if !post_to_host(WM_APP_RELOAD, 0) {
                    self.notify("Guardado. La aplicación principal no está en ejecución.", false);
                }
            }
            Err(e) => self.notify(e, true),
        }
        self.dirty_since = None;
    }

    fn texture(&mut self, ctx: &egui::Context, file: &str) -> Option<TextureHandle> {
        if let Some(t) = self.textures.get(file) {
            return t.clone();
        }
        let tex = image::open(config::icons_dir().join(file)).ok().map(|img| {
            let rgba = img.to_rgba8();
            let ci = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
            ctx.load_texture(file, ci, egui::TextureOptions::LINEAR)
        });
        self.textures.insert(file.to_string(), tex.clone());
        tex
    }

    fn glyph_text(&self, c: char, size: f32) -> RichText {
        if self.has_symbols {
            RichText::new(c.to_string()).family(FontFamily::Name("icons".into())).size(size)
        } else {
            RichText::new("■").size(size * 0.7)
        }
    }

    //Paneles

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.label(RichText::new("Cuadros").size(20.0).strong());
        ui.weak("Organiza tu escritorio");
        ui.add_space(12.0);
        if ui.selectable_label(self.page == Page::General, "⚙  General").clicked() {
            self.page = Page::General;
        }
        ui.add_space(6.0);
        ui.separator();
        ui.weak("Tus cuadros");
        let mut pick = None;
        egui::ScrollArea::vertical().max_height(ui.available_height() - 50.0).show(ui, |ui| {
            for g in &self.cfg.groups {
                let glyph = match &g.icon {
                    GroupIcon::Preset(k) => crate::presets::find(k).glyph,
                    GroupIcon::Image(_) => '\u{EB9F}',
                };
                ui.horizontal(|ui| {
                    ui.label(self.glyph_text(glyph, 16.0));
                    let r = ui.selectable_label(self.page == Page::Group(g.id), format!("{}  ({})", g.name, g.items.len()));
                    if r.clicked() {
                        pick = Some(g.id);
                    }
                });
            }
        });
        if let Some(id) = pick {
            self.page = Page::Group(id);
        }
        ui.add_space(6.0);
        if ui.add_sized([ui.available_width(), 30.0], egui::Button::new("＋  Nuevo cuadro")).clicked() {
            let (wa, _) = work_area_at(windows::Win32::Foundation::POINT { x: 0, y: 0 });
            let k = self.cfg.groups.len() as i32;
            let id = self.cfg.new_group(wa.left + 40 + k * 24, wa.top + 40 + k * 24);
            self.page = Page::Group(id);
            self.save_now();
        }
    }

    fn general(&mut self, ui: &mut egui::Ui) {
        let c = &mut self.cfg;
        section(ui, "Comportamiento", |ui| {
            ui.checkbox(&mut c.enabled, "Aplicación activada (muestra los cuadros en el escritorio)");
            ui.horizontal(|ui| {
                ui.add_sized([150.0, 20.0], egui::Label::new("Abrir el cuadro al"));
                ui.radio_value(&mut c.trigger, Trigger::Hover, "Pasar el mouse");
                ui.radio_value(&mut c.trigger, Trigger::Click, "Hacer clic");
            });
            if c.trigger == Trigger::Hover {
                ui.horizontal(|ui| {
                    ui.add_sized([150.0, 20.0], egui::Label::new("Retardo de apertura"));
                    ui.add(egui::Slider::new(&mut c.hover_delay_ms, 0..=1000).suffix(" ms"));
                });
                ui.horizontal(|ui| {
                    ui.add_sized([150.0, 20.0], egui::Label::new("Retardo de cierre"));
                    ui.add(egui::Slider::new(&mut c.close_delay_ms, 100..=2000).suffix(" ms"));
                });
            }
            ui.horizontal(|ui| {
                ui.add_sized([150.0, 20.0], egui::Label::new("Abrir elementos con"));
                ui.radio_value(&mut c.single_click_open, true, "Un clic");
                ui.radio_value(&mut c.single_click_open, false, "Doble clic");
            });
            ui.checkbox(&mut c.animations, "Animación de apertura");
            ui.checkbox(&mut c.locked, "Bloquear la posición de los cuadros");
        });
        section(ui, "Archivos agregados", |ui| {
            ui.radio_value(&mut c.storage, StorageMode::Reference, "Guardar solo una referencia (el original queda donde está)");
            ui.radio_value(&mut c.storage, StorageMode::Move, "Mover el archivo dentro del cuadro (limpia el escritorio)");
            ui.weak(format!("Los archivos movidos se guardan en {}", config::data_dir().join("store").display()));
            ui.weak("Al quitar un elemento movido, vuelve a su ubicación original.");
        });
        let mut autostart = self.autostart;
        let mut err = None;
        section(ui, "Sistema", |ui| {
            if ui.checkbox(&mut autostart, "Iniciar con Windows").changed() {
                if let Err(e) = set_autostart(autostart) {
                    err = Some(e);
                }
            }
            if ui.button("Abrir carpeta de datos").clicked() {
                let _ = std::fs::create_dir_all(config::data_dir());
                let _ = std::process::Command::new("explorer.exe").arg(config::data_dir()).spawn();
            }
        });
        self.autostart = autostart_enabled();
        if let Some(e) = err {
            self.notify(e, true);
        }
    }

    fn group_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, gid: u64) {
        let Some(gi) = self.cfg.groups.iter().position(|g| g.id == gid) else {
            self.page = Page::General;
            return;
        };
        let custom_file = match &self.cfg.groups[gi].icon {
            GroupIcon::Image(f) => Some(f.clone()),
            _ => None,
        };
        let custom_tex = custom_file.as_deref().and_then(|f| self.texture(ctx, f));
        let has_symbols = self.has_symbols;
        let mut pick_image = false;
        let mut delete = false;
        let mut remove_item: Option<usize> = None;
        let mut swap: Option<(usize, usize)> = None;
        let glyph = |c: char, size: f32| {
            if has_symbols {
                RichText::new(c.to_string()).family(FontFamily::Name("icons".into())).size(size)
            } else {
                RichText::new("■").size(size * 0.7)
            }
        };
        let g = &mut self.cfg.groups[gi];

        section(ui, "Cuadro", |ui| {
            ui.horizontal(|ui| {
                ui.add_sized([150.0, 20.0], egui::Label::new("Nombre"));
                ui.add(egui::TextEdit::singleline(&mut g.name).desired_width(260.0));
            });
            ui.horizontal(|ui| {
                ui.add_sized([150.0, 20.0], egui::Label::new("Tamaño del icono"));
                ui.add(egui::Slider::new(&mut g.tile_size, 32..=160).suffix(" px"));
            });
            ui.checkbox(&mut g.show_tile_label, "Mostrar el nombre bajo el icono");
        });

        section(ui, "Icono", |ui| {
            ui.horizontal_wrapped(|ui| {
                for p in PRESETS {
                    let selected = matches!(&g.icon, GroupIcon::Preset(k) if k == p.key);
                    let btn = egui::Button::new(glyph(p.glyph, 22.0)).min_size(Vec2::new(44.0, 40.0)).selected(selected);
                    if ui.add(btn).on_hover_text(p.label).clicked() {
                        g.icon = GroupIcon::Preset(p.key.into());
                    }
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Usar imagen propia…").on_hover_text("PNG, JPG, ICO, WEBP o BMP. Se reduce a 128 px para ahorrar recursos.").clicked() {
                    pick_image = true;
                }
                if let Some(tex) = &custom_tex {
                    ui.add(egui::Image::new(tex).fit_to_exact_size(Vec2::splat(40.0)).corner_radius(6));
                    ui.weak("Imagen personalizada en uso");
                }
            });
        });

        section(ui, "Cuadro desplegado", |ui| {
            let p = &mut g.popup;
            auto_slider(ui, "Ancho", &mut p.width, 120..=1800, 480, " px");
            auto_slider(ui, "Alto", &mut p.height, 80..=1400, 320, " px");
            auto_slider(ui, "Columnas", &mut p.columns, 1..=16, 4, "");
            ui.horizontal(|ui| {
                let mut same = p.cell_w == 0 && p.cell_h == 0;
                ui.add_sized([150.0, 20.0], egui::Label::new("Tamaño de celda"));
                if ui.checkbox(&mut same, "Igual que el escritorio").changed() {
                    (p.cell_w, p.cell_h) = if same { (0, 0) } else { (80, 96) };
                }
                if !same {
                    ui.add(egui::DragValue::new(&mut p.cell_w).range(40..=320).prefix("ancho ").suffix(" px"));
                    ui.add(egui::DragValue::new(&mut p.cell_h).range(40..=320).prefix("alto ").suffix(" px"));
                }
            });
            auto_slider(ui, "Tamaño de iconos", &mut p.icon_size, 16..=256, 48, " px");
            ui.horizontal(|ui| {
                let mut auto = p.font_px <= 0.0;
                ui.add_sized([150.0, 20.0], egui::Label::new("Tamaño de texto"));
                if ui.checkbox(&mut auto, "Automático").changed() {
                    p.font_px = if auto { 0.0 } else { 12.0 };
                }
                if !auto {
                    ui.add(egui::Slider::new(&mut p.font_px, 8.0..=24.0).suffix(" px"));
                }
            });
            ui.horizontal(|ui| {
                ui.add_sized([150.0, 20.0], egui::Label::new("Ordenar por"));
                let label = |m: SortMode| match m {
                    SortMode::Manual => "Manual (arrastrando)",
                    SortMode::Name => "Nombre",
                    SortMode::Kind => "Tipo",
                    SortMode::Added => "Fecha de agregado",
                };
                egui::ComboBox::from_id_salt("sort").selected_text(label(p.sort)).show_ui(ui, |ui| {
                    for m in [SortMode::Manual, SortMode::Name, SortMode::Kind, SortMode::Added] {
                        ui.selectable_value(&mut p.sort, m, label(m));
                    }
                });
                ui.checkbox(&mut p.sort_desc, "Descendente");
            });
            ui.checkbox(&mut p.show_title, "Mostrar título");
        });

        section(ui, "Apariencia", |ui| {
            let p = &mut g.popup;
            color_row(ui, "Color de fondo", &mut p.background);
            ui.horizontal(|ui| {
                ui.add_sized([150.0, 20.0], egui::Label::new("Opacidad del fondo"));
                let mut pct = (p.opacity * 100.0).round() as u32;
                if ui.add(egui::Slider::new(&mut pct, 0..=100).suffix(" %")).changed() {
                    p.opacity = pct as f32 / 100.0;
                }
            });
            ui.horizontal(|ui| {
                ui.add_sized([150.0, 20.0], egui::Label::new("Desenfoque del fondo"));
                ui.add(egui::Slider::new(&mut p.blur, 0..=40).suffix(" px"));
                if p.blur == 0 {
                    ui.weak("(transparente real)");
                }
            });
            color_row(ui, "Color del borde", &mut p.outline);
            ui.horizontal(|ui| {
                ui.add_sized([150.0, 20.0], egui::Label::new("Grosor del borde"));
                ui.add(egui::Slider::new(&mut p.outline_width, 0.0..=6.0).step_by(0.5).suffix(" px"));
            });
            ui.horizontal(|ui| {
                ui.add_sized([150.0, 20.0], egui::Label::new("Redondeo de esquinas"));
                ui.add(egui::Slider::new(&mut p.corner_radius, 0.0..=32.0).suffix(" px"));
            });
            color_row(ui, "Color del texto", &mut p.text_color);
            ui.checkbox(&mut p.text_shadow, "Sombra en el texto");
            if ui.button("Restaurar apariencia por defecto").clicked() {
                let d = PopupStyle::default();
                p.background = d.background;
                p.opacity = d.opacity;
                p.blur = d.blur;
                p.outline = d.outline;
                p.outline_width = d.outline_width;
                p.corner_radius = d.corner_radius;
                p.text_color = d.text_color;
                p.text_shadow = d.text_shadow;
            }
        });

        let manual = g.popup.sort == SortMode::Manual;
        let count = g.items.len();
        section(ui, &format!("Elementos ({count})"), |ui| {
            if g.items.is_empty() {
                ui.weak("Arrastra archivos, carpetas o accesos directos sobre el cuadro en el escritorio, esté abierto o cerrado.");
            }
            for (i, it) in g.items.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(manual && i > 0, |ui| {
                        if ui.small_button("▲").clicked() {
                            swap = Some((i, i - 1));
                        }
                    });
                    ui.add_enabled_ui(manual && i + 1 < count, |ui| {
                        if ui.small_button("▼").clicked() {
                            swap = Some((i, i + 1));
                        }
                    });
                    let exists = std::path::Path::new(&it.path).exists();
                    let name = RichText::new(it.display_name()).strong();
                    ui.label(if exists { name } else { name.color(Color32::from_rgb(240, 120, 120)) });
                    if it.origin.is_some() {
                        ui.label(RichText::new("movido").small().color(ACCENT)).on_hover_text(format!("Original: {}", it.origin.as_deref().unwrap_or("")));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let tip = if it.origin.is_some() { "Devolver a su ubicación original" } else { "Quitar del cuadro" };
                        if ui.small_button("✕").on_hover_text(tip).clicked() {
                            remove_item = Some(i);
                        }
                        ui.add(egui::Label::new(RichText::new(&it.path).weak().small()).truncate());
                    });
                });
            }
            if !manual && count > 1 {
                ui.weak("Para reordenar a mano, elige el orden «Manual».");
            }
        });

        ui.horizontal(|ui| {
            let btn = egui::Button::new(RichText::new("Eliminar este cuadro").color(Color32::from_rgb(250, 140, 140)));
            if ui.add(btn).clicked() {
                delete = true;
            }
        });

        if let Some((a, b)) = swap {
            g.items.swap(a, b);
        }
        if pick_image {
            if let Some(path) = pick_image_dialog() {
                match storage::import_icon(&path, gid) {
                    Ok(file) => {
                        if let GroupIcon::Image(old) = &self.cfg.groups[gi].icon {
                            let _ = std::fs::remove_file(config::icons_dir().join(old));
                        }
                        self.cfg.groups[gi].icon = GroupIcon::Image(file);
                    }
                    Err(e) => self.notify(e, true),
                }
            }
        }
        if let Some(i) = remove_item {
            match storage::remove_item(&mut self.cfg, gid, i) {
                Ok(()) => self.save_now(),
                Err(e) => self.notify(e, true),
            }
        }
        if delete {
            let name = self.cfg.groups[gi].name.clone();
            if confirm_box(&format!("¿Eliminar el cuadro \"{name}\"? Los archivos movidos volverán a su ubicación original.")) {
                match storage::delete_group(&mut self.cfg, gid) {
                    Ok(()) => {
                        self.page = self.cfg.groups.first().map(|g| Page::Group(g.id)).unwrap_or(Page::General);
                        self.save_now();
                    }
                    Err(e) => {
                        self.notify(e, true);
                        self.save_now();
                    }
                }
            }
        }
    }
}

fn pick_image_dialog() -> Option<std::path::PathBuf> {
    pick_image(None)
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_disk();
        if let Some(t) = self.dirty_since {
            if t.elapsed() >= Duration::from_millis(250) {
                self.save_now();
            } else {
                ctx.request_repaint_after(Duration::from_millis(60));
            }
        }
        ctx.request_repaint_after(Duration::from_millis(800));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let before = self.cfg.clone();
        egui::Panel::left("sidebar")
            .resizable(false)
            .default_size(230.0)
            .frame(egui::Frame::new().fill(Color32::from_rgb(24, 25, 30)).inner_margin(Margin::same(14)))
            .show(ui, |ui| self.sidebar(ui));
        egui::Panel::bottom("status")
            .frame(egui::Frame::new().fill(Color32::from_rgb(24, 25, 30)).inner_margin(Margin::symmetric(14, 6)))
            .show(ui, |ui| {
                ui.horizontal(|ui| match &self.status {
                    Some((t, err, at)) if at.elapsed() < Duration::from_secs(8) => {
                        let color = if *err { Color32::from_rgb(250, 140, 140) } else { ACCENT };
                        ui.label(RichText::new(t).color(color));
                    }
                    _ => {
                        ui.weak("Los cambios se guardan y aplican automáticamente.");
                    }
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::from_rgb(28, 29, 35)).inner_margin(Margin::same(18)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.page {
                    Page::General => {
                        ui.heading("General");
                        ui.add_space(8.0);
                        self.general(ui);
                    }
                    Page::Group(id) => {
                        let title = self.cfg.group(id).map(|g| g.name.clone()).unwrap_or_default();
                        ui.heading(title);
                        ui.add_space(8.0);
                        self.group_page(ui, &ctx, id);
                    }
                });
            });
        if self.cfg != before && self.cfg != self.loaded {
            self.dirty_since = Some(Instant::now());
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.dirty_since.is_some() {
            self.save_now();
        }
    }
}
