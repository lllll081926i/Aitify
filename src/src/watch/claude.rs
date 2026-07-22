// ============ Claude Watch ============

fn is_claude_agent_progress(obj: &Value) -> bool {
    if obj.get("type").and_then(|v| v.as_str()) != Some("progress") {
        return false;
    }

    let data = obj.get("data").unwrap_or(&Value::Null);
    data.get("type").and_then(|v| v.as_str()) == Some("agent_progress")
        || data.get("agentId").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false)
}

struct ClaudeState {
    current_file: Option<PathBuf>,
    last_file_size: u64,
    last_user_at: Option<i64>,
    last_assistant_at: Option<i64>,
    last_turn_end_at: Option<i64>,
    last_notified_at: Option<i64>,
    notified_for_turn: bool,
    confirm_notified_for_turn: bool,
    last_cwd: Option<String>,
    last_agent_content: Option<String>,
    last_assistant_had_tool_use: bool,
    has_active_subagent_progress: bool,
    pending_cancel: Option<Arc<AtomicBool>>,
}

impl ClaudeState {
    fn new() -> Self {
        Self {
            current_file: None,
            last_file_size: 0,
            last_user_at: None,
            last_assistant_at: None,
            last_turn_end_at: None,
            last_notified_at: None,
            notified_for_turn: false,
            confirm_notified_for_turn: false,
            last_cwd: None,
            last_agent_content: None,
            last_assistant_had_tool_use: false,
            has_active_subagent_progress: false,
            pending_cancel: None,
        }
    }

    fn cancel_pending(&mut self) {
        if let Some(flag) = self.pending_cancel.take() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    fn reset_for_new_file(&mut self) {
        self.cancel_pending();
        self.last_file_size = 0;
        self.last_user_at = None;
        self.last_assistant_at = None;
        self.last_turn_end_at = None;
        self.last_notified_at = None;
        self.notified_for_turn = false;
        self.confirm_notified_for_turn = false;
        self.last_agent_content = None;
        self.last_assistant_had_tool_use = false;
        self.has_active_subagent_progress = false;
    }
}

fn is_claude_turn_end_system(obj: &Value) -> bool {
    obj.get("type").and_then(|v| v.as_str()) == Some("system")
        && obj.get("subtype").and_then(|v| v.as_str()) == Some("turn_duration")
}

fn current_claude_completion(state: &ClaudeState) -> Option<(i64, i64, i64)> {
    let user_at = state.last_user_at?;
    let assistant_at = state.last_assistant_at?;
    let turn_end_at = state.last_turn_end_at?;
    if assistant_at < user_at || state.has_active_subagent_progress {
        return None;
    }

    if turn_end_at < assistant_at {
        return None;
    }

    Some((user_at, assistant_at, turn_end_at))
}

fn has_tool_use_content(obj: &Value) -> bool {
    let message = match obj.get("message") {
        Some(m) => m,
        None => return false,
    };
    if let Some(arr) = message.get("content").and_then(|c| c.as_array()) {
        return arr.iter().any(|item| {
            item.get("type").and_then(|t| t.as_str()) == Some("tool_use")
        });
    }
    false
}

fn process_claude_object(
    obj: &Value,
    _seed: bool,
    state: &mut ClaudeState,
) {
    if obj.get("isSidechain").and_then(|v| v.as_bool()) == Some(true) {
        return;
    }

    let ts = obj.get("timestamp").and_then(parse_timestamp);
    let record_type = obj.get("type").and_then(|v| v.as_str());

    if let Some(cwd) = obj.get("cwd").and_then(|v| v.as_str()) {
        state.last_cwd = Some(cwd.to_string());
    }

    match record_type {
        Some("user") => {
            state.cancel_pending();
            state.confirm_notified_for_turn = false;
            state.notified_for_turn = false;
            state.last_agent_content = None;
            state.last_assistant_had_tool_use = false;
            state.has_active_subagent_progress = false;
            state.last_turn_end_at = None;
            state.last_user_at = ts;
        }
        Some("assistant") => {
            state.last_assistant_had_tool_use = has_tool_use_content(obj);
            state.has_active_subagent_progress = false;
            let assistant_text = obj
                .get("message")
                .map(extract_text_from_any)
                .unwrap_or_default();
            if !assistant_text.trim().is_empty() {
                state.last_agent_content = Some(compact_state_text(&assistant_text));
            }
            state.last_assistant_at = ts.or_else(|| Some(now_unix_millis_i64()));

            if state.last_user_at.is_none() {
                state.last_user_at = state.last_assistant_at;
                state.notified_for_turn = false;
            }
        }
        Some(work_type) if is_claude_work_type(work_type) => {
            // work in progress — cancel any pending completion timer
            state.cancel_pending();
            if is_claude_agent_progress(obj) {
                state.has_active_subagent_progress = true;
            }
        }
        Some("system") if is_claude_turn_end_system(obj) => {
            state.last_turn_end_at = ts.or(state.last_assistant_at).or_else(|| Some(now_unix_millis_i64()));
        }
        _ => {}
    }
}

