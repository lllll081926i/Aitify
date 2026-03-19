// ============ Claude Watch ============

struct ClaudeState {
    current_file: Option<PathBuf>,
    last_file_size: u64,
    last_user_at: Option<i64>,
    last_assistant_at: Option<i64>,
    last_notified_at: Option<i64>,
    notified_for_turn: bool,
    confirm_notified_for_turn: bool,
    last_cwd: Option<String>,
    last_assistant_had_tool_use: bool,
    pending_cancel: Option<Arc<AtomicBool>>,
}

impl ClaudeState {
    fn new() -> Self {
        Self {
            current_file: None,
            last_file_size: 0,
            last_user_at: None,
            last_assistant_at: None,
            last_notified_at: None,
            notified_for_turn: false,
            confirm_notified_for_turn: false,
            last_cwd: None,
            last_assistant_had_tool_use: false,
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
        self.last_notified_at = None;
        self.notified_for_turn = false;
        self.confirm_notified_for_turn = false;
        self.last_assistant_had_tool_use = false;
    }
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
            state.last_assistant_had_tool_use = false;
            state.last_user_at = ts;
        }
        Some("assistant") => {
            state.last_assistant_had_tool_use = has_tool_use_content(obj);
            state.last_assistant_at = ts.or_else(|| Some(now_unix_millis_i64()));

            if state.last_user_at.is_none() {
                state.last_user_at = state.last_assistant_at;
                state.notified_for_turn = false;
            }
        }
        Some(work_type) if is_claude_work_type(work_type) => {
            // work in progress — cancel any pending completion timer
            state.cancel_pending();
        }
        _ => {}
    }
}

