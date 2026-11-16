pub struct Preset {
    pub key: &'static str,
    pub label: &'static str,
    pub glyph: char,
}

pub const PRESETS: &[Preset] = &[
    Preset { key: "apps", label: "Aplicaciones", glyph: '\u{E71D}' },
    Preset { key: "games", label: "Juegos", glyph: '\u{E7FC}' },
    Preset { key: "folder", label: "Carpetas", glyph: '\u{E8B7}' },
    Preset { key: "documents", label: "Documentos", glyph: '\u{E8A5}' },
    Preset { key: "photos", label: "Fotos", glyph: '\u{E8B9}' },
    Preset { key: "videos", label: "Videos", glyph: '\u{E714}' },
    Preset { key: "music", label: "Música", glyph: '\u{E8D6}' },
    Preset { key: "downloads", label: "Descargas", glyph: '\u{E896}' },
    Preset { key: "code", label: "Código", glyph: '\u{E943}' },
    Preset { key: "work", label: "Trabajo", glyph: '\u{E821}' },
    Preset { key: "school", label: "Estudios", glyph: '\u{E7BE}' },
    Preset { key: "web", label: "Web", glyph: '\u{E774}' },
    Preset { key: "chat", label: "Mensajes", glyph: '\u{E8BD}' },
    Preset { key: "cloud", label: "Nube", glyph: '\u{E753}' },
    Preset { key: "tools", label: "Herramientas", glyph: '\u{E90F}' },
    Preset { key: "settings", label: "Ajustes", glyph: '\u{E713}' },
    Preset { key: "home", label: "Inicio", glyph: '\u{E80F}' },
    Preset { key: "shopping", label: "Compras", glyph: '\u{E719}' },
    Preset { key: "star", label: "Favoritos", glyph: '\u{E735}' },
    Preset { key: "heart", label: "Especial", glyph: '\u{EB51}' },
];

pub fn find(key: &str) -> &'static Preset {
    PRESETS.iter().find(|p| p.key == key).unwrap_or(&PRESETS[0])
}
