// ============ Pi Watch ============

struct PiSessionState {
    processed_offset: u64,
    last_user_at: Option<i64>,
    last_assistant_at: Option<i64>,
    last_notified_assistant_at: Option<i64>,
    last_agent_content: Option<String>,
    last_cwd: Option<String>,
    confirm_notified_for_turn: bool,
}

impl PiSessionState {
    fn new() -> Self {
        Self {
            processed_offset: 0,
            last_user_at: None,
            last_assistant_at: None,
            last_notified_assistant_at: None,
            last_agent_content: None,
            last_cwd: None,
            confirm_notified_for_turn: false,
        }
    }
}

fn is_pi_session_file(_full_path: &Path, name: &str) -> bool {
    name.to_lowercase().ends_with(".jsonl")
}

fn extract_pi_message_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => {
            let mut texts = Vec::new();
            for part in parts {
                if part.get("type").and_then(|v| v.as_str()) == Some("text") {
                    if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                        if !text.trim().is_empty() {
                            texts.push(text.to_string());
                        }
                    }
                }
            }
            texts.join("\n")
        }
        Some(other) => extract_text_from_any(other),
        None => String::new(),
    }
}

fn process_pi_object(obj: &Value, _seed: bool, state: &mut PiSessionState) {
    let entry_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");

    if entry_type == "session" {
        if let Some(cwd) = obj.get("cwd").and_then(|v| v.as_str()) {
            if !cwd.trim().is_empty() {
                state.last_cwd = Some(cwd.to_string());
            }
        }
        return;
    }

    if entry_type != "message" {
        return;
    }

    let message = match obj.get("message") {
        Some(m) => m,
        None => return,
    };

    let ts = obj
        .get("timestamp")
        .and_then(parse_timestamp)
        .or_else(|| message.get("timestamp").and_then(parse_timestamp))
        .or_else(|| Some(now_unix_millis_i64()));

    match message.get("role").and_then(|v| v.as_str()) {
        Some("user") => {
            state.last_user_at = ts;
            state.confirm_notified_for_turn = false;
            state.last_agent_content = None;
        }
        Some("assistant") => {
            // Only terminal assistant replies count as turn completion.
            // Intermediate tool loops use stopReason = "toolUse".
            let stop_reason = message.get("stopReason").and_then(|v| v.as_str());
            if stop_reason != Some("stop") {
                return;
            }

            state.last_assistant_at = ts;
            let content = extract_pi_message_text(message);
            if !content.trim().is_empty() {
                state.last_agent_content = Some(compact_state_text(&content));
            }
        }
        _ => {}
    }
}
