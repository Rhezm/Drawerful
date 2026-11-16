#![windows_subsystem = "windows"]
#![allow(clippy::too_many_arguments)]

mod canvas;
mod config;
mod host;
mod icons;
mod presets;
mod settings;
mod storage;
mod surface;
mod tray;
mod wallpaper;
mod winutil;

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("Error inesperado: {info}");
        winutil::log(&msg);
        winutil::error_box(&msg);
    }));
    let args: Vec<String> = std::env::args().collect();
    let code = if args.iter().any(|a| a == "--settings") {
        let group = args.iter().position(|a| a == "--group").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok());
        settings::run(group)
    } else {
        host::run()
    };
    std::process::exit(code);
}
