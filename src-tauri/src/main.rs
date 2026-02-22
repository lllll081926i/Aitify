// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, State, WindowEvent,
};
use tauri_plugin_autostart::AppHandleExt;

mod config;
mod notify;
mod watch;

use config::{load_config, save_config, get_config_path, get_data_dir};
use notify::send_notifications;
use watch::start_watch;

const PRODUCT_NAME: &str = "ai-cli-complete-notify";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub version: i32,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub channels: ChannelsConfig,
    #[serde(default)]
    pub sources: SourcesConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiConfig {
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_close_behavior")]
    pub close_behavior: String,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub silent_start: bool,
    #[serde(default = "default_watch_log_retention")]
    pub watch_log_retention_days: i32,
    #[serde(default)]
    pub auto_focus_on_notify: bool,
    #[serde(default)]
    pub force_maximize_on_focus: bool,
    #[serde(default = "default_focus_target")]
    pub focus_target: String,
    #[serde(default)]
    pub confirm_alert: ConfirmAlertConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfirmAlertConfig {
    #[serde(default)]
    pub enabled: bool,
}

fn default_language() -> String { "zh-CN".to_string() }
fn default_close_behavior() -> String { "ask".to_string() }
fn default_watch_log_retention() -> i32 { 7 }
fn default_focus_target() -> String { "auto".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelsConfig {
    #[serde(default)]
    pub telegram: TelegramConfig,
    #[serde(default)]
    pub sound: SoundConfig,
    #[serde(default)]
    pub desktop: DesktopConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelegramConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub bot_token: String,
    #[serde(default)]
    pub chat_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SoundConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub tts: bool,
    #[serde(default)]
    pub use_custom: bool,
    #[serde(default)]
    pub custom_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DesktopConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_balloon_ms")]
    pub balloon_ms: i32,
}

fn default_true() -> bool { true }
fn default_balloon_ms() -> i32 { 6000 }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourcesConfig {
    #[serde(default)]
    pub claude: SourceConfig,
    #[serde(default)]
    pub codex: SourceConfig,
    #[serde(default)]
    pub gemini: SourceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub min_duration_minutes: i32,
    #[serde(default)]
    pub channels: SourceChannelsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceChannelsConfig {
    #[serde(default)]
    pub telegram: bool,
    #[serde(default = "default_true")]
    pub sound: bool,
    #[serde(default = "default_true")]
    pub desktop: bool,
}

#[derive(Serialize)]
pub struct MetaInfo {
    product_name: String,
    data_dir: String,
    config_path: String,
    version: String,
}

#[derive(Serialize)]
pub struct WatchStatus {
    running: bool,
}

#[derive(Serialize)]
pub struct SystemAutostartStatus {
    open_at_login: bool,
}

#[derive(Serialize)]
pub struct AutostartStatus {
    platform: String,
    system: Option<SystemAutostartStatus>,
    ok: bool,
    error: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct ClosePromptPayload {
    id: String,
    epoch: i64,
}

#[derive(Deserialize)]
pub struct WatchStartPayload {
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
pub struct TestNotifyPayload {
    #[serde(default = "default_test_source")]
    source: String,
    #[serde(default)]
    task_info: String,
    #[serde(default)]
    duration_minutes: Option<i32>,
}

fn default_test_source() -> String { "claude".to_string() }

#[derive(Deserialize)]
pub struct TestSoundPayload {
    #[serde(default)]
    title: String,
    #[serde(default)]
    sound: Option<SoundConfig>,
}

#[derive(Deserialize)]
pub struct ClosePromptResponsePayload {
    id: String,
    action: String,
    remember: bool,
}

pub struct AppState {
    watch_stop: Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>,
    is_quitting: Arc<Mutex<bool>>,
    close_prompt_seq: Arc<Mutex<i64>>,
    close_prompt_epoch: Arc<Mutex<i64>>,
    close_prompt_open: Arc<Mutex<bool>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            watch_stop: Arc::new(Mutex::new(None)),
            is_quitting: Arc::new(Mutex::new(false)),
            close_prompt_seq: Arc::new(Mutex::new(0)),
            close_prompt_epoch: Arc::new(Mutex::new(0)),
            close_prompt_open: Arc::new(Mutex::new(false)),
        }
    }
}

#[tauri::command]
fn get_meta(app_handle: tauri::AppHandle) -> MetaInfo {
    MetaInfo {
        product_name: PRODUCT_NAME.to_string(),
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
fn save_config_command(config: AppConfig) -> Result<(), String> {
    save_config(&config).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_ui_language(language: String) -> Result<(), String> {
    let mut config = load_config().map_err(|e| e.to_string())?;
    config.ui.language = language;
    save_config(&config).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_close_behavior(behavior: String) -> Result<(), String> {
    let mut config = load_config().map_err(|e| e.to_string())?;
    config.ui.close_behavior = behavior;
    save_config(&config).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_autostart(app: tauri::AppHandle) -> AutostartStatus {
    let cfg = load_config().unwrap_or_default();
    let autostart = cfg.ui.autostart;
    let mut system: Option<SystemAutostartStatus> = None;

    // Use tauri_plugin_autostart API via AppHandleExt
    match app.autostart() {
        Ok(autostart_plugin) => {
            match autostart_plugin.is_enabled() {
                Ok(enabled) => {
                    system = Some(SystemAutostartStatus { open_at_login: enabled });
                }
                Err(_) => {
                    // Error getting status, system will be None
                }
            }
        }
        Err(_) => {
            // Plugin not available
        }
    }

    AutostartStatus {
        platform: std::env::consts::OS.to_string(),
        system,
        ok: true,
        error: None,
    }
}

#[tauri::command]
fn set_autostart(enabled: bool, app: tauri::AppHandle) -> Result<AutostartStatus, String> {
    let mut system: Option<SystemAutostartStatus> = None;
    let mut error: Option<String> = None;

    // Try to set autostart using tauri_plugin_autostart via AppHandleExt
    match app.autostart() {
        Ok(autostart_plugin) => {
            let result = if enabled {
                autostart_plugin.enable()
            } else {
                autostart_plugin.disable()
            };

            if let Err(e) = result {
                error = Some(e.to_string());
            }

            // Get the updated status
            match autostart_plugin.is_enabled() {
                Ok(enabled_state) => {
                    system = Some(SystemAutostartStatus { open_at_login: enabled_state });
                }
                Err(_) => {
                    // Error getting status after setting
                }
            }
        }
        Err(_) => {
            error = Some("Autostart plugin not available".to_string());
        }
    }

    // Update config
    let mut config = load_config().map_err(|e| e.to_string())?;
    config.ui.autostart = enabled;
    save_config(&config).map_err(|e| e.to_string())?;

    Ok(AutostartStatus {
        platform: std::env::consts::OS.to_string(),
        system,
        ok: error.is_none(),
        error,
    })
}

/// 处理前端关闭弹窗响应
/// 根据用户选择执行：隐藏到托盘、退出应用、取消关闭
#[tauri::command]
fn respond_close_prompt(
    payload: ClosePromptResponsePayload,
    app: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    let id = payload.id.clone();
    let action = payload.action.clone();
    let remember = payload.remember;

    // 验证这是当前活跃的关闭请求
    let mut open_guard = state.close_prompt_open.lock().unwrap();
    if !*open_guard {
        return Ok(()); // 没有活跃的关闭请求，忽略
    }

    // 递增 epoch 以标记此关闭提示已处理
    let mut epoch_guard = state.close_prompt_epoch.lock().unwrap();
    *epoch_guard += 1;
    let new_epoch = *epoch_guard;
    drop(epoch_guard);

    // 发送 dismiss 事件通知前端
    let _ = app.emit("dismiss-close-prompt", serde_json::json!({ "epoch": new_epoch }));

    *open_guard = false;
    drop(open_guard);

    // 如果需要记住选择，更新配置
    if remember {
        let mut config = load_config().map_err(|e| e.to_string())?;
        match action.as_str() {
            "tray" => config.ui.close_behavior = "tray".to_string(),
            "exit" => config.ui.close_behavior = "exit".to_string(),
            _ => {}
        }
        save_config(&config).map_err(|e| e.to_string())?;
    }

    // 执行操作
    match action.as_str() {
        "tray" => {
            if let Some(window) = app.get_webview_window("main") {
                hide_to_tray(&window);
            }
        }
        "exit" => {
            let is_quitting = state.is_quitting.lock().unwrap();
            *is_quitting = true;
            drop(is_quitting);

            // 停止 watch
            let mut watch_guard = state.watch_stop.lock().unwrap();
            if let Some(stop) = watch_guard.take() {
                stop();
            }
            drop(watch_guard);

            app.exit(0);
        }
        _ => {
            // cancel 或其他，不做任何操作
        }
    }

    Ok(())
}

///  dismissing the close prompt without any action
#[tauri::command]
fn dismiss_close_prompt(app: tauri::AppHandle, state: State<AppState>) -> Result<(), String> {
    // 递增 epoch
    let mut epoch_guard = state.close_prompt_epoch.lock().unwrap();
    *epoch_guard += 1;
    let new_epoch = *epoch_guard;
    drop(epoch_guard);

    // 发送 dismiss 事件
    let _ = app.emit("dismiss-close-prompt", serde_json::json!({ "epoch": new_epoch }));

    // 重置 open 标志
    let mut open_guard = state.close_prompt_open.lock().unwrap();
    *open_guard = false;

    Ok(())
}

#[tauri::command]
async fn test_notify(payload: TestNotifyPayload) -> Result<serde_json::Value, String> {
    let duration_ms = payload.duration_minutes.map(|m| m as i64 * 60 * 1000);

    send_notifications(
        &payload.source,
        &payload.task_info,
        duration_ms,
        std::env::current_dir().unwrap_or_default().to_string_lossy().to_string(),
        true,
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn test_sound(payload: TestSoundPayload) -> Result<(), String> {
    notify::notify_sound(&payload.title, payload.sound.as_ref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn watch_status(state: State<AppState>) -> WatchStatus {
    let guard = state.watch_stop.lock().unwrap();
    WatchStatus {
        running: guard.is_some(),
    }
}

#[tauri::command]
fn watch_start(
    payload: WatchStartPayload,
    state: State<AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let mut guard = state.watch_stop.lock().unwrap();

    if guard.is_some() {
        return Ok(());
    }

    let window = app_handle.get_webview_window("main").ok_or("Main window not found")?;

    let stop = start_watch(
        &payload.sources,
        payload.interval_ms,
        payload.gemini_quiet_ms,
        payload.claude_quiet_ms,
        move |line: String| {
            let _ = window.emit("watch-log", line);
        },
    )
    .map_err(|e| e.to_string())?;

    *guard = Some(stop);

    Ok(())
}

#[tauri::command]
fn watch_stop(state: State<AppState>) -> Result<(), String> {
    let mut guard = state.watch_stop.lock().unwrap();

    if let Some(stop) = guard.take() {
        stop();
    }

    Ok(())
}

#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .args(["/select,", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn open_watch_log() -> Result<String, String> {
    let log_dir = get_data_dir().join("watch-logs");
    fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let log_path = log_dir.join(format!("watch-{}.log", today));

    if !log_path.exists() {
        fs::write(&log_path, "").map_err(|e| e.to_string())?;
    }

    open_path(log_path.to_string_lossy().to_string())?;
    Ok(log_path.to_string_lossy().to_string())
}

fn setup_tray(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let open_i = MenuItem::with_id(app, "open", "打开", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_i, &quit_i])?;

    let _tray = TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .menu_on_left_click(true)
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
                "quit" => {
                    app.exit(0);
                }
                _ => {}
            }
        })
        .build(app)?;

    Ok(())
}

fn show_main_window(window: &tauri::WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.set_skip_taskbar(false);
}

fn hide_to_tray(window: &tauri::WebviewWindow) {
    let _ = window.hide();
    let _ = window.set_skip_taskbar(true);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--silent"]),
        ))
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = app.get_webview_window("main")
                .expect("no main window")
                .set_focus();
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_os::init())
        .manage(AppState::default())
        .setup(|app| {
            setup_tray(app.handle())?;

            let window = app.get_webview_window("main").unwrap();

            let config = load_config().unwrap_or_default();

            let silent_start = config.ui.silent_start;
            let close_behavior = config.ui.close_behavior.clone();

            if !silent_start {
                show_main_window(&window);
            } else {
                hide_to_tray(&window);
            }

            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(mut state) = app_handle.state::<AppState>().watch_stop.lock() {
                    if state.is_none() {
                        if let Ok(stop) = start_watch(
                            "all",
                            1000,
                            3000,
                            60000,
                            move |line: String| {
                                let _ = app_handle.emit("watch-log", line);
                            },
                        ) {
                            *state = Some(stop);
                        }
                    }
                }
            });

            let close_behavior_clone = close_behavior.clone();
            let window_clone = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    match close_behavior_clone.as_str() {
                        "tray" => {
                            api.prevent_close();
                            hide_to_tray(&window_clone);
                        }
                        "exit" => {
                            // Allow close
                        }
                        "ask" | _ => {
                            api.prevent_close();
                            // Emit close-prompt event to frontend
                            let app_handle = window_clone.app_handle();
                            let state = app_handle.state::<AppState>();

                            // Check if already open
                            {
                                let open_guard = state.close_prompt_open.lock().unwrap();
                                if *open_guard {
                                    return; // Already showing prompt
                                }
                            }

                            // Set open flag
                            {
                                let mut open_guard = state.close_prompt_open.lock().unwrap();
                                *open_guard = true;
                            }

                            // Increment sequence and get epoch
                            let mut seq_guard = state.close_prompt_seq.lock().unwrap();
                            *seq_guard += 1;
                            let id = format!("close-{}", *seq_guard);
                            drop(seq_guard);

                            let mut epoch_guard = state.close_prompt_epoch.lock().unwrap();
                            let epoch = *epoch_guard;
                            drop(epoch_guard);

                            let payload = ClosePromptPayload { id, epoch };
                            let _ = app_handle.emit("close-prompt", payload);
                        }
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_meta,
            get_config,
            save_config_command,
            set_ui_language,
            set_close_behavior,
            get_autostart,
            set_autostart,
            respond_close_prompt,
            dismiss_close_prompt,
            test_notify,
            test_sound,
            watch_status,
            watch_start,
            watch_stop,
            open_path,
            open_watch_log,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn main() {
    run();
}
