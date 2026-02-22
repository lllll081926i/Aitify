use serde_json::json;
use crate::config::{AppConfig, load_config};

const APP_ID: &str = "Aitify.Notify";

#[cfg(target_os = "windows")]
fn register_app_id() {
    use windows_registry::*;

    let hkcu = CURRENT_USER;
    let path = r"Software\Classes\AppUserModelId\Aitify.Notify";

    if let Ok(key) = hkcu.create(path) {
        let _ = key.set_string("DisplayName", "Aitify");
    }
}

pub async fn send_notifications(
    source: &str,
    task_info: &str,
    duration_ms: Option<i64>,
    _cwd: String,
    _force: bool,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let config = load_config()?;
    let result = send_desktop(&config, source, task_info, &duration_ms).await;

    Ok(json!({
        "skipped": false,
        "reason": null,
        "results": [result]
    }))
}

async fn send_desktop(
    config: &AppConfig,
    source: &str,
    task_info: &str,
    duration_ms: &Option<i64>,
) -> serde_json::Value {
    if !config.channels.desktop.enabled {
        return json!({ "channel": "desktop", "ok": false, "error": "disabled" });
    }

    let source_config = match source {
        "claude" => &config.sources.claude,
        "codex" => &config.sources.codex,
        "gemini" => &config.sources.gemini,
        _ => &config.sources.claude,
    };

    if !source_config.enabled || !source_config.channels.desktop {
        return json!({ "channel": "desktop", "ok": false, "error": "source disabled" });
    }

    #[cfg(target_os = "windows")]
    {
        use winrt_notification::Toast;

        register_app_id();

        let duration_text = duration_ms.map(|ms| {
            let minutes = ms / 60000;
            let seconds = (ms % 60000) / 1000;
            if minutes > 0 {
                format!("{}分{}秒", minutes, seconds)
            } else {
                format!("{}秒", seconds)
            }
        });

        let title = format!("{} 任务完成", source.to_uppercase());
        let content = if let Some(dur) = duration_text {
            format!("{} · 耗时 {}", task_info, dur)
        } else {
            task_info.to_string()
        };

        let toast = Toast::new(APP_ID)
            .title(&title)
            .text1(&content);

        match toast.show() {
            Ok(_) => json!({ "channel": "desktop", "ok": true }),
            Err(e) => json!({ "channel": "desktop", "ok": false, "error": e.to_string() }),
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        json!({ "channel": "desktop", "ok": false, "error": "not supported on this platform" })
    }
}
