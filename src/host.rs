use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, ReleaseCapture, SetCapture, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::Shell::{DragFinish, DragQueryFileW, ShellExecuteW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::canvas::{Canvas, Image, Rgba, Text};
use crate::config::{self, Config, GroupIcon, Item, PopupStyle, SortMode, Trigger};
use crate::icons::IconLoader;
use crate::storage;
use crate::surface::Surface;
use crate::tray::Tray;
use crate::wallpaper::{Backdrop, Wallpaper};
use crate::winutil::{self, *};

const TILE_CLASS: PCWSTR = w!("Cuadros.Tile");
const POPUP_CLASS: PCWSTR = w!("Cuadros.Popup");
const TIMER_ID: usize = 1;
const TIMER_MS: u32 = 30;
const IDLE_MS: u32 = 150;
const SPRING_MS: u128 = 400;
const FADE_MS: f32 = 120.0;

//Comandos de menú

const CMD_SETTINGS: usize = 1;
const CMD_NEW_GROUP: usize = 2;
const CMD_TOGGLE_ENABLED: usize = 3;
const CMD_TOGGLE_AUTOSTART: usize = 4;
const CMD_TOGGLE_LOCK: usize = 5;
const CMD_EXIT: usize = 9;
const CMD_TOGGLE_POPUP: usize = 10;
const CMD_GROUP_SETTINGS: usize = 11;
const CMD_DELETE_GROUP: usize = 14;
const CMD_ITEM_OPEN: usize = 20;
const CMD_ITEM_LOCATION: usize = 21;
const CMD_ITEM_REMOVE: usize = 22;
const CMD_SORT_BASE: usize = 30;
const CMD_SORT_DESC: usize = 34;

enum Deferred {
    TrayMenu,
    TileMenu(u64),
    PopupMenu(u64, Option<usize>),
    ConfirmDelete(u64),
    ConfirmMissing(u64, usize),
    Message(String),
}

//Estado de cada cuadro

#[derive(Default)]
struct Cell {
    item: usize,
    x: f32,
    y: f32,
    name: Vec<String>,
    missing: bool,
}

#[derive(Default)]
struct Layout {
    scale: f32,
    w: i32,
    h: i32,
    pad: f32,
    title_h: f32,
    cell_w: f32,
    cell_h: f32,
    icon: u32,
    font: f32,
    cols: usize,
    grid_x: f32,
    grid_y: f32,
    view_h: f32,
    max_scroll: f32,
    cells: Vec<Cell>,
}

struct TileDrag {
    cursor: POINT,
    origin: POINT,
    moved: bool,
}

struct ItemDrag {
    from: usize,
    start: (i32, i32),
    at: (i32, i32),
    active: bool,
}

struct GroupWin {
    id: u64,
    tile: HWND,
    popup: HWND,
    tile_surf: Option<Surface>,
    popup_surf: Option<Surface>,
    tile_rect: RECT,
    popup_rect: RECT,
    open: bool,
    fade: f32,
    last_tick: Instant,
    hover_since: Option<Instant>,
    outside_since: Option<Instant>,
    hot: Option<usize>,
    pressed: Option<usize>,
    scroll: f32,
    layout: Layout,
    bg: Canvas,
    canvas: Canvas,
    tile_drag: Option<TileDrag>,
    item_drag: Option<ItemDrag>,
}

impl GroupWin {
    fn destroy(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.popup);
            let _ = DestroyWindow(self.tile);
        }
    }
}

//Host principal

struct Host {
    hwnd: HWND,
    cfg: Config,
    text: Text,
    wall: Wallpaper,
    icons: IconLoader,
    tray: Tray,
    groups: Vec<GroupWin>,
    timer_on: bool,
    idle_on: bool,
    modal: u32,
    pending: Vec<Deferred>,
    taskbar_created: u32,
}

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

fn with_host<R>(f: impl FnOnce(&mut Host) -> R) -> Option<R> {
    HOST.with(|h| h.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

pub fn run() -> i32 {
    let Some(_instance) = SingleInstance::acquire("Local\\Cuadros.Host") else {
        post_to_host(WM_APP_SETTINGS, 0);
        return 0;
    };
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    match start() {
        Ok(()) => {}
        Err(e) => {
            log(&e);
            error_box(&format!("Cuadros no pudo iniciar:\n{e}"));
            return 1;
        }
    }
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    HOST.with(|h| {
        if let Ok(mut g) = h.try_borrow_mut() {
            if let Some(host) = g.as_mut() {
                host.shutdown();
            }
            *g = None;
        }
    });
    0
}

fn register_class(name: PCWSTR, proc_: WNDPROC, dblclk: bool) -> Result<(), String> {
    unsafe {
        let inst = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: if dblclk { CS_DBLCLKS } else { WNDCLASS_STYLES(0) },
            lpfnWndProc: proc_,
            hInstance: inst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: name,
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return Err("No se pudo registrar la clase de ventana".into());
        }
    }
    Ok(())
}

fn start() -> Result<(), String> {
    register_class(HOST_CLASS, Some(host_proc), false)?;
    register_class(TILE_CLASS, Some(tile_proc), false)?;
    register_class(POPUP_CLASS, Some(popup_proc), true)?;
    let hwnd = unsafe {
        CreateWindowExW(WS_EX_TOOLWINDOW, HOST_CLASS, w!("Cuadros"), WS_POPUP, 0, 0, 0, 0, None, None, None, None)
            .map_err(|e| format!("No se pudo crear la ventana principal: {e}"))?
    };
    let (cfg, warning) = match config::load() {
        Ok(c) => (c, None),
        Err(e) => (Config::default(), Some(e)),
    };
    let mut host = Host {
        hwnd,
        cfg,
        text: Text::new()?,
        wall: Wallpaper::new(),
        icons: IconLoader::new(hwnd),
        tray: Tray::new(hwnd),
        groups: Vec::new(),
        timer_on: false,
        idle_on: false,
        modal: 0,
        pending: Vec::new(),
        taskbar_created: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
    };
    host.tray.add();
    winutil::refresh_autostart_path();
    let first_run = host.cfg.groups.is_empty() && !config::config_path().exists();
    if first_run {
        let (wa, _) = work_area_at(POINT { x: 0, y: 0 });
        host.cfg.new_group(wa.left + 40, wa.top + 40);
        host.save();
    }
    host.sync_windows();
    host.update_tip();
    host.idle_timer();
    if first_run {
        host.tray.balloon("Cuadros está activo", "Arrastra archivos o accesos directos sobre el cuadro. Doble clic en este icono abre la configuración.", false);
    }
    if let Some(w) = warning {
        host.tray.balloon("Configuración restaurada", &w, true);
        log(&w);
    }
    HOST.with(|h| *h.borrow_mut() = Some(host));
    Ok(())
}

//Procedimientos de ventana

fn group_of(hwnd: HWND) -> u64 {
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as u64 }
}

fn finish(res: Option<Option<LRESULT>>, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    drain_deferred();
    match res {
        Some(Some(r)) => r,
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

unsafe extern "system" fn host_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_DESTROY {
        PostQuitMessage(0);
        return LRESULT(0);
    }
    let res = with_host(|h| h.on_host(msg, wp, lp));
    finish(res, hwnd, msg, wp, lp)
}

unsafe extern "system" fn tile_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_WINDOWPOSCHANGING => {
            let pos = &mut *(lp.0 as *mut WINDOWPOS);
            if !pos.flags.contains(SWP_NOZORDER) {
                pos.hwndInsertAfter = HWND_BOTTOM;
            }
            return LRESULT(0);
        }
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        _ => {}
    }
    let gid = group_of(hwnd);
    let res = with_host(|h| h.on_tile(gid, msg, wp, lp));
    finish(res, hwnd, msg, wp, lp)
}

unsafe extern "system" fn popup_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_MOUSEACTIVATE {
        return LRESULT(MA_NOACTIVATE as isize);
    }
    let gid = group_of(hwnd);
    let res = with_host(|h| h.on_popup(gid, msg, wp, lp));
    finish(res, hwnd, msg, wp, lp)
}

//Acciones diferidas (menús y diálogos modales fuera del préstamo del estado)

fn drain_deferred() {
    while let Some(Some(d)) = with_host(|h| h.pending.pop()) {
        run_deferred(d);
    }
}

fn cursor() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}

struct MenuBuilder(HMENU);

impl MenuBuilder {
    fn new() -> Option<Self> {
        unsafe { CreatePopupMenu().ok().map(Self) }
    }
    fn item(&self, id: usize, text: &str, checked: bool, enabled: bool) -> &Self {
        let mut flags = MF_STRING;
        if checked {
            flags |= MF_CHECKED;
        }
        if !enabled {
            flags |= MF_GRAYED;
        }
        unsafe {
            let _ = AppendMenuW(self.0, flags, id, &hs(text));
        }
        self
    }
    fn sub(&self, text: &str, sub: MenuBuilder) -> &Self {
        unsafe {
            let _ = AppendMenuW(self.0, MF_POPUP, sub.0 .0 as usize, &hs(text));
        }
        std::mem::forget(sub);
        self
    }
    fn sep(&self) -> &Self {
        unsafe {
            let _ = AppendMenuW(self.0, MF_SEPARATOR, 0, PCWSTR::null());
        }
        self
    }
    fn default_item(&self, id: usize) -> &Self {
        unsafe {
            let _ = SetMenuDefaultItem(self.0, id as u32, 0);
        }
        self
    }
    fn track(&self, owner: HWND) -> usize {
        let p = cursor();
        with_host(|h| h.modal += 1);
        let cmd = unsafe {
            let _ = SetForegroundWindow(owner);
            let r = TrackPopupMenu(self.0, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, p.x, p.y, None, owner, None);
            let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
            r.0 as usize
        };
        with_host(|h| h.modal = h.modal.saturating_sub(1));
        cmd
    }
}

impl Drop for MenuBuilder {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.0);
        }
    }
}

fn run_deferred(d: Deferred) {
    match d {
        Deferred::Message(text) => warn_box(&text),
        Deferred::TrayMenu => {
            let Some((owner, enabled, locked)) = with_host(|h| (h.hwnd, h.cfg.enabled, h.cfg.locked)) else { return };
            let Some(m) = MenuBuilder::new() else { return };
            m.item(CMD_SETTINGS, "Configuración…", false, true)
                .default_item(CMD_SETTINGS)
                .item(CMD_NEW_GROUP, "Nuevo cuadro", false, enabled)
                .sep()
                .item(CMD_TOGGLE_ENABLED, "Activado", enabled, true)
                .item(CMD_TOGGLE_AUTOSTART, "Iniciar con Windows", autostart_enabled(), true)
                .item(CMD_TOGGLE_LOCK, "Bloquear posición de los cuadros", locked, true)
                .sep()
                .item(CMD_EXIT, "Salir", false, true);
            let cmd = m.track(owner);
            command(cmd, None, None);
        }
        Deferred::TileMenu(gid) => {
            let Some((owner, open, locked)) = with_host(|h| (h.hwnd, h.win(gid).map(|g| g.open).unwrap_or(false), h.cfg.locked)) else { return };
            let Some(m) = MenuBuilder::new() else { return };
            m.item(CMD_TOGGLE_POPUP, if open { "Cerrar" } else { "Abrir" }, false, true)
                .default_item(CMD_TOGGLE_POPUP)
                .item(CMD_GROUP_SETTINGS, "Configurar este cuadro…", false, true)
                .sep()
                .item(CMD_NEW_GROUP, "Nuevo cuadro", false, true)
                .item(CMD_TOGGLE_LOCK, "Bloquear posición", locked, true)
                .sep()
                .item(CMD_DELETE_GROUP, "Eliminar cuadro", false, true);
            let cmd = m.track(owner);
            command(cmd, Some(gid), None);
        }
        Deferred::PopupMenu(gid, item) => {
            let Some((owner, sort, desc, stored)) = with_host(|h| {
                let g = h.cfg.group(gid);
                let sort = g.map(|g| g.popup.sort).unwrap_or(SortMode::Manual);
                let desc = g.map(|g| g.popup.sort_desc).unwrap_or(false);
                let stored = item.and_then(|i| g.and_then(|g| g.items.get(i))).map(|it| it.origin.is_some()).unwrap_or(false);
                (h.hwnd, sort, desc, stored)
            }) else {
                return;
            };
            let Some(m) = MenuBuilder::new() else { return };
            if item.is_some() {
                m.item(CMD_ITEM_OPEN, "Abrir", false, true)
                    .default_item(CMD_ITEM_OPEN)
                    .item(CMD_ITEM_LOCATION, "Abrir ubicación", false, true)
                    .sep()
                    .item(CMD_ITEM_REMOVE, if stored { "Devolver a su ubicación original" } else { "Quitar del cuadro" }, false, true)
                    .sep();
            }
            if let Some(s) = MenuBuilder::new() {
                let modes = [(SortMode::Manual, "Manual"), (SortMode::Name, "Nombre"), (SortMode::Kind, "Tipo"), (SortMode::Added, "Fecha de agregado")];
                for (i, (mode, label)) in modes.iter().enumerate() {
                    s.item(CMD_SORT_BASE + i, label, sort == *mode, true);
                }
                s.sep().item(CMD_SORT_DESC, "Orden descendente", desc, true);
                m.sub("Ordenar por", s);
            }
            m.item(CMD_GROUP_SETTINGS, "Configurar este cuadro…", false, true);
            let cmd = m.track(owner);
            command(cmd, Some(gid), item);
        }
        Deferred::ConfirmDelete(gid) => {
            let name = with_host(|h| h.cfg.group(gid).map(|g| g.name.clone())).flatten().unwrap_or_default();
            let moved = with_host(|h| h.cfg.group(gid).map(|g| g.items.iter().any(|i| i.origin.is_some()))).flatten().unwrap_or(false);
            let extra = if moved { "\nLos archivos guardados en el cuadro se devolverán a su ubicación original." } else { "" };
            if confirm_box(&format!("¿Eliminar el cuadro \"{name}\"?{extra}")) {
                with_host(|h| h.delete_group(gid));
            }
        }
        Deferred::ConfirmMissing(gid, idx) => {
            if confirm_box("Este elemento ya no existe en su ubicación. ¿Quitarlo del cuadro?") {
                with_host(|h| h.remove_item(gid, idx));
            }
        }
    }
}

fn command(cmd: usize, gid: Option<u64>, item: Option<usize>) {
    with_host(|h| h.command(cmd, gid, item));
}

//Lógica del host

impl Host {
    fn win(&self, gid: u64) -> Option<&GroupWin> {
        self.groups.iter().find(|g| g.id == gid)
    }

    fn win_index(&self, gid: u64) -> Option<usize> {
        self.groups.iter().position(|g| g.id == gid)
    }

    fn save(&mut self) {
        if let Err(e) = config::save(&self.cfg) {
            log(&e);
            self.tray.balloon("No se pudo guardar", &e, true);
        }
    }

    fn update_tip(&self) {
        let n = self.cfg.groups.len();
        let state = if self.cfg.enabled { "activo" } else { "desactivado" };
        self.tray.set_tip(&format!("Cuadros — {n} cuadro(s), {state}"));
    }

    fn shutdown(&mut self) {
        for g in &mut self.groups {
            g.destroy();
        }
        self.groups.clear();
        self.tray.remove();
    }

    fn quit(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_ID);
        }
        self.shutdown();
        unsafe { PostQuitMessage(0) };
    }

    fn ensure_timer(&mut self) {
        if !self.timer_on {
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_ID, TIMER_MS, None);
            }
            self.timer_on = true;
        }
    }

    fn idle_timer(&mut self) {
        if self.timer_on || !self.idle_on {
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_ID, IDLE_MS, None);
            }
            self.timer_on = false;
            self.idle_on = true;
        }
    }

    fn open_settings(&self, gid: Option<u64>) {
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => return error_box(&format!("No se pudo ubicar el ejecutable: {e}")),
        };
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--settings");
        if let Some(g) = gid {
            cmd.arg("--group").arg(g.to_string());
        }
        unsafe {
            let _ = AllowSetForegroundWindow(u32::MAX);
        }
        if let Err(e) = cmd.spawn() {
            error_box(&format!("No se pudo abrir la configuración: {e}"));
        }
    }

    fn new_group_near(&mut self, anchor: Option<u64>) {
        let (x, y) = match anchor.and_then(|a| self.win(a)) {
            Some(g) => (g.tile_rect.right + 16, g.tile_rect.top),
            None => {
                let (wa, _) = work_area_at(POINT { x: 0, y: 0 });
                let k = self.cfg.groups.len() as i32;
                (wa.left + 40 + k * 24, wa.top + 40 + k * 24)
            }
        };
        let (wa, _) = work_area_at(POINT { x, y });
        let x = x.min(wa.right - 96).max(wa.left);
        let y = y.min(wa.bottom - 96).max(wa.top);
        self.cfg.enabled = true;
        self.cfg.new_group(x, y);
        self.save();
        self.sync_windows();
        self.update_tip();
    }

    fn command(&mut self, cmd: usize, gid: Option<u64>, item: Option<usize>) {
        match cmd {
            0 => {}
            CMD_SETTINGS => self.open_settings(None),
            CMD_NEW_GROUP => self.new_group_near(gid),
            CMD_TOGGLE_ENABLED => {
                self.cfg.enabled = !self.cfg.enabled;
                self.save();
                self.sync_windows();
                self.update_tip();
            }
            CMD_TOGGLE_AUTOSTART => {
                if let Err(e) = set_autostart(!autostart_enabled()) {
                    self.pending.push(Deferred::Message(e));
                }
            }
            CMD_TOGGLE_LOCK => {
                self.cfg.locked = !self.cfg.locked;
                self.save();
            }
            CMD_EXIT => self.quit(),
            CMD_TOGGLE_POPUP => {
                if let Some(g) = gid {
                    self.toggle_popup(g);
                }
            }
            CMD_GROUP_SETTINGS => self.open_settings(gid),
            CMD_DELETE_GROUP => {
                if let Some(g) = gid {
                    self.pending.push(Deferred::ConfirmDelete(g));
                }
            }
            CMD_ITEM_OPEN => {
                if let (Some(g), Some(i)) = (gid, item) {
                    self.open_item(g, i);
                }
            }
            CMD_ITEM_LOCATION => {
                if let Some(path) = gid.zip(item).and_then(|(g, i)| self.cfg.group(g)?.items.get(i)).map(|it| it.path.clone()) {
                    let params = hs(&format!("/select,\"{path}\""));
                    unsafe {
                        ShellExecuteW(None, w!("open"), w!("explorer.exe"), &params, PCWSTR::null(), SW_SHOWNORMAL);
                    }
                }
            }
            CMD_ITEM_REMOVE => {
                if let (Some(g), Some(i)) = (gid, item) {
                    self.remove_item(g, i);
                }
            }
            c if (CMD_SORT_BASE..CMD_SORT_BASE + 4).contains(&c) || c == CMD_SORT_DESC => {
                if let Some(g) = gid.and_then(|g| self.cfg.group_mut(g)) {
                    if c == CMD_SORT_DESC {
                        g.popup.sort_desc = !g.popup.sort_desc;
                    } else {
                        g.popup.sort = [SortMode::Manual, SortMode::Name, SortMode::Kind, SortMode::Added][c - CMD_SORT_BASE];
                    }
                    self.save();
                    if let Some(g) = gid {
                        self.refresh_popup(g, true);
                    }
                }
            }
            _ => {}
        }
    }

    //Sincronización ventanas ↔ configuración

    fn sync_windows(&mut self) {
        let ids: HashSet<u64> = self.cfg.groups.iter().map(|g| g.id).collect();
        let mut i = 0;
        while i < self.groups.len() {
            if !ids.contains(&self.groups[i].id) {
                let mut g = self.groups.remove(i);
                g.destroy();
            } else {
                i += 1;
            }
        }
        let owner = desktop_owner();
        let existing: Vec<u64> = self.groups.iter().map(|g| g.id).collect();
        for gid in self.cfg.groups.iter().map(|g| g.id).filter(|id| !existing.contains(id)).collect::<Vec<_>>() {
            match create_group_windows(gid, owner) {
                Ok(gw) => self.groups.push(gw),
                Err(e) => {
                    log(&e);
                    self.tray.balloon("Error al crear un cuadro", &e, true);
                }
            }
        }
        let keep: HashSet<String> = self.cfg.groups.iter().flat_map(|g| g.items.iter().map(|i| i.path.clone())).collect();
        self.icons.retain_paths(&keep);
        let enabled = self.cfg.enabled;
        for idx in 0..self.groups.len() {
            let gid = self.groups[idx].id;
            if enabled {
                self.render_tile(gid);
                unsafe {
                    let _ = ShowWindow(self.groups[idx].tile, SW_SHOWNOACTIVATE);
                }
                if self.groups[idx].open {
                    self.refresh_popup(gid, true);
                }
            } else {
                self.close_popup(gid);
                unsafe {
                    let _ = ShowWindow(self.groups[idx].tile, SW_HIDE);
                }
            }
        }
    }

    fn reattach_owner(&mut self) {
        let owner = desktop_owner();
        for g in &self.groups {
            unsafe {
                SetWindowLongPtrW(g.tile, GWLP_HWNDPARENT, owner.map(|o| o.0 as isize).unwrap_or(0));
            }
        }
    }

    fn reload(&mut self) {
        match config::load() {
            Ok(c) => self.cfg = c,
            Err(e) => {
                log(&e);
                self.tray.balloon("Configuración inválida", &e, true);
                return;
            }
        }
        self.icons.clear_custom();
        self.wall.refresh();
        self.sync_windows();
        self.update_tip();
    }

    //Mensajes de la ventana principal

    fn on_host(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_APP_TRAY => {
                match lp.0 as u32 {
                    WM_LBUTTONDBLCLK => self.open_settings(None),
                    WM_RBUTTONUP | WM_CONTEXTMENU => self.pending.push(Deferred::TrayMenu),
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_APP_RELOAD => {
                self.reload();
                Some(LRESULT(0))
            }
            WM_APP_SETTINGS => {
                self.open_settings(None);
                Some(LRESULT(0))
            }
            WM_APP_ICON_READY => {
                if self.icons.collect() {
                    let open: Vec<u64> = self.groups.iter().filter(|g| g.open).map(|g| g.id).collect();
                    for g in open {
                        self.render_popup(g);
                    }
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_ID => {
                self.tick();
                Some(LRESULT(0))
            }
            WM_SETTINGCHANGE | WM_DISPLAYCHANGE | WM_DPICHANGED => {
                self.wall.refresh();
                let ids: Vec<u64> = self.groups.iter().map(|g| g.id).collect();
                for g in ids {
                    self.render_tile(g);
                    self.refresh_popup(g, true);
                }
                None
            }
            WM_QUERYENDSESSION => Some(LRESULT(1)),
            WM_ENDSESSION if wp.0 != 0 => {
                self.quit();
                Some(LRESULT(0))
            }
            WM_CLOSE => {
                self.quit();
                Some(LRESULT(0))
            }
            m if m == self.taskbar_created && m != 0 => {
                self.tray.add();
                self.update_tip();
                self.groups.retain(|g| unsafe { IsWindow(Some(g.tile)).as_bool() && IsWindow(Some(g.popup)).as_bool() });
                self.reattach_owner();
                self.sync_windows();
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    //Temporizador: hover, cierre automático y animación

    fn tick(&mut self) {
        let p = cursor();
        let now = Instant::now();
        let hover_delay = self.cfg.hover_delay_ms as u128;
        let close_delay = self.cfg.close_delay_ms as u128;
        let trigger = self.cfg.trigger;
        let lbutton = unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 };
        let buttons = lbutton || unsafe { (GetAsyncKeyState(VK_RBUTTON.0 as i32) as u16 & 0x8000) != 0 };
        let enabled = self.cfg.enabled;
        let modal = self.modal > 0;
        let mut to_open = None;
        let mut to_close = Vec::new();
        let mut to_fade = Vec::new();
        let mut to_cool = Vec::new();
        for g in &mut self.groups {
            let in_tile = contains(&g.tile_rect, p);
            let in_popup = g.open && contains(&inflate(&g.popup_rect, 2), p);
            let dragging_in = enabled && lbutton && in_tile && !g.open && g.tile_drag.is_none();
            if dragging_in && g.hover_since.is_none() {
                g.hover_since = Some(now);
            }
            let delay = if lbutton { hover_delay.max(SPRING_MS) } else { hover_delay };
            if let Some(since) = g.hover_since {
                if !in_tile || g.tile_drag.is_some() || (trigger == Trigger::Click && !lbutton) {
                    g.hover_since = None;
                } else if !g.open && now.duration_since(since).as_millis() >= delay {
                    g.hover_since = None;
                    to_open = Some(g.id);
                }
            }
            if !g.open {
                continue;
            }
            if g.fade < 1.0 {
                let dt = now.duration_since(g.last_tick).as_secs_f32() * 1000.0;
                g.fade = (g.fade + dt / FADE_MS).min(1.0);
                to_fade.push(g.id);
            }
            g.last_tick = now;
            let busy = modal || g.item_drag.is_some() || g.pressed.is_some();
            match trigger {
                Trigger::Hover => {
                    if in_tile || in_popup || busy {
                        g.outside_since = None;
                    } else {
                        let since = *g.outside_since.get_or_insert(now);
                        if now.duration_since(since).as_millis() >= close_delay {
                            to_close.push(g.id);
                        }
                    }
                }
                Trigger::Click => {
                    if buttons && !in_tile && !in_popup && !busy {
                        to_close.push(g.id);
                    }
                }
            }
            if !in_popup && g.hot.is_some() && g.item_drag.is_none() {
                g.hot = None;
                to_cool.push(g.id);
            }
        }
        for gid in to_close {
            self.close_popup(gid);
        }
        for gid in to_cool {
            self.render_popup(gid);
        }
        for gid in to_fade {
            if let Some(g) = self.win(gid) {
                if let Some(s) = &g.popup_surf {
                    let _ = s.update(g.popup, g.popup_rect.left, g.popup_rect.top, (g.fade * 255.0) as u8);
                }
            }
        }
        if let Some(gid) = to_open {
            self.open_popup(gid);
            self.ensure_timer();
        }
        if self.groups.iter().any(|g| g.hover_since.is_some()) {
            self.ensure_timer();
        }
        let needed = self.groups.iter().any(|g| g.open || g.hover_since.is_some());
        if !needed {
            self.idle_timer();
        }
    }

    //Apertura y cierre del cuadro desplegable

    fn toggle_popup(&mut self, gid: u64) {
        if self.win(gid).map(|g| g.open).unwrap_or(false) {
            self.close_popup(gid);
        } else {
            self.open_popup(gid);
        }
    }

    fn open_popup(&mut self, gid: u64) {
        if !self.cfg.enabled {
            return;
        }
        let others: Vec<u64> = self.groups.iter().filter(|g| g.open && g.id != gid).map(|g| g.id).collect();
        for o in others {
            self.close_popup(o);
        }
        self.wall.refresh();
        let animations = self.cfg.animations;
        let Some(idx) = self.win_index(gid) else { return };
        {
            let g = &mut self.groups[idx];
            g.open = true;
            g.fade = if animations { 0.0 } else { 1.0 };
            g.last_tick = Instant::now();
            g.outside_since = None;
            g.hot = None;
            g.pressed = None;
            g.scroll = 0.0;
        }
        self.refresh_popup(gid, true);
        let g = &self.groups[idx];
        unsafe {
            let _ = SetWindowPos(g.popup, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
        self.render_tile(gid);
        self.ensure_timer();
    }

    fn close_popup(&mut self, gid: u64) {
        let Some(idx) = self.win_index(gid) else { return };
        let g = &mut self.groups[idx];
        if !g.open {
            return;
        }
        g.open = false;
        g.hot = None;
        g.pressed = None;
        g.item_drag = None;
        g.outside_since = None;
        unsafe {
            let _ = ShowWindow(g.popup, SW_HIDE);
        }
        g.popup_surf = None;
        g.bg = Canvas::new(1, 1);
        g.canvas = Canvas::new(1, 1);
        self.text.trim_cache();
        self.render_tile(gid);
    }

    fn refresh_popup(&mut self, gid: u64, relayout: bool) {
        let Some(idx) = self.win_index(gid) else { return };
        if !self.groups[idx].open {
            return;
        }
        if relayout {
            self.layout_popup(idx);
            self.paint_popup_bg(idx);
        }
        self.render_popup(gid);
    }

    //Distribución de celdas

    fn layout_popup(&mut self, idx: usize) {
        let gid = self.groups[idx].id;
        let Some(group) = self.cfg.group(gid) else { return };
        let st = &group.popup;
        let dpi = dpi_of(self.groups[idx].tile);
        let m = desktop_metrics(dpi);
        let s = m.scale;
        let cell_w = if st.cell_w > 0 { st.cell_w as f32 * s } else { m.cell_w as f32 };
        let cell_h = if st.cell_h > 0 { st.cell_h as f32 * s } else { m.cell_h as f32 };
        let icon = if st.icon_size > 0 { (st.icon_size as f32 * s) as u32 } else { m.icon };
        let font = if st.font_px > 0.0 { st.font_px * s } else { m.font_px };
        let pad = (10.0 * s).round();
        let title_h = if st.show_title { (font * 1.5 + 12.0 * s).round() } else { 0.0 };
        let order = group.sorted_indices();
        let n = order.len();
        let tile_rect = self.groups[idx].tile_rect;
        let (wa, _) = work_area_at(POINT { x: (tile_rect.left + tile_rect.right) / 2, y: (tile_rect.top + tile_rect.bottom) / 2 });
        let max_w = (wa.right - wa.left) as f32 - 2.0 * pad;
        let max_h = (wa.bottom - wa.top) as f32 * 0.85;
        let cols = if st.columns > 0 {
            st.columns as usize
        } else if st.width > 0 {
            (((st.width as f32 * s) - 2.0 * pad) / cell_w).floor().max(1.0) as usize
        } else {
            ((n.max(1) as f32 * 1.6).sqrt().ceil() as usize).clamp(3, 8)
        };
        let cols = cols.min(((max_w - 2.0 * pad) / cell_w).floor().max(1.0) as usize).max(1);
        let rows = n.div_ceil(cols).max(1);
        let grid_w = cols as f32 * cell_w;
        let w = if st.width > 0 { (st.width as f32 * s).max(grid_w.min(max_w) + 2.0 * pad) } else { grid_w + 2.0 * pad };
        let content_h = rows as f32 * cell_h;
        let h = if st.height > 0 { st.height as f32 * s } else { title_h + content_h + 2.0 * pad };
        let h = h.clamp(title_h + cell_h * 0.6 + 2.0 * pad, max_h);
        let view_h = h - title_h - 2.0 * pad;
        let mut cells = Vec::with_capacity(n);
        let label_w = cell_w - 6.0 * s;
        for (vi, &item_idx) in order.iter().enumerate() {
            let it = &group.items[item_idx];
            let col = vi % cols;
            let row = vi / cols;
            let name = self.text.wrap(&it.display_name(), font, label_w, 2);
            cells.push(Cell { item: item_idx, x: col as f32 * cell_w, y: row as f32 * cell_h, name, missing: !Path::new(&it.path).exists() });
        }
        let g = &mut self.groups[idx];
        g.layout = Layout {
            scale: s,
            w: w.round() as i32,
            h: h.round() as i32,
            pad,
            title_h,
            cell_w,
            cell_h,
            icon,
            font,
            cols,
            grid_x: ((w - grid_w) * 0.5).max(pad).round(),
            grid_y: title_h + pad,
            view_h,
            max_scroll: (content_h - view_h).max(0.0),
            cells,
        };
        g.scroll = g.scroll.clamp(0.0, g.layout.max_scroll);
        g.popup_rect = place_popup(&tile_rect, g.layout.w, g.layout.h, (6.0 * s) as i32, &wa);
    }

    fn backdrop_for(&mut self, rect: &RECT, st: &PopupStyle) -> Option<(Rc<Backdrop>, RECT)> {
        if st.blur == 0 {
            return None;
        }
        let (_, mon) = work_area_at(POINT { x: (rect.left + rect.right) / 2, y: (rect.top + rect.bottom) / 2 });
        Some((self.wall.backdrop(mon.right - mon.left, mon.bottom - mon.top, st.blur), mon))
    }

    fn paint_popup_bg(&mut self, idx: usize) {
        let gid = self.groups[idx].id;
        let Some(group) = self.cfg.group(gid).cloned() else { return };
        let st = &group.popup;
        let rect = self.groups[idx].popup_rect;
        let bd = self.backdrop_for(&rect, st);
        let g = &mut self.groups[idx];
        let l = &g.layout;
        g.bg.resize(l.w, l.h);
        g.bg.clear(0);
        let s = l.scale;
        draw_panel(&mut g.bg, 0.0, 0.0, l.w as f32, l.h as f32, st, s, (rect.left, rect.top), bd.as_ref());
        if st.show_title {
            let color = Rgba::from_u8(st.text_color);
            let ty = ((l.title_h - self.text.line_height(l.font * 1.05)) * 0.5 + 3.0 * s).max(0.0);
            let title = self.text.ellipsize(&group.name, l.font * 1.05, l.w as f32 - 2.0 * l.pad - 40.0 * s);
            self.text.draw(&mut g.bg, &title, l.pad + 4.0 * s, ty, l.font * 1.05, color, st.text_shadow);
            let count = group.items.len().to_string();
            let cw = self.text.measure(&count, l.font * 0.9);
            self.text.draw(&mut g.bg, &count, l.w as f32 - l.pad - 4.0 * s - cw, ty + l.font * 0.1, l.font * 0.9, color.with_alpha(0.55), false);
            let sep = Rgba::from_u8(st.outline).with_alpha(0.6);
            g.bg.fill_rrect(l.pad, l.title_h - 1.0 * s, l.w as f32 - 2.0 * l.pad, 1.0 * s.max(1.0), 0.0, sep);
        }
    }

    fn render_popup(&mut self, gid: u64) {
        let Some(idx) = self.win_index(gid) else { return };
        if !self.groups[idx].open {
            return;
        }
        let Some(group) = self.cfg.group(gid) else { return };
        let st = group.popup.clone();
        let manual = st.sort == SortMode::Manual;
        let paths: Vec<String> = self.groups[idx].layout.cells.iter().map(|c| group.items.get(c.item).map(|i| i.path.clone()).unwrap_or_default()).collect();
        let icon_px = self.groups[idx].layout.icon;
        let icons: Vec<Option<Rc<Image>>> = paths.iter().map(|p| self.icons.get(p, icon_px)).collect();
        let empty_text = "Arrastra aquí archivos, carpetas o accesos directos";
        let g = &mut self.groups[idx];
        let l = &g.layout;
        let s = l.scale;
        g.canvas.resize(l.w, l.h);
        g.canvas.copy_from(&g.bg);
        let color = Rgba::from_u8(st.text_color);
        let clip_top = l.grid_y as i32;
        let clip_bottom = (l.grid_y + l.view_h) as i32;
        g.canvas.set_clip(0, clip_top, l.w, clip_bottom);
        let drag = g.item_drag.as_ref().filter(|d| d.active);
        for (vi, cell) in l.cells.iter().enumerate() {
            let x = l.grid_x + cell.x;
            let y = l.grid_y + cell.y - g.scroll;
            if y + l.cell_h < clip_top as f32 || y > clip_bottom as f32 {
                continue;
            }
            let dragged = drag.map(|d| d.from == vi).unwrap_or(false);
            let alpha = if cell.missing { 0.4 } else if dragged { 0.3 } else { 1.0 };
            if g.hot == Some(vi) && drag.is_none() {
                let a = if g.pressed == Some(vi) { 0.24 } else { 0.14 };
                g.canvas.fill_rrect(x + 2.0 * s, y + 2.0 * s, l.cell_w - 4.0 * s, l.cell_h - 4.0 * s, 4.0 * s, Rgba::new(1.0, 1.0, 1.0, a));
                g.canvas.stroke_rrect(x + 2.0 * s, y + 2.0 * s, l.cell_w - 4.0 * s, l.cell_h - 4.0 * s, 4.0 * s, 1.0, Rgba::new(1.0, 1.0, 1.0, 0.18));
            }
            draw_cell(&mut g.canvas, &mut self.text, cell, icons[vi].as_deref(), x, y, l, color, st.text_shadow, alpha);
        }
        if l.cells.is_empty() {
            let lines = self.text.wrap(empty_text, l.font, l.w as f32 - 2.0 * l.pad - 8.0 * s, 3);
            let lh = self.text.line_height(l.font);
            let mut ty = l.grid_y + (l.view_h - lh * lines.len() as f32) * 0.5;
            for line in lines {
                self.text.draw_centered(&mut g.canvas, &line, l.w as f32 * 0.5, ty, l.font, color.with_alpha(0.7), st.text_shadow);
                ty += lh;
            }
        }
        if let Some(d) = drag {
            if manual {
                let t = insertion_index(l, g.scroll, d.at);
                let col = t % l.cols;
                let row = t / l.cols;
                let (bx, by) = if col == 0 && t > 0 && t == l.cells.len() && l.cells.len().is_multiple_of(l.cols) {
                    (l.grid_x + l.cols as f32 * l.cell_w, l.grid_y + (row as f32 - 1.0) * l.cell_h - g.scroll)
                } else {
                    (l.grid_x + col as f32 * l.cell_w, l.grid_y + row as f32 * l.cell_h - g.scroll)
                };
                g.canvas.fill_rrect(bx - 1.5 * s, by + 6.0 * s, 3.0 * s, l.cell_h - 12.0 * s, 1.5 * s, Rgba::new(1.0, 1.0, 1.0, 0.85));
            }
            g.canvas.reset_clip();
            if let Some(cell) = l.cells.get(d.from) {
                let x = d.at.0 as f32 - l.cell_w * 0.5;
                let y = d.at.1 as f32 - l.icon as f32 * 0.5 - 6.0 * s;
                draw_cell(&mut g.canvas, &mut self.text, cell, icons[d.from].as_deref(), x, y, l, color, st.text_shadow, 0.85);
            }
        }
        g.canvas.reset_clip();
        if l.max_scroll > 0.0 {
            let track = l.view_h - 8.0 * s;
            let thumb = (track * l.view_h / (l.view_h + l.max_scroll)).max(24.0 * s);
            let ty = l.grid_y + 4.0 * s + (track - thumb) * (g.scroll / l.max_scroll);
            g.canvas.fill_rrect(l.w as f32 - 7.0 * s, ty, 3.0 * s, thumb, 1.5 * s, Rgba::new(1.0, 1.0, 1.0, 0.35));
        }
        let r = g.popup_rect;
        if let Err(e) = Surface::present(&mut g.popup_surf, g.popup, &g.canvas, r.left, r.top, (g.fade * 255.0) as u8) {
            log(&e);
        }
    }

    //Icono pequeño del escritorio

    fn render_tile(&mut self, gid: u64) {
        let Some(idx) = self.win_index(gid) else { return };
        let Some(group) = self.cfg.group(gid).cloned() else { return };
        let hwnd = self.groups[idx].tile;
        let m = desktop_metrics(dpi_of(hwnd));
        let s = m.scale;
        let st = &group.popup;
        let font = if st.font_px > 0.0 { st.font_px * s } else { m.font_px };
        let bx = (group.tile_size as f32 * s).round();
        let lh = self.text.line_height(font);
        let label_w = bx.max(m.cell_w as f32) - 4.0 * s;
        let lines = if group.show_tile_label { self.text.wrap(&group.name, font, label_w, 2) } else { Vec::new() };
        let w = (bx.max(label_w + 4.0 * s) + 4.0).round() as i32;
        let h = (bx + 4.0 + if lines.is_empty() { 0.0 } else { 4.0 * s + lh * lines.len() as f32 }).round() as i32;
        let (x, y) = (group.x, group.y);
        let rect = RECT { left: x, top: y, right: x + w, bottom: y + h };
        let bd = self.backdrop_for(&rect, st);
        let custom = match &group.icon {
            GroupIcon::Image(f) => self.icons.custom(f),
            GroupIcon::Preset(_) => None,
        };
        let open = self.groups[idx].open;
        let mut cv = Canvas::new(w, h);
        cv.clear(0x0100_0000);
        let ox = ((w as f32 - bx) * 0.5).round();
        draw_panel(&mut cv, ox, 2.0, bx, bx, st, s, (x, y), bd.as_ref());
        if open {
            cv.fill_rrect(ox, 2.0, bx, bx, st.corner_radius * s, Rgba::new(1.0, 1.0, 1.0, 0.10));
        }
        let color = Rgba::from_u8(st.text_color);
        let (cx, cy) = (ox + bx * 0.5, 2.0 + bx * 0.5);
        match (&group.icon, custom) {
            (_, Some(img)) => {
                let side = bx * 0.66;
                let (iw, ih) = fit(img.w as f32, img.h as f32, side);
                cv.draw_image(&img, (cx - iw * 0.5).round(), (cy - ih * 0.5).round(), iw, ih, 1.0);
            }
            (GroupIcon::Preset(key), None) if self.text.has_symbols() => {
                let p = crate::presets::find(key);
                self.text.draw_symbol(&mut cv, p.glyph, cx, cy, bx * 0.42, color, st.text_shadow);
            }
            _ => {
                let letter: String = group.name.chars().next().unwrap_or('C').to_uppercase().collect();
                let fs = bx * 0.42;
                let lh2 = self.text.line_height(fs);
                self.text.draw_centered(&mut cv, &letter, cx, cy - lh2 * 0.5, fs, color, st.text_shadow);
            }
        }
        let mut ty = 2.0 + bx + 4.0 * s;
        for line in &lines {
            self.text.draw_centered(&mut cv, line, w as f32 * 0.5, ty, font, color, st.text_shadow);
            ty += lh;
        }
        let g = &mut self.groups[idx];
        g.tile_rect = rect;
        if let Err(e) = Surface::present(&mut g.tile_surf, hwnd, &cv, x, y, 255) {
            log(&e);
        }
    }

    //Mensajes del icono del escritorio

    fn on_tile(&mut self, gid: u64, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let idx = self.win_index(gid)?;
        match msg {
            WM_MOUSEMOVE => {
                let locked = self.cfg.locked;
                let hover = self.cfg.trigger == Trigger::Hover;
                let g = &mut self.groups[idx];
                let mut close = false;
                let mut start_timer = false;
                if let Some(d) = &mut g.tile_drag {
                    let p = cursor();
                    let s = dpi_of(g.tile) as f32 / 96.0;
                    let threshold = (5.0 * s) as i32;
                    if !d.moved && !locked && ((p.x - d.cursor.x).abs() > threshold || (p.y - d.cursor.y).abs() > threshold) {
                        d.moved = true;
                    }
                    if d.moved {
                        let nx = d.origin.x + p.x - d.cursor.x;
                        let ny = d.origin.y + p.y - d.cursor.y;
                        let (w, h) = (g.tile_rect.right - g.tile_rect.left, g.tile_rect.bottom - g.tile_rect.top);
                        g.tile_rect = RECT { left: nx, top: ny, right: nx + w, bottom: ny + h };
                        unsafe {
                            let _ = SetWindowPos(g.tile, None, nx, ny, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                        }
                        close = g.open;
                    }
                } else if hover && !g.open && g.hover_since.is_none() {
                    g.hover_since = Some(Instant::now());
                    start_timer = true;
                }
                if close {
                    self.close_popup(gid);
                }
                if start_timer {
                    self.ensure_timer();
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                let g = &mut self.groups[idx];
                g.hover_since = None;
                g.tile_drag = Some(TileDrag { cursor: cursor(), origin: POINT { x: g.tile_rect.left, y: g.tile_rect.top }, moved: false });
                unsafe {
                    SetCapture(g.tile);
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let drag = self.groups[idx].tile_drag.take();
                unsafe {
                    let _ = ReleaseCapture();
                }
                match drag {
                    Some(d) if d.moved => {
                        let r = self.groups[idx].tile_rect;
                        if let Some(gc) = self.cfg.group_mut(gid) {
                            gc.x = r.left;
                            gc.y = r.top;
                        }
                        self.save();
                        self.render_tile(gid);
                    }
                    Some(_) => match self.cfg.trigger {
                        Trigger::Click => self.toggle_popup(gid),
                        Trigger::Hover if !self.groups[idx].open => self.open_popup(gid),
                        Trigger::Hover => {}
                    },
                    None => {}
                }
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                self.pending.push(Deferred::TileMenu(gid));
                Some(LRESULT(0))
            }
            WM_DROPFILES => {
                self.on_drop(gid, HDROP(wp.0 as *mut _));
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                self.render_tile(gid);
                Some(LRESULT(0))
            }
            _ => {
                let _ = lp;
                None
            }
        }
    }

    //Mensajes del cuadro desplegado

    fn on_popup(&mut self, gid: u64, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let idx = self.win_index(gid)?;
        let pt = (loword(lp.0), hiword(lp.0));
        match msg {
            WM_MOUSEMOVE => {
                let g = &mut self.groups[idx];
                let mut redraw = false;
                if let Some(d) = &mut g.item_drag {
                    d.at = pt;
                    let s = g.layout.scale;
                    if !d.active && ((pt.0 - d.start.0).abs() > (5.0 * s) as i32 || (pt.1 - d.start.1).abs() > (5.0 * s) as i32) {
                        d.active = true;
                    }
                    redraw = d.active;
                }
                let hot = hit_cell(&g.layout, g.scroll, pt);
                if hot != g.hot {
                    g.hot = hot;
                    redraw = true;
                }
                if redraw {
                    self.render_popup(gid);
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                let manual = self.cfg.group(gid).map(|g| g.popup.sort == SortMode::Manual).unwrap_or(false);
                let g = &mut self.groups[idx];
                g.pressed = hit_cell(&g.layout, g.scroll, pt);
                if let Some(vi) = g.pressed.filter(|_| manual) {
                    g.item_drag = Some(ItemDrag { from: vi, start: pt, at: pt, active: false });
                }
                unsafe {
                    SetCapture(g.popup);
                }
                self.render_popup(gid);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                unsafe {
                    let _ = ReleaseCapture();
                }
                let g = &mut self.groups[idx];
                let pressed = g.pressed.take();
                let drag = g.item_drag.take();
                let hit = hit_cell(&g.layout, g.scroll, pt);
                let inside = pt.0 >= 0 && pt.1 >= 0 && pt.0 < g.layout.w && pt.1 < g.layout.h;
                match drag {
                    Some(d) if d.active => {
                        if inside {
                            let target = insertion_index(&g.layout, g.scroll, pt);
                            self.reorder(gid, d.from, target);
                        } else {
                            self.render_popup(gid);
                        }
                    }
                    _ => {
                        let single = self.cfg.single_click_open;
                        match (pressed, hit) {
                            (Some(a), Some(b)) if a == b && single => self.open_visible(gid, a),
                            _ => self.render_popup(gid),
                        }
                    }
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDBLCLK => {
                if !self.cfg.single_click_open {
                    let g = &self.groups[idx];
                    if let Some(vi) = hit_cell(&g.layout, g.scroll, pt) {
                        self.open_visible(gid, vi);
                    }
                }
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                let g = &self.groups[idx];
                let item = hit_cell(&g.layout, g.scroll, pt).and_then(|vi| g.layout.cells.get(vi)).map(|c| c.item);
                self.pending.push(Deferred::PopupMenu(gid, item));
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let g = &mut self.groups[idx];
                if g.layout.max_scroll > 0.0 {
                    let delta = hiword(wp.0 as isize) as f32 / 120.0;
                    g.scroll = (g.scroll - delta * g.layout.cell_h * 0.6).clamp(0.0, g.layout.max_scroll);
                    let p = cursor();
                    let r = g.popup_rect;
                    g.hot = hit_cell(&g.layout, g.scroll, (p.x - r.left, p.y - r.top));
                    self.render_popup(gid);
                }
                Some(LRESULT(0))
            }
            WM_DROPFILES => {
                self.on_drop(gid, HDROP(wp.0 as *mut _));
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    //Operaciones sobre elementos

    fn on_drop(&mut self, gid: u64, drop: HDROP) {
        let mut paths = Vec::new();
        unsafe {
            let count = DragQueryFileW(drop, u32::MAX, None);
            for i in 0..count {
                let len = DragQueryFileW(drop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                DragQueryFileW(drop, i, Some(&mut buf));
                paths.push(PathBuf::from(from_wide(&buf)));
            }
            DragFinish(drop);
        }
        if paths.is_empty() {
            return;
        }
        let warnings = storage::add_paths(&mut self.cfg, gid, &paths);
        self.save();
        if !warnings.is_empty() {
            let shown: Vec<&str> = warnings.iter().take(3).map(String::as_str).collect();
            let more = if warnings.len() > 3 { format!("\n…y {} más", warnings.len() - 3) } else { String::new() };
            self.tray.balloon("Algunos elementos no se movieron", &(shown.join("\n") + &more), true);
            for w in &warnings {
                log(w);
            }
        }
        self.render_tile(gid);
        self.refresh_popup(gid, true);
    }

    fn reorder(&mut self, gid: u64, from_visible: usize, target: usize) {
        let mut order: Vec<usize> = self.win(gid).map(|g| g.layout.cells.iter().map(|c| c.item).collect()).unwrap_or_default();
        if let Some(group) = self.cfg.group_mut(gid) {
            if group.popup.sort == SortMode::Manual && from_visible < order.len() && order.len() == group.items.len() {
                let moved = order.remove(from_visible);
                let t = if target > from_visible { target - 1 } else { target }.min(order.len());
                order.insert(t, moved);
                if group.popup.sort_desc {
                    order.reverse();
                }
                let old = std::mem::take(&mut group.items);
                group.items = order.iter().map(|&i| old[i].clone()).collect();
                self.save();
            }
        }
        self.refresh_popup(gid, true);
    }

    fn open_visible(&mut self, gid: u64, vi: usize) {
        let item = self.win(gid).and_then(|g| g.layout.cells.get(vi)).map(|c| c.item);
        if let Some(i) = item {
            self.open_item(gid, i);
        }
    }

    fn open_item(&mut self, gid: u64, index: usize) {
        let Some(item) = self.cfg.group(gid).and_then(|g| g.items.get(index)).cloned() else { return };
        if !Path::new(&item.path).exists() {
            self.pending.push(Deferred::ConfirmMissing(gid, index));
            return;
        }
        let dir = Path::new(&item.path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let ok = unsafe {
            let _ = AllowSetForegroundWindow(u32::MAX);
            ShellExecuteW(None, w!("open"), &hs(&item.path), PCWSTR::null(), &hs(&dir), SW_SHOWNORMAL).0 as isize > 32
        };
        if !ok {
            self.pending.push(Deferred::Message(format!("No se pudo abrir \"{}\".", Item::display_name(&item))));
        }
        if self.cfg.trigger == Trigger::Hover || ok {
            self.close_popup(gid);
        }
    }

    fn remove_item(&mut self, gid: u64, index: usize) {
        let path = self.cfg.group(gid).and_then(|g| g.items.get(index)).map(|i| i.path.clone());
        match storage::remove_item(&mut self.cfg, gid, index) {
            Ok(()) => {
                if let Some(p) = path {
                    self.icons.invalidate(&p);
                }
                self.save();
                self.render_tile(gid);
                self.refresh_popup(gid, true);
            }
            Err(e) => self.pending.push(Deferred::Message(e)),
        }
    }

    fn delete_group(&mut self, gid: u64) {
        self.close_popup(gid);
        match storage::delete_group(&mut self.cfg, gid) {
            Ok(()) => {}
            Err(e) => self.pending.push(Deferred::Message(e)),
        }
        self.save();
        self.sync_windows();
        self.update_tip();
    }
}

//Funciones auxiliares de dibujo y geometría

fn draw_panel(cv: &mut Canvas, x: f32, y: f32, w: f32, h: f32, st: &PopupStyle, s: f32, screen: (i32, i32), bd: Option<&(Rc<Backdrop>, RECT)>) {
    let tint = Rgba::from_u8(st.background);
    let op = (st.opacity * tint.a).clamp(0.0, 1.0);
    let r = st.corner_radius * s;
    match bd {
        Some((b, mon)) => {
            let (ox, oy) = ((screen.0 - mon.left) as f32, (screen.1 - mon.top) as f32);
            cv.fill_rrect_with(x, y, w, h, r, |px, py| {
                let c = b.sample(ox + px as f32 + 0.5, oy + py as f32 + 0.5);
                [c[0] * (1.0 - op) + tint.r * op, c[1] * (1.0 - op) + tint.g * op, c[2] * (1.0 - op) + tint.b * op, 1.0]
            });
        }
        None => cv.fill_rrect(x, y, w, h, r, Rgba { a: op.max(1.0 / 255.0), ..tint }),
    }
    let ow = st.outline_width * s.max(1.0);
    cv.stroke_rrect(x, y, w, h, r, ow, Rgba::from_u8(st.outline));
}

fn draw_cell(cv: &mut Canvas, text: &mut Text, cell: &Cell, icon: Option<&Image>, x: f32, y: f32, l: &Layout, color: Rgba, shadow: bool, alpha: f32) {
    let s = l.scale;
    let size = l.icon as f32;
    let top = y + 6.0 * s;
    let cx = x + l.cell_w * 0.5;
    match icon {
        Some(img) => {
            let (iw, ih) = fit(img.w as f32, img.h as f32, size);
            cv.draw_image(img, (cx - iw * 0.5).round(), (top + (size - ih) * 0.5).round(), iw, ih, alpha);
        }
        None => cv.fill_rrect(cx - size * 0.4, top + size * 0.1, size * 0.8, size * 0.8, 6.0 * s, Rgba::new(1.0, 1.0, 1.0, 0.10 * alpha)),
    }
    let lh = text.line_height(l.font);
    let mut ty = top + size + 4.0 * s;
    for line in &cell.name {
        text.draw_centered(cv, line, cx, ty, l.font, color.with_alpha(alpha), shadow);
        ty += lh;
    }
}

fn fit(w: f32, h: f32, side: f32) -> (f32, f32) {
    if w <= 0.0 || h <= 0.0 {
        return (side, side);
    }
    let k = (side / w).min(side / h);
    (w * k, h * k)
}

fn contains(r: &RECT, p: POINT) -> bool {
    p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
}

fn inflate(r: &RECT, d: i32) -> RECT {
    RECT { left: r.left - d, top: r.top - d, right: r.right + d, bottom: r.bottom + d }
}

fn hit_cell(l: &Layout, scroll: f32, pt: (i32, i32)) -> Option<usize> {
    let (px, py) = (pt.0 as f32, pt.1 as f32);
    if py < l.grid_y || py > l.grid_y + l.view_h {
        return None;
    }
    l.cells.iter().position(|c| {
        let x = l.grid_x + c.x;
        let y = l.grid_y + c.y - scroll;
        px >= x && px < x + l.cell_w && py >= y && py < y + l.cell_h
    })
}

fn insertion_index(l: &Layout, scroll: f32, pt: (i32, i32)) -> usize {
    let rx = ((pt.0 as f32 - l.grid_x) / l.cell_w).max(0.0);
    let ry = ((pt.1 as f32 - l.grid_y + scroll) / l.cell_h).max(0.0);
    let col = (rx.floor() as usize).min(l.cols - 1);
    let row = ry.floor() as usize;
    let mut i = row * l.cols + col;
    if rx.fract() > 0.5 || rx.floor() as usize >= l.cols {
        i += 1;
    }
    i.min(l.cells.len())
}

fn place_popup(tile: &RECT, w: i32, h: i32, gap: i32, wa: &RECT) -> RECT {
    let mut x = tile.left;
    if x + w > wa.right {
        x = tile.right - w;
    }
    x = x.clamp(wa.left, (wa.right - w).max(wa.left));
    let mut y = tile.bottom + gap;
    if y + h > wa.bottom {
        y = tile.top - gap - h;
    }
    if y < wa.top {
        y = (wa.bottom - h).max(wa.top);
    }
    RECT { left: x, top: y, right: x + w, bottom: y + h }
}

fn desktop_owner() -> Option<HWND> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), PCWSTR::null()).ok().filter(|h| !h.is_invalid());
        let has_view = |h: HWND| FindWindowExW(Some(h), None, w!("SHELLDLL_DefView"), PCWSTR::null()).map(|v| !v.is_invalid()).unwrap_or(false);
        if let Some(p) = progman.filter(|&p| has_view(p)) {
            return Some(p);
        }
        let mut prev: Option<HWND> = None;
        while let Ok(wk) = FindWindowExW(None, prev, w!("WorkerW"), PCWSTR::null()) {
            if wk.is_invalid() {
                break;
            }
            if has_view(wk) {
                return Some(wk);
            }
            prev = Some(wk);
        }
        progman
    }
}

fn create_group_windows(gid: u64, owner: Option<HWND>) -> Result<GroupWin, String> {
    unsafe {
        let ex_tile = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_ACCEPTFILES;
        let tile = CreateWindowExW(ex_tile, TILE_CLASS, w!("Cuadro"), WS_POPUP, 0, 0, 1, 1, owner, None, None, None)
            .map_err(|e| format!("No se pudo crear el icono del cuadro: {e}"))?;
        let ex_popup = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_ACCEPTFILES;
        let popup = match CreateWindowExW(ex_popup, POPUP_CLASS, w!("Cuadro abierto"), WS_POPUP, 0, 0, 1, 1, None, None, None, None) {
            Ok(p) => p,
            Err(e) => {
                let _ = DestroyWindow(tile);
                return Err(format!("No se pudo crear el cuadro desplegable: {e}"));
            }
        };
        for h in [tile, popup] {
            SetWindowLongPtrW(h, GWLP_USERDATA, gid as isize);
            for m in [WM_DROPFILES, WM_COPYDATA, 0x0049] {
                let _ = ChangeWindowMessageFilterEx(h, m, MSGFLT_ALLOW, None);
            }
        }
        Ok(GroupWin {
            id: gid,
            tile,
            popup,
            tile_surf: None,
            popup_surf: None,
            tile_rect: RECT::default(),
            popup_rect: RECT::default(),
            open: false,
            fade: 1.0,
            last_tick: Instant::now(),
            hover_since: None,
            outside_since: None,
            hot: None,
            pressed: None,
            scroll: 0.0,
            layout: Layout::default(),
            bg: Canvas::new(1, 1),
            canvas: Canvas::new(1, 1),
            tile_drag: None,
            item_drag: None,
        })
    }
}
