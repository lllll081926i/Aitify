use serde_json::json;
use tauri_plugin_notification::NotificationExt;

use crate::config::{AppConfig, load_config, SoundConfig, TelegramConfig};

pub async fn send_notifications(
    source: &str,
    task_info: &str,
    duration_ms: Option<i64>,
    cwd: String,
    force: bool,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let config = load_config()?;

    let results = json!([
        send_telegram(&config, source, task_info, &duration_ms).await,
        send_desktop(&config, source, task_info, &duration_ms).await,
        send_sound(&config, task_info).await,
    ]);

    Ok(json!({
        "skipped": false,
        "reason": null,
        "results": results
    }))
}

async fn send_telegram(
    config: &AppConfig,
    source: &str,
    task_info: &str,
    duration_ms: &Option<i64>,
) -> serde_json::Value {
    if !config.channels.telegram.enabled {
        return json!({ "channel": "telegram", "ok": false, "error": "disabled" });
    }

    let source_config = match source {
        "claude" => &config.sources.claude,
        "codex" => &config.sources.codex,
        "gemini" => &config.sources.gemini,
        _ => &config.sources.claude,
    };

    if !source_config.enabled || !source_config.channels.telegram {
        return json!({ "channel": "telegram", "ok": false, "error": "source disabled" });
    }

    let bot_token = &config.channels.telegram.bot_token;
    let chat_id = &config.channels.telegram.chat_id;

    if bot_token.is_empty() || chat_id.is_empty() {
        return json!({ "channel": "telegram", "ok": false, "error": "missing credentials" });
    }

    let duration_text = duration_ms.map(|ms| {
        let minutes = ms / 60000;
        let seconds = (ms % 60000) / 1000;
        if minutes > 0 {
            format!("{}分{}秒", minutes, seconds)
        } else {
            format!("{}秒", seconds)
        }
    });

    let message = if let Some(dur) = duration_text {
        format!("{}\n耗时: {}", task_info, dur)
    } else {
        task_info.to_string()
    };

    let url = format!("https://api.telegram.org/bot{}/sendMessage", bot_token);
    let body = json!({
        "chat_id": chat_id,
        "text": message,
        "parse_mode": "HTML"
    });

    match reqwest::Client::new()
        .post(&url)
        .json(&body)
        .send()
        .await
    {
        Ok(resp) => {
            if resp.status().is_success() {
                json!({ "channel": "telegram", "ok": true })
            } else {
                json!({
                    "channel": "telegram",
                    "ok": false,
                    "error": format!("HTTP {}", resp.status())
                })
            }
        }
        Err(e) => {
            json!({ "channel": "telegram", "ok": false, "error": e.to_string() })
        }
    }
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
        use winrt_notification::{Duration, Sound, Toast};

        let duration_text = duration_ms.map(|ms| {
            let minutes = ms / 60000;
            let seconds = (ms % 60000) / 1000;
            if minutes > 0 {
                format!("耗时: {}分{}秒", minutes, seconds)
            } else {
                format!("耗时: {}秒", seconds)
            }
        });

        let toast = Toast::new(Toast::POWERSHELL_APP_ID)
            .title(task_info)
            .text1(&duration_text.unwrap_or_else(|| "任务完成".to_string()));

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

async fn send_sound(
    config: &AppConfig,
    task_info: &str,
) -> serde_json::Value {
    if !config.channels.sound.enabled {
        return json!({ "channel": "sound", "ok": false, "error": "disabled" });
    }

    match notify_sound(task_info, Some(&config.channels.sound)).await {
        Ok(_) => json!({ "channel": "sound", "ok": true }),
        Err(e) => json!({ "channel": "sound", "ok": false, "error": e.to_string() }),
    }
}

pub async fn notify_sound(
    title: &str,
    sound_config: Option<&SoundConfig>,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = sound_config.map(|s| s.clone()).unwrap_or_else(|| {
        load_config().map(|c| c.channels.sound).unwrap_or_default()
    });

    if !config.enabled {
        return Ok(());
    }

    if config.use_custom && !config.custom_path.is_empty() {
        // Play custom sound
        #[cfg(target_os = "windows")]
        {
            use std::process::Command;
            Command::new("powershell")
                .args([
                    "-c",
                    &format!("(New-Object Media.SoundPlayer '{}').PlaySync()", config.custom_path)
                ])
                .spawn()?;
        }
    } else if config.tts {
        // Text-to-speech
        #[cfg(target_os = "windows")]
        {
            use std::process::Command;
            let script = format!(
                "Add-Type -AssemblyName System.Speech; $synth = New-Object System.Speech.Synthesis.SpeechSynthesizer; $synth.Speak('{}');",
                title.replace("'", "''")
            );
            Command::new("powershell")
                .args(["-c", &script])
                .spawn()?;
        }

        #[cfg(target_os = "macos")]
        {
            use std::process::Command;
            Command::new("say")
                .arg(title)
                .spawn()?;
        }

        #[cfg(target_os = "linux")]
        {
            use std::process::Command;
            Command::new("espeak")
                .arg(title)
                .spawn()
                .or_else(|_| {
                    Command::new("spd-say")
                        .arg(title)
                        .spawn()
                })?;
        }
    }

    Ok(())
}
