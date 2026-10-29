// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bevy;
#[cfg(feature = "ci-smoke")]
mod ci_smoke;
#[cfg(target_os = "linux")]
mod linux_overlay;
mod wgpu;
mod tauri_plugin;

pub fn generate_tauri_context() -> tauri::Context {
    tauri::generate_context!()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let use_wgpu = args.contains(&String::from("--use-wgpu"));

    if !use_wgpu {
        bevy::setup_bevy();
    } else {
        wgpu::setup_wgpu();
    }

    Ok(())
}
