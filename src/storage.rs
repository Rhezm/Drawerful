use std::path::{Path, PathBuf};

use windows::core::PCWSTR;
use windows::Win32::UI::Shell::{SHFileOperationW, FOF_NOCONFIRMATION, FOF_NOCONFIRMMKDIR, FOF_NOERRORUI, FOF_SILENT, FO_MOVE, SHFILEOPSTRUCTW};

use crate::config::{store_dir, Config, GroupIcon, Item, StorageMode};
use crate::winutil::desktop_dir;

//Movimiento de archivos

fn unique_path(dir: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..10_000).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|c| !c.exists()).unwrap_or(first)
}

fn double_null(p: &Path) -> Vec<u16> {
    let mut v: Vec<u16> = p.to_string_lossy().encode_utf16().collect();
    v.push(0);
    v.push(0);
    v
}

fn shell_move(src: &Path, dst: &Path) -> Result<(), String> {
    let from = double_null(src);
    let to = double_null(dst);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_MOVE,
        pFrom: PCWSTR(from.as_ptr()),
        pTo: PCWSTR(to.as_ptr()),
        fFlags: (FOF_NOCONFIRMATION | FOF_NOCONFIRMMKDIR | FOF_NOERRORUI | FOF_SILENT).0 as u16,
        ..Default::default()
    };
    let r = unsafe { SHFileOperationW(&mut op) };
    if r != 0 || op.fAnyOperationsAborted.as_bool() {
        return Err(format!("código {r:#x}"));
    }
    Ok(())
}

pub fn move_into(src: &Path, dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("No se pudo crear {}: {e}", dir.display()))?;
    let name = src.file_name().ok_or("Ruta sin nombre")?;
    let dst = unique_path(dir, name);
    match std::fs::rename(src, &dst) {
        Ok(()) => Ok(dst),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Err(format!("Sin permisos para mover \"{}\"", src.display())),
        Err(_) => shell_move(src, &dst).map(|_| dst).map_err(|e| format!("No se pudo mover \"{}\" ({e})", src.display())),
    }
}

fn is_inside(path: &Path, dir: &Path) -> bool {
    let a = path.to_string_lossy().to_lowercase();
    let b = dir.to_string_lossy().to_lowercase();
    a.starts_with(&b)
}

//Operaciones sobre grupos

pub fn add_paths(cfg: &mut Config, gid: u64, paths: &[PathBuf]) -> Vec<String> {
    let mut warnings = Vec::new();
    let mode = cfg.storage;
    let store_root = crate::config::data_dir().join("store");
    let Some(group) = cfg.group_mut(gid) else { return warnings };
    let dir = store_dir(gid);
    for src in paths {
        let src_s = src.to_string_lossy().into_owned();
        if group.items.iter().any(|i| i.path.eq_ignore_ascii_case(&src_s)) {
            continue;
        }
        if !src.exists() {
            warnings.push(format!("No existe: {src_s}"));
            continue;
        }
        if mode == StorageMode::Move && !is_inside(src, &store_root) {
            match move_into(src, &dir) {
                Ok(dst) => group.items.push(Item::new(dst.to_string_lossy().into_owned(), Some(src_s))),
                Err(e) => {
                    warnings.push(format!("{e}. Se agregó solo como referencia."));
                    group.items.push(Item::new(src_s, None));
                }
            }
        } else {
            group.items.push(Item::new(src_s, None));
        }
    }
    warnings
}

pub fn restore(item: &Item) -> Result<(), String> {
    let Some(origin) = &item.origin else { return Ok(()) };
    let src = Path::new(&item.path);
    if !src.exists() {
        return Ok(());
    }
    let origin = Path::new(origin);
    let dir = origin.parent().filter(|d| d.is_dir()).map(Path::to_path_buf).unwrap_or_else(desktop_dir);
    let name = origin.file_name().or(src.file_name()).ok_or("Ruta sin nombre")?;
    let dst = unique_path(&dir, name);
    match std::fs::rename(src, &dst) {
        Ok(()) => Ok(()),
        Err(_) => shell_move(src, &dst).map_err(|e| format!("No se pudo devolver \"{}\" a {} ({e})", src.display(), dir.display())),
    }
}

pub fn remove_item(cfg: &mut Config, gid: u64, index: usize) -> Result<(), String> {
    let group = cfg.group_mut(gid).ok_or("Grupo no encontrado")?;
    let item = group.items.get(index).ok_or("Elemento no encontrado")?;
    restore(item)?;
    group.items.remove(index);
    Ok(())
}

pub fn delete_group(cfg: &mut Config, gid: u64) -> Result<(), String> {
    let group = cfg.group_mut(gid).ok_or("Grupo no encontrado")?;
    let mut errors = Vec::new();
    group.items.retain(|it| match restore(it) {
        Ok(()) => false,
        Err(e) => {
            errors.push(e);
            true
        }
    });
    if !errors.is_empty() {
        return Err(format!("El cuadro no se eliminó porque algunos archivos no se pudieron devolver:\n{}", errors.join("\n")));
    }
    if let Some(GroupIcon::Image(f)) = cfg.group(gid).map(|g| g.icon.clone()) {
        let _ = std::fs::remove_file(crate::config::icons_dir().join(f));
    }
    cfg.groups.retain(|g| g.id != gid);
    let _ = std::fs::remove_dir(store_dir(gid));
    Ok(())
}

//Icono personalizado: se reduce y se guarda en caché como PNG

pub fn import_icon(src: &Path, gid: u64) -> Result<String, String> {
    let img = image::ImageReader::open(src)
        .map_err(|e| format!("No se pudo abrir la imagen: {e}"))?
        .with_guessed_format()
        .map_err(|e| format!("Formato no reconocido: {e}"))?
        .decode()
        .map_err(|e| format!("No se pudo leer la imagen: {e}"))?;
    let small = img.resize(128, 128, image::imageops::FilterType::Lanczos3).to_rgba8();
    let dir = crate::config::icons_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = format!("{gid}-{}.png", crate::config::now_secs());
    small.save(dir.join(&name)).map_err(|e| format!("No se pudo guardar el icono: {e}"))?;
    Ok(name)
}
