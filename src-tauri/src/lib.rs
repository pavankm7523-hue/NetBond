mod adapters;
mod engine;
mod models;
mod settings;
mod torrent;
mod traffic;

use engine::Engine;
use models::{AdapterInfo, DownloadSnapshot, Settings, StartRequest};
use std::path::PathBuf;
use tauri::{Emitter, Manager, State};
use torrent::{TorrentEngine, TorrentPreview, TorrentSnapshot, TorrentStartRequest};

struct AppState {
    engine: Engine,
    torrents: TorrentEngine,
    settings_path: PathBuf,
}

#[tauri::command]
async fn enumerate_adapters() -> Result<Vec<AdapterInfo>, String> {
    tokio::task::spawn_blocking(adapters::enumerate)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn start_download(
    request: StartRequest,
    state: State<'_, AppState>,
) -> Result<String, String> {
    state.engine.start(request).await
}

#[tauri::command]
async fn get_download(id: String, state: State<'_, AppState>) -> Result<DownloadSnapshot, String> {
    state.engine.get(&id).await
}

#[tauri::command]
async fn list_downloads(state: State<'_, AppState>) -> Result<Vec<DownloadSnapshot>, String> {
    Ok(state.engine.list().await)
}

#[tauri::command]
async fn pause_download(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.engine.pause(&id).await
}

#[tauri::command]
async fn resume_download(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.engine.resume(&id).await
}

#[tauri::command]
async fn cancel_download(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.engine.cancel(&id).await
}

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    Ok(state.engine.current_settings().await)
}

#[tauri::command]
async fn save_settings(value: Settings, state: State<'_, AppState>) -> Result<(), String> {
    settings::save(&state.settings_path, &value)?;
    state.engine.set_settings(value).await;
    Ok(())
}

#[tauri::command]
async fn preview_torrent(
    source: String,
    destination: String,
    adapters: Vec<models::SelectedAdapter>,
    state: State<'_, AppState>,
) -> Result<TorrentPreview, String> {
    state.torrents.preview(source, destination, adapters).await
}
#[tauri::command]
async fn start_torrent(
    request: TorrentStartRequest,
    state: State<'_, AppState>,
) -> Result<String, String> {
    state.torrents.start(request).await
}
#[tauri::command]
async fn pause_torrent(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.torrents.pause(&id).await
}
#[tauri::command]
async fn resume_torrent(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.torrents.resume(&id).await
}
#[tauri::command]
async fn list_torrents(state: State<'_, AppState>) -> Result<Vec<TorrentSnapshot>, String> {
    Ok(state.torrents.list().await)
}

#[tauri::command]
async fn update_torrent_adapters(
    adapters: Vec<models::SelectedAdapter>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.torrents.update_adapters(adapters).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_local_data_dir()
                .map_err(|e| format!("Cannot locate app data: {e}"))?;
            let settings_path = data_dir.join("settings.json");
            let app_settings = settings::load(&settings_path);
            let engine = Engine::new(app.handle().clone(), app_settings.clone());
            let torrents = TorrentEngine::new(app.handle().clone());
            let recover_engine = engine.clone();
            let recover_dir = PathBuf::from(&app_settings.download_directory);
            tauri::async_runtime::spawn(async move {
                recover_engine.recover_from(&recover_dir).await;
            });
            let monitor_app = app.handle().clone();
            let monitor_engine = engine.clone();
            tauri::async_runtime::spawn(traffic::monitor(app.handle().clone(), engine.clone()));
            if app_settings.start_minimized {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.minimize();
                }
            }
            tauri::async_runtime::spawn(async move {
                let mut previous = String::new();
                loop {
                    let fresh = tokio::task::spawn_blocking(adapters::enumerate)
                        .await
                        .ok()
                        .and_then(Result::ok);
                    let current = fresh
                        .as_ref()
                        .and_then(|a| serde_json::to_string(a).ok())
                        .unwrap_or_default();
                    if let Some(fresh) = fresh {
                        monitor_engine.update_adapters(fresh).await;
                    }
                    if !previous.is_empty() && current != previous {
                        let _ = monitor_app.emit("adapters-changed", ());
                    }
                    previous = current;
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            });
            app.manage(AppState {
                engine,
                torrents,
                settings_path,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            enumerate_adapters,
            start_download,
            get_download,
            list_downloads,
            pause_download,
            resume_download,
            cancel_download,
            get_settings,
            save_settings,
            preview_torrent,
            start_torrent,
            pause_torrent,
            resume_torrent,
            list_torrents,
            update_torrent_adapters
        ])
        .run(tauri::generate_context!())
        .expect("error while running NetBond");
}
