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
    format: Option<organizer::NameFormat>,
) -> Result<organizer::Summary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let fmt = format.unwrap_or_default();
        organizer::organize(&sources, &dest, move_files, &fmt, |p| {
            let _ = app.emit("progress", p);
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Vérifie au lancement si une mise à jour est publiée ; si oui, propose de
/// l'installer via un dialogue natif, puis relance l'app.
fn verifier_mise_a_jour(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        use tauri_plugin_dialog::MessageDialogButtons;
        use tauri_plugin_updater::UpdaterExt;
        let Ok(updater) = app.updater() else { return };
        let Ok(Some(maj)) = updater.check().await else { return };
        let version = maj.version.clone();
        let installer = app
            .dialog()
            .message(format!(
                "La version {version} est disponible.\nL'installer maintenant ?"
            ))
            .title("Mise à jour")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Mettre à jour".into(),
                "Plus tard".into(),
            ))
            .blocking_show();
        if installer && maj.download_and_install(|_, _| {}, || {}).await.is_ok() {
            app.restart();
        }
    });
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            verifier_mise_a_jour(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![choose_dest, open_dest, import])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de Flash");
}
