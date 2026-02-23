use serde_json::json;
use crate::config::{AppConfig, load_config};

const PRIMARY_APP_ID: &str = "com.aitify.desktop";
const LEGACY_APP_ID: &str = "Aitify.Notify";

#[cfg(target_os = "windows")]
fn register_app_id(app_id: &str) {
    use windows_registry::*;

    let hkcu = CURRENT_USER;
    let path = format!(r"Software\Classes\AppUserModelId\{}", app_id);

    if let Ok(key) = hkcu.create(&path) {
        let _ = key.set_string("DisplayName", "Aitify");
    }
}

pub async fn send_notifications(
    source: &str,
    task_info: &str,
    duration_ms: Option<i64>,
    _cwd: String,
    force: bool,
    notification_type: Option<&str>,
) -> Result<serde_json::Value, String> {
    let config = load_config().map_err(|e| e.to_string())?;
    let result = send_desktop(&config, source, task_info, &duration_ms, force, notification_type).await;
    let ok = result.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let error_text = result
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown notification error");

    // 配置禁用时保持非错误返回，避免监控流程把"禁用通知"视为异常。
    if !ok
        && error_text != "disabled"
        && error_text != "source disabled"
        && error_text != "below min duration"
    {
        return Err(error_text.to_string());
    }

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
    force: bool,
    notification_type: Option<&str>,
) -> serde_json::Value {
    let source_config = match source {
        "claude" => &config.sources.claude,
        "codex" => &config.sources.codex,
        "gemini" => &config.sources.gemini,
        _ => &config.sources.claude,
    };

    if !force && (!source_config.enabled || !source_config.channels.desktop) {
        return json!({ "channel": "desktop", "ok": false, "error": "source disabled" });
    }

    if !force {
        let min_minutes = source_config.min_duration_minutes.max(0) as i64;
        if min_minutes > 0 {
            let min_duration_ms = min_minutes * 60_000;
            let below_minimum = duration_ms.map(|ms| ms < min_duration_ms).unwrap_or(true);
            if below_minimum {
                return json!({ "channel": "desktop", "ok": false, "error": "below min duration" });
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        use winrt_notification::Toast;

        let duration_text = duration_ms.map(|ms| {
            let minutes = ms / 60000;
            let seconds = (ms % 60000) / 1000;
            if minutes > 0 {
                format!("{} 分{}秒", minutes, seconds)
            } else {
                format!("{} 秒", seconds)
            }
        });

        // 根据通知类型设置不同的标题
        let title = match notification_type {
            Some("confirm") => format!("{} 待确认", source.to_uppercase()),
            Some("complete") | None => format!("{} 任务完成", source.to_uppercase()),
            _ => format!("{} 任务完成", source.to_uppercase()),
        };

        let base_content = if task_info.trim().is_empty() {
            match notification_type {
                Some("confirm") => "需要你的确认".to_string(),
                _ => "任务已完成".to_string(),
            }
        } else {
            task_info.to_string()
        };

        let content = if let Some(dur) = duration_text {
            format!("{} · 耗时 {}", base_content, dur)
        } else {
            base_content
        };

        let mut errors = Vec::with_capacity(3);

        register_app_id(PRIMARY_APP_ID);
        register_app_id(LEGACY_APP_ID);

        for app_id in [PRIMARY_APP_ID, LEGACY_APP_ID, Toast::POWERSHELL_APP_ID] {
            let toast = Toast::new(app_id).title(&title).text1(&content);
            match toast.show() {
                Ok(_) => {
                    return json!({
                        "channel": "desktop",
                        "ok": true,
                        "app_id": app_id
                    })
                }
                Err(e) => {
                    errors.push(format!("{}: {}", app_id, e));
                }
            }
        }

        json!({
            "channel": "desktop",
            "ok": false,
            "error": errors.join(" | ")
        })
    }

    #[cfg(not(target_os = "windows"))]
    {
        json!({ "channel": "desktop", "ok": false, "error": "not supported on this platform" })
    }
}
