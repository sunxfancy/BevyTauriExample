//! Only compiled for desktop smoke tests. Normal builds expose no test commands.
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use tauri::Manager;

static LAST_CONTROL: AtomicU64 = AtomicU64::new(0);

fn output_dir() -> Result<PathBuf, String> {
    std::env::var_os("BTEXAMPLE_SMOKE_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "BTEXAMPLE_SMOKE_DIR is required for the CI probe".into())
}

pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Desktop startup: CI setup entered");
    if std::env::var_os("BTEXAMPLE_SMOKE_DIR").is_some() {
        let window = app
            .get_webview_window("main")
            .ok_or("missing main window")?;
        window.set_position(tauri::LogicalPosition::new(40., 40.))?;
        window.set_focus()?;
    }
    eprintln!("Desktop startup: CI setup complete");
    Ok(())
}

#[tauri::command]
pub fn report(window: tauri::WebviewWindow, mut state: serde_json::Value) -> Result<(), String> {
    let dir = output_dir()?;
    let position = window.inner_position().map_err(|e| e.to_string())?;
    let size = window.inner_size().map_err(|e| e.to_string())?;
    state["native"] = serde_json::json!({
        "x": position.x, "y": position.y, "width": size.width, "height": size.height,
        "scale": window.scale_factor().map_err(|e| e.to_string())?,
    });
    std::fs::write(
        dir.join("state.json"),
        serde_json::to_vec(&state).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    // File controls are confined to the explicitly selected test output directory.
    // The Python driver performs input through the OS, not through this probe.
    if let Ok(data) = std::fs::read(dir.join("control.json")) {
        if let Ok(control) = serde_json::from_slice::<serde_json::Value>(&data) {
            let sequence = control["sequence"].as_u64().unwrap_or(0);
            if sequence > LAST_CONTROL.load(Ordering::Relaxed) {
                if let (Some(width), Some(height)) =
                    (control["width"].as_u64(), control["height"].as_u64())
                {
                    if !(320..=1600).contains(&width) || !(240..=1200).contains(&height) {
                        return Err("invalid test window size".into());
                    }
                    window
                        .set_size(tauri::LogicalSize::new(width as f64, height as f64))
                        .map_err(|e| e.to_string())?;
                    LAST_CONTROL.store(sequence, Ordering::Relaxed);
                }
            }
        }
    }
    Ok(())
}
