// ============ Gemini Watch ============

struct GeminiState {
    current_file: Option<PathBuf>,
    current_mtime_ms: u64,
    last_count: usize,
    last_user_at: Option<i64>,
    last_gemini_at: Option<i64>,
    last_notified_gemini_at: Option<i64>,
    confirm_notified_for_turn: bool,
    // cancel flag for debounced notify timer
    pending_cancel: Option<Arc<AtomicBool>>,
}

impl GeminiState {
    fn new() -> Self {
        Self {
            current_file: None,
            current_mtime_ms: 0,
            last_count: 0,
            last_user_at: None,
            last_gemini_at: None,
            last_notified_gemini_at: None,
            confirm_notified_for_turn: false,
            pending_cancel: None,
        }
    }

    fn cancel_pending(&mut self) {
        if let Some(flag) = self.pending_cancel.take() {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

#[derive(Deserialize)]
struct GeminiMessagesEnvelope<'a> {
    #[serde(borrow)]
    messages: Vec<&'a RawValue>,
}

fn collect_gemini_message_jsons<'a>(content: &'a str, skip: usize) -> Option<(Vec<&'a str>, usize)> {
    let envelope: GeminiMessagesEnvelope<'a> = serde_json::from_str(content).ok()?;
    let total_count = envelope.messages.len();
    let new_items = envelope
        .messages
        .into_iter()
        .skip(skip)
        .map(|raw| raw.get())
        .collect();

    Some((new_items, total_count))
}

fn process_gemini_messages_from_content(
    content: &str,
    skip: usize,
    state: &mut GeminiState,
    quiet_ms: u64,
) -> Option<usize> {
    let (new_items, total_count) = collect_gemini_message_jsons(content, skip)?;

    for raw in new_items {
        let msg = serde_json::from_str::<Value>(raw).ok()?;
        process_gemini_message(&msg, state, quiet_ms);
    }

    Some(total_count)
}

// ============ Qwen Watch ============

struct QwenSessionState {
    processed_offset: u64,
    last_user_at: Option<i64>,
    last_assistant_at: Option<i64>,
    last_notified_assistant_at: Option<i64>,
    last_agent_content: Option<String>,
    last_cwd: Option<String>,
    confirm_notified_for_turn: bool,
}

impl QwenSessionState {
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

fn is_qwen_chat_file(full_path: &Path, name: &str) -> bool {
    if !name.to_lowercase().ends_with(".jsonl") {
        return false;
    }

    full_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case("chats"))
        .unwrap_or(false)
}

fn process_qwen_object(obj: &Value, _seed: bool, state: &mut QwenSessionState) {
    let ts = obj
        .get("timestamp")
        .and_then(parse_timestamp)
        .or_else(|| Some(now_unix_millis_i64()));

    if let Some(cwd) = obj.get("cwd").and_then(|v| v.as_str()) {
        if !cwd.trim().is_empty() {
            state.last_cwd = Some(cwd.to_string());
        }
    }

    match obj.get("type").and_then(|v| v.as_str()) {
        Some("user") => {
            state.last_user_at = ts;
            state.confirm_notified_for_turn = false;
        }
        Some("assistant") => {
            state.last_assistant_at = ts;
            let content = obj
                .get("message")
                .map(extract_text_from_any)
                .unwrap_or_default();
            if !content.trim().is_empty() {
                state.last_agent_content = Some(compact_state_text(&content));
            }
        }
        _ => {}
    }
}

fn process_gemini_message(
    msg: &Value,
    state: &mut GeminiState,
    quiet_ms: u64,
) {
    let ts = msg.get("timestamp").and_then(parse_timestamp);
    let msg_type = msg.get("type").and_then(|v| v.as_str());

    match msg_type {
        Some("user") => {
            state.cancel_pending();
            state.last_user_at = ts;
            state.last_gemini_at = None;
            state.last_notified_gemini_at = None;
            state.confirm_notified_for_turn = false;
        }
        Some("gemini") => {
            state.last_gemini_at = ts;

            if state.confirm_notified_for_turn {
                state.cancel_pending();
                return;
            }

            // Schedule debounced notification
            state.cancel_pending();
            let cancel = Arc::new(AtomicBool::new(false));
            state.pending_cancel = Some(cancel.clone());
            let target_gemini_at = state.last_gemini_at;
            let user_at = state.last_user_at;
            let last_notified = state.last_notified_gemini_at;

            if last_notified == target_gemini_at {
                return;
            }

            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(quiet_ms)).await;
                if cancel.load(Ordering::Relaxed) { return; }
                let end_at = match target_gemini_at { Some(t) => t, None => return };
                let start_at = match user_at { Some(t) => t, None => return };
                let duration_ms = if end_at >= start_at { Some(end_at - start_at) } else { None };
                let _ = crate::notify::send_notifications("gemini", "Gemini 任务已完成", duration_ms, String::new(), false, Some("complete")).await;
            });
        }
        _ => {}
    }
}

