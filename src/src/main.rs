// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use tauri::{menu::{Menu, MenuItem}, tray::{MouseButton, TrayIconBuilder, TrayIconEvent}, Emitter, Manager, State};

mod config;
mod notify;
mod watch;

use config::{load_config, save_config as save_config_to_file, get_config_path, get_data_dir, AppConfig};
use notify::send_notifications;
use watch::start_watch as start_watch_fn;

#[derive(Serialize)]
struct MetaInfo {
    product_name: String,
    data_dir: String,
    config_path: String,
    version: String,
}

#[derive(Serialize)]
struct WatchStatus {
    running: bool,
}

#[derive(Deserialize)]
struct WatchStartPayload {
    #[serde(default = "default_sources")]
    sources: String,
    #[serde(default = "default_interval_ms")]
    interval_ms: i32,
    #[serde(default = "default_gemini_quiet_ms")]
    gemini_quiet_ms: i32,
    #[serde(default = "default_claude_quiet_ms")]
    claude_quiet_ms: i32,
}

fn default_sources() -> String { "all".to_string() }
fn default_interval_ms() -> i32 { 1000 }
fn default_gemini_quiet_ms() -> i32 { 3000 }
fn default_claude_quiet_ms() -> i32 { 60000 }

#[derive(Deserialize)]
struct TestNotifyPayload {
    #[serde(default = "default_test_source")]
    source: String,
    #[serde(default)]
    task_info: String,
    #[serde(default)]
    duration_minutes: Option<i32>,
}

fn default_test_source() -> String { "claude".to_string() }

struct AppState {
    watch_stop: Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            watch_stop: Arc::new(Mutex::new(None)),
        }
    }
}

#[tauri::command]
fn get_meta(app_handle: tauri::AppHandle) -> MetaInfo {
    MetaInfo {
        product_name: "ai-cli-complete-notify".to_string(),
        data_dir: get_data_dir().to_string_lossy().to_string(),
        config_path: get_config_path().to_string_lossy().to_string(),
        version: app_handle.package_info().version.to_string(),
    }
}

#[tauri::command]
fn get_config() -> Result<AppConfig, String> {
    load_config().map_err(|e| e.to_string())
}

#[tauri::command]
fn save_config(config: AppConfig) -> Result<(), String> {
    save_config_to_file(&config).map_err(|e| e.to_string())
}

#[tauri::command]
fn watch_status(state: State<AppState>) -> WatchStatus {
    let guard = state.watch_stop.lock().unwrap();
    WatchStatus { running: guard.is_some() }
}

#[tauri::command]
async fn start_watch(payload: WatchStartPayload, app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.watch_stop.lock().unwrap();
    if guard.is_some() {
        return Err("Watch already running".to_string());
    }

    let stop = start_watch_fn(
        &payload.sources,
        payload.interval_ms,
        payload.gemini_quiet_ms,
        payload.claude_quiet_ms,
        move |line: String| {
            let _ = app.emit("watch-log", line);
        },
    ).map_err(|e| e.to_string())?;

    *guard = Some(stop);
    Ok(())
}

#[tauri::command]
fn stop_watch(state: State<AppState>) -> Result<(), String> {
    let mut guard = state.watch_stop.lock().unwrap();
    if let Some(stop) = guard.take() {
        stop();
        Ok(())
    } else {
        Err("Watch not running".to_string())
    }
}

#[tauri::command]
async fn test_notification(payload: TestNotifyPayload) -> Result<(), String> {
    let duration_ms = payload.duration_minutes.map(|m| (m as i64) * 60000);
    send_notifications(
        &payload.source,
        &payload.task_info,
        duration_ms,
        String::new(),
        true,
    ).await.map_err(|e| e.to_string())?;
    Ok(())
}

fn setup_tray(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let open_i = MenuItem::with_id(app, "open", "打开", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_i, &quit_i])?;

    let _tray = TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, .. } = event {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .on_menu_event(|app, event| {
            match event.id.as_ref() {
                "open" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                "quit" => app.exit(0),
                _ => {}
            }
        })
        .build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .manage(AppState::default())
        .setup(|app| {
            setup_tray(app.handle())?;

            // 显示主窗口
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_meta,
            get_config,
            save_config,
            watch_status,
            start_watch,
            stop_watch,
            test_notification,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn main() {
    run();
}
