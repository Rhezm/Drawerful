use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

//Tipos de configuración

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trigger {
    Hover,
    Click,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum StorageMode {
    Reference,
    Move,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortMode {
    Manual,
    Name,
    Kind,
    Added,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub enum GroupIcon {
    Preset(String),
    Image(String),
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub trigger: Trigger,
    pub hover_delay_ms: u32,
    pub close_delay_ms: u32,
    pub storage: StorageMode,
    pub single_click_open: bool,
    pub locked: bool,
    pub animations: bool,
    pub next_id: u64,
    pub groups: Vec<Group>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Group {
    pub id: u64,
    pub name: String,
    pub icon: GroupIcon,
    pub tile_size: u32,
    pub show_tile_label: bool,
    pub x: i32,
    pub y: i32,
    pub popup: PopupStyle,
    pub items: Vec<Item>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct PopupStyle {
    pub width: u32,
    pub height: u32,
    pub columns: u32,
    pub cell_w: u32,
    pub cell_h: u32,
    pub icon_size: u32,
    pub font_px: f32,
    pub sort: SortMode,
    pub sort_desc: bool,
    pub show_title: bool,
    pub background: [u8; 4],
    pub opacity: f32,
    pub blur: u32,
    pub outline: [u8; 4],
    pub outline_width: f32,
    pub corner_radius: f32,
    pub text_color: [u8; 4],
    pub text_shadow: bool,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(default)]
pub struct Item {
    pub path: String,
    pub origin: Option<String>,
    pub added: u64,
}

//Valores por defecto

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger: Trigger::Hover,
            hover_delay_ms: 150,
            close_delay_ms: 350,
            storage: StorageMode::Reference,
            single_click_open: true,
            locked: false,
            animations: true,
            next_id: 1,
            groups: Vec::new(),
        }
    }
}

impl Default for Group {
    fn default() -> Self {
        Self {
            id: 0,
            name: "Grupo".into(),
            icon: GroupIcon::Preset("apps".into()),
            tile_size: 64,
            show_tile_label: true,
            x: 120,
            y: 120,
            popup: PopupStyle::default(),
            items: Vec::new(),
        }
    }
}

impl Default for PopupStyle {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            columns: 0,
            cell_w: 0,
            cell_h: 0,
            icon_size: 0,
            font_px: 0.0,
            sort: SortMode::Manual,
            sort_desc: false,
            show_title: true,
            background: [0, 0, 0, 255],
            opacity: 0.20,
            blur: 8,
            outline: [255, 255, 255, 110],
            outline_width: 1.0,
            corner_radius: 10.0,
            text_color: [255, 255, 255, 255],
            text_shadow: true,
        }
    }
}

//Operaciones

impl Config {
    pub fn new_group(&mut self, x: i32, y: i32) -> u64 {
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        let n = self.groups.len() + 1;
        self.groups.push(Group {
            id,
            name: format!("Grupo {n}"),
            x,
            y,
            ..Group::default()
        });
        id
    }

    pub fn group(&self, id: u64) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn group_mut(&mut self, id: u64) -> Option<&mut Group> {
        self.groups.iter_mut().find(|g| g.id == id)
    }

    fn sanitize(&mut self) {
        let max = self.groups.iter().map(|g| g.id).max().unwrap_or(0);
        if self.next_id <= max {
            self.next_id = max + 1;
        }
        for g in &mut self.groups {
            g.tile_size = g.tile_size.clamp(32, 256);
            let p = &mut g.popup;
            p.opacity = p.opacity.clamp(0.0, 1.0);
            p.blur = p.blur.min(64);
            p.outline_width = p.outline_width.clamp(0.0, 8.0);
            p.corner_radius = p.corner_radius.clamp(0.0, 64.0);
            p.columns = p.columns.min(32);
        }
    }
}

impl Item {
    pub fn new(path: String, origin: Option<String>) -> Self {
        Self { path, origin, added: now_secs() }
    }

    pub fn display_name(&self) -> String {
        let p = Path::new(&self.path);
        let name = if p.is_dir() { p.file_name() } else { p.file_stem().or(p.file_name()) };
        name.map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.path.clone())
    }

    pub fn kind_key(&self) -> String {
        let p = Path::new(&self.path);
        if p.is_dir() {
            return String::from("\0");
        }
        p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
    }
}

impl Group {
    pub fn sorted_indices(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.items.len()).collect();
        let items = &self.items;
        match self.popup.sort {
            SortMode::Manual => {}
            SortMode::Name => idx.sort_by_cached_key(|&i| items[i].display_name().to_lowercase()),
            SortMode::Kind => idx.sort_by_cached_key(|&i| (items[i].kind_key(), items[i].display_name().to_lowercase())),
            SortMode::Added => idx.sort_by_key(|&i| items[i].added),
        }
        if self.popup.sort_desc {
            idx.reverse();
        }
        idx
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

//Rutas y persistencia

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("Cuadros")
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

pub fn icons_dir() -> PathBuf {
    data_dir().join("icons")
}

pub fn store_dir(group: u64) -> PathBuf {
    data_dir().join("store").join(group.to_string())
}

pub fn config_mtime() -> Option<SystemTime> {
    std::fs::metadata(config_path()).and_then(|m| m.modified()).ok()
}

pub fn load() -> Result<Config, String> {
    let path = config_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(format!("No se pudo leer {}: {e}", path.display())),
    };
    match serde_json::from_str::<Config>(&text) {
        Ok(mut c) => {
            c.sanitize();
            Ok(c)
        }
        Err(e) => {
            let backup = path.with_extension(format!("corrupto-{}.json", now_secs()));
            let _ = std::fs::copy(&path, &backup);
            Err(format!("La configuración estaba dañada ({e}). Se respaldó en {} y se usarán valores por defecto.", backup.display()))
        }
    }
}

pub fn save(cfg: &Config) -> Result<(), String> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("No se pudo crear {}: {e}", dir.display()))?;
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    let tmp = config_path().with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("No se pudo escribir la configuración: {e}"))?;
    std::fs::rename(&tmp, config_path()).map_err(|e| format!("No se pudo reemplazar la configuración: {e}"))
}
