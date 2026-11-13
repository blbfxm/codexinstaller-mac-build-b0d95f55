// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = codexinstaller_lib::image_plugin::run_if_requested() {
        std::process::exit(code);
    }
    codexinstaller_lib::run()
}
