// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bevy;
mod wgpu;

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
