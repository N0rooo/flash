#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use flash::organizer;

use tauri::{AppHandle, Emitter};
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
async fn choose_dest(app: AppHandle) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .blocking_pick_folder()
            .and_then(|p| p.into_path().ok())
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten()
}

#[tauri::command]
fn open_dest(path: String) -> Result<(), String> {
    tauri_plugin_opener::open_path(path, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
async fn import(
    app: AppHandle,
    sources: Vec<String>,
    dest: String,
    move_files: bool,
) -> Result<organizer::Summary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        organizer::organize(&sources, &dest, move_files, |p| {
            let _ = app.emit("progress", p);
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![choose_dest, open_dest, import])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de Flash");
}
